//! Indexed token with creation information.

/// Indexed token with creation info and associated boxes.
///
/// Tracks token creation details and all boxes containing this token.
#[derive(Debug, Clone)]
pub struct IndexedToken {
    /// Token ID (32 bytes).
    pub token_id: [u8; 32],
    /// Box ID that created this token.
    pub creation_box_id: Option<[u8; 32]>,
    /// Total emission amount.
    pub amount: Option<u64>,
    /// Token name (from R4).
    pub name: Option<String>,
    /// Token description (from R5).
    pub description: Option<String>,
    /// Decimal places (from R6).
    pub decimals: Option<u8>,
    /// Box global indexes containing this token (negative = spent).
    pub box_indexes: Vec<i64>,
}

impl IndexedToken {
    /// Create a new indexed token.
    pub fn new(token_id: [u8; 32]) -> Self {
        Self {
            token_id,
            creation_box_id: None,
            amount: None,
            name: None,
            description: None,
            decimals: None,
            box_indexes: Vec::new(),
        }
    }

    /// Create a token with full creation info.
    pub fn with_creation_info(
        token_id: [u8; 32],
        creation_box_id: [u8; 32],
        amount: u64,
        name: String,
        description: String,
        decimals: u8,
    ) -> Self {
        Self {
            token_id,
            creation_box_id: Some(creation_box_id),
            amount: Some(amount),
            name: Some(name),
            description: Some(description),
            decimals: Some(decimals),
            box_indexes: Vec::new(),
        }
    }

    /// Add a box containing this token.
    pub fn add_box(&mut self, global_box_index: u64) {
        self.box_indexes.push(global_box_index as i64);
    }

    /// Mark a box as spent.
    pub fn spend_box(&mut self, global_box_index: u64) {
        for idx in &mut self.box_indexes {
            if *idx == global_box_index as i64 {
                *idx = -(*idx);
                break;
            }
        }
    }

    /// Add to emission amount (for tokens created in multiple boxes).
    pub fn add_emission(&mut self, additional: u64) {
        self.amount = Some(self.amount.unwrap_or(0) + additional);
    }

    /// Get count of boxes ever containing this token.
    pub fn box_count(&self) -> usize {
        self.box_indexes.len()
    }

    /// Get count of unspent boxes containing this token.
    pub fn unspent_box_count(&self) -> usize {
        self.box_indexes.iter().filter(|&&idx| idx > 0).count()
    }

    /// Serialize to bytes.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();

        // Token ID (32 bytes)
        buf.extend_from_slice(&self.token_id);

        // Creation box ID (1 byte flag + optional 32 bytes)
        if let Some(box_id) = &self.creation_box_id {
            buf.push(1);
            buf.extend_from_slice(box_id);
        } else {
            buf.push(0);
        }

        // Amount (1 byte flag + optional 8 bytes)
        if let Some(amount) = self.amount {
            buf.push(1);
            buf.extend_from_slice(&amount.to_be_bytes());
        } else {
            buf.push(0);
        }

        // Name (2 byte length + bytes, or 0 if None)
        if let Some(name) = &self.name {
            let bytes = name.as_bytes();
            buf.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
            buf.extend_from_slice(bytes);
        } else {
            buf.extend_from_slice(&0u16.to_be_bytes());
        }

        // Description (2 byte length + bytes, or 0 if None)
        if let Some(desc) = &self.description {
            let bytes = desc.as_bytes();
            buf.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
            buf.extend_from_slice(bytes);
        } else {
            buf.extend_from_slice(&0u16.to_be_bytes());
        }

        // Decimals (1 byte flag + optional 1 byte)
        if let Some(decimals) = self.decimals {
            buf.push(1);
            buf.push(decimals);
        } else {
            buf.push(0);
        }

        // Box indexes
        buf.extend_from_slice(&(self.box_indexes.len() as u32).to_be_bytes());
        for idx in &self.box_indexes {
            buf.extend_from_slice(&idx.to_be_bytes());
        }

