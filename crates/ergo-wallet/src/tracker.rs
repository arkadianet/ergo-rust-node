//! Box (UTXO) tracking for wallet.

use crate::WalletResult;
use ergo_state::StateManager;
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tracing::debug;

/// A wallet-tracked box.
#[derive(Debug, Clone)]
pub struct WalletBox {
    /// Box ID.
    pub box_id: Vec<u8>,
    /// ERG value in nanoERG.
    pub value: u64,
    /// Token amounts.
    pub tokens: Vec<(Vec<u8>, u64)>,
    /// Creation height.
    pub creation_height: u32,
    /// Associated address (as string).
    pub address: String,
    /// Is spent in mempool.
    pub pending_spent: bool,
}

/// Wallet balance summary.
#[derive(Debug, Clone, Default)]
pub struct WalletBalance {
    /// Total ERG balance in nanoERG.
    pub total_erg: u64,
    /// Confirmed ERG.
    pub confirmed_erg: u64,
    /// Pending (unconfirmed) ERG.
    pub pending_erg: u64,
    /// Token balances (token_id -> amount).
    pub tokens: HashMap<Vec<u8>, u64>,
}

/// Box tracker for wallet addresses.
pub struct BoxTracker {
    /// State manager.
    state: Arc<StateManager>,
    /// Tracked addresses (as strings, e.g., base58 encoded).
    addresses: RwLock<HashSet<String>>,
    /// Known boxes by address string.
    boxes: RwLock<HashMap<String, Vec<WalletBox>>>,
    /// Last scanned height.
    scanned_height: RwLock<u32>,
}

impl BoxTracker {
    /// Create a new box tracker.
    pub fn new(state: Arc<StateManager>) -> Self {
        Self {
            state,
            addresses: RwLock::new(HashSet::new()),
            boxes: RwLock::new(HashMap::new()),
            scanned_height: RwLock::new(0),
        }
    }

    /// Add an address to track (raw bytes).
    pub fn track_address(&self, address: Vec<u8>) {
        let addr_str = hex::encode(&address);
        self.track_address_str(&addr_str);
    }

    /// Add an address string to track.
    pub fn track_address_str(&self, address: &str) {
        let mut addresses = self.addresses.write();
        if !addresses.contains(address) {
            addresses.insert(address.to_string());
            self.boxes.write().insert(address.to_string(), Vec::new());
        }
    }

    /// Get tracked addresses.
    pub fn addresses(&self) -> Vec<String> {
        self.addresses.read().iter().cloned().collect()
    }

    /// Check if an address is tracked.
    pub fn is_tracked(&self, address: &str) -> bool {
        self.addresses.read().contains(address)
    }

    /// Get current blockchain height.
    pub fn current_height(&self) -> u32 {
        self.state.utxo.height()
    }

    /// Get boxes for an address.
    pub fn get_boxes_for_address(&self, address: &str) -> Vec<WalletBox> {
        self.boxes.read().get(address).cloned().unwrap_or_default()
    }

    /// Get all unspent boxes.
    pub fn get_unspent(&self) -> Vec<WalletBox> {
        self.boxes
            .read()
            .values()
            .flatten()
            .filter(|b| !b.pending_spent)
            .cloned()
            .collect()
    }

    /// Calculate wallet balance.
    pub fn balance(&self) -> WalletBalance {
        let boxes = self.get_unspent();

        let mut balance = WalletBalance::default();

        for b in boxes {
            balance.total_erg += b.value;
            balance.confirmed_erg += b.value;

            for (token_id, amount) in b.tokens {
                *balance.tokens.entry(token_id).or_insert(0) += amount;
            }
        }

        balance
    }

    /// Scan blockchain for boxes.
    pub fn scan(&self) -> WalletResult<u32> {
        let current_height = self.state.utxo.height();
        let last_scanned = *self.scanned_height.read();

        if current_height <= last_scanned {
            return Ok(0);
        }

        // Would scan blocks from last_scanned to current_height
        // Looking for boxes belonging to tracked addresses

        *self.scanned_height.write() = current_height;

        let scanned = current_height - last_scanned;
        debug!(blocks = scanned, "Scanned blocks for wallet");

        Ok(scanned)
    }

