//! Transaction pool implementation.

use crate::{FeeOrdering, MempoolError, MempoolResult};
use crate::{DEFAULT_MAX_SIZE, DEFAULT_MAX_TXS, DEFAULT_TX_EXPIRY_SECS};
use dashmap::DashMap;
use parking_lot::RwLock;
use std::collections::{BTreeSet, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info, instrument, warn};

/// Mempool configuration.
#[derive(Debug, Clone)]
pub struct MempoolConfig {
    /// Maximum total size in bytes.
    pub max_size: usize,
    /// Maximum number of transactions.
    pub max_transactions: usize,
    /// Transaction expiry time in seconds.
    pub tx_expiry_secs: u64,
    /// Minimum fee per byte.
    pub min_fee_per_byte: f64,
}

impl Default for MempoolConfig {
    fn default() -> Self {
        Self {
            max_size: DEFAULT_MAX_SIZE,
            max_transactions: DEFAULT_MAX_TXS,
            tx_expiry_secs: DEFAULT_TX_EXPIRY_SECS,
            min_fee_per_byte: 0.001, // 0.001 nanoERG per byte
        }
    }
}

/// Stored transaction in the mempool.
#[derive(Debug, Clone)]
pub struct PooledTransaction {
    /// Transaction ID.
    pub id: Vec<u8>,
    /// Serialized transaction bytes.
    pub bytes: Vec<u8>,
    /// Transaction fee.
    pub fee: u64,
    /// Input box IDs.
    pub inputs: Vec<Vec<u8>>,
    /// Arrival timestamp.
    pub arrival_time: u64,
}

/// Mempool statistics.
#[derive(Debug, Clone, Default)]
pub struct MempoolStats {
    /// Number of transactions.
    pub tx_count: usize,
    /// Total size in bytes.
    pub total_size: usize,
    /// Minimum fee per byte.
    pub min_fee_per_byte: f64,
    /// Maximum fee per byte.
    pub max_fee_per_byte: f64,
}

/// Transaction mempool.
pub struct Mempool {
    /// Configuration.
    config: MempoolConfig,
    /// Transactions by ID.
    transactions: DashMap<Vec<u8>, PooledTransaction>,
    /// Input box to transaction mapping (for double-spend detection).
    input_to_tx: DashMap<Vec<u8>, Vec<u8>>,
    /// Fee-ordered transaction set.
    fee_order: RwLock<BTreeSet<FeeOrdering>>,
    /// Current total size.
    total_size: RwLock<usize>,
}

impl Mempool {
    /// Create a new mempool with the given configuration.
    pub fn new(config: MempoolConfig) -> Self {
        Self {
            config,
            transactions: DashMap::new(),
            input_to_tx: DashMap::new(),
            fee_order: RwLock::new(BTreeSet::new()),
            total_size: RwLock::new(0),
        }
    }

