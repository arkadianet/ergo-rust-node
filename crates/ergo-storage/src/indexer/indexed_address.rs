//! Indexed address (ErgoTree) with balance tracking.

use super::BalanceInfo;

/// Indexed address with transaction/box tracking and balance info.
///
/// Tracks all transactions and boxes associated with an ErgoTree,
/// along with current balance information.
#[derive(Debug, Clone)]
pub struct IndexedErgoAddress {
    /// ErgoTree hash (32 bytes).
    pub tree_hash: [u8; 32],
    /// Transaction global indexes (in order of first appearance).
    pub tx_indexes: Vec<u64>,
    /// Box global indexes (positive = unspent, entry present = ever associated).
    pub box_indexes: Vec<i64>,
    /// Current balance information.
    pub balance: BalanceInfo,
}

impl IndexedErgoAddress {
    /// Create a new indexed address.
    pub fn new(tree_hash: [u8; 32]) -> Self {
        Self {
            tree_hash,
            tx_indexes: Vec::new(),
            box_indexes: Vec::new(),
            balance: BalanceInfo::new(),
        }
    }

    /// Add a transaction to this address (if not already present).
    pub fn add_tx(&mut self, global_tx_index: u64) {
        // Check for duplicates (last entry)
        if self.tx_indexes.last() != Some(&global_tx_index) {
            self.tx_indexes.push(global_tx_index);
        }
    }

    /// Add a box to this address and update balance.
    pub fn add_box(&mut self, global_box_index: u64, value: u64, tokens: &[([u8; 32], u64)]) {
        self.box_indexes.push(global_box_index as i64);
        self.balance.add(value, tokens);
    }

    /// Spend a box from this address and update balance.
    /// The box index is negated to indicate spent status.
    pub fn spend_box(&mut self, global_box_index: u64, value: u64, tokens: &[([u8; 32], u64)]) {
        // Find and negate the box index
        for idx in &mut self.box_indexes {
            if *idx == global_box_index as i64 {
                *idx = -(*idx);
                break;
            }
        }
        self.balance.subtract(value, tokens);
    }

    /// Get count of transactions associated with this address.
    pub fn tx_count(&self) -> usize {
        self.tx_indexes.len()
    }

    /// Get count of boxes ever associated with this address.
    pub fn box_count(&self) -> usize {
        self.box_indexes.len()
    }

    /// Get count of currently unspent boxes.
    pub fn unspent_box_count(&self) -> usize {
        self.box_indexes.iter().filter(|&&idx| idx > 0).count()
    }

    /// Get unspent box indexes.
    pub fn unspent_boxes(&self) -> Vec<u64> {
        self.box_indexes
            .iter()
            .filter(|&&idx| idx > 0)
            .map(|&idx| idx as u64)
            .collect()
    }

    /// Serialize to bytes.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();

        // Tree hash (32 bytes)
        buf.extend_from_slice(&self.tree_hash);

        // Balance info
        let balance_bytes = self.balance.serialize();
        buf.extend_from_slice(&(balance_bytes.len() as u32).to_be_bytes());
        buf.extend_from_slice(&balance_bytes);

        // Transaction indexes
        buf.extend_from_slice(&(self.tx_indexes.len() as u32).to_be_bytes());
        for idx in &self.tx_indexes {
            buf.extend_from_slice(&idx.to_be_bytes());
        }

        // Box indexes (as i64 to preserve spent flag)
        buf.extend_from_slice(&(self.box_indexes.len() as u32).to_be_bytes());
        for idx in &self.box_indexes {
            buf.extend_from_slice(&idx.to_be_bytes());
        }

        buf
    }

    /// Deserialize from bytes.
    pub fn deserialize(data: &[u8]) -> Result<Self, String> {
        if data.len() < 32 + 4 {
            return Err("IndexedErgoAddress data too short".to_string());
        }

        let mut pos = 0;

        // Tree hash
        let mut tree_hash = [0u8; 32];
        tree_hash.copy_from_slice(&data[pos..pos + 32]);
        pos += 32;

        // Balance info
        let balance_len =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;

        let balance = BalanceInfo::deserialize(&data[pos..pos + balance_len])?;
        pos += balance_len;

        // Transaction indexes
        let tx_count =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;

        let mut tx_indexes = Vec::with_capacity(tx_count);
        for _ in 0..tx_count {
            let idx = u64::from_be_bytes([
                data[pos],
                data[pos + 1],
                data[pos + 2],
                data[pos + 3],
                data[pos + 4],
                data[pos + 5],
                data[pos + 6],
                data[pos + 7],
            ]);
            pos += 8;
            tx_indexes.push(idx);
        }

        // Box indexes
        let box_count =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;

        let mut box_indexes = Vec::with_capacity(box_count);
        for _ in 0..box_count {
            let idx = i64::from_be_bytes([
                data[pos],
                data[pos + 1],
                data[pos + 2],
                data[pos + 3],
                data[pos + 4],
                data[pos + 5],
                data[pos + 6],
                data[pos + 7],
            ]);
            pos += 8;
            box_indexes.push(idx);
        }

        Ok(Self {
            tree_hash,
            tx_indexes,
            box_indexes,
            balance,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_indexed_address_basic() {
        let mut addr = IndexedErgoAddress::new([1u8; 32]);

        // Add transaction
        addr.add_tx(100);
        addr.add_tx(100); // Duplicate, should not be added
        addr.add_tx(101);

        assert_eq!(addr.tx_count(), 2);

        // Add boxes
        let token1 = [2u8; 32];
        addr.add_box(1, 1000000000, &[(token1, 100)]);
        addr.add_box(2, 500000000, &[]);

        assert_eq!(addr.box_count(), 2);
        assert_eq!(addr.unspent_box_count(), 2);
        assert_eq!(addr.balance.nano_ergs, 1500000000);

        // Spend a box
        addr.spend_box(1, 1000000000, &[(token1, 100)]);

        assert_eq!(addr.unspent_box_count(), 1);
        assert_eq!(addr.balance.nano_ergs, 500000000);
    }

    #[test]
    fn test_indexed_address_roundtrip() {
        let mut addr = IndexedErgoAddress::new([1u8; 32]);

        let token1 = [2u8; 32];
        addr.add_tx(100);
        addr.add_tx(101);
        addr.add_box(1, 1000000000, &[(token1, 100)]);
        addr.add_box(2, 500000000, &[]);
        addr.spend_box(1, 1000000000, &[(token1, 100)]);

        let serialized = addr.serialize();
        let deserialized = IndexedErgoAddress::deserialize(&serialized).unwrap();

        assert_eq!(deserialized.tree_hash, [1u8; 32]);
        assert_eq!(deserialized.tx_count(), 2);
        assert_eq!(deserialized.box_count(), 2);
        assert_eq!(deserialized.unspent_box_count(), 1);
        assert_eq!(deserialized.balance.nano_ergs, 500000000);
    }

    #[test]
    fn test_indexed_address_unspent_boxes() {
        let mut addr = IndexedErgoAddress::new([1u8; 32]);

        addr.add_box(1, 100, &[]);
        addr.add_box(2, 200, &[]);
        addr.add_box(3, 300, &[]);
        addr.spend_box(2, 200, &[]);

        let unspent = addr.unspent_boxes();
        assert_eq!(unspent, vec![1, 3]);
    }
}
