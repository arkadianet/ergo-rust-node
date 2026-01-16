//! Network-specific consensus parameters.
//!
//! All network-specific values are encapsulated in `ChainParams` to avoid
//! hardcoded constants scattered throughout the codebase.
//!
//! Use `ChainParams::mainnet()` for mainnet, or `ChainParams::from_config()`
//! for testnet/devnet/private networks loaded from configuration.

use num_bigint::BigUint;
use num_traits::Zero;
use std::fmt;

/// Error when constructing ChainParams from configuration.
#[derive(Debug, Clone)]
pub struct ChainParamsError {
    /// The field that is missing or invalid.
    pub field: &'static str,
    /// Description of the error.
    pub message: String,
}

impl fmt::Display for ChainParamsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ChainParams error for '{}': {}", self.field, self.message)
    }
}

impl std::error::Error for ChainParamsError {}

/// Configuration for loading ChainParams from TOML/JSON.
///
/// Mirrors Scala's ChainSettings structure.
/// All fields are optional so partial configs can be validated with clear errors.
#[derive(Debug, Clone, Default)]
pub struct ChainParamsConfig {
    /// Target block interval in milliseconds.
    pub block_interval_ms: Option<u64>,
    /// EIP-37 activation height (None = never activated).
    pub eip37_activation_height: Option<u32>,
    /// Epoch length before EIP-37 activation.
    pub pre_eip37_epoch_length: Option<u32>,
    /// Epoch length after EIP-37 activation.
    pub eip37_epoch_length: Option<u32>,
    /// Number of epochs for difficulty regression (typically 8).
    pub use_last_epochs: Option<u32>,
    /// Precision constant for fixed-point math (typically 10^9).
    pub precision_constant: Option<u64>,
    /// Initial difficulty as hex string (matches Scala's `initialDifficultyHex`).
    /// Example: "011765000000" for mainnet.
    pub initial_difficulty_hex: Option<String>,
    /// Autolykos v2 activation height.
    pub autolykos_v2_activation_height: Option<u32>,
    /// Height at which N-growth schedule begins.
    pub n_increase_start: Option<u32>,
    /// N increases by 5% every this many blocks.
    pub n_increase_period: Option<u32>,
}

/// Network-specific consensus parameters.
///
/// Passed to difficulty calculation and other consensus functions.
/// NOT hardcoded - different networks (mainnet, testnet) have different values.
#[derive(Debug, Clone)]
pub struct ChainParams {
    /// Target block interval in milliseconds.
    pub block_interval_ms: u64,

    /// EIP-37 activation height (None = never activated).
    pub eip37_activation_height: Option<u32>,

    /// Epoch length before EIP-37 activation.
    pub pre_eip37_epoch_length: u32,

    /// Epoch length after EIP-37 activation.
    pub eip37_epoch_length: u32,

    /// Number of epochs used for difficulty regression.
    /// With this value = 8, we use (0..=8) = 9 epoch-boundary headers.
    pub use_last_epochs: u32,

    /// Precision constant for fixed-point arithmetic in difficulty calculation.
    /// Scala uses 10^9.
    pub precision_constant: u64,

    /// Initial difficulty for genesis block as BigUint.
    /// Parsed from hex string (Scala's `initialDifficultyHex`).
    ///
    /// NOTE: When implementing compact nBits encoding, you MUST match Scala's
    /// `DifficultySerializer.encodeCompactBits()` behavior, which uses Java
    /// `BigInteger.toByteArray()` semantics (includes leading 0x00 when MSB is set
    /// to distinguish from negative numbers).
    initial_difficulty: BigUint,

    /// Autolykos v2 activation height.
    pub autolykos_v2_activation_height: u32,

    /// Height at which N-growth schedule begins.
    pub n_increase_start: u32,

    /// N increases by 5% every this many blocks after n_increase_start.
    pub n_increase_period: u32,
}

impl ChainParams {
    /// Create mainnet parameters (stable, verified values).
    pub fn mainnet() -> Self {
        // Mainnet initialDifficultyHex from Scala: "011765000000"
        let initial_difficulty =
            BigUint::parse_bytes(b"011765000000", 16).expect("valid mainnet difficulty hex");

        Self {
            block_interval_ms: 120_000, // 2 minutes
            eip37_activation_height: Some(844_673),
            pre_eip37_epoch_length: 1024,
            eip37_epoch_length: 128,
            use_last_epochs: 8,
            precision_constant: 1_000_000_000, // 10^9
            initial_difficulty,
            autolykos_v2_activation_height: 417_792,
            n_increase_start: 614_400,  // 600 * 1024
            n_increase_period: 51_200,  // 50 * 1024
        }
    }

