//! Re-emission rules and calculations (EIP-27).

use super::ReemissionSettings;

/// Number of nanoERG in one ERG.
pub const COINS_IN_ONE_ERG: u64 = 1_000_000_000;

/// Initial block reward in nanoERG (75 ERG).
pub const INITIAL_REWARD: u64 = 75 * COINS_IN_ONE_ERG;

/// Reward reduction per epoch in nanoERG (3 ERG).
pub const REWARD_REDUCTION: u64 = 3 * COINS_IN_ONE_ERG;

/// Blocks per reduction epoch (~3 months at 2 min/block).
pub const REDUCTION_PERIOD: u64 = 64_800;

/// Fixed rate period (first ~2 years).
pub const FIXED_RATE_PERIOD: u64 = 525_600;

/// Minimum block reward in nanoERG (3 ERG).
pub const MIN_REWARD: u64 = 3 * COINS_IN_ONE_ERG;

/// Total ERG supply in nanoERG (97,739,924 ERG).
pub const TOTAL_SUPPLY: u64 = 97_739_924 * COINS_IN_ONE_ERG;

/// Re-emission rules implementation.
///
/// Calculates emission and re-emission amounts according to EIP-27.
#[derive(Debug, Clone)]
pub struct ReemissionRules {
    /// Re-emission settings.
    settings: ReemissionSettings,

    /// Basic charge amount taken from emission to re-emission (12 ERG).
    basic_charge_amount: u64,

    /// Amount miner can claim per block from re-emission contract (3 ERG).
    reemission_reward_per_block: u64,
}

impl ReemissionRules {
    /// Create new re-emission rules with the given settings.
    pub fn new(settings: ReemissionSettings) -> Self {
        Self {
            settings,
            basic_charge_amount: 12 * COINS_IN_ONE_ERG,
            reemission_reward_per_block: 3 * COINS_IN_ONE_ERG,
        }
    }

    /// Get the settings.
    pub fn settings(&self) -> &ReemissionSettings {
        &self.settings
    }

    /// Get the basic charge amount (12 ERG by default).
    pub fn basic_charge_amount(&self) -> u64 {
        self.basic_charge_amount
    }

    /// Get the re-emission reward per block (3 ERG).
    pub fn reemission_reward_per_block(&self) -> u64 {
        self.reemission_reward_per_block
    }

    /// Calculate the standard emission at a given height.
    ///
    /// This is the original emission schedule before EIP-27 modifications.
    pub fn emission_at_height(&self, height: u64) -> u64 {
        if height == 0 {
            return 0;
        }

        if height <= FIXED_RATE_PERIOD {
            // First ~2 years: fixed 75 ERG
            INITIAL_REWARD
        } else {
            // After ~2 years: decreasing by 3 ERG every ~3 months
            let blocks_after_fixed = height - FIXED_RATE_PERIOD;
            let reduction_epochs = blocks_after_fixed / REDUCTION_PERIOD;
            let reduction = reduction_epochs * REWARD_REDUCTION;

            if reduction >= INITIAL_REWARD - MIN_REWARD {
                MIN_REWARD
            } else {
                INITIAL_REWARD - reduction
            }
        }
    }

    /// Calculate how much ERG is redirected to the re-emission contract at a given height.
    ///
    /// After activation, if emission >= 15 ERG (basic_charge + 3), redirect 12 ERG.
    /// If emission is between 3 and 15 ERG, redirect (emission - 3 ERG).
    /// If emission <= 3 ERG, redirect 0.
    pub fn reemission_for_height(&self, height: u32) -> u64 {
        let emission = self.emission_at_height(height as u64);
        let threshold = self.basic_charge_amount + MIN_REWARD;

        if height >= self.settings.activation_height && emission >= threshold {
            // Full charge: 12 ERG goes to re-emission
            self.basic_charge_amount
        } else if height >= self.settings.activation_height && emission > MIN_REWARD {
            // Partial charge: everything above minimum goes to re-emission
            emission - MIN_REWARD
        } else {
            // No re-emission contribution
            0
        }
    }

    /// Calculate the miner's reward at a given height (after EIP-27).
    ///
    /// This is the emission minus the re-emission contribution.
    pub fn miner_reward_at_height(&self, height: u32) -> u64 {
        let emission = self.emission_at_height(height as u64);
        let reemission = self.reemission_for_height(height);
        emission.saturating_sub(reemission)
    }

