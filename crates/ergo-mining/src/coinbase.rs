//! Coinbase transaction creation.
//!
//! The coinbase transaction is the first transaction in each block and creates
//! the block reward and collects transaction fees.
//!
//! In Ergo, the coinbase transaction is created by spending the emission box
//! and creating outputs for the miner reward and the new emission box.
//!
//! ## EIP-27 Re-emission
//!
//! After EIP-27 activation, a portion of the block reward is redirected to
//! the re-emission contract:
//! - 12 ERG per block goes to re-emission when emission >= 15 ERG
//! - After emission drops to 3 ERG, miners can claim 3 ERG/block from re-emission

use crate::{MiningError, MiningResult};
use ergo_consensus::reemission::{ReemissionRules, ReemissionSettings};
use ergo_lib::ergotree_ir::chain::address::{AddressEncoder, NetworkPrefix};
use ergo_lib::ergotree_ir::chain::ergo_box::{
    box_value::BoxValue, ErgoBoxCandidate, NonMandatoryRegisters,
};
use tracing::debug;

/// Emission schedule parameters (legacy - kept for compatibility).
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
    /// Calculate block reward for a given height (legacy method).
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

/// Coinbase transaction builder with EIP-27 re-emission support.
pub struct CoinbaseBuilder {
    /// Re-emission rules (includes emission schedule).
    reemission_rules: ReemissionRules,
    /// Network prefix for address parsing.
    network: NetworkPrefix,
}

impl Default for CoinbaseBuilder {
    fn default() -> Self {
        Self::new(NetworkPrefix::Mainnet)
    }
}

impl CoinbaseBuilder {
    /// Create a new coinbase builder with mainnet settings.
    pub fn new(network: NetworkPrefix) -> Self {
        let settings = match network {
            NetworkPrefix::Mainnet => ReemissionSettings::mainnet(),
            NetworkPrefix::Testnet => ReemissionSettings::testnet(),
        };
        Self {
            reemission_rules: ReemissionRules::new(settings),
            network,
        }
    }

    /// Create with custom re-emission settings.
    pub fn with_reemission(settings: ReemissionSettings, network: NetworkPrefix) -> Self {
        Self {
            reemission_rules: ReemissionRules::new(settings),
            network,
        }
    }

