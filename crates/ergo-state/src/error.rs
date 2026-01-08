//! Error types for state management.

use thiserror::Error;

/// State management errors.
#[derive(Error, Debug)]
pub enum StateError {
    /// Storage error.
    #[error("Storage error: {0}")]
    Storage(#[from] ergo_storage::StorageError),

    /// Consensus error.
    #[error("Consensus error: {0}")]
    Consensus(#[from] ergo_consensus::ConsensusError),

    /// Box not found.
    #[error("Box not found: {0}")]
    BoxNotFound(String),

    /// Block not found.
    #[error("Block not found: {0}")]
    BlockNotFound(String),

    /// Header not found.
    #[error("Header not found: {0}")]
    HeaderNotFound(String),

    /// Invalid state transition.
    #[error("Invalid state transition: {0}")]
    InvalidTransition(String),

    /// Rollback failed.
    #[error("Rollback failed: {0}")]
    RollbackFailed(String),

    /// Snapshot not found.
    #[error("Snapshot not found: height {0}")]
    SnapshotNotFound(u32),

    /// State root mismatch.
    #[error("State root mismatch at height {height}: expected {expected}, computed {computed}")]
    StateRootMismatch {
        height: u32,
        expected: String,
        computed: String,
    },

    /// Serialization error.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// AVL tree error.
    #[error("AVL tree error: {0}")]
    AvlTree(String),
}

/// Result type for state operations.
pub type StateResult<T> = Result<T, StateError>;
