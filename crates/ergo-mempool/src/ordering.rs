//! Transaction ordering by fee.

use std::cmp::Ordering;

/// Transaction with fee information for ordering.
#[derive(Debug, Clone)]
pub struct FeeOrdering {
    /// Transaction ID.
    pub tx_id: Vec<u8>,
    /// Transaction fee in nanoERG.
    pub fee: u64,
    /// Transaction size in bytes.
    pub size: usize,
    /// Arrival time (unix timestamp in millis).
    pub arrival_time: u64,
}

impl FeeOrdering {
    /// Create a new fee ordering entry.
    pub fn new(tx_id: Vec<u8>, fee: u64, size: usize, arrival_time: u64) -> Self {
        Self {
            tx_id,
            fee,
            size,
            arrival_time,
        }
    }

    /// Calculate fee per byte (for ordering).
    pub fn fee_per_byte(&self) -> f64 {
        if self.size == 0 {
            0.0
        } else {
            self.fee as f64 / self.size as f64
        }
    }
}

impl PartialEq for FeeOrdering {
    fn eq(&self, other: &Self) -> bool {
        self.tx_id == other.tx_id
    }
}

impl Eq for FeeOrdering {}

impl PartialOrd for FeeOrdering {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FeeOrdering {
    fn cmp(&self, other: &Self) -> Ordering {
        // Higher fee per byte = higher priority
        let self_fpb = self.fee_per_byte();
        let other_fpb = other.fee_per_byte();

        // Compare by fee per byte first (reverse so higher fee comes first in BTreeSet)
        match other_fpb.partial_cmp(&self_fpb) {
            Some(Ordering::Equal) | None => {
                // If equal, prefer earlier arrival (lower timestamp first)
                self.arrival_time.cmp(&other.arrival_time)
            }
            Some(ord) => ord,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn test_fee_ordering() {
        let tx1 = FeeOrdering::new(vec![1], 1000, 100, 1000); // 10 per byte
        let tx2 = FeeOrdering::new(vec![2], 2000, 100, 1001); // 20 per byte
        let tx3 = FeeOrdering::new(vec![3], 1000, 100, 999); // 10 per byte, earlier

        let mut set = BTreeSet::new();
        set.insert(tx1.clone());
        set.insert(tx2.clone());
        set.insert(tx3.clone());

        let ordered: Vec<_> = set.into_iter().collect();

        // tx2 should be first (highest fee), then tx3 (earlier), then tx1
        assert_eq!(ordered[0].tx_id, vec![2]);
        assert_eq!(ordered[1].tx_id, vec![3]);
        assert_eq!(ordered[2].tx_id, vec![1]);
    }
}