    /// Create with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(MempoolConfig::default())
    }

    /// Add a transaction to the mempool.
    #[instrument(skip(self, tx), fields(tx_id = hex::encode(&tx.id)))]
    pub fn add(&self, tx: PooledTransaction) -> MempoolResult<()> {
        // Check if already exists
        if self.transactions.contains_key(&tx.id) {
            return Err(MempoolError::AlreadyExists(hex::encode(&tx.id)));
        }

        // Check size
        let tx_size = tx.bytes.len();
        if tx_size > self.config.max_size / 10 {
            return Err(MempoolError::TooLarge {
                size: tx_size,
                max: self.config.max_size / 10,
            });
        }

        // Check fee
        let fee_per_byte = tx.fee as f64 / tx_size as f64;
        if fee_per_byte < self.config.min_fee_per_byte {
            return Err(MempoolError::FeeTooLow {
                fee: tx.fee,
                min: (self.config.min_fee_per_byte * tx_size as f64) as u64,
            });
        }

        // Check for double spends
        for input in &tx.inputs {
            if let Some(existing) = self.input_to_tx.get(input) {
                return Err(MempoolError::DoubleSpend(hex::encode(input)));
            }
        }

        // Check capacity
        let current_count = self.transactions.len();
        if current_count >= self.config.max_transactions {
            // Try to evict lowest fee transaction
            self.evict_lowest_fee()?;
        }

        let current_size = *self.total_size.read();
        if current_size + tx_size > self.config.max_size {
            // Try to evict to make room
            self.evict_for_size(tx_size)?;
        }

        // Add to mempool
        let ordering = FeeOrdering::new(tx.id.clone(), tx.fee, tx_size, tx.arrival_time);

        // Add input mappings
        for input in &tx.inputs {
            self.input_to_tx.insert(input.clone(), tx.id.clone());
        }

        // Update fee order
        self.fee_order.write().insert(ordering);

        // Update size
        *self.total_size.write() += tx_size;

        // Store transaction
        self.transactions.insert(tx.id.clone(), tx);

        debug!(
            count = self.transactions.len(),
            "Transaction added to mempool"
        );
        Ok(())
    }

    /// Remove a transaction by ID.
    #[instrument(skip(self), fields(tx_id = hex::encode(tx_id)))]
    pub fn remove(&self, tx_id: &[u8]) -> MempoolResult<PooledTransaction> {
        let (_, tx) = self
            .transactions
            .remove(tx_id)
            .ok_or_else(|| MempoolError::NotFound(hex::encode(tx_id)))?;

        // Remove input mappings
        for input in &tx.inputs {
            self.input_to_tx.remove(input);
        }

        // Update fee order
        let ordering = FeeOrdering::new(tx.id.clone(), tx.fee, tx.bytes.len(), tx.arrival_time);
        self.fee_order.write().remove(&ordering);

        // Update size
        *self.total_size.write() -= tx.bytes.len();

        debug!(
            count = self.transactions.len(),
            "Transaction removed from mempool"
        );
        Ok(tx)
    }

    /// Get a transaction by ID.
    pub fn get(&self, tx_id: &[u8]) -> Option<PooledTransaction> {
        self.transactions.get(tx_id).map(|r| r.clone())
    }

    /// Check if a transaction exists.
    pub fn contains(&self, tx_id: &[u8]) -> bool {
        self.transactions.contains_key(tx_id)
    }

    /// Check if an input is already spent by a mempool transaction.
    pub fn is_input_spent(&self, input_id: &[u8]) -> bool {
        self.input_to_tx.contains_key(input_id)
    }

    /// Get the transaction that spends a given input.
    pub fn get_spending_tx(&self, input_id: &[u8]) -> Option<Vec<u8>> {
        self.input_to_tx.get(input_id).map(|r| r.clone())
    }

    /// Get transactions ordered by fee (highest first).
    pub fn get_by_fee(&self, limit: usize) -> Vec<PooledTransaction> {
        let order = self.fee_order.read();
        order
            .iter()
            .take(limit)
            .filter_map(|o| self.get(&o.tx_id))
            .collect()
    }

    /// Get all transaction IDs.
    pub fn get_all_ids(&self) -> Vec<Vec<u8>> {
        self.transactions.iter().map(|r| r.key().clone()).collect()
    }

    /// Get mempool statistics.
    pub fn stats(&self) -> MempoolStats {
        let order = self.fee_order.read();

        let (min_fpb, max_fpb) = if order.is_empty() {
            (0.0, 0.0)
        } else {
            let min = order.iter().last().map(|o| o.fee_per_byte()).unwrap_or(0.0);
            let max = order.iter().next().map(|o| o.fee_per_byte()).unwrap_or(0.0);
            (min, max)
        };

        MempoolStats {
            tx_count: self.transactions.len(),
            total_size: *self.total_size.read(),
            min_fee_per_byte: min_fpb,
            max_fee_per_byte: max_fpb,
        }
    }

    /// Clear all transactions.
    pub fn clear(&self) {
        self.transactions.clear();
        self.input_to_tx.clear();
        self.fee_order.write().clear();
        *self.total_size.write() = 0;
        info!("Mempool cleared");
    }

    /// Remove transactions that conflict with a new block.
    pub fn remove_confirmed(&self, tx_ids: &[Vec<u8>], spent_inputs: &[Vec<u8>]) {
        // Remove confirmed transactions
        for tx_id in tx_ids {
            let _ = self.remove(tx_id);
        }

        // Remove transactions that spend now-confirmed inputs
        let mut to_remove = Vec::new();
        for input in spent_inputs {
            if let Some(tx_id) = self.get_spending_tx(input) {
                to_remove.push(tx_id);
            }
        }

        for tx_id in to_remove {
            let _ = self.remove(&tx_id);
        }
    }

    /// Remove expired transactions.
    pub fn remove_expired(&self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let expiry_ms = self.config.tx_expiry_secs * 1000;

        let expired: Vec<_> = self
            .transactions
            .iter()
            .filter(|r| now - r.arrival_time > expiry_ms)
            .map(|r| r.key().clone())
            .collect();

        for tx_id in expired {
            let _ = self.remove(&tx_id);
        }
    }

    /// Evict lowest fee transaction.
    fn evict_lowest_fee(&self) -> MempoolResult<()> {
        let lowest = {
            let order = self.fee_order.read();
            order.iter().last().map(|o| o.tx_id.clone())
        };

        if let Some(tx_id) = lowest {
            self.remove(&tx_id)?;
            warn!("Evicted lowest fee transaction");
        }

        Ok(())
    }

    /// Evict transactions to make room for given size.
    fn evict_for_size(&self, needed: usize) -> MempoolResult<()> {
        let mut freed = 0usize;

        while freed < needed {
            let lowest = {
                let order = self.fee_order.read();
                order.iter().last().map(|o| o.tx_id.clone())
            };

            if let Some(tx_id) = lowest {
                if let Ok(tx) = self.remove(&tx_id) {
                    freed += tx.bytes.len();
                }
            } else {
                break;
            }
        }

        Ok(())
    }
}

