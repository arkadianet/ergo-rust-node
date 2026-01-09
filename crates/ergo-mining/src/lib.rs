//! # ergo-mining
//!
//! Mining support for the Ergo blockchain.
//!
//! This crate provides:
//! - Block candidate generation
//! - Transaction selection by fee
//! - Coinbase transaction creation
//! - External miner protocol (stratum-like)
//! - Solution verification and submission

mod candidate;
mod coinbase;
mod error;
mod miner;

pub use candidate::{BlockCandidate, CandidateGenerator};
pub use coinbase::{
    block_reward_at_height, calculate_total_reward, emission_at_height, reemission_at_height,
    CoinbaseBuilder, EmissionParams,
};
pub use ergo_lib::ergotree_ir::chain::address::NetworkPrefix;
pub use error::{MiningError, MiningResult};
pub use miner::{Miner, MinerConfig};

/// Default mining reward address (placeholder).
pub const DEFAULT_REWARD_ADDRESS: &str = "";

/// Maximum transactions per block.
pub const MAX_TRANSACTIONS_PER_BLOCK: usize = 1000;

/// Coinbase maturity (blocks before coinbase can be spent).
pub const COINBASE_MATURITY: u32 = 720;
