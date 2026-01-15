//! Dynamic chain parameters parsed from block extensions.
//!
//! Ergo protocol parameters can be changed through miner voting.
//! Parameters are stored in block extensions and updated at epoch boundaries
//! (every 1024 blocks).
//!
//! This module provides:
//! - Parameter ID constants matching the Scala node
//! - ChainParameters struct holding current parameter values
//! - Parsing logic for extracting parameters from extensions

use std::collections::HashMap;

/// Parameter IDs as defined in Ergo protocol.
/// These match the Scala node's Parameters.scala definitions.
pub mod param_ids {
    /// Storage fee factor (nanoERG per byte per storage period).
    pub const STORAGE_FEE_FACTOR: u8 = 1;
    /// Minimum value per byte for box creation.
    pub const MIN_VALUE_PER_BYTE: u8 = 2;
    /// Maximum block size in bytes.
    pub const MAX_BLOCK_SIZE: u8 = 3;
    /// Maximum cumulative computational cost per block.
    pub const MAX_BLOCK_COST: u8 = 4;
    /// Cost per token access in scripts.
    pub const TOKEN_ACCESS_COST: u8 = 5;
    /// Base cost per input.
    pub const INPUT_COST: u8 = 6;
    /// Base cost per data input.
    pub const DATA_INPUT_COST: u8 = 7;
    /// Base cost per output.
    pub const OUTPUT_COST: u8 = 8;
}

/// Epoch length for parameter voting (1024 blocks).
pub const VOTING_EPOCH_LENGTH: u32 = 1024;

/// Dynamic protocol parameters parsed from block extensions.
///
/// These parameters can be changed through miner voting and are updated
/// at epoch boundaries (every 1024 blocks). The values here represent
/// the currently active parameters for block validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainParameters {
    /// Height at which these parameters became active.
    pub height: u32,
    /// Maximum cumulative computational cost per block.
    /// Used as the cost limit for transaction validation during block processing.
    pub max_block_cost: u64,
    /// Storage fee factor (nanoERG per byte per storage period).
    pub storage_fee_factor: u64,
    /// Minimum value per byte for box creation.
    pub min_value_per_byte: u64,
    /// Maximum block size in bytes.
    pub max_block_size: u64,
    /// Cost per token access in scripts.
    pub token_access_cost: u64,
    /// Base cost per input (UTXO lookup + proof verification setup).
    pub input_cost: u64,
    /// Base cost per data input (read-only UTXO lookup).
    pub data_input_cost: u64,
    /// Base cost per output (serialization + storage prep).
    pub output_cost: u64,
}

impl Default for ChainParameters {
    fn default() -> Self {
        // Genesis defaults matching Scala node's Parameters.DefaultParameters
        Self {
            height: 0,
            max_block_cost: 1_000_000,
            storage_fee_factor: 1_250_000,
            min_value_per_byte: 360,
            max_block_size: 524_288,
            token_access_cost: 100,
            input_cost: 2_000,
            data_input_cost: 100,
            output_cost: 100,
        }
    }
}

impl ChainParameters {
    /// Create parameters from parsed extension values.
    ///
    /// Uses previous parameters as fallback for any missing values.
    /// This allows partial updates where only changed parameters are
    /// included in the extension.
    pub fn from_extension(
        height: u32,
        parsed: &HashMap<u8, i32>,
        previous: &ChainParameters,
    ) -> Self {
        Self {
            height,
            max_block_cost: parsed
                .get(&param_ids::MAX_BLOCK_COST)
                .map(|&v| v as u64)
                .unwrap_or(previous.max_block_cost),
            storage_fee_factor: parsed
                .get(&param_ids::STORAGE_FEE_FACTOR)
                .map(|&v| v as u64)
                .unwrap_or(previous.storage_fee_factor),
            min_value_per_byte: parsed
                .get(&param_ids::MIN_VALUE_PER_BYTE)
                .map(|&v| v as u64)
                .unwrap_or(previous.min_value_per_byte),
            max_block_size: parsed
                .get(&param_ids::MAX_BLOCK_SIZE)
                .map(|&v| v as u64)
                .unwrap_or(previous.max_block_size),
            token_access_cost: parsed
                .get(&param_ids::TOKEN_ACCESS_COST)
                .map(|&v| v as u64)
                .unwrap_or(previous.token_access_cost),
            input_cost: parsed
                .get(&param_ids::INPUT_COST)
                .map(|&v| v as u64)
                .unwrap_or(previous.input_cost),
            data_input_cost: parsed
                .get(&param_ids::DATA_INPUT_COST)
                .map(|&v| v as u64)
                .unwrap_or(previous.data_input_cost),
            output_cost: parsed
                .get(&param_ids::OUTPUT_COST)
                .map(|&v| v as u64)
                .unwrap_or(previous.output_cost),
        }
    }

