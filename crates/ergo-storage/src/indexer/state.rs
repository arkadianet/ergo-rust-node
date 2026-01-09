//! Indexer state tracking.

/// State of the extra indexer.
#[derive(Debug, Clone, Default)]
pub struct IndexerState {
    /// Height of the last indexed block.
    pub indexed_height: u32,
    /// Global box index counter (incremented for each output).
    pub global_box_index: u64,
    /// Global transaction index counter (incremented for each tx).
    pub global_tx_index: u64,
}

impl IndexerState {
    /// Create new indexer state.
    pub fn new(indexed_height: u32, global_box_index: u64, global_tx_index: u64) -> Self {
        Self {
            indexed_height,
            global_box_index,
            global_tx_index,
        }
    }

    /// Serialize the state to bytes.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(20);
        buf.extend_from_slice(&self.indexed_height.to_be_bytes());
        buf.extend_from_slice(&self.global_box_index.to_be_bytes());
        buf.extend_from_slice(&self.global_tx_index.to_be_bytes());
        buf
    }

    /// Deserialize state from bytes.
    pub fn deserialize(data: &[u8]) -> Result<Self, String> {
        if data.len() < 20 {
            return Err("IndexerState data too short".to_string());
        }

        let indexed_height = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        let global_box_index = u64::from_be_bytes([
            data[4], data[5], data[6], data[7], data[8], data[9], data[10], data[11],
        ]);
        let global_tx_index = u64::from_be_bytes([
            data[12], data[13], data[14], data[15], data[16], data[17], data[18], data[19],
        ]);

        Ok(Self {
            indexed_height,
            global_box_index,
            global_tx_index,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_roundtrip() {
        let state = IndexerState::new(123456, 9876543210, 1234567890);
        let serialized = state.serialize();
        let deserialized = IndexerState::deserialize(&serialized).unwrap();

        assert_eq!(deserialized.indexed_height, 123456);
        assert_eq!(deserialized.global_box_index, 9876543210);
        assert_eq!(deserialized.global_tx_index, 1234567890);
    }

    #[test]
    fn test_state_default() {
        let state = IndexerState::default();
        assert_eq!(state.indexed_height, 0);
        assert_eq!(state.global_box_index, 0);
        assert_eq!(state.global_tx_index, 0);
    }
}
