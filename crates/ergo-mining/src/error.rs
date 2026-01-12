//! Mining error types.

use thiserror::Error;

/// Mining errors.
#[derive(Error, Debug)]
pub enum MiningError {
    /// No reward address configured.
    #[error("No reward address configured")]
    NoRewardAddress,

    /// Invalid solution.
    #[error("Invalid solution: {0}")]
    InvalidSolution(String),

    /// Candidate generation failed.
    #[error("Candidate generation failed: {0}")]
    CandidateFailed(String),

    /// State error.
    #[error("State error: {0}")]
    State(#[from] ergo_state::StateError),

    /// Consensus error.
    #[error("Consensus error: {0}")]
    Consensus(#[from] ergo_consensus::ConsensusError),

    /// Mempool error.
    #[error("Mempool error: {0}")]
    Mempool(#[from] ergo_mempool::MempoolError),

    /// Other error.
    #[error("{0}")]
    Other(String),
}

/// Result type for mining operations.
pub type MiningResult<T> = Result<T, MiningError>;