    /// Check if a given height is an epoch boundary where parameters may change.
    pub fn is_epoch_boundary(height: u32) -> bool {
        height > 0 && height % VOTING_EPOCH_LENGTH == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_parameters() {
        let params = ChainParameters::default();
        assert_eq!(params.height, 0);
        assert_eq!(params.max_block_cost, 1_000_000);
        assert_eq!(params.storage_fee_factor, 1_250_000);
        assert_eq!(params.min_value_per_byte, 360);
        assert_eq!(params.max_block_size, 524_288);
        assert_eq!(params.token_access_cost, 100);
        assert_eq!(params.input_cost, 2_000);
        assert_eq!(params.data_input_cost, 100);
        assert_eq!(params.output_cost, 100);
    }

    #[test]
    fn test_from_extension_partial_update() {
        let previous = ChainParameters::default();
        let mut parsed = HashMap::new();
        // Only update max_block_cost
        parsed.insert(param_ids::MAX_BLOCK_COST, 2_000_000i32);

        let updated = ChainParameters::from_extension(1024, &parsed, &previous);

        assert_eq!(updated.height, 1024);
        assert_eq!(updated.max_block_cost, 2_000_000);
        // Other values should remain unchanged
        assert_eq!(updated.storage_fee_factor, previous.storage_fee_factor);
        assert_eq!(updated.min_value_per_byte, previous.min_value_per_byte);
        assert_eq!(updated.input_cost, previous.input_cost);
    }

    #[test]
    fn test_from_extension_full_update() {
        let previous = ChainParameters::default();
        let mut parsed = HashMap::new();
        parsed.insert(param_ids::MAX_BLOCK_COST, 3_000_000i32);
        parsed.insert(param_ids::STORAGE_FEE_FACTOR, 2_000_000i32);
        parsed.insert(param_ids::MIN_VALUE_PER_BYTE, 500i32);
        parsed.insert(param_ids::MAX_BLOCK_SIZE, 1_000_000i32);
        parsed.insert(param_ids::TOKEN_ACCESS_COST, 200i32);
        parsed.insert(param_ids::INPUT_COST, 3_000i32);
        parsed.insert(param_ids::DATA_INPUT_COST, 150i32);
        parsed.insert(param_ids::OUTPUT_COST, 150i32);

        let updated = ChainParameters::from_extension(2048, &parsed, &previous);

        assert_eq!(updated.height, 2048);
        assert_eq!(updated.max_block_cost, 3_000_000);
        assert_eq!(updated.storage_fee_factor, 2_000_000);
        assert_eq!(updated.min_value_per_byte, 500);
        assert_eq!(updated.max_block_size, 1_000_000);
        assert_eq!(updated.token_access_cost, 200);
        assert_eq!(updated.input_cost, 3_000);
        assert_eq!(updated.data_input_cost, 150);
        assert_eq!(updated.output_cost, 150);
    }

    #[test]
    fn test_is_epoch_boundary() {
        assert!(!ChainParameters::is_epoch_boundary(0));
        assert!(!ChainParameters::is_epoch_boundary(1));
        assert!(!ChainParameters::is_epoch_boundary(1023));
        assert!(ChainParameters::is_epoch_boundary(1024));
        assert!(!ChainParameters::is_epoch_boundary(1025));
        assert!(ChainParameters::is_epoch_boundary(2048));
        assert!(ChainParameters::is_epoch_boundary(1024 * 100));
    }
}
