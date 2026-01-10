//! Scan types following EIP-0001.

use super::predicate::ScanningPredicate;
use serde::{Deserialize, Serialize};

/// Scan identifier (i16 in Scala, we use i16 for compatibility).
pub type ScanId = i16;

/// How a scan interacts with the built-in P2PK wallet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScanWalletInteraction {
    /// Box is added to scan only, not to wallet.
    Off,
    /// Box can be in both wallet and scan if wallet finds it (P2PK protected).
    Shared,
    /// Box is always added to wallet if added to scan.
    Forced,
}

impl Default for ScanWalletInteraction {
    fn default() -> Self {
        Self::Shared
    }
}

impl ScanWalletInteraction {
    /// Convert to byte for serialization.
    pub fn to_byte(self) -> i8 {
        match self {
            Self::Off => -1,
            Self::Shared => -2,
            Self::Forced => -3,
        }
    }

    /// Convert from byte.
    pub fn from_byte(b: i8) -> Self {
        match b {
            -1 => Self::Off,
            -2 => Self::Shared,
            -3 => Self::Forced,
            _ => Self::Off, // Default for unknown values
        }
    }

    /// Whether boxes should also be added to the P2PK wallet.
    pub fn interacts_with_wallet(self) -> bool {
        matches!(self, Self::Shared | Self::Forced)
    }
}

/// Maximum length of scan name in bytes (UTF-8).
pub const MAX_SCAN_NAME_LENGTH: usize = 255;

/// A registered scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scan {
    /// Unique identifier of the scan.
    pub scan_id: ScanId,
    /// Human-readable scan name/description.
    pub scan_name: String,
    /// Predicate to match boxes.
    pub tracking_rule: ScanningPredicate,
    /// How scan interacts with wallet.
    pub wallet_interaction: ScanWalletInteraction,
    /// Whether to remove boxes spent offchain from unspent set.
    pub remove_offchain: bool,
}

impl Scan {
    /// Create a new scan.
    pub fn new(
        scan_id: ScanId,
        scan_name: String,
        tracking_rule: ScanningPredicate,
        wallet_interaction: ScanWalletInteraction,
        remove_offchain: bool,
    ) -> Self {
        Self {
            scan_id,
            scan_name,
            tracking_rule,
            wallet_interaction,
            remove_offchain,
        }
    }

    /// Serialize scan to bytes.
    pub fn serialize(&self) -> Vec<u8> {
        // Simple JSON serialization for now
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Deserialize scan from bytes.
    pub fn deserialize(data: &[u8]) -> Option<Self> {
        serde_json::from_slice(data).ok()
    }
}

/// Request to create a new scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanRequest {
    /// Human-readable scan name/description.
    pub scan_name: String,
    /// Predicate to match boxes.
    pub tracking_rule: ScanningPredicate,
    /// How scan interacts with wallet (optional, defaults to Shared).
    #[serde(default)]
    pub wallet_interaction: Option<ScanWalletInteraction>,
    /// Whether to remove boxes spent offchain (optional, defaults to true).
    #[serde(default)]
    pub remove_offchain: Option<bool>,
}

impl ScanRequest {
    /// Convert request to a scan with the given ID.
    pub fn to_scan(self, scan_id: ScanId) -> Result<Scan, String> {
        if self.scan_name.as_bytes().len() > MAX_SCAN_NAME_LENGTH {
            return Err(format!("Scan name too long: {}", self.scan_name));
        }

        Ok(Scan::new(
            scan_id,
            self.scan_name,
            self.tracking_rule,
            self.wallet_interaction
                .unwrap_or(ScanWalletInteraction::Shared),
            self.remove_offchain.unwrap_or(true),
        ))
    }
}

/// A box tracked by a scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanBox {
    /// Box ID (32 bytes).
    pub box_id: Vec<u8>,
    /// Scan IDs this box belongs to.
    pub scan_ids: Vec<ScanId>,
    /// Whether the box is spent.
    pub spent: bool,
    /// Spending transaction ID (if spent).
    pub spending_tx_id: Option<Vec<u8>>,
    /// Height at which box was spent.
    pub spending_height: Option<u32>,
    /// Inclusion height of the box.
    pub inclusion_height: u32,
    /// Number of confirmations at time of recording.
    pub confirmations_num: u32,
    /// Serialized ErgoBox data.
    pub box_data: Vec<u8>,
}

impl ScanBox {
    /// Create a new unspent scan box.
    pub fn new(
        box_id: Vec<u8>,
        scan_ids: Vec<ScanId>,
        inclusion_height: u32,
        confirmations_num: u32,
        box_data: Vec<u8>,
    ) -> Self {
        Self {
            box_id,
            scan_ids,
            spent: false,
            spending_tx_id: None,
            spending_height: None,
            inclusion_height,
            confirmations_num,
            box_data,
        }
    }

    /// Mark box as spent.
    pub fn mark_spent(&mut self, spending_tx_id: Vec<u8>, spending_height: u32) {
        self.spent = true;
        self.spending_tx_id = Some(spending_tx_id);
        self.spending_height = Some(spending_height);
    }

    /// Serialize to bytes.
    pub fn serialize(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Deserialize from bytes.
    pub fn deserialize(data: &[u8]) -> Option<Self> {
        serde_json::from_slice(data).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wallet_interaction_serialization() {
        assert_eq!(ScanWalletInteraction::Off.to_byte(), -1);
        assert_eq!(ScanWalletInteraction::Shared.to_byte(), -2);
        assert_eq!(ScanWalletInteraction::Forced.to_byte(), -3);

        assert_eq!(
            ScanWalletInteraction::from_byte(-1),
            ScanWalletInteraction::Off
        );
        assert_eq!(
            ScanWalletInteraction::from_byte(-2),
            ScanWalletInteraction::Shared
        );
        assert_eq!(
            ScanWalletInteraction::from_byte(-3),
            ScanWalletInteraction::Forced
        );
    }

    #[test]
    fn test_scan_request_validation() {
        let request = ScanRequest {
            scan_name: "test".to_string(),
            tracking_rule: ScanningPredicate::ContainsAsset {
                asset_id: vec![1, 2, 3],
            },
            wallet_interaction: None,
            remove_offchain: None,
        };

        let scan = request.to_scan(1).unwrap();
        assert_eq!(scan.scan_id, 1);
        assert_eq!(scan.scan_name, "test");
        assert_eq!(scan.wallet_interaction, ScanWalletInteraction::Shared);
        assert!(scan.remove_offchain);
    }

    #[test]
    fn test_scan_name_too_long() {
        let long_name = "x".repeat(300);
        let request = ScanRequest {
            scan_name: long_name,
            tracking_rule: ScanningPredicate::ContainsAsset {
                asset_id: vec![1, 2, 3],
            },
            wallet_interaction: None,
            remove_offchain: None,
        };

        assert!(request.to_scan(1).is_err());
    }
}
