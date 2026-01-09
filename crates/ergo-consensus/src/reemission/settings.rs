//! Re-emission configuration settings (EIP-27).

use serde::{Deserialize, Serialize};

/// Configuration for re-emission (EIP-27).
///
/// These settings control when and how re-emission is activated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReemissionSettings {
    /// Whether to check re-emission rules during validation.
    pub check_reemission_rules: bool,

    /// NFT ID for the emission contract (hex-encoded, 32 bytes).
    pub emission_nft_id: [u8; 32],

    /// Token ID for re-emission tokens (hex-encoded, 32 bytes).
    pub reemission_token_id: [u8; 32],

    /// NFT ID for the re-emission contract (hex-encoded, 32 bytes).
    pub reemission_nft_id: [u8; 32],

    /// Height at which EIP-27 rules become active.
    pub activation_height: u32,

    /// Height at which re-emission starts (when regular emission drops to minimum).
    /// For mainnet: 2,080,800
    pub reemission_start_height: u32,

    /// Serialized injection box bytes (used for bootstrapping).
    pub injection_box_bytes: Option<Vec<u8>>,
}

impl ReemissionSettings {
    /// Create settings for mainnet.
    pub fn mainnet() -> Self {
        Self {
            check_reemission_rules: true,
            // Mainnet emission NFT ID
            emission_nft_id: hex_to_bytes32(
                "20fa2bf23962cdf51b07722d6237c0c7b8a44f78856c0f7ec308dc1ef1a92a51",
            ),
            // Mainnet re-emission token ID
            reemission_token_id: hex_to_bytes32(
                "d9a2cc8a09abfaed87afacfbb7daee79a6b26f10c6613fc13d3f3953e5521d1a",
            ),
            // Mainnet re-emission NFT ID
            reemission_nft_id: hex_to_bytes32(
                "d6b2a40fbf32a30a5d5ee44c0c9e9cafc8ce9d42e7e1c21e2b2a0b20b80da9c3",
            ),
            activation_height: 777_217, // EIP-27 soft-fork activation
            reemission_start_height: 2_080_800,
            injection_box_bytes: None,
        }
    }

    /// Create settings for testnet.
    pub fn testnet() -> Self {
        Self {
            check_reemission_rules: true,
            emission_nft_id: [0u8; 32],
            reemission_token_id: [0u8; 32],
            reemission_nft_id: [0u8; 32],
            activation_height: 188_001,
            reemission_start_height: 2_080_800,
            injection_box_bytes: None,
        }
    }

    /// Create settings with re-emission disabled.
    pub fn disabled() -> Self {
        Self {
            check_reemission_rules: false,
            emission_nft_id: [0u8; 32],
            reemission_token_id: [0u8; 32],
            reemission_nft_id: [0u8; 32],
            activation_height: u32::MAX,
            reemission_start_height: u32::MAX,
            injection_box_bytes: None,
        }
    }

    /// Check if re-emission is active at the given height.
    pub fn is_active(&self, height: u32) -> bool {
        self.check_reemission_rules && height >= self.activation_height
    }

    /// Check if re-emission rewards can be claimed at the given height.
    pub fn can_claim_reemission(&self, height: u32) -> bool {
        self.check_reemission_rules && height >= self.reemission_start_height
    }
}

impl Default for ReemissionSettings {
    fn default() -> Self {
        Self::mainnet()
    }
}

/// Convert a hex string to a 32-byte array.
fn hex_to_bytes32(hex: &str) -> [u8; 32] {
    let bytes = hex::decode(hex).expect("Invalid hex string");
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    arr
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mainnet_settings() {
        let settings = ReemissionSettings::mainnet();
        assert!(settings.check_reemission_rules);
        assert_eq!(settings.activation_height, 777_217);
        assert_eq!(settings.reemission_start_height, 2_080_800);
    }

    #[test]
    fn test_is_active() {
        let settings = ReemissionSettings::mainnet();

        // Before activation
        assert!(!settings.is_active(777_216));

        // At activation
        assert!(settings.is_active(777_217));

        // After activation
        assert!(settings.is_active(1_000_000));
    }

    #[test]
    fn test_can_claim_reemission() {
        let settings = ReemissionSettings::mainnet();

        // Before reemission start
        assert!(!settings.can_claim_reemission(2_080_799));

        // At reemission start
        assert!(settings.can_claim_reemission(2_080_800));

        // After reemission start
        assert!(settings.can_claim_reemission(3_000_000));
    }

    #[test]
    fn test_disabled_settings() {
        let settings = ReemissionSettings::disabled();
        assert!(!settings.check_reemission_rules);
        assert!(!settings.is_active(1_000_000));
        assert!(!settings.can_claim_reemission(3_000_000));
    }
}