    /// Create ChainParams from configuration.
    ///
    /// Returns an error naming the specific field if any required field is missing or invalid.
    ///
    /// Note: `initial_difficulty_hex` is trimmed and accepts an optional "0x" prefix.
    pub fn from_config(config: &ChainParamsConfig) -> Result<Self, ChainParamsError> {
        let initial_difficulty_hex =
            config
                .initial_difficulty_hex
                .as_ref()
                .ok_or_else(|| ChainParamsError {
                    field: "initial_difficulty_hex",
                    message: "required field missing".to_string(),
                })?;

        // Hex hygiene: trim whitespace and strip optional 0x prefix
        let hex_cleaned = initial_difficulty_hex.trim();
        let hex_cleaned = hex_cleaned
            .strip_prefix("0x")
            .or_else(|| hex_cleaned.strip_prefix("0X"))
            .unwrap_or(hex_cleaned);

        let initial_difficulty =
            BigUint::parse_bytes(hex_cleaned.as_bytes(), 16).ok_or_else(|| ChainParamsError {
                field: "initial_difficulty_hex",
                message: format!("invalid hex string: '{}'", initial_difficulty_hex),
            })?;

        if initial_difficulty.is_zero() {
            return Err(ChainParamsError {
                field: "initial_difficulty_hex",
                message: "difficulty cannot be zero".to_string(),
            });
        }

        Ok(Self {
            block_interval_ms: config.block_interval_ms.ok_or_else(|| ChainParamsError {
                field: "block_interval_ms",
                message: "required field missing".to_string(),
            })?,
            eip37_activation_height: config.eip37_activation_height, // None is valid
            pre_eip37_epoch_length: config.pre_eip37_epoch_length.ok_or_else(|| {
                ChainParamsError {
                    field: "pre_eip37_epoch_length",
                    message: "required field missing".to_string(),
                }
            })?,
            eip37_epoch_length: config.eip37_epoch_length.ok_or_else(|| ChainParamsError {
                field: "eip37_epoch_length",
                message: "required field missing".to_string(),
            })?,
            use_last_epochs: config.use_last_epochs.ok_or_else(|| ChainParamsError {
                field: "use_last_epochs",
                message: "required field missing".to_string(),
            })?,
            precision_constant: config.precision_constant.ok_or_else(|| ChainParamsError {
                field: "precision_constant",
                message: "required field missing".to_string(),
            })?,
            initial_difficulty,
            autolykos_v2_activation_height: config
                .autolykos_v2_activation_height
                .ok_or_else(|| ChainParamsError {
                    field: "autolykos_v2_activation_height",
                    message: "required field missing".to_string(),
                })?,
            n_increase_start: config.n_increase_start.ok_or_else(|| ChainParamsError {
                field: "n_increase_start",
                message: "required field missing".to_string(),
            })?,
            n_increase_period: config.n_increase_period.ok_or_else(|| ChainParamsError {
                field: "n_increase_period",
                message: "required field missing".to_string(),
            })?,
        })
    }

    /// Get the initial difficulty as BigUint.
    pub fn initial_difficulty(&self) -> &BigUint {
        &self.initial_difficulty
    }

    /// Get the epoch length for a given height.
    ///
    /// # Arguments
    /// * `next_height` - The height of the block being validated (NOT parent height)
    pub fn epoch_length(&self, next_height: u32) -> u32 {
        match self.eip37_activation_height {
            Some(activation) if next_height >= activation => self.eip37_epoch_length,
            _ => self.pre_eip37_epoch_length,
        }
    }

    /// Check if EIP-37 is active at a given height.
    ///
    /// # Arguments
    /// * `next_height` - The height of the block being validated (NOT parent height)
    pub fn is_eip37_active(&self, next_height: u32) -> bool {
        self.eip37_activation_height
            .map(|activation| next_height >= activation)
            .unwrap_or(false)
    }

    /// Check if we're at an epoch boundary (difficulty recalculation point).
    ///
    /// # Arguments
    /// * `next_height` - The height of the block being validated
    ///
    /// Scala logic: `parentHeight % epochLength == 0`
    /// Which means: `(next_height - 1) % epochLength == 0`
    pub fn is_epoch_boundary(&self, next_height: u32) -> bool {
        if next_height == 0 {
            return false;
        }
        let parent_height = next_height - 1;
        let epoch_length = self.epoch_length(next_height);
        parent_height % epoch_length == 0
    }