    /// Create with legacy emission parameters (for backward compatibility).
    ///
    /// **Deprecated**: Use `new()` or `with_reemission()` instead.
    /// This method ignores the `emission` parameter and uses the accurate
    /// EIP-27 re-emission rules.
    #[deprecated(
        since = "0.1.0",
        note = "Use CoinbaseBuilder::new() or CoinbaseBuilder::with_reemission() instead. \
                Legacy EmissionParams is an approximation; accurate EIP-27 rules are now used."
    )]
    pub fn with_emission(_emission: EmissionParams, network: NetworkPrefix) -> Self {
        Self::new(network)
    }

    /// Get the miner's direct reward at a given height.
    ///
    /// This is the emission minus any re-emission contribution (EIP-27).
    pub fn miner_reward_at_height(&self, height: u32) -> u64 {
        self.reemission_rules.miner_reward_at_height(height)
    }

    /// Get the re-emission contribution at a given height.
    ///
    /// This is the amount redirected to the re-emission contract (EIP-27).
    pub fn reemission_at_height(&self, height: u32) -> u64 {
        self.reemission_rules.reemission_for_height(height)
    }

    /// Get the claimable re-emission reward at a given height.
    ///
    /// After reemission_start_height, miners can claim 3 ERG/block from
    /// the re-emission contract.
    pub fn claimable_reemission_at_height(&self, height: u32) -> u64 {
        self.reemission_rules.claimable_reemission_at_height(height)
    }

    /// Get the total miner income at a given height (direct + claimable).
    pub fn total_miner_income_at_height(&self, height: u32) -> u64 {
        self.reemission_rules.total_miner_income_at_height(height)
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
    /// Note: This creates the direct miner reward. After EIP-27 activation,
    /// a separate re-emission output would be created with `reemission_at_height()`.
    pub fn build_reward_box(
        &self,
        height: u32,
        reward_address: &str,
        total_fees: u64,
    ) -> MiningResult<ErgoBoxCandidate> {
        // Calculate direct block reward (after EIP-27 deductions)
        let block_reward = self.miner_reward_at_height(height);
        let total_reward = block_reward + total_fees;

        let reemission_amount = self.reemission_at_height(height);

        debug!(
            height,
            block_reward,
            reemission_amount,
            total_fees,
            total_reward,
            "Building coinbase reward box"
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

    /// Calculate total reward for a block (direct reward + fees).
    ///
    /// Note: This does not include claimable re-emission.
    pub fn calculate_reward(&self, height: u32, total_fees: u64) -> u64 {
        self.miner_reward_at_height(height) + total_fees
    }

    /// Get the re-emission rules.
    pub fn reemission_rules(&self) -> &ReemissionRules {
        &self.reemission_rules
    }

    /// Legacy method - get emission parameters as approximation.
    pub fn emission(&self) -> EmissionParams {
        EmissionParams::default()
    }
}

/// Calculate the total reward for a block (direct miner reward + fees).
///
/// Uses EIP-27 rules to calculate the proper miner reward.
pub fn calculate_total_reward(height: u32, total_fees: u64) -> u64 {
    let rules = ReemissionRules::new(ReemissionSettings::mainnet());
    rules.miner_reward_at_height(height) + total_fees
}

/// Calculate the block reward at a given height (without fees).
///
/// This returns the miner's direct reward after EIP-27 deductions.
pub fn block_reward_at_height(height: u32) -> u64 {
    let rules = ReemissionRules::new(ReemissionSettings::mainnet());
    rules.miner_reward_at_height(height)
}

/// Calculate the raw emission at a given height (before EIP-27 deductions).
pub fn emission_at_height(height: u32) -> u64 {
    let rules = ReemissionRules::new(ReemissionSettings::mainnet());
    rules.emission_at_height(height as u64)
}

/// Calculate the re-emission amount at a given height.
pub fn reemission_at_height(height: u32) -> u64 {
    let rules = ReemissionRules::new(ReemissionSettings::mainnet());
    rules.reemission_for_height(height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ergo_consensus::reemission::COINS_IN_ONE_ERG;

    #[test]
    fn test_emission_schedule_early() {
        // Before EIP-27 activation, miner gets full emission
        let reward = block_reward_at_height(100);
        assert_eq!(reward, 75 * COINS_IN_ONE_ERG);
    }

    #[test]
    fn test_emission_schedule_after_activation() {
        // After EIP-27 activation (height 777,217+)
        // Emission is ~66 ERG, re-emission is 12 ERG, miner gets 54 ERG
        let reward = block_reward_at_height(777_217);
        assert_eq!(reward, 54 * COINS_IN_ONE_ERG);

        let reemission = reemission_at_height(777_217);
        assert_eq!(reemission, 12 * COINS_IN_ONE_ERG);
    }

    #[test]
    fn test_emission_at_reemission_start() {
        // At height 2,080,800, emission is at minimum (3 ERG)
        // No re-emission deduction when at minimum
        let reward = block_reward_at_height(2_080_800);
        assert_eq!(reward, 3 * COINS_IN_ONE_ERG);

        let reemission = reemission_at_height(2_080_800);
        assert_eq!(reemission, 0);
    }

    #[test]
    fn test_total_reward_with_fees() {
        let fees = 100_000_000u64; // 0.1 ERG

        // Early block: 75 ERG + fees
        assert_eq!(
            calculate_total_reward(100, fees),
            75 * COINS_IN_ONE_ERG + fees
        );

        // After activation: 54 ERG + fees
        assert_eq!(
            calculate_total_reward(777_217, fees),
            54 * COINS_IN_ONE_ERG + fees
        );
    }

    #[test]
    fn test_coinbase_builder() {
        let builder = CoinbaseBuilder::new(NetworkPrefix::Mainnet);

        // Test early block
        assert_eq!(builder.miner_reward_at_height(100), 75 * COINS_IN_ONE_ERG);
        assert_eq!(builder.reemission_at_height(100), 0);

        // Test after activation
        assert_eq!(
            builder.miner_reward_at_height(777_217),
            54 * COINS_IN_ONE_ERG
        );
        assert_eq!(builder.reemission_at_height(777_217), 12 * COINS_IN_ONE_ERG);

        // Test at reemission start
        assert_eq!(
            builder.claimable_reemission_at_height(2_080_800),
            3 * COINS_IN_ONE_ERG
        );
        assert_eq!(
            builder.total_miner_income_at_height(2_080_800),
            6 * COINS_IN_ONE_ERG
        );
    }
}
