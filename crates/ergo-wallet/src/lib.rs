//! # ergo-wallet
//!
//! Wallet functionality for the Ergo blockchain.
//!
//! This crate provides:
//! - HD wallet (BIP32/BIP39) following EIP-3
//! - Address generation and derivation
//! - Box tracking and balance calculation
//! - Transaction building and signing
//! - Encrypted key storage
//!
//! ## Example
//!
//! ```ignore
//! use ergo_wallet::{Wallet, WalletConfig};
//! use ergo_state::StateManager;
//! use std::sync::Arc;
//!
//! // Create wallet with default config
//! let wallet = Wallet::new(WalletConfig::default(), state);
//!
//! // Initialize from mnemonic
//! wallet.init(
//!     "slow silly start wash bundle suffer bulb ancient height spin express remind today effort helmet",
//!     "",  // no BIP39 passphrase
//!     "encryption_password"
//! ).unwrap();
//!
//! // Get addresses
//! let addresses = wallet.addresses().unwrap();
//! println!("First address: {}", addresses[0]);
//!
//! // Generate new address
//! let new_addr = wallet.new_address().unwrap();
//! ```

mod error;
mod keystore;
mod tracker;
mod tx_builder;
mod wallet;

pub use error::{WalletError, WalletResult};
pub use keystore::{DerivedKey, EncryptedKey, KeyStore};
pub use tracker::{BoxTracker, WalletBalance, WalletBox};
pub use tx_builder::{
    create_send_tx, sign_tx, PaymentRequest, TxBuildError, WalletTxBuilder, DEFAULT_FEE,
    MIN_BOX_VALUE,
};
pub use wallet::{Wallet, WalletConfig, WalletStatus};

// Re-export useful types from ergo-lib
pub use ergo_lib::ergotree_ir::chain::address::{Address, NetworkPrefix};
pub use ergo_lib::wallet::derivation_path::DerivationPath;
pub use ergo_lib::wallet::mnemonic::{Mnemonic, MnemonicSeed};
pub use ergo_lib::wallet::secret_key::SecretKey;

/// Default derivation path for Ergo (EIP-3).
/// Format: m/44'/429'/{account}'/0/{address_index}
pub const DEFAULT_DERIVATION_PATH: &str = "m/44'/429'/0'/0";

/// Ergo's BIP44 coin type (registered).
pub const ERGO_COIN_TYPE: u32 = 429;

/// Address types supported by Ergo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressType {
    /// Pay-to-public-key (P2PK) - most common for wallets.
    P2PK,
    /// Pay-to-script-hash (P2SH).
    P2SH,
    /// Pay-to-script (P2S) - for complex contracts.
    P2S,
}