        buf
    }

    /// Deserialize from bytes.
    pub fn deserialize(data: &[u8]) -> Result<Self, String> {
        if data.len() < 32 + 1 {
            return Err("IndexedToken data too short".to_string());
        }

        let mut pos = 0;

        // Token ID
        let mut token_id = [0u8; 32];
        token_id.copy_from_slice(&data[pos..pos + 32]);
        pos += 32;

        // Creation box ID
        let creation_box_id = if data[pos] == 1 {
            pos += 1;
            let mut box_id = [0u8; 32];
            box_id.copy_from_slice(&data[pos..pos + 32]);
            pos += 32;
            Some(box_id)
        } else {
            pos += 1;
            None
        };

        // Amount
        let amount = if data[pos] == 1 {
            pos += 1;
            let amt = u64::from_be_bytes([
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
            Some(amt)
        } else {
            pos += 1;
            None
        };

        // Name
        let name_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;
        let name = if name_len > 0 {
            let s = String::from_utf8_lossy(&data[pos..pos + name_len]).to_string();
            pos += name_len;
            Some(s)
        } else {
            None
        };

        // Description
        let desc_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;
        let description = if desc_len > 0 {
            let s = String::from_utf8_lossy(&data[pos..pos + desc_len]).to_string();
            pos += desc_len;
            Some(s)
        } else {
            None
        };

        // Decimals
        let decimals = if data[pos] == 1 {
            pos += 1;
            let d = data[pos];
            pos += 1;
            Some(d)
        } else {
            pos += 1;
            None
        };

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
            token_id,
            creation_box_id,
            amount,
            name,
            description,
            decimals,
            box_indexes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_indexed_token_basic() {
        let mut token = IndexedToken::with_creation_info(
            [1u8; 32],
            [2u8; 32],
            1000000,
            "Test Token".to_string(),
            "A test token".to_string(),
            2,
        );

        token.add_box(1);
        token.add_box(2);
        token.add_box(3);

        assert_eq!(token.box_count(), 3);
        assert_eq!(token.unspent_box_count(), 3);

        token.spend_box(2);
        assert_eq!(token.unspent_box_count(), 2);
    }

    #[test]
    fn test_indexed_token_roundtrip() {
        let mut token = IndexedToken::with_creation_info(
            [1u8; 32],
            [2u8; 32],
            1000000,
            "Test Token".to_string(),
            "A test token".to_string(),
            2,
        );

        token.add_box(1);
        token.add_box(2);
        token.spend_box(1);

        let serialized = token.serialize();
        let deserialized = IndexedToken::deserialize(&serialized).unwrap();

        assert_eq!(deserialized.token_id, [1u8; 32]);
        assert_eq!(deserialized.creation_box_id.unwrap(), [2u8; 32]);
        assert_eq!(deserialized.amount.unwrap(), 1000000);
        assert_eq!(deserialized.name.as_ref().unwrap(), "Test Token");
        assert_eq!(deserialized.decimals.unwrap(), 2);
        assert_eq!(deserialized.box_count(), 2);
        assert_eq!(deserialized.unspent_box_count(), 1);
    }

    #[test]
    fn test_indexed_token_minimal() {
        let token = IndexedToken::new([1u8; 32]);

        let serialized = token.serialize();
        let deserialized = IndexedToken::deserialize(&serialized).unwrap();

        assert_eq!(deserialized.token_id, [1u8; 32]);
        assert!(deserialized.creation_box_id.is_none());
        assert!(deserialized.amount.is_none());
        assert!(deserialized.name.is_none());
        assert!(deserialized.box_indexes.is_empty());
    }

    #[test]
    fn test_indexed_token_add_emission() {
        let mut token = IndexedToken::new([1u8; 32]);

        token.add_emission(1000);
        assert_eq!(token.amount.unwrap(), 1000);

        token.add_emission(500);
        assert_eq!(token.amount.unwrap(), 1500);
    }
}