    /// Mark a box as spent (in mempool).
    pub fn mark_spent(&self, box_id: &[u8]) {
        let mut boxes = self.boxes.write();
        for addr_boxes in boxes.values_mut() {
            for b in addr_boxes.iter_mut() {
                if b.box_id == box_id {
                    b.pending_spent = true;
                    return;
                }
            }
        }
    }

    /// Add a new box.
    pub fn add_box(&self, address: &str, wallet_box: WalletBox) {
        let mut boxes = self.boxes.write();
        if let Some(addr_boxes) = boxes.get_mut(address) {
            addr_boxes.push(wallet_box);
        } else {
            // Auto-track the address if not already tracked
            boxes.insert(address.to_string(), vec![wallet_box]);
            self.addresses.write().insert(address.to_string());
        }
    }

    /// Remove confirmed spent boxes.
    pub fn remove_spent(&self, box_ids: &[Vec<u8>]) {
        let mut boxes = self.boxes.write();
        for addr_boxes in boxes.values_mut() {
            addr_boxes.retain(|b| !box_ids.contains(&b.box_id));
        }
    }

    /// Clear all tracked data (for testing or reset).
    pub fn clear(&self) {
        self.addresses.write().clear();
        self.boxes.write().clear();
        *self.scanned_height.write() = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ergo_storage::Database;
    use tempfile::TempDir;

    fn create_test_tracker() -> (BoxTracker, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db = Database::open(tmp.path()).unwrap();
        let state = Arc::new(StateManager::new(Arc::new(db)));
        let tracker = BoxTracker::new(state);
        (tracker, tmp)
    }

    #[test]
    fn test_address_tracking() {
        let (tracker, _tmp) = create_test_tracker();

        assert!(tracker.addresses().is_empty());

        tracker.track_address_str("9addr1");
        tracker.track_address_str("9addr2");

        assert_eq!(tracker.addresses().len(), 2);
        assert!(tracker.is_tracked("9addr1"));
        assert!(tracker.is_tracked("9addr2"));
        assert!(!tracker.is_tracked("9addr3"));
    }

    #[test]
    fn test_balance() {
        let (tracker, _tmp) = create_test_tracker();

        let addr = "9testaddr";
        tracker.track_address_str(addr);

        tracker.add_box(
            addr,
            WalletBox {
                box_id: vec![10],
                value: 1000,
                tokens: vec![],
                creation_height: 1,
                address: addr.to_string(),
                pending_spent: false,
            },
        );

        let balance = tracker.balance();
        assert_eq!(balance.total_erg, 1000);
    }

    #[test]
    fn test_mark_spent() {
        let (tracker, _tmp) = create_test_tracker();

        let addr = "9testaddr";
        tracker.track_address_str(addr);

        tracker.add_box(
            addr,
            WalletBox {
                box_id: vec![10, 20, 30],
                value: 1000,
                tokens: vec![],
                creation_height: 1,
                address: addr.to_string(),
                pending_spent: false,
            },
        );

        assert_eq!(tracker.balance().total_erg, 1000);

        tracker.mark_spent(&[10, 20, 30]);

        // Box is now pending spent, so should not count in unspent
        assert_eq!(tracker.get_unspent().len(), 0);
    }

    #[test]
    fn test_token_balance() {
        let (tracker, _tmp) = create_test_tracker();

        let addr = "9testaddr";
        let token_id = vec![1, 2, 3, 4];

        tracker.add_box(
            addr,
            WalletBox {
                box_id: vec![10],
                value: 1000,
                tokens: vec![(token_id.clone(), 100)],
                creation_height: 1,
                address: addr.to_string(),
                pending_spent: false,
            },
        );

        tracker.add_box(
            addr,
            WalletBox {
                box_id: vec![20],
                value: 2000,
                tokens: vec![(token_id.clone(), 200)],
                creation_height: 2,
                address: addr.to_string(),
                pending_spent: false,
            },
        );

        let balance = tracker.balance();
        assert_eq!(balance.total_erg, 3000);
        assert_eq!(balance.tokens.get(&token_id), Some(&300));
    }
}
