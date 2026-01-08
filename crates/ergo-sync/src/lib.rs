//! # ergo-sync
//!
//! Block synchronization for the Ergo blockchain.
//!
//! This crate provides:
//! - Header-first synchronization strategy
//! - Block download scheduling
//! - Parallel block fetching
//! - Chain reorganization handling
//! - UTXO snapshot bootstrap support

mod download;
mod error;
mod protocol;
mod sync;

pub use download::{BlockDownloader, DownloadConfig, DownloadStats, DownloadTask};
pub use error::{SyncError, SyncResult};
pub use protocol::{SyncCommand, SyncEvent, SyncProtocol};
pub use sync::{SyncConfig, SyncState, Synchronizer};

/// Number of headers to request at once.
pub const HEADERS_BATCH_SIZE: usize = 100;

/// Number of blocks to download in parallel.
pub const PARALLEL_DOWNLOADS: usize = 16;

/// Maximum blocks to keep in download queue.
pub const MAX_DOWNLOAD_QUEUE: usize = 1000;