    /// Calculate how much a miner can claim from the re-emission contract.
    ///
    /// Returns 3 ERG per block after reemission_start_height.
    pub fn claimable_reemission_at_height(&self, height: u32) -> u64 {
        if height >= self.settings.reemission_start_height {
            self.reemission_reward_per_block
        } else {
            0
        }
    }

    /// Calculate the total miner income at a given height.
    ///
    /// This includes:
    /// - Direct mining reward (emission - re-emission contribution)
    /// - Claimable re-emission (3 ERG after reemission_start_height)
    pub fn total_miner_income_at_height(&self, height: u32) -> u64 {
        self.miner_reward_at_height(height) + self.claimable_reemission_at_height(height)
    }

    /// Calculate total coins issued up to and including a given height.
    ///
    /// Uses a closed-form calculation for O(1) performance.
    pub fn issued_coins_after_height(&self, height: u64) -> u64 {
        if height == 0 {
            return 0;
        }

        let total = if height <= FIXED_RATE_PERIOD {
            // All blocks at initial reward
            height * INITIAL_REWARD
        } else {
            // Fixed rate period contribution
            let fixed_period_total = FIXED_RATE_PERIOD * INITIAL_REWARD;

            // Decreasing rate period
            let blocks_after_fixed = height - FIXED_RATE_PERIOD;

            // Number of full reduction epochs completed
            // Each epoch is REDUCTION_PERIOD blocks, reward decreases by REWARD_REDUCTION
            let max_reduction_epochs = (INITIAL_REWARD - MIN_REWARD) / REWARD_REDUCTION; // 24 epochs
            let full_epochs =
                std::cmp::min(blocks_after_fixed / REDUCTION_PERIOD, max_reduction_epochs);

            // Blocks in the partial epoch (if any, before hitting minimum)
            let blocks_in_full_epochs = full_epochs * REDUCTION_PERIOD;
            let remaining_before_min = if full_epochs < max_reduction_epochs {
                std::cmp::min(blocks_after_fixed - blocks_in_full_epochs, REDUCTION_PERIOD)
            } else {
                0
            };

            // Blocks at minimum reward
            let blocks_at_minimum = blocks_after_fixed
                .saturating_sub(max_reduction_epochs * REDUCTION_PERIOD + remaining_before_min);

            // Sum of arithmetic series for full epochs:
            // Each epoch i (0-indexed) has reward = INITIAL_REWARD - i * REWARD_REDUCTION
            // Total for epoch i = REDUCTION_PERIOD * (INITIAL_REWARD - i * REWARD_REDUCTION)
            // Sum over epochs 0..n = REDUCTION_PERIOD * (n * INITIAL_REWARD - REWARD_REDUCTION * (0+1+...+(n-1)))
            //                      = REDUCTION_PERIOD * (n * INITIAL_REWARD - REWARD_REDUCTION * n*(n-1)/2)
            let full_epochs_total = if full_epochs > 0 {
                let n = full_epochs;
                REDUCTION_PERIOD * (n * INITIAL_REWARD - REWARD_REDUCTION * n * (n - 1) / 2)
            } else {
                0
            };

            // Partial epoch contribution (at the reward level after full_epochs reductions)
            let partial_epoch_reward = INITIAL_REWARD - full_epochs * REWARD_REDUCTION;
            let partial_epoch_total = remaining_before_min * partial_epoch_reward;

            // Minimum reward period contribution
            let minimum_total = blocks_at_minimum * MIN_REWARD;

            fixed_period_total + full_epochs_total + partial_epoch_total + minimum_total
        };

        std::cmp::min(total, TOTAL_SUPPLY)
    }
}

