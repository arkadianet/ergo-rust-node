//! Scan storage and management (EIP-0001).
//!
//! This module provides scan registration, deregistration, and box tracking
//! for external applications following EIP-0001.

mod predicate;
mod storage;
mod types;

pub use predicate::ScanningPredicate;
pub use storage::ScanStorage;
pub use types::{Scan, ScanBox, ScanId, ScanRequest, ScanWalletInteraction};
