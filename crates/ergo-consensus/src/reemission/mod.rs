//! Re-emission support (EIP-27).
//!
//! This module implements the re-emission mechanism defined in EIP-27,
//! which provides long-term sustainability for Ergo by redirecting
//! a portion of mining rewards to a re-emission contract after the
//! initial emission ends.
//!
//! ## Key Concepts
//!
//! - **Re-emission**: After block 2,080,800 (when regular emission drops to 3 ERG),
//!   12 ERG from each block reward is redirected to a re-emission contract
//! - **Re-emission Contract**: Holds funds that can be unlocked by miners at 3 ERG/block
//! - **Pay-to-Reemission Contract**: Allows merging boxes into the re-emission contract
//!
//! ## References
//!
//! - [EIP-27](https://github.com/ergoplatform/eips/blob/master/eip-0027.md)

mod rules;
mod settings;

pub use rules::{
    ReemissionRules, COINS_IN_ONE_ERG, FIXED_RATE_PERIOD, INITIAL_REWARD, MIN_REWARD,
    REDUCTION_PERIOD, REWARD_REDUCTION, TOTAL_SUPPLY,
};
pub use settings::ReemissionSettings;
