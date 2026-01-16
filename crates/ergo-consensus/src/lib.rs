//! # ergo-consensus
//!
//! Consensus rules for the Ergo blockchain.
//!
//! This crate provides:
//! - Autolykos v2 Proof-of-Work verification
//! - Difficulty adjustment algorithm
//! - Block and header validation
//! - Extension validation and protocol parameter parsing
//!
//! ## Autolykos v2
//!
//! Autolykos v2 is a memory-hard Proof-of-Work algorithm designed for GPU mining.
//! Key parameters:
//! - N = 2^26 (table size, ~2GB memory)
//! - k = 32 (number of elements to sum)
//! - Uses BLAKE2b-256 for hashing
//!
//! ## Difficulty Adjustment
//!
//! Ergo uses a linear least squares regression over 8 epochs (8192 blocks)
//! to adjust difficulty, targeting 2-minute block times.

mod autolykos;
pub mod block;
pub mod block_validation;
mod chain_params;
pub mod cost;
mod difficulty;
mod eip37_difficulty;
mod error;
mod extension;
pub mod nipopow;
pub mod reemission;
pub mod tx_validation;
mod validation;

#[cfg(test)]
mod pow_test_vectors;

pub use autolykos::{
    difficulty_to_nbits, nbits_to_difficulty, nbits_to_target, validate_pow, AutolykosSolution,
    AutolykosV2,
};
pub use block::{
    ADProofs, BlockInfo, BlockStatus, BlockTransactions, FullBlock, ModifierId, ModifierType,
};
pub use block_validation::{
    BlockValidationResult, CreatedBox, FullBlockValidator, SpentBox, ValidatedStateChange,
};
pub use cost::{
    calculate_base_cost, estimate_tx_cost, CostAccumulator, CostConstants, CostError,
    TransactionCostResult,
};
pub use difficulty::{calculate_required_difficulty, DifficultyAdjustment, HeaderForDifficulty};
pub use error::{ConsensusError, ConsensusResult};
pub use params::AUTOLYKOS_V2_ACTIVATION_HEIGHT;
pub use chain_params::{ChainParams, ChainParamsConfig, ChainParamsError};
pub use eip37_difficulty::{Eip37DifficultyCalculator, EpochHeader};
pub use extension::{
    verify_extension_root, Extension, ExtensionField, ExtensionParseError, EMPTY_MERKLE_ROOT,
    FIELD_KEY_SIZE, FIELD_VALUE_MAX_SIZE, MAX_EXTENSION_SIZE,
};
pub use validation::{BlockValidator, HeaderValidator, TransactionValidator};

/// Ergo network parameters.
pub mod params {
    /// Target block interval in seconds (2 minutes).
    pub const BLOCK_INTERVAL_SECS: u64 = 120;

    /// Epoch length for difficulty adjustment (1024 blocks).
    pub const EPOCH_LENGTH: u32 = 1024;

    /// Number of epochs used for difficulty calculation (8 epochs = 8192 blocks).
    pub const DIFFICULTY_EPOCHS: u32 = 8;

    /// Maximum difficulty adjustment factor per epoch.
    pub const MAX_DIFFICULTY_CHANGE: f64 = 2.0;

    /// Autolykos v2 table size parameter (N = 2^26).
    pub const AUTOLYKOS_N: u32 = 67_108_864; // 2^26

    /// Autolykos v2 number of elements to sum (k = 32).
    pub const AUTOLYKOS_K: u32 = 32;

    /// Block version for Autolykos v2 (version 2+).
    pub const AUTOLYKOS_V2_VERSION: u8 = 2;

    /// Height at which the 5% N-growth schedule begins (mainnet).
    /// This is 600 * 1024 = 614,400.
    /// MAINNET ONLY. For testnet/devnet, this should be loaded from chain config.
    pub const AUTOLYKOS_N_V2_HARDFORK_HEIGHT: u32 = 600 * 1024;

    /// Alias for N-growth start height, derived from AUTOLYKOS_N_V2_HARDFORK_HEIGHT.
    pub const N_INCREASE_START: u32 = AUTOLYKOS_N_V2_HARDFORK_HEIGHT;

    /// N increases by 5% every this many blocks after N_INCREASE_START.
    /// This is 50 * 1024 = 51,200 blocks.
    /// MAINNET ONLY. For testnet/devnet, this should be loaded from chain config.
    pub const N_INCREASE_PERIOD: u32 = 50 * 1024;

    /// Maximum height at which N stops growing.
    /// MAINNET ONLY. For testnet/devnet, this should be loaded from chain config.
    pub const N_MAX_HEIGHT: u32 = 4_198_400;

    /// Autolykos v2 activation height (mainnet).
    /// This is MAINNET ONLY. For testnet/devnet, this should be loaded from chain config.
    /// If running testnet with checkpoints disabled, the v1 guardrail will trigger incorrectly.
    pub const AUTOLYKOS_V2_ACTIVATION_HEIGHT: u32 = 417_792;

    /// Maximum block size in bytes.
    pub const MAX_BLOCK_SIZE: usize = 1_048_576; // 1MB

    /// Maximum block cost (computational units).
    pub const MAX_BLOCK_COST: u64 = 8_000_000;

    /// Maximum transaction cost (computational units).
    ///
    /// Limits script complexity per transaction to prevent
    /// resource exhaustion attacks.
    pub const MAX_TX_COST: u64 = 1_000_000;

    /// Base cost per input (UTXO lookup + proof verification setup).
    pub const INPUT_BASE_COST: u64 = 2_000;

    /// Base cost per output (serialization + storage prep).
    pub const OUTPUT_BASE_COST: u64 = 100;

    /// Base cost per data input (read-only UTXO lookup).
    pub const DATA_INPUT_COST: u64 = 100;

    /// Cost per byte of transaction size.
    pub const SIZE_COST_PER_BYTE: u64 = 2;

    /// Storage rent period in blocks (~4 years).
    pub const STORAGE_RENT_PERIOD: u32 = 1_051_200;

    /// Minimum box value in nanoERG.
    pub const MIN_BOX_VALUE: u64 = 360;

    /// Emission delay (number of blocks before tokens can be spent from coinbase).
    pub const EMISSION_DELAY: u32 = 720;
}
