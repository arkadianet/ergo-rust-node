//! Error types for consensus validation.

use thiserror::Error;

/// Consensus validation errors.
#[derive(Error, Debug)]
pub enum ConsensusError {
    /// Invalid Proof-of-Work solution.
    #[error("Invalid PoW solution: {0}")]
    InvalidPow(String),

    /// Invalid block header.
    #[error("Invalid block header: {0}")]
    InvalidHeader(String),

    /// Invalid block structure.
    #[error("Invalid block: {0}")]
    InvalidBlock(String),

    /// Invalid transaction.
    #[error("Invalid transaction: {0}")]
    InvalidTransaction(String),

    /// Parent block not found.
    #[error("Parent block not found: {0}")]
    ParentNotFound(String),

    /// Invalid timestamp.
    #[error("Invalid timestamp: block {block_time}, expected after {parent_time}")]
    InvalidTimestamp { block_time: u64, parent_time: u64 },

    /// Invalid difficulty.
    #[error("Invalid difficulty: got {got}, expected {expected}")]
    InvalidDifficulty { got: String, expected: String },

    /// Block too large.
    #[error("Block too large: {size} bytes, max {max} bytes")]
    BlockTooLarge { size: usize, max: usize },

    /// Block cost exceeded.
    #[error("Block cost exceeded: {cost}, max {max}")]
    BlockCostExceeded { cost: u64, max: u64 },

    /// Invalid extension.
    #[error("Invalid extension: {0}")]
    InvalidExtension(String),

    /// Invalid state root.
    #[error("Invalid state root: got {got}, expected {expected}")]
    InvalidStateRoot { got: String, expected: String },

    /// Script execution failed.
    #[error("Script execution failed: {0}")]
    ScriptError(String),

    /// Insufficient funds.
    #[error("Insufficient funds: inputs {input_sum}, outputs {output_sum}")]
    InsufficientFunds { input_sum: u64, output_sum: u64 },

    /// Missing input box.
    #[error("Missing input for tx {tx_id}: input {input_idx} box {box_id} not found")]
    MissingInput {
        tx_id: String,
        input_idx: usize,
        box_id: String,
    },

    /// Missing data input box.
    #[error("Missing data input for tx {tx_id}: input {input_idx} box {box_id} not found")]
    MissingDataInput {
        tx_id: String,
        input_idx: usize,
        box_id: String,
    },

    /// Invalid token amount.
    #[error("Invalid token amount for {token_id}: input {input_amount}, output {output_amount}")]
    InvalidTokenAmount {
        token_id: String,
        input_amount: u64,
        output_amount: u64,
    },

    /// Double spend detected.
    #[error("Double spend detected: box {box_id} already spent")]
    DoubleSpend { box_id: String },

    /// Script verification failed.
    #[error("Script verification failed for tx {tx_id}: {error}")]
    ScriptVerificationFailed { tx_id: String, error: String },

    /// Insufficient fee.
    #[error("Insufficient fee: provided {provided}, required {required}")]
    InsufficientFee { provided: u64, required: u64 },

    /// Data input intersects with regular input.
    #[error("Data input {box_id} cannot also be a regular input")]
    DataInputIntersection { box_id: String },

    /// Box not found.
    #[error("Box not found: {0}")]
    BoxNotFound(String),

    /// Invalid token operation.
    #[error("Invalid token operation: {0}")]
    InvalidToken(String),

    /// Storage error.
    #[error("Storage error: {0}")]
    Storage(#[from] ergo_storage::StorageError),

    /// Generic validation error.
    #[error("Validation error: {0}")]
    Validation(String),
}

/// Result type for consensus operations.
pub type ConsensusResult<T> = Result<T, ConsensusError>;
