//! Scanning predicates for matching boxes (EIP-0001).

use serde::{Deserialize, Serialize};

/// Register ID for box registers (R0-R9).
pub type RegisterId = u8;

/// A predicate for filtering boxes during scanning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "predicate", rename_all = "camelCase")]
pub enum ScanningPredicate {
    /// Match boxes containing a specific asset/token.
    #[serde(rename = "containsAsset")]
    ContainsAsset {
        /// Token ID to match (hex-encoded in JSON).
        #[serde(with = "hex_serde")]
        asset_id: Vec<u8>,
    },

    /// Match boxes where a register contains specific bytes (exact match).
    #[serde(rename = "equals")]
    Equals {
        /// Register ID (0-9 for R0-R9).
        #[serde(rename = "register")]
        reg_id: RegisterId,
        /// Value to match (hex-encoded bytes in JSON).
        #[serde(with = "hex_serde")]
        value: Vec<u8>,
    },

    /// Match boxes where a register contains specific bytes (substring match).
    #[serde(rename = "contains")]
    Contains {
        /// Register ID (0-9 for R0-R9).
        #[serde(rename = "register")]
        reg_id: RegisterId,
        /// Value to find within register (hex-encoded bytes in JSON).
        #[serde(with = "hex_serde")]
        value: Vec<u8>,
    },

    /// Match boxes satisfying all sub-predicates.
    #[serde(rename = "and")]
    And {
        /// Sub-predicates that must all match.
        args: Vec<ScanningPredicate>,
    },

    /// Match boxes satisfying any sub-predicate.
    #[serde(rename = "or")]
    Or {
        /// Sub-predicates where at least one must match.
        args: Vec<ScanningPredicate>,
    },
}

impl ScanningPredicate {
    /// Create a predicate to match boxes containing a specific asset.
    pub fn contains_asset(asset_id: Vec<u8>) -> Self {
        Self::ContainsAsset { asset_id }
    }

    /// Create a predicate to match boxes where register equals value.
    pub fn equals(reg_id: RegisterId, value: Vec<u8>) -> Self {
        Self::Equals { reg_id, value }
    }

    /// Create a predicate to match boxes where register contains value.
    pub fn contains(reg_id: RegisterId, value: Vec<u8>) -> Self {
        Self::Contains { reg_id, value }
    }

    /// Create an AND predicate.
    pub fn and(predicates: Vec<ScanningPredicate>) -> Self {
        Self::And { args: predicates }
    }

    /// Create an OR predicate.
    pub fn or(predicates: Vec<ScanningPredicate>) -> Self {
        Self::Or { args: predicates }
    }

    /// Check if a box matches this predicate.
    ///
    /// # Arguments
    /// * `box_tokens` - List of (token_id, amount) pairs from the box
    /// * `box_registers` - Map of register_id -> register_value bytes
    pub fn matches(
        &self,
        box_tokens: &[(Vec<u8>, u64)],
        box_registers: &std::collections::HashMap<u8, Vec<u8>>,
    ) -> bool {
        match self {
            Self::ContainsAsset { asset_id } => box_tokens.iter().any(|(tid, _)| tid == asset_id),

            Self::Equals { reg_id, value } => box_registers
                .get(reg_id)
                .map(|v| v == value)
                .unwrap_or(false),

            Self::Contains { reg_id, value } => box_registers
                .get(reg_id)
                .map(|v| contains_slice(v, value))
                .unwrap_or(false),

            Self::And { args } => args.iter().all(|p| p.matches(box_tokens, box_registers)),

            Self::Or { args } => args.iter().any(|p| p.matches(box_tokens, box_registers)),
        }
    }
}

/// Check if haystack contains needle as a subsequence.
fn contains_slice(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Hex serialization for Vec<u8>.
mod hex_serde {
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        hex::decode(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_contains_asset() {
        let pred = ScanningPredicate::contains_asset(vec![1, 2, 3]);
        let tokens = vec![(vec![1, 2, 3], 100), (vec![4, 5, 6], 200)];
        let registers = HashMap::new();

        assert!(pred.matches(&tokens, &registers));

        let tokens_without = vec![(vec![4, 5, 6], 200)];
        assert!(!pred.matches(&tokens_without, &registers));
    }

    #[test]
    fn test_equals() {
        let pred = ScanningPredicate::equals(4, vec![0xDE, 0xAD, 0xBE, 0xEF]);
        let mut registers = HashMap::new();
        registers.insert(4, vec![0xDE, 0xAD, 0xBE, 0xEF]);

        assert!(pred.matches(&[], &registers));

        registers.insert(4, vec![0xDE, 0xAD]);
        assert!(!pred.matches(&[], &registers));
    }

    #[test]
    fn test_contains() {
        let pred = ScanningPredicate::contains(4, vec![0xAD, 0xBE]);
        let mut registers = HashMap::new();
        registers.insert(4, vec![0xDE, 0xAD, 0xBE, 0xEF]);

        assert!(pred.matches(&[], &registers));

        registers.insert(4, vec![0xDE, 0xAD, 0xEF]);
        assert!(!pred.matches(&[], &registers));
    }

    #[test]
    fn test_and() {
        let pred = ScanningPredicate::and(vec![
            ScanningPredicate::contains_asset(vec![1, 2, 3]),
            ScanningPredicate::equals(4, vec![0xAB]),
        ]);

        let tokens = vec![(vec![1, 2, 3], 100)];
        let mut registers = HashMap::new();
        registers.insert(4, vec![0xAB]);

        assert!(pred.matches(&tokens, &registers));

        // Missing token
        assert!(!pred.matches(&[], &registers));

        // Wrong register value
        registers.insert(4, vec![0xCD]);
        assert!(!pred.matches(&tokens, &registers));
    }

    #[test]
    fn test_or() {
        let pred = ScanningPredicate::or(vec![
            ScanningPredicate::contains_asset(vec![1, 2, 3]),
            ScanningPredicate::contains_asset(vec![4, 5, 6]),
        ]);

        let tokens1 = vec![(vec![1, 2, 3], 100)];
        let tokens2 = vec![(vec![4, 5, 6], 100)];
        let tokens3 = vec![(vec![7, 8, 9], 100)];
        let registers = HashMap::new();

        assert!(pred.matches(&tokens1, &registers));
        assert!(pred.matches(&tokens2, &registers));
        assert!(!pred.matches(&tokens3, &registers));
    }

    #[test]
    fn test_json_serialization() {
        let pred = ScanningPredicate::contains_asset(vec![1, 2, 3]);
        let json = serde_json::to_string(&pred).unwrap();
        assert!(json.contains("containsAsset"));
        assert!(json.contains("010203")); // hex-encoded

        let parsed: ScanningPredicate = serde_json::from_str(&json).unwrap();
        assert_eq!(pred, parsed);
    }
}
