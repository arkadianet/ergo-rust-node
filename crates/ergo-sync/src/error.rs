//! Sync error types.

use thiserror::Error;

/// Sync errors.
#[derive(Error, Debug)]
pub enum SyncError {
    /// Network error.
    #[error("Network error: {0}")]
    Network(#[from] ergo_network::NetworkError),

    /// State error.
    #[error("State error: {0}")]
    State(#[from] ergo_state::StateError),

    /// Consensus error.
    #[error("Consensus error: {0}")]
    Consensus(#[from] ergo_consensus::ConsensusError),

    /// No peers available.
    #[error("No peers available for sync")]
    NoPeers,

    /// Sync stalled.
    #[error("Sync stalled: {0}")]
    Stalled(String),

    /// Invalid chain.
    #[error("Invalid chain: {0}")]
    InvalidChain(String),

    /// Invalid data received.
    #[error("Invalid data: {0}")]
    InvalidData(String),

    /// Download failed.
    #[error("Download failed: {0}")]
    DownloadFailed(String),

    /// Internal error.
    #[error("Internal error: {0}")]
    Internal(String),

    /// Timeout.
    #[error("Sync timeout")]
    Timeout,
}

/// Result type for sync operations.
pub type SyncResult<T> = Result<T, SyncError>;
