//! Error types for the storage layer.

use thiserror::Error;

/// Storage-specific errors.
#[derive(Error, Debug)]
pub enum StorageError {
    /// RocksDB error.
    #[error("Database error: {0}")]
    Database(#[from] rocksdb::Error),

    /// Serialization error.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Deserialization error.
    #[error("Deserialization error: {0}")]
    Deserialization(String),

    /// Column family not found.
    #[error("Column family not found: {0}")]
    ColumnFamilyNotFound(String),

    /// Key not found.
    #[error("Key not found")]
    KeyNotFound,

    /// Corruption detected.
    #[error("Data corruption detected: {0}")]
    Corruption(String),

    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type for storage operations.
pub type StorageResult<T> = Result<T, StorageError>;
