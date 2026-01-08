//! Network error types.

use thiserror::Error;

/// Network errors.
#[derive(Error, Debug)]
pub enum NetworkError {
    /// Connection failed.
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// Connection closed.
    #[error("Connection closed")]
    ConnectionClosed,

    /// Handshake failed.
    #[error("Handshake failed: {0}")]
    HandshakeFailed(String),

    /// Invalid message.
    #[error("Invalid message: {0}")]
    InvalidMessage(String),

    /// Message too large.
    #[error("Message too large: {size} bytes, max {max} bytes")]
    MessageTooLarge { size: usize, max: usize },

    /// Peer not found.
    #[error("Peer not found: {0}")]
    PeerNotFound(String),

    /// Peer banned.
    #[error("Peer banned: {0}")]
    PeerBanned(String),

    /// Too many connections.
    #[error("Too many connections: {count}, max {max}")]
    TooManyConnections { count: usize, max: usize },

    /// Protocol version mismatch.
    #[error("Protocol version mismatch: got {got:?}, expected {expected:?}")]
    VersionMismatch {
        got: (u8, u8, u8),
        expected: (u8, u8, u8),
    },

    /// Network magic mismatch.
    #[error("Network magic mismatch")]
    MagicMismatch,

    /// Timeout.
    #[error("Timeout: {0}")]
    Timeout(String),

    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization error.
    #[error("Serialization error: {0}")]
    Serialization(String),
}

/// Result type for network operations.
pub type NetworkResult<T> = Result<T, NetworkError>;