    /// Get the heights of epoch-boundary headers needed for difficulty calculation.
    ///
    /// Returns up to `use_last_epochs + 1` heights (9 for mainnet with use_last_epochs=8).
    ///
    /// # Arguments
    /// * `next_height` - The height of the block being validated
    pub fn previous_heights_for_recalculation(&self, next_height: u32) -> Vec<u32> {
        if next_height == 0 {
            return vec![];
        }

        let parent_height = next_height - 1;
        let epoch_length = self.epoch_length(next_height);

        // Only recalculate at epoch boundaries
        if parent_height % epoch_length != 0 {
            return vec![parent_height];
        }

        // At epoch boundary: collect (0 to use_last_epochs) epoch boundaries
        // This is inclusive, so with use_last_epochs=8, we get indices 0..=8 (9 values)
        let mut heights: Vec<u32> = (0..=self.use_last_epochs)
            .filter_map(|i| {
                let h = parent_height as i64 - (i as i64 * epoch_length as i64);
                if h >= 0 {
                    Some(h as u32)
                } else {
                    None
                }
            })
            .collect();

        // Reverse to get chronological order (oldest first)
        heights.reverse();
        heights
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mainnet_params() {
        let params = ChainParams::mainnet();
        assert_eq!(params.block_interval_ms, 120_000);
        assert_eq!(params.eip37_activation_height, Some(844_673));
        assert_eq!(params.pre_eip37_epoch_length, 1024);
        assert_eq!(params.eip37_epoch_length, 128);
        assert_eq!(params.use_last_epochs, 8);
        // Verify initial difficulty matches Scala's "011765000000"
        assert_eq!(
            params.initial_difficulty(),
            &BigUint::parse_bytes(b"011765000000", 16).unwrap()
        );
    }

    #[test]
    fn test_from_config_missing_field_returns_error() {
        let config = ChainParamsConfig {
            block_interval_ms: Some(120_000),
            // Missing other required fields
            ..Default::default()
        };

        let result = ChainParams::from_config(&config);
        assert!(result.is_err());

        let err = result.unwrap_err();
        // initial_difficulty_hex is validated first in from_config()
        assert_eq!(err.field, "initial_difficulty_hex");
        assert!(err.message.contains("missing"));
    }

    #[test]
    fn test_from_config_invalid_hex_returns_error() {
        let config = ChainParamsConfig {
            block_interval_ms: Some(120_000),
            pre_eip37_epoch_length: Some(1024),
            eip37_epoch_length: Some(128),
            use_last_epochs: Some(8),
            precision_constant: Some(1_000_000_000),
            initial_difficulty_hex: Some("not_valid_hex".to_string()),
            autolykos_v2_activation_height: Some(417_792),
            n_increase_start: Some(614_400),
            n_increase_period: Some(51_200),
            ..Default::default()
        };

        let result = ChainParams::from_config(&config);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert_eq!(err.field, "initial_difficulty_hex");
        assert!(err.message.contains("invalid hex"));
    }

    #[test]
    fn test_from_config_zero_difficulty_returns_error() {
        let config = ChainParamsConfig {
            block_interval_ms: Some(120_000),
            pre_eip37_epoch_length: Some(1024),
            eip37_epoch_length: Some(128),
            use_last_epochs: Some(8),
            precision_constant: Some(1_000_000_000),
            initial_difficulty_hex: Some("00".to_string()),
            autolykos_v2_activation_height: Some(417_792),
            n_increase_start: Some(614_400),
            n_increase_period: Some(51_200),
            ..Default::default()
        };

        let result = ChainParams::from_config(&config);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert_eq!(err.field, "initial_difficulty_hex");
        assert!(err.message.contains("zero"));
    }

    #[test]
    fn test_from_config_error_names_field() {
        let full_config = ChainParamsConfig {
            block_interval_ms: Some(120_000),
            eip37_activation_height: None, // This one is allowed to be None
            pre_eip37_epoch_length: Some(1024),
            eip37_epoch_length: Some(128),
            use_last_epochs: Some(8),
            precision_constant: Some(1_000_000_000),
            initial_difficulty_hex: Some("011765000000".to_string()),
            autolykos_v2_activation_height: Some(417_792),
            n_increase_start: Some(614_400),
            n_increase_period: Some(51_200),
        };

        // Full config should succeed
        assert!(ChainParams::from_config(&full_config).is_ok());

        // Test each required field produces named error
        let mut config = full_config.clone();
        config.block_interval_ms = None;
        let err = ChainParams::from_config(&config).unwrap_err();
        assert_eq!(err.field, "block_interval_ms");

        let mut config = full_config.clone();
        config.initial_difficulty_hex = None;
        let err = ChainParams::from_config(&config).unwrap_err();
        assert_eq!(err.field, "initial_difficulty_hex");

        let mut config = full_config.clone();
        config.n_increase_period = None;
        let err = ChainParams::from_config(&config).unwrap_err();
        assert_eq!(err.field, "n_increase_period");
    }

    #[test]
    fn test_epoch_length_before_eip37() {
        let params = ChainParams::mainnet();
        assert_eq!(params.epoch_length(100_000), 1024);
        assert_eq!(params.epoch_length(844_672), 1024);
    }

    #[test]
    fn test_epoch_length_after_eip37() {
        let params = ChainParams::mainnet();
        assert_eq!(params.epoch_length(844_673), 128);
        assert_eq!(params.epoch_length(1_000_000), 128);
    }

    #[test]
    fn test_is_eip37_active() {
        let params = ChainParams::mainnet();
        assert!(!params.is_eip37_active(844_672));
        assert!(params.is_eip37_active(844_673));
        assert!(params.is_eip37_active(1_000_000));
    }

    #[test]
    fn test_is_epoch_boundary_pre_eip37() {
        let params = ChainParams::mainnet();
        assert!(params.is_epoch_boundary(1025)); // parent=1024
        assert!(params.is_epoch_boundary(2049)); // parent=2048
        assert!(!params.is_epoch_boundary(1024)); // parent=1023
        assert!(!params.is_epoch_boundary(1026)); // parent=1025
    }

    #[test]
    fn test_is_epoch_boundary_post_eip37() {
        let params = ChainParams::mainnet();
        assert!(params.is_epoch_boundary(844_801)); // parent=844_800, 844_800 % 128 == 0
        assert!(params.is_epoch_boundary(844_929)); // parent=844_928
        assert!(!params.is_epoch_boundary(844_802)); // parent=844_801
    }

    #[test]
    fn test_previous_heights_for_recalculation_at_boundary() {
        let params = ChainParams::mainnet();
        let heights = params.previous_heights_for_recalculation(1025);
        assert_eq!(heights, vec![0, 1024]);
    }

    #[test]
    fn test_previous_heights_for_recalculation_not_at_boundary() {
        let params = ChainParams::mainnet();
        let heights = params.previous_heights_for_recalculation(1026);
        assert_eq!(heights, vec![1025]);
    }

    #[test]
    fn test_previous_heights_for_recalculation_established_chain() {
        let params = ChainParams::mainnet();
        let heights = params.previous_heights_for_recalculation(9217);
        let expected: Vec<u32> = (0..=8u32).rev().map(|i| 9216 - i * 1024).collect();
        assert_eq!(heights.len(), 9);
        assert_eq!(heights, expected);
    }

    #[test]
    fn test_from_config_hex_hygiene() {
        // Whitespace and 0x prefix should be handled gracefully
        let config = ChainParamsConfig {
            block_interval_ms: Some(120_000),
            eip37_activation_height: Some(844_673),
            pre_eip37_epoch_length: Some(1024),
            eip37_epoch_length: Some(128),
            use_last_epochs: Some(8),
            precision_constant: Some(1_000_000_000),
            initial_difficulty_hex: Some(" 0x011765000000 ".to_string()),
            autolykos_v2_activation_height: Some(417_792),
            n_increase_start: Some(614_400),
            n_increase_period: Some(51_200),
        };

        let params = ChainParams::from_config(&config).expect("hex hygiene should work");
        assert_eq!(
            params.initial_difficulty(),
            &BigUint::parse_bytes(b"011765000000", 16).unwrap()
        );
    }

    #[test]
    fn test_eip37_none_means_never_active() {
        let config = ChainParamsConfig {
            block_interval_ms: Some(120_000),
            eip37_activation_height: None,
            pre_eip37_epoch_length: Some(1024),
            eip37_epoch_length: Some(128),
            use_last_epochs: Some(8),
            precision_constant: Some(1_000_000_000),
            initial_difficulty_hex: Some("01".to_string()),
            autolykos_v2_activation_height: Some(0),
            n_increase_start: Some(0),
            n_increase_period: Some(51_200),
        };

        let params = ChainParams::from_config(&config).unwrap();

        assert!(!params.is_eip37_active(0));
        assert!(!params.is_eip37_active(1_000_000));
        assert!(!params.is_eip37_active(u32::MAX));
        assert_eq!(params.epoch_length(1_000_000), 1024);
    }
}