impl Default for Mempool {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_tx(id: u8, fee: u64, size: usize) -> PooledTransaction {
        PooledTransaction {
            id: vec![id],
            bytes: vec![0; size],
            fee,
            inputs: vec![vec![id, 1]],
            arrival_time: 1000,
        }
    }

    #[test]
    fn test_add_and_get() {
        let pool = Mempool::with_defaults();
        let tx = create_test_tx(1, 1000, 100);

        pool.add(tx.clone()).unwrap();

        assert!(pool.contains(&[1]));
        assert_eq!(pool.get(&[1]).unwrap().fee, 1000);
    }

    #[test]
    fn test_double_spend_detection() {
        let pool = Mempool::with_defaults();

        let tx1 = PooledTransaction {
            id: vec![1],
            bytes: vec![0; 100],
            fee: 1000,
            inputs: vec![vec![10, 10]],
            arrival_time: 1000,
        };

        let tx2 = PooledTransaction {
            id: vec![2],
            bytes: vec![0; 100],
            fee: 2000,
            inputs: vec![vec![10, 10]], // Same input!
            arrival_time: 1001,
        };

        pool.add(tx1).unwrap();

        let result = pool.add(tx2);
        assert!(matches!(result, Err(MempoolError::DoubleSpend(_))));
    }

    #[test]
    fn test_fee_ordering() {
        let pool = Mempool::with_defaults();

        pool.add(create_test_tx(1, 100, 100)).unwrap(); // 1 per byte
        pool.add(create_test_tx(2, 300, 100)).unwrap(); // 3 per byte
        pool.add(create_test_tx(3, 200, 100)).unwrap(); // 2 per byte

        let ordered = pool.get_by_fee(10);

        assert_eq!(ordered[0].id, vec![2]); // Highest fee first
        assert_eq!(ordered[1].id, vec![3]);
        assert_eq!(ordered[2].id, vec![1]);
    }

    // ============ Transaction Removal Tests ============
    // Corresponds to Scala's mempool remove operations

    #[test]
    fn test_remove_transaction() {
        let pool = Mempool::with_defaults();
        let tx = create_test_tx(1, 1000, 100);

        pool.add(tx).unwrap();
        assert!(pool.contains(&[1]));

        let removed = pool.remove(&[1]).unwrap();
        assert_eq!(removed.id, vec![1]);
        assert!(!pool.contains(&[1]));
    }

    #[test]
    fn test_remove_nonexistent() {
        let pool = Mempool::with_defaults();
        let result = pool.remove(&[99]);
        assert!(matches!(result, Err(MempoolError::NotFound(_))));
    }

    #[test]
    fn test_remove_frees_inputs() {
        let pool = Mempool::with_defaults();

        let tx1 = PooledTransaction {
            id: vec![1],
            bytes: vec![0; 100],
            fee: 1000,
            inputs: vec![vec![10, 10]],
            arrival_time: 1000,
        };

        pool.add(tx1).unwrap();
        assert!(pool.is_input_spent(&[10, 10]));

        pool.remove(&[1]).unwrap();
        assert!(!pool.is_input_spent(&[10, 10]));

        // Now we can add a tx spending the same input
        let tx2 = PooledTransaction {
            id: vec![2],
            bytes: vec![0; 100],
            fee: 2000,
            inputs: vec![vec![10, 10]],
            arrival_time: 1001,
        };

        assert!(pool.add(tx2).is_ok());
    }

