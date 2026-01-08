//! Wallet error types.

use thiserror::Error;

/// Wallet errors.
#[derive(Error, Debug)]
pub enum WalletError {
    /// Wallet not initialized.
    #[error("Wallet not initialized")]
    NotInitialized,

    /// Wallet locked.
    #[error("Wallet is locked")]
    Locked,

    /// Invalid password.
    #[error("Invalid password")]
    InvalidPassword,

    /// Invalid mnemonic.
    #[error("Invalid mnemonic: {0}")]
    InvalidMnemonic(String),

    /// Key derivation error.
    #[error("Key derivation error: {0}")]
    KeyDerivation(String),

    /// Insufficient funds.
    #[error("Insufficient funds: need {needed}, have {available}")]
    InsufficientFunds { needed: u64, available: u64 },

    /// Address not found.
    #[error("Address not found: {0}")]
    AddressNotFound(String),

    /// Transaction building error.
    #[error("Transaction building error: {0}")]
    TxBuildError(String),

    /// Signing error.
    #[error("Signing error: {0}")]
    SigningError(String),

    /// State error.
    #[error("State error: {0}")]
    State(#[from] ergo_state::StateError),

    /// IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type for wallet operations.
pub type WalletResult<T> = Result<T, WalletError>;
