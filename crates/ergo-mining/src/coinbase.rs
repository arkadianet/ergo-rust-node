//! Coinbase transaction creation.
//!
//! The coinbase transaction is the first transaction in each block and creates
//! the block reward and collects transaction fees.
//!
//! In Ergo, the coinbase transaction is created by spending the emission box
//! and creating outputs for the miner reward and the new emission box.

use crate::{MiningError, MiningResult};
use ergo_lib::ergotree_ir::chain::address::{AddressEncoder, NetworkPrefix};
use ergo_lib::ergotree_ir::chain::ergo_box::{
    box_value::BoxValue, ErgoBoxCandidate, NonMandatoryRegisters,
};
use tracing::debug;

/// Emission schedule parameters.
pub struct EmissionParams {
    /// Initial reward in nanoERG.
    pub initial_reward: u64,
    /// Blocks per year (~2 min blocks).
    pub blocks_per_year: u32,
    /// Years before emission reduction starts.
    pub fixed_rate_years: u32,
    /// Yearly reduction in nanoERG.
    pub yearly_reduction: u64,
    /// Minimum reward in nanoERG.
    pub min_reward: u64,
}

impl Default for EmissionParams {
    fn default() -> Self {
        let erg = 1_000_000_000u64; // nanoERG per ERG
        Self {
            initial_reward: 75 * erg,
            blocks_per_year: 262_800, // ~2 min blocks
            fixed_rate_years: 2,
            yearly_reduction: 3 * erg,
            min_reward: 3 * erg,
        }
    }
}

impl EmissionParams {
    /// Calculate block reward for a given height.
    pub fn reward_at_height(&self, height: u32) -> u64 {
        let fixed_blocks = self.fixed_rate_years * self.blocks_per_year;

        if height <= fixed_blocks {
            self.initial_reward
        } else {
            let years_after_fixed = (height - fixed_blocks) / self.blocks_per_year;
            let reduction = years_after_fixed as u64 * self.yearly_reduction;
            self.initial_reward
                .saturating_sub(reduction)
                .max(self.min_reward)
        }
    }
}

/// Coinbase transaction builder.
pub struct CoinbaseBuilder {
    /// Emission parameters.
    emission: EmissionParams,
    /// Network prefix for address parsing.
    network: NetworkPrefix,
}

impl Default for CoinbaseBuilder {
    fn default() -> Self {
        Self::new(NetworkPrefix::Mainnet)
    }
}

impl CoinbaseBuilder {
    /// Create a new coinbase builder with default emission parameters.
    pub fn new(network: NetworkPrefix) -> Self {
        Self {
            emission: EmissionParams::default(),
            network,
        }
    }

    /// Create with custom emission parameters.
    pub fn with_emission(emission: EmissionParams, network: NetworkPrefix) -> Self {
        Self { emission, network }
    }

    /// Build a coinbase reward box candidate.
    ///
    /// # Arguments
    /// * `height` - Block height
    /// * `reward_address` - Address to receive the reward (base58 encoded)
    /// * `total_fees` - Sum of all transaction fees in the block
    ///
    /// # Returns
    /// An ErgoBoxCandidate for the miner reward output.
    ///
    /// Note: The full coinbase transaction requires spending the emission box,
    /// which is handled by the emission contract. This method creates the
    /// miner's reward output box.
    pub fn build_reward_box(
        &self,
        height: u32,
        reward_address: &str,
        total_fees: u64,
    ) -> MiningResult<ErgoBoxCandidate> {
        // Calculate block reward
        let block_reward = self.emission.reward_at_height(height);
        let total_reward = block_reward + total_fees;

        debug!(
            height,
            block_reward, total_fees, total_reward, "Building coinbase reward box"
        );

        // Parse reward address to get ErgoTree
        let encoder = AddressEncoder::new(self.network);
        let address = encoder
            .parse_address_from_str(reward_address)
            .map_err(|e| MiningError::CandidateFailed(format!("Invalid reward address: {}", e)))?;

        let ergo_tree = address
            .script()
            .map_err(|e| MiningError::CandidateFailed(format!("Failed to get script: {}", e)))?;

        // Create the coinbase output box
        let box_value = BoxValue::try_from(total_reward)
            .map_err(|e| MiningError::CandidateFailed(format!("Invalid box value: {}", e)))?;

        let output = ErgoBoxCandidate {
            value: box_value,
            ergo_tree,
            tokens: None,
            additional_registers: NonMandatoryRegisters::empty(),
            creation_height: height,
        };

        Ok(output)
    }

    /// Calculate total reward for a block (block reward + fees).
    pub fn calculate_reward(&self, height: u32, total_fees: u64) -> u64 {
        self.emission.reward_at_height(height) + total_fees
    }

    /// Get the emission parameters.
    pub fn emission(&self) -> &EmissionParams {
        &self.emission
    }
}

/// Calculate the total reward for a block.
pub fn calculate_total_reward(height: u32, total_fees: u64) -> u64 {
    let emission = EmissionParams::default();
    emission.reward_at_height(height) + total_fees
}

/// Calculate the block reward at a given height (without fees).
pub fn block_reward_at_height(height: u32) -> u64 {
    EmissionParams::default().reward_at_height(height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emission_schedule() {
        let emission = EmissionParams::default();
        let erg = 1_000_000_000u64;
        let blocks_per_year = 262_800u32;

        // First 2 years (blocks 1 to 525,600)
        assert_eq!(emission.reward_at_height(1), 75 * erg);
        assert_eq!(emission.reward_at_height(blocks_per_year), 75 * erg);
        assert_eq!(emission.reward_at_height(2 * blocks_per_year), 75 * erg);

        // After 2 years, reduction starts
        // Block 525,601 is first block after fixed period
        // But reduction is calculated by complete years after the fixed period
        // So blocks 525,601 to 788,400 (year 3) still get 75 ERG
        // Year 4 (blocks 788,401+) gets 72 ERG (reduced by 3)
        assert_eq!(emission.reward_at_height(2 * blocks_per_year + 1), 75 * erg);

        // After one full year past fixed period (3 years total)
        // years_after_fixed = (788401 - 525600) / 262800 = 1
        // reduction = 1 * 3 ERG = 3 ERG
        // reward = 75 - 3 = 72 ERG
        assert_eq!(emission.reward_at_height(3 * blocks_per_year + 1), 72 * erg);

        // Much later - minimum reward
        assert_eq!(emission.reward_at_height(100_000_000), 3 * erg);
    }

    #[test]
    fn test_total_reward() {
        let erg = 1_000_000_000u64;
        let fees = 100_000_000u64; // 0.1 ERG in fees

        assert_eq!(calculate_total_reward(1, fees), 75 * erg + fees);
    }
}
