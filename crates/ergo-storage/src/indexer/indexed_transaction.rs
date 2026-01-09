//! Indexed transaction with minimal information.

/// Indexed transaction with references to inputs/outputs by global index.
///
/// Stores minimal information to save space while allowing full
/// transaction reconstruction via box lookups.
#[derive(Debug, Clone)]
pub struct IndexedErgoTransaction {
    /// Transaction ID (32 bytes).
    pub tx_id: [u8; 32],
    /// Index of transaction in the block.
    pub tx_index: u16,
    /// Height of the containing block.
    pub height: u32,
    /// Transaction size in bytes.
    pub size: u32,
    /// Global transaction index (serial number from genesis).
    pub global_index: u64,
    /// Input box global indexes.
    pub input_indexes: Vec<u64>,
    /// Output box global indexes.
    pub output_indexes: Vec<u64>,
    /// Data input box IDs (not global indexes since they're read-only).
    pub data_inputs: Vec<[u8; 32]>,
}

impl IndexedErgoTransaction {
    /// Create a new indexed transaction.
    pub fn new(
        tx_id: [u8; 32],
        tx_index: u16,
        height: u32,
        size: u32,
        global_index: u64,
        input_indexes: Vec<u64>,
        output_indexes: Vec<u64>,
        data_inputs: Vec<[u8; 32]>,
    ) -> Self {
        Self {
            tx_id,
            tx_index,
            height,
            size,
            global_index,
            input_indexes,
            output_indexes,
            data_inputs,
        }
    }

    /// Serialize to bytes.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();

        // Tx ID (32 bytes)
        buf.extend_from_slice(&self.tx_id);

        // Tx index (2 bytes)
        buf.extend_from_slice(&self.tx_index.to_be_bytes());

        // Height (4 bytes)
        buf.extend_from_slice(&self.height.to_be_bytes());

        // Size (4 bytes)
        buf.extend_from_slice(&self.size.to_be_bytes());

        // Global index (8 bytes)
        buf.extend_from_slice(&self.global_index.to_be_bytes());

        // Input indexes count (2 bytes) + indexes (8 bytes each)
        buf.extend_from_slice(&(self.input_indexes.len() as u16).to_be_bytes());
        for idx in &self.input_indexes {
            buf.extend_from_slice(&idx.to_be_bytes());
        }

        // Output indexes count (2 bytes) + indexes (8 bytes each)
        buf.extend_from_slice(&(self.output_indexes.len() as u16).to_be_bytes());
        for idx in &self.output_indexes {
            buf.extend_from_slice(&idx.to_be_bytes());
        }

        // Data inputs count (2 bytes) + IDs (32 bytes each)
        buf.extend_from_slice(&(self.data_inputs.len() as u16).to_be_bytes());
        for id in &self.data_inputs {
            buf.extend_from_slice(id);
        }

        buf
    }

    /// Deserialize from bytes.
    pub fn deserialize(data: &[u8]) -> Result<Self, String> {
        if data.len() < 32 + 2 + 4 + 4 + 8 + 2 + 2 + 2 {
            return Err("IndexedErgoTransaction data too short".to_string());
        }

        let mut pos = 0;

        // Tx ID
        let mut tx_id = [0u8; 32];
        tx_id.copy_from_slice(&data[pos..pos + 32]);
        pos += 32;

        // Tx index
        let tx_index = u16::from_be_bytes([data[pos], data[pos + 1]]);
        pos += 2;

        // Height
        let height = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        pos += 4;

        // Size
        let size = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        pos += 4;

        // Global index
        let global_index = u64::from_be_bytes([
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

        // Input indexes
        let input_count = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;

        let mut input_indexes = Vec::with_capacity(input_count);
        for _ in 0..input_count {
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
            input_indexes.push(idx);
        }

        // Output indexes
        let output_count = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;

        let mut output_indexes = Vec::with_capacity(output_count);
        for _ in 0..output_count {
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
            output_indexes.push(idx);
        }

        // Data inputs
        let data_input_count = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;

        let mut data_inputs = Vec::with_capacity(data_input_count);
        for _ in 0..data_input_count {
            let mut id = [0u8; 32];
            id.copy_from_slice(&data[pos..pos + 32]);
            pos += 32;
            data_inputs.push(id);
        }

        Ok(Self {
            tx_id,
            tx_index,
            height,
            size,
            global_index,
            input_indexes,
            output_indexes,
            data_inputs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_indexed_tx_roundtrip() {
        let tx = IndexedErgoTransaction::new(
            [1u8; 32],
            5,
            100,
            500,
            12345,
            vec![1, 2, 3],
            vec![4, 5],
            vec![[6u8; 32], [7u8; 32]],
        );

        let serialized = tx.serialize();
        let deserialized = IndexedErgoTransaction::deserialize(&serialized).unwrap();

        assert_eq!(deserialized.tx_id, [1u8; 32]);
        assert_eq!(deserialized.tx_index, 5);
        assert_eq!(deserialized.height, 100);
        assert_eq!(deserialized.size, 500);
        assert_eq!(deserialized.global_index, 12345);
        assert_eq!(deserialized.input_indexes, vec![1, 2, 3]);
        assert_eq!(deserialized.output_indexes, vec![4, 5]);
        assert_eq!(deserialized.data_inputs.len(), 2);
    }

    #[test]
    fn test_indexed_tx_empty_inputs() {
        let tx = IndexedErgoTransaction::new([1u8; 32], 0, 1, 100, 1, vec![], vec![1], vec![]);

        let serialized = tx.serialize();
        let deserialized = IndexedErgoTransaction::deserialize(&serialized).unwrap();

        assert!(deserialized.input_indexes.is_empty());
        assert_eq!(deserialized.output_indexes, vec![1]);
        assert!(deserialized.data_inputs.is_empty());
    }
}