impl Default for ReemissionRules {
    fn default() -> Self {
        Self::new(ReemissionSettings::mainnet())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rules() -> ReemissionRules {
        ReemissionRules::new(ReemissionSettings::mainnet())
    }

    #[test]
    fn test_emission_at_height_0() {
        let rules = make_rules();
        assert_eq!(rules.emission_at_height(0), 0);
    }

    #[test]
    fn test_emission_first_block() {
        let rules = make_rules();
        assert_eq!(rules.emission_at_height(1), INITIAL_REWARD);
    }

    #[test]
    fn test_emission_fixed_period() {
        let rules = make_rules();
        assert_eq!(rules.emission_at_height(100), INITIAL_REWARD);
        assert_eq!(rules.emission_at_height(FIXED_RATE_PERIOD), INITIAL_REWARD);
    }

    #[test]
    fn test_emission_decreasing() {
        let rules = make_rules();

        // First block after fixed period still at 75 ERG
        assert_eq!(
            rules.emission_at_height(FIXED_RATE_PERIOD + 1),
            INITIAL_REWARD
        );

        // After one reduction period: 72 ERG
        assert_eq!(
            rules.emission_at_height(FIXED_RATE_PERIOD + REDUCTION_PERIOD + 1),
            INITIAL_REWARD - REWARD_REDUCTION
        );

        // After two reduction periods: 69 ERG
        assert_eq!(
            rules.emission_at_height(FIXED_RATE_PERIOD + 2 * REDUCTION_PERIOD + 1),
            INITIAL_REWARD - 2 * REWARD_REDUCTION
        );
    }

    #[test]
    fn test_emission_minimum() {
        let rules = make_rules();
        // Very late block should be at minimum
        assert_eq!(rules.emission_at_height(10_000_000), MIN_REWARD);
    }

    #[test]
    fn test_reemission_before_activation() {
        let rules = make_rules();
        // Before activation height, no re-emission
        assert_eq!(rules.reemission_for_height(777_216), 0);
    }

    #[test]
    fn test_reemission_after_activation() {
        let rules = make_rules();
        // After activation at height 777,217
        // Blocks after fixed period: 777,217 - 525,600 = 251,617
        // Reduction epochs: 251,617 / 64,800 = 3 (floor)
        // Emission: 75 - (3 * 3) = 66 ERG
        let height = 777_217;
        let emission = rules.emission_at_height(height as u64);
        assert_eq!(emission, 66 * COINS_IN_ONE_ERG);

        // Re-emission should be 12 ERG (since 66 >= 15)
        assert_eq!(rules.reemission_for_height(height), 12 * COINS_IN_ONE_ERG);
    }

    #[test]
    fn test_miner_reward_after_activation() {
        let rules = make_rules();
        let height = 777_217;

        // Emission at this height is 66 ERG, re-emission is 12 ERG
        // Miner gets 66 - 12 = 54 ERG
        assert_eq!(rules.miner_reward_at_height(height), 54 * COINS_IN_ONE_ERG);
    }

    #[test]
    fn test_reemission_when_emission_drops() {
        let rules = make_rules();

        // When emission is 15 ERG (basic_charge + min), still full charge
        // When emission is 12 ERG, redirect 9 ERG (12 - 3)
        // When emission is 6 ERG, redirect 3 ERG (6 - 3)
        // When emission is 3 ERG, redirect 0 ERG

        // At reemission_start_height (2,080,800), emission is 3 ERG
        let height = rules.settings.reemission_start_height;
        assert_eq!(rules.emission_at_height(height as u64), MIN_REWARD);
        assert_eq!(rules.reemission_for_height(height), 0);
    }

    #[test]
    fn test_claimable_reemission() {
        let rules = make_rules();

        // Before reemission start
        assert_eq!(rules.claimable_reemission_at_height(2_080_799), 0);

        // At and after reemission start
        assert_eq!(
            rules.claimable_reemission_at_height(2_080_800),
            3 * COINS_IN_ONE_ERG
        );
        assert_eq!(
            rules.claimable_reemission_at_height(3_000_000),
            3 * COINS_IN_ONE_ERG
        );
    }

    #[test]
    fn test_total_miner_income() {
        let rules = make_rules();

        // Before activation at height 777,216 (emission is ~66 ERG, no reemission deduction)
        let emission_before = rules.emission_at_height(777_216);
        assert_eq!(rules.total_miner_income_at_height(777_216), emission_before);

        // After activation but before reemission start: emission - reemission
        // Emission is 66 ERG, reemission is 12 ERG, miner gets 54 ERG
        assert_eq!(
            rules.total_miner_income_at_height(777_217),
            54 * COINS_IN_ONE_ERG
        );

        // After reemission start: min emission + claimable reemission (3 + 3 = 6 ERG)
        assert_eq!(
            rules.total_miner_income_at_height(2_080_800),
            6 * COINS_IN_ONE_ERG
        );
    }

    #[test]
    fn test_issued_coins() {
        let rules = make_rules();
        assert_eq!(rules.issued_coins_after_height(0), 0);
        assert_eq!(rules.issued_coins_after_height(1), INITIAL_REWARD);
        assert_eq!(
            rules.issued_coins_after_height(FIXED_RATE_PERIOD),
            FIXED_RATE_PERIOD * INITIAL_REWARD
        );
    }
}