    // ============ Pool Overflow Tests ============
    // Corresponds to Scala's "pool overflow" tests

    #[test]
    fn test_max_transactions_limit() {
        let config = MempoolConfig {
            max_transactions: 3,
            ..Default::default()
        };
        let pool = Mempool::new(config);

        pool.add(create_test_tx(1, 100, 100)).unwrap();
        pool.add(create_test_tx(2, 200, 100)).unwrap();
        pool.add(create_test_tx(3, 300, 100)).unwrap();

        // Pool is full, adding should evict lowest fee tx
        pool.add(create_test_tx(4, 400, 100)).unwrap();

        assert!(!pool.contains(&[1])); // Lowest fee evicted
        assert!(pool.contains(&[2]));
        assert!(pool.contains(&[3]));
        assert!(pool.contains(&[4]));
    }

    #[test]
    fn test_max_size_limit() {
        // Individual tx can't exceed max_size/10, so we need max_size >= 10 * tx_size
        // For 30-byte txs, max_size must be >= 300
        let config = MempoolConfig {
            max_size: 350,         // Room for ~11 x 30 byte txs
            max_transactions: 100, // High limit so size is the constraint
            ..Default::default()
        };
        let pool = Mempool::new(config);

        // Use 30-byte txs (which fit in max_size/10 = 35)
        pool.add(create_test_tx(1, 100, 30)).unwrap();
        pool.add(create_test_tx(2, 200, 30)).unwrap();
        pool.add(create_test_tx(3, 300, 30)).unwrap();
        pool.add(create_test_tx(4, 400, 30)).unwrap();
        pool.add(create_test_tx(5, 500, 30)).unwrap();
        pool.add(create_test_tx(6, 600, 30)).unwrap();
        pool.add(create_test_tx(7, 700, 30)).unwrap();
        pool.add(create_test_tx(8, 800, 30)).unwrap();
        pool.add(create_test_tx(9, 900, 30)).unwrap();
        pool.add(create_test_tx(10, 1000, 30)).unwrap();
        pool.add(create_test_tx(11, 1100, 30)).unwrap();

        // 11 * 30 = 330 bytes, adding another 30-byte tx would exceed 350
        // Should evict lowest fee tx
        pool.add(create_test_tx(12, 1200, 30)).unwrap();

        // Lowest fee (tx 1) should be evicted to make room
        assert!(!pool.contains(&[1]));
        assert!(pool.contains(&[12]));
    }

    // ============ Stats Tests ============

    #[test]
    fn test_stats_tracking() {
        let pool = Mempool::with_defaults();

        let stats = pool.stats();
        assert_eq!(stats.tx_count, 0);
        assert_eq!(stats.total_size, 0);

        pool.add(create_test_tx(1, 1000, 100)).unwrap();
        pool.add(create_test_tx(2, 2000, 200)).unwrap();

        let stats = pool.stats();
        assert_eq!(stats.tx_count, 2);
        assert_eq!(stats.total_size, 300);
    }

    // ============ Clear Tests ============

    #[test]
    fn test_clear() {
        let pool = Mempool::with_defaults();

        pool.add(create_test_tx(1, 1000, 100)).unwrap();
        pool.add(create_test_tx(2, 2000, 100)).unwrap();
        pool.add(create_test_tx(3, 3000, 100)).unwrap();

        assert_eq!(pool.stats().tx_count, 3);

        pool.clear();

        assert_eq!(pool.stats().tx_count, 0);
        assert_eq!(pool.stats().total_size, 0);
        assert!(!pool.contains(&[1]));
        assert!(!pool.contains(&[2]));
        assert!(!pool.contains(&[3]));
    }

    // ============ Remove Confirmed Tests ============
    // When a block is added, confirmed txs are removed

    #[test]
    fn test_remove_confirmed() {
        let pool = Mempool::with_defaults();

        pool.add(create_test_tx(1, 1000, 100)).unwrap();
        pool.add(create_test_tx(2, 2000, 100)).unwrap();
        pool.add(create_test_tx(3, 3000, 100)).unwrap();

        // Tx 1 and 2 are confirmed
        let confirmed_ids = vec![vec![1u8], vec![2u8]];
        let spent_inputs: Vec<Vec<u8>> = vec![];

        pool.remove_confirmed(&confirmed_ids, &spent_inputs);

        assert!(!pool.contains(&[1]));
        assert!(!pool.contains(&[2]));
        assert!(pool.contains(&[3])); // Still in pool
    }

