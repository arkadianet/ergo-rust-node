//! Indexed ErgoBox with additional tracking information.

/// Indexed ErgoBox with spending information and global index.
///
/// Wraps an ErgoBox with additional metadata for efficient querying:
/// - Inclusion height
/// - Spending transaction info (if spent)
/// - Global index (serial number from genesis)
#[derive(Debug, Clone)]
pub struct IndexedErgoBox {
    /// Box ID (32 bytes).
    pub box_id: [u8; 32],
    /// Height at which the box was created.
    pub inclusion_height: u32,
    /// Spending transaction ID (if spent).
    pub spending_tx_id: Option<[u8; 32]>,
    /// Height at which the box was spent (if spent).
    pub spending_height: Option<u32>,
    /// Global index (serial number counting from genesis).
    pub global_index: u64,
    /// Serialized ErgoBox bytes.
    pub box_bytes: Vec<u8>,
    /// Value in nanoERGs.
    pub value: u64,
    /// ErgoTree hash (for address lookup).
    pub ergo_tree_hash: [u8; 32],
    /// Tokens in this box: (token_id, amount).
    pub tokens: Vec<([u8; 32], u64)>,
}

impl IndexedErgoBox {
    /// Create a new indexed box.
    pub fn new(
        box_id: [u8; 32],
        inclusion_height: u32,
        global_index: u64,
        box_bytes: Vec<u8>,
        value: u64,
        ergo_tree_hash: [u8; 32],
        tokens: Vec<([u8; 32], u64)>,
    ) -> Self {
        Self {
            box_id,
            inclusion_height,
            spending_tx_id: None,
            spending_height: None,
            global_index,
            box_bytes,
            value,
            ergo_tree_hash,
            tokens,
        }
    }

    /// Mark the box as spent.
    pub fn mark_spent(&mut self, tx_id: [u8; 32], height: u32) {
        self.spending_tx_id = Some(tx_id);
        self.spending_height = Some(height);
    }

    /// Check if the box is spent.
    pub fn is_spent(&self) -> bool {
        self.spending_tx_id.is_some()
    }

    /// Serialize to bytes.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();

        // Box ID (32 bytes)
        buf.extend_from_slice(&self.box_id);

        // Inclusion height (4 bytes)
        buf.extend_from_slice(&self.inclusion_height.to_be_bytes());

        // Spending tx ID (1 byte flag + optional 32 bytes)
        if let Some(tx_id) = &self.spending_tx_id {
            buf.push(1);
            buf.extend_from_slice(tx_id);
        } else {
            buf.push(0);
        }

        // Spending height (1 byte flag + optional 4 bytes)
        if let Some(height) = self.spending_height {
            buf.push(1);
            buf.extend_from_slice(&height.to_be_bytes());
        } else {
            buf.push(0);
        }

        // Global index (8 bytes)
        buf.extend_from_slice(&self.global_index.to_be_bytes());

        // Value (8 bytes)
        buf.extend_from_slice(&self.value.to_be_bytes());

        // ErgoTree hash (32 bytes)
        buf.extend_from_slice(&self.ergo_tree_hash);

        // Tokens count (4 bytes)
        buf.extend_from_slice(&(self.tokens.len() as u32).to_be_bytes());

        // Tokens (32 + 8 bytes each)
        for (token_id, amount) in &self.tokens {
            buf.extend_from_slice(token_id);
            buf.extend_from_slice(&amount.to_be_bytes());
        }

        // Box bytes length (4 bytes) + box bytes
        buf.extend_from_slice(&(self.box_bytes.len() as u32).to_be_bytes());
        buf.extend_from_slice(&self.box_bytes);

        buf
    }

    /// Deserialize from bytes.
    pub fn deserialize(data: &[u8]) -> Result<Self, String> {
        if data.len() < 32 + 4 + 1 {
            return Err("IndexedErgoBox data too short".to_string());
        }

        let mut pos = 0;

        // Box ID
        let mut box_id = [0u8; 32];
        box_id.copy_from_slice(&data[pos..pos + 32]);
        pos += 32;

        // Inclusion height
        let inclusion_height =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        pos += 4;

        // Spending tx ID
        let spending_tx_id = if data[pos] == 1 {
            pos += 1;
            let mut tx_id = [0u8; 32];
            tx_id.copy_from_slice(&data[pos..pos + 32]);
            pos += 32;
            Some(tx_id)
        } else {
            pos += 1;
            None
        };

        // Spending height
        let spending_height = if data[pos] == 1 {
            pos += 1;
            let height =
                u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
            pos += 4;
            Some(height)
        } else {
            pos += 1;
            None
        };

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

        // Value
        let value = u64::from_be_bytes([
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

        // ErgoTree hash
        let mut ergo_tree_hash = [0u8; 32];
        ergo_tree_hash.copy_from_slice(&data[pos..pos + 32]);
        pos += 32;

        // Tokens
        let token_count =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;

        let mut tokens = Vec::with_capacity(token_count);
        for _ in 0..token_count {
            let mut token_id = [0u8; 32];
            token_id.copy_from_slice(&data[pos..pos + 32]);
            pos += 32;

            let amount = u64::from_be_bytes([
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

            tokens.push((token_id, amount));
        }

        // Box bytes
        let box_bytes_len =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;

        let box_bytes = data[pos..pos + box_bytes_len].to_vec();

        Ok(Self {
            box_id,
            inclusion_height,
            spending_tx_id,
            spending_height,
            global_index,
            box_bytes,
            value,
            ergo_tree_hash,
            tokens,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_indexed_box_roundtrip() {
        let box_id = [1u8; 32];
        let ergo_tree_hash = [2u8; 32];
        let token1 = [3u8; 32];
        let token2 = [4u8; 32];

        let mut ieb = IndexedErgoBox::new(
            box_id,
            100,
            12345,
            vec![1, 2, 3, 4, 5],
            1000000000,
            ergo_tree_hash,
            vec![(token1, 100), (token2, 50)],
        );

        // Roundtrip unspent
        let serialized = ieb.serialize();
        let deserialized = IndexedErgoBox::deserialize(&serialized).unwrap();

        assert_eq!(deserialized.box_id, box_id);
        assert_eq!(deserialized.inclusion_height, 100);
        assert_eq!(deserialized.global_index, 12345);
        assert!(!deserialized.is_spent());
        assert_eq!(deserialized.value, 1000000000);
        assert_eq!(deserialized.tokens.len(), 2);

        // Mark spent and roundtrip
        ieb.mark_spent([5u8; 32], 200);
        let serialized = ieb.serialize();
        let deserialized = IndexedErgoBox::deserialize(&serialized).unwrap();

        assert!(deserialized.is_spent());
        assert_eq!(deserialized.spending_tx_id.unwrap(), [5u8; 32]);
        assert_eq!(deserialized.spending_height.unwrap(), 200);
    }

    #[test]
    fn test_indexed_box_no_tokens() {
        let ieb = IndexedErgoBox::new(
            [1u8; 32],
            100,
            12345,
            vec![1, 2, 3],
            1000000000,
            [2u8; 32],
            vec![],
        );

        let serialized = ieb.serialize();
        let deserialized = IndexedErgoBox::deserialize(&serialized).unwrap();

        assert!(deserialized.tokens.is_empty());
    }
}
