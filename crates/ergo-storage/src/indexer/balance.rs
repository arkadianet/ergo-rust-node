//! Balance tracking for addresses.

use std::collections::HashMap;

/// Balance information for an address.
#[derive(Debug, Clone, Default)]
pub struct BalanceInfo {
    /// Total nanoERG balance.
    pub nano_ergs: i64,
    /// Token balances: token_id -> amount.
    pub tokens: HashMap<[u8; 32], i64>,
}

impl BalanceInfo {
    /// Create new empty balance info.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a box's value to the balance.
    pub fn add(&mut self, nano_ergs: u64, tokens: &[([u8; 32], u64)]) {
        self.nano_ergs += nano_ergs as i64;
        for (token_id, amount) in tokens {
            *self.tokens.entry(*token_id).or_insert(0) += *amount as i64;
        }
    }

    /// Subtract a box's value from the balance.
    pub fn subtract(&mut self, nano_ergs: u64, tokens: &[([u8; 32], u64)]) {
        self.nano_ergs -= nano_ergs as i64;
        for (token_id, amount) in tokens {
            let entry = self.tokens.entry(*token_id).or_insert(0);
            *entry -= *amount as i64;
            // Remove token if balance is zero
            if *entry == 0 {
                self.tokens.remove(token_id);
            }
        }
    }

    /// Serialize the balance info to bytes.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();

        // ERG balance (8 bytes, signed)
        buf.extend_from_slice(&self.nano_ergs.to_be_bytes());

        // Token count (4 bytes)
        buf.extend_from_slice(&(self.tokens.len() as u32).to_be_bytes());

        // Tokens (32 bytes id + 8 bytes amount each)
        for (token_id, amount) in &self.tokens {
            buf.extend_from_slice(token_id);
            buf.extend_from_slice(&amount.to_be_bytes());
        }

        buf
    }

    /// Deserialize balance info from bytes.
    pub fn deserialize(data: &[u8]) -> Result<Self, String> {
        if data.len() < 12 {
            return Err("BalanceInfo data too short".to_string());
        }

        let nano_ergs = i64::from_be_bytes([
            data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
        ]);

        let token_count = u32::from_be_bytes([data[8], data[9], data[10], data[11]]) as usize;

        let expected_len = 12 + token_count * 40;
        if data.len() < expected_len {
            return Err(format!(
                "BalanceInfo data too short for {} tokens",
                token_count
            ));
        }

        let mut tokens = HashMap::new();
        let mut pos = 12;
        for _ in 0..token_count {
            let mut token_id = [0u8; 32];
            token_id.copy_from_slice(&data[pos..pos + 32]);
            pos += 32;

            let amount = i64::from_be_bytes([
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

            tokens.insert(token_id, amount);
        }

        Ok(Self { nano_ergs, tokens })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_balance_add_subtract() {
        let mut balance = BalanceInfo::new();

        // Add some ERGs and tokens
        let token1 = [1u8; 32];
        let token2 = [2u8; 32];

        balance.add(1000000000, &[(token1, 100), (token2, 50)]);
        assert_eq!(balance.nano_ergs, 1000000000);
        assert_eq!(balance.tokens.get(&token1), Some(&100));
        assert_eq!(balance.tokens.get(&token2), Some(&50));

        // Add more
        balance.add(500000000, &[(token1, 50)]);
        assert_eq!(balance.nano_ergs, 1500000000);
        assert_eq!(balance.tokens.get(&token1), Some(&150));

        // Subtract
        balance.subtract(300000000, &[(token1, 150)]);
        assert_eq!(balance.nano_ergs, 1200000000);
        assert_eq!(balance.tokens.get(&token1), None); // Removed when zero
    }

    #[test]
    fn test_balance_roundtrip() {
        let mut balance = BalanceInfo::new();
        let token1 = [1u8; 32];
        let token2 = [2u8; 32];

        balance.add(1000000000, &[(token1, 100), (token2, 50)]);

        let serialized = balance.serialize();
        let deserialized = BalanceInfo::deserialize(&serialized).unwrap();

        assert_eq!(deserialized.nano_ergs, 1000000000);
        assert_eq!(deserialized.tokens.get(&token1), Some(&100));
        assert_eq!(deserialized.tokens.get(&token2), Some(&50));
    }

    #[test]
    fn test_empty_balance_roundtrip() {
        let balance = BalanceInfo::new();
        let serialized = balance.serialize();
        let deserialized = BalanceInfo::deserialize(&serialized).unwrap();

        assert_eq!(deserialized.nano_ergs, 0);
        assert!(deserialized.tokens.is_empty());
    }
}
