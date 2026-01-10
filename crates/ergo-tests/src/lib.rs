//! # ergo-tests
//!
//! Integration tests for the Ergo Rust Node.
//!
//! This crate provides comprehensive integration testing including:
//! - Sanity tests for basic node operations
//! - Storage tests for database operations
//! - Property-based tests for consensus rules
//! - State tests for UTXO management

pub mod generators;
pub mod harness;

#[cfg(test)]
mod sanity_tests;

#[cfg(test)]
mod storage_tests;

#[cfg(test)]
mod property_tests;

#[cfg(test)]
mod api_tests;

#[cfg(test)]
mod mining_tests;

#[cfg(test)]
mod sync_tests;

#[cfg(test)]
mod node_tests;

pub use generators::*;
pub use harness::*;
