//! Error types for the mempool.

use thiserror::Error;

/// Mempool errors.
#[derive(Error, Debug)]
pub enum MempoolError {
    /// Transaction already exists in mempool.
    #[error("Transaction already in mempool: {0}")]
    AlreadyExists(String),

    /// Transaction validation failed.
    #[error("Transaction validation failed: {0}")]
    ValidationFailed(String),

    /// Double spend detected.
    #[error("Double spend detected: input {0} already spent")]
    DoubleSpend(String),

    /// Transaction too large.
    #[error("Transaction too large: {size} bytes, max {max} bytes")]
    TooLarge { size: usize, max: usize },

    /// Mempool full.
    #[error("Mempool full: {count} transactions, max {max}")]
    Full { count: usize, max: usize },

    /// Fee too low.
    #[error("Fee too low: {fee} nanoERG, minimum {min} nanoERG")]
    FeeTooLow { fee: u64, min: u64 },

    /// Transaction not found.
    #[error("Transaction not found: {0}")]
    NotFound(String),

    /// Input box not found.
    #[error("Input box not found: {0}")]
    InputNotFound(String),

    /// Consensus error.
    #[error("Consensus error: {0}")]
    Consensus(#[from] ergo_consensus::ConsensusError),

    /// State error.
    #[error("State error: {0}")]
    State(#[from] ergo_state::StateError),
}

/// Result type for mempool operations.
pub type MempoolResult<T> = Result<T, MempoolError>;
