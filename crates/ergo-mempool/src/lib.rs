//! # ergo-mempool
//!
//! Transaction mempool for the Ergo blockchain.
//!
//! This crate provides:
//! - Transaction storage with fee-based ordering
//! - Transaction validation before acceptance
//! - Double-spend detection
//! - Size limits and eviction policies
//! - Transaction dependency tracking

mod error;
mod ordering;
mod pool;

pub use error::{MempoolError, MempoolResult};
pub use ordering::FeeOrdering;
pub use pool::{Mempool, MempoolConfig, MempoolStats, PooledTransaction};

/// Default maximum mempool size in bytes.
pub const DEFAULT_MAX_SIZE: usize = 100 * 1024 * 1024; // 100 MB

/// Default maximum number of transactions.
pub const DEFAULT_MAX_TXS: usize = 10_000;

/// Default transaction expiry time in seconds.
pub const DEFAULT_TX_EXPIRY_SECS: u64 = 3600; // 1 hour
