//! Emission API handlers.
//!
//! Provides endpoints for querying emission schedule information:
//! - Emission at a specific height
//! - Emission-related scripts

use crate::ApiResult;
use axum::{extract::Path, Json};
use ergo_consensus::reemission::{ReemissionRules, ReemissionSettings};
use once_cell::sync::Lazy;
use serde::Serialize;
use utoipa::ToSchema;

/// Static mainnet re-emission rules (avoids re-creation on each request).
static MAINNET_RULES: Lazy<ReemissionRules> =
    Lazy::new(|| ReemissionRules::new(ReemissionSettings::mainnet()));

/// Static mainnet settings (for scripts endpoint).
static MAINNET_SETTINGS: Lazy<ReemissionSettings> = Lazy::new(ReemissionSettings::mainnet);

// ==================== Response Types ====================

/// Emission information at a specific height.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EmissionInfo {
    /// Block height.
    #[schema(example = 1000000)]
    pub height: u32,
    /// Miner reward at this height in nanoERG (after EIP-27 deductions).
    #[schema(example = 54000000000_u64)]
    pub miner_reward: u64,
    /// Total coins issued up to this height in nanoERG.
    #[schema(example = 75000000000000000_u64)]
    pub total_coins_issued: u64,
    /// Total remaining coins in emission contract in nanoERG.
    #[schema(example = 22500000000000000_u64)]
    pub total_remain_coins: u64,
    /// Re-emission amount (EIP-27) redirected to re-emission contract in nanoERG.
    #[schema(example = 12000000000_u64)]
    pub reemission_amt: u64,
}

/// Emission scripts information.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EmissionScripts {
    /// Emission box proposition address.
    #[schema(example = "emission_nft:...")]
    pub emission: String,
    /// Re-emission box proposition address (EIP-27).
    #[schema(example = "reemission_nft:...")]
    pub reemission: String,
    /// Pay-to-reemission address (EIP-27).
    #[schema(example = "reemission_token:...")]
    pub pay_to_reemission: String,
}

// ==================== Handlers ====================

/// GET /emission/at/:height
///
/// Get emission information at a specific block height including miner reward,
/// total coins issued, and re-emission amounts (EIP-27).
#[utoipa::path(
    get,
    path = "/emission/at/{height}",
    tag = "emission",
    params(
        ("height" = u32, Path, description = "Block height to query emission for")
    ),
    responses(
        (status = 200, description = "Emission information at the specified height", body = EmissionInfo)
    )
)]
pub async fn get_emission_at_height(Path(height): Path<u32>) -> ApiResult<Json<EmissionInfo>> {
    let info = calculate_emission_info(&MAINNET_RULES, height);
    Ok(Json(info))
}

/// GET /emission/scripts
///
/// Get emission-related script addresses (NFT IDs for emission and re-emission contracts).
#[utoipa::path(
    get,
    path = "/emission/scripts",
    tag = "emission",
    responses(
        (status = 200, description = "Emission script addresses", body = EmissionScripts)
    )
)]
pub async fn get_emission_scripts() -> ApiResult<Json<EmissionScripts>> {
    let settings = &*MAINNET_SETTINGS;

    // Format token IDs as addresses (hex-encoded)
    let scripts = EmissionScripts {
        emission: format!("emission_nft:{}", hex::encode(settings.emission_nft_id)),
        reemission: format!("reemission_nft:{}", hex::encode(settings.reemission_nft_id)),
        pay_to_reemission: format!(
            "reemission_token:{}",
            hex::encode(settings.reemission_token_id)
        ),
    };
    Ok(Json(scripts))
}

// ==================== Helper Functions ====================

/// Calculate emission information at a given height.
fn calculate_emission_info(rules: &ReemissionRules, height: u32) -> EmissionInfo {
    use ergo_consensus::reemission::TOTAL_SUPPLY;

    let miner_reward = rules.miner_reward_at_height(height);
    let total_coins_issued = rules.issued_coins_after_height(height as u64);
    let total_remain_coins = TOTAL_SUPPLY.saturating_sub(total_coins_issued);
    let reemission_amt = rules.reemission_for_height(height);

    EmissionInfo {
        height,
        miner_reward,
        total_coins_issued,
        total_remain_coins,
        reemission_amt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ergo_consensus::reemission::{COINS_IN_ONE_ERG, FIXED_RATE_PERIOD, INITIAL_REWARD};

    fn make_rules() -> ReemissionRules {
        ReemissionRules::new(ReemissionSettings::mainnet())
    }

    #[test]
    fn test_emission_info_early_block() {
        let rules = make_rules();
        let info = calculate_emission_info(&rules, 100);

        assert_eq!(info.height, 100);
        // Before activation, miner gets full emission
        assert_eq!(info.miner_reward, INITIAL_REWARD);
        assert_eq!(info.total_coins_issued, 100 * INITIAL_REWARD);
        assert_eq!(info.reemission_amt, 0);
    }

    #[test]
    fn test_emission_info_after_activation() {
        let rules = make_rules();
        let info = calculate_emission_info(&rules, 777_217);

        assert_eq!(info.height, 777_217);
        // After activation, 12 ERG goes to re-emission
        assert_eq!(info.reemission_amt, 12 * COINS_IN_ONE_ERG);
        // Miner gets emission - reemission
        assert_eq!(info.miner_reward, 54 * COINS_IN_ONE_ERG);
    }

    #[test]
    fn test_emission_info_at_reemission_start() {
        let rules = make_rules();
        let info = calculate_emission_info(&rules, 2_080_800);

        assert_eq!(info.height, 2_080_800);
        // At reemission start, emission is at minimum (3 ERG)
        assert_eq!(info.miner_reward, 3 * COINS_IN_ONE_ERG);
        // No more re-emission contribution when at minimum
        assert_eq!(info.reemission_amt, 0);
    }

    #[test]
    fn test_emission_info_fixed_period_boundary() {
        let rules = make_rules();
        let info = calculate_emission_info(&rules, FIXED_RATE_PERIOD as u32);

        assert_eq!(info.height, FIXED_RATE_PERIOD as u32);
        // At boundary, still full initial reward (before activation)
        assert_eq!(info.miner_reward, INITIAL_REWARD);
        assert_eq!(info.total_coins_issued, FIXED_RATE_PERIOD * INITIAL_REWARD);
    }
}