    #[test]
    fn test_remove_confirmed_by_spent_inputs() {
        let pool = Mempool::with_defaults();

        let tx1 = PooledTransaction {
            id: vec![1],
            bytes: vec![0; 100],
            fee: 1000,
            inputs: vec![vec![10, 10]],
            arrival_time: 1000,
        };

        let tx2 = PooledTransaction {
            id: vec![2],
            bytes: vec![0; 100],
            fee: 2000,
            inputs: vec![vec![20, 20]],
            arrival_time: 1001,
        };

        pool.add(tx1).unwrap();
        pool.add(tx2).unwrap();

        // Input [10, 10] was spent in a block (different tx)
        let confirmed_ids: Vec<Vec<u8>> = vec![];
        let spent_inputs = vec![vec![10u8, 10u8]];

        pool.remove_confirmed(&confirmed_ids, &spent_inputs);

        // tx1 should be removed because its input was spent
        assert!(!pool.contains(&[1]));
        assert!(pool.contains(&[2])); // Still in pool
    }

    // ============ Input Tracking Tests ============

    #[test]
    fn test_is_input_spent() {
        let pool = Mempool::with_defaults();

        let tx = PooledTransaction {
            id: vec![1],
            bytes: vec![0; 100],
            fee: 1000,
            inputs: vec![vec![10, 10], vec![20, 20]],
            arrival_time: 1000,
        };

        pool.add(tx).unwrap();

        assert!(pool.is_input_spent(&[10, 10]));
        assert!(pool.is_input_spent(&[20, 20]));
        assert!(!pool.is_input_spent(&[30, 30]));
    }

    #[test]
    fn test_get_spending_tx() {
        let pool = Mempool::with_defaults();

        let tx = PooledTransaction {
            id: vec![1],
            bytes: vec![0; 100],
            fee: 1000,
            inputs: vec![vec![10, 10]],
            arrival_time: 1000,
        };

        pool.add(tx).unwrap();

        let spending_tx = pool.get_spending_tx(&[10, 10]);
        assert_eq!(spending_tx, Some(vec![1]));

        let spending_tx = pool.get_spending_tx(&[99, 99]);
        assert_eq!(spending_tx, None);
    }

    // ============ Get All IDs Test ============

    #[test]
    fn test_get_all_ids() {
        let pool = Mempool::with_defaults();

        pool.add(create_test_tx(1, 1000, 100)).unwrap();
        pool.add(create_test_tx(2, 2000, 100)).unwrap();
        pool.add(create_test_tx(3, 3000, 100)).unwrap();

        let all_ids = pool.get_all_ids();
        assert_eq!(all_ids.len(), 3);
        assert!(all_ids.contains(&vec![1]));
        assert!(all_ids.contains(&vec![2]));
        assert!(all_ids.contains(&vec![3]));
    }

    // ============ Fee Per Byte Limit Test ============

    #[test]
    fn test_get_by_fee_limit() {
        let pool = Mempool::with_defaults();

        pool.add(create_test_tx(1, 100, 100)).unwrap();
        pool.add(create_test_tx(2, 200, 100)).unwrap();
        pool.add(create_test_tx(3, 300, 100)).unwrap();
        pool.add(create_test_tx(4, 400, 100)).unwrap();
        pool.add(create_test_tx(5, 500, 100)).unwrap();

        // Only get top 3
        let ordered = pool.get_by_fee(3);
        assert_eq!(ordered.len(), 3);
        assert_eq!(ordered[0].id, vec![5]); // Highest
        assert_eq!(ordered[1].id, vec![4]);
        assert_eq!(ordered[2].id, vec![3]);
    }

    // ============ Multiple Inputs Test ============

    #[test]
    fn test_multiple_inputs_double_spend() {
        let pool = Mempool::with_defaults();

        let tx1 = PooledTransaction {
            id: vec![1],
            bytes: vec![0; 100],
            fee: 1000,
            inputs: vec![vec![10], vec![20], vec![30]],
            arrival_time: 1000,
        };

        pool.add(tx1).unwrap();

        // Try to add tx that spends one of the same inputs
        let tx2 = PooledTransaction {
            id: vec![2],
            bytes: vec![0; 100],
            fee: 2000,
            inputs: vec![vec![20]], // Conflicts with tx1
            arrival_time: 1001,
        };

        let result = pool.add(tx2);
        assert!(matches!(result, Err(MempoolError::DoubleSpend(_))));
    }
}
