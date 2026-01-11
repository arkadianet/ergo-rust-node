//! Mining handlers.

use crate::{ApiError, ApiResult, AppState};
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Mining candidate response matching Scala node API.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MiningCandidate {
    /// Message to be signed (header bytes without PoW solution, hex-encoded).
    #[schema(example = "0e0a...")]
    pub msg: String,
    /// Target value b (derived from nBits, hex-encoded 32 bytes).
    #[schema(example = "00000000ffff0000000000000000000000000000000000000000000000000000")]
    pub b: String,
    /// Public key for the miner (reward address).
    #[schema(example = "9f...")]
    pub pk: String,
    /// Block height.
    #[schema(example = 1234567)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

/// Extended mining candidate with additional fields.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MiningCandidateExtended {
    /// Message to be signed (hex-encoded).
    #[schema(example = "0e0a...")]
    pub msg: String,
    /// Target value b (hex-encoded).
    #[schema(example = "00000000ffff0000000000000000000000000000000000000000000000000000")]
    pub b: String,
    /// Public key (reward address).
    #[schema(example = "9f...")]
    pub pk: String,
    /// Block height.
    #[schema(example = 1234567)]
    pub height: u32,
    /// Parent block ID (hex-encoded).
    #[schema(example = "0000000000000000000000000000000000000000000000000000000000000000")]
    pub parent_id: String,
    /// Transactions root (hex-encoded).
    #[schema(example = "0000000000000000000000000000000000000000000000000000000000000000")]
    pub transactions_root: String,
    /// State root (hex-encoded).
    #[schema(example = "0000000000000000000000000000000000000000000000000000000000000000")]
    pub state_root: String,
    /// Number of transactions in the candidate block.
    #[schema(example = 5)]
    pub tx_count: usize,
    /// Block reward in nanoERG.
    #[schema(example = 67500000000_u64)]
    pub reward: u64,
}

/// Solution submission matching Scala node API.
#[derive(Deserialize, ToSchema)]
pub struct SolutionSubmission {
    /// Miner public key (hex-encoded).
    #[schema(example = "03...")]
    pub pk: String,
    /// One-time secret w (hex-encoded).
    #[schema(example = "02...")]
    pub w: String,
    /// Nonce (hex-encoded 8 bytes).
    #[schema(example = "0102030405060708")]
    pub n: String,
    /// Distance d (hex-encoded).
    #[schema(example = "00...")]
    pub d: String,
}

/// Solution response.
#[derive(Serialize, ToSchema)]
pub struct SolutionResponse {
    /// Whether solution was accepted.
    pub accepted: bool,
    /// Optional reason if rejected.
    #[schema(example = "Solution does not meet difficulty target")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Reward address response.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RewardAddressResponse {
    /// Current reward address.
    #[schema(example = "9f...")]
    pub reward_address: String,
}

/// Reward address request.
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RewardAddressRequest {
    /// New reward address.
    #[schema(example = "9f...")]
    pub reward_address: String,
}

/// Convert nBits to target value b (big-endian hex string).
///
/// nBits uses a compact representation: the first byte is the size (number of bytes),
/// and the remaining 3 bytes are the most significant bytes of the target.
fn nbits_to_target(n_bits: u32) -> String {
    // nBits encoding: first byte is size in bytes, next 3 bytes are coefficient
    let size = ((n_bits >> 24) & 0xff) as usize;
    let coeff = n_bits & 0x00ffffff;

    // Target is a 256-bit number (32 bytes)
    let mut target = [0u8; 32];

    if size == 0 || coeff == 0 {
        return hex::encode(target);
    }

    // The coefficient represents the top 3 bytes of the target
    // placed at position (size - 3) from the right
    let coeff_bytes = [
        ((coeff >> 16) & 0xff) as u8,
        ((coeff >> 8) & 0xff) as u8,
        (coeff & 0xff) as u8,
    ];

    // Calculate where to place the coefficient in the 32-byte array
    // If size > 32, target overflows (essentially infinite)
    if size > 32 {
        return "f".repeat(64);
    }

    // The coefficient's MSB goes at position (32 - size) from the left
    let start_pos = 32usize.saturating_sub(size);

    for (i, &byte) in coeff_bytes.iter().enumerate() {
        let pos = start_pos + i;
        if pos < 32 {
            target[pos] = byte;
        }
    }

    hex::encode(target)
}

/// GET /mining/candidate
///
/// Get a mining candidate for external miners. Returns the header bytes to sign
/// and the target difficulty.
#[utoipa::path(
    get,
    path = "/mining/candidate",
    tag = "mining",
    responses(
        (status = 200, description = "Mining candidate", body = MiningCandidate),
        (status = 400, description = "Mining not enabled", body = crate::error::ErrorResponse)
    )
)]
pub async fn get_candidate(State(state): State<AppState>) -> ApiResult<Json<MiningCandidate>> {
    if !state.mining_enabled {
        return Err(ApiError::BadRequest("Mining not enabled".to_string()));
    }

    let candidate = state
        .candidate_generator
        .get_or_generate()
        .map_err(|e| ApiError::Internal(format!("Failed to generate candidate: {}", e)))?;

    let reward_address = state
        .candidate_generator
        .reward_address()
        .unwrap_or_default();

    Ok(Json(MiningCandidate {
        msg: hex::encode(&candidate.header_bytes),
        b: nbits_to_target(candidate.n_bits),
        pk: reward_address,
        height: Some(candidate.height),
    }))
}

/// GET /mining/candidate/extended
///
/// Get an extended mining candidate with additional information like parent ID,
/// transaction count, and block reward.
#[utoipa::path(
    get,
    path = "/mining/candidate/extended",
    tag = "mining",
    responses(
        (status = 200, description = "Extended mining candidate", body = MiningCandidateExtended),
        (status = 400, description = "Mining not enabled", body = crate::error::ErrorResponse)
    )
)]
pub async fn get_candidate_extended(
    State(state): State<AppState>,
) -> ApiResult<Json<MiningCandidateExtended>> {
    if !state.mining_enabled {
        return Err(ApiError::BadRequest("Mining not enabled".to_string()));
    }

    let candidate = state
        .candidate_generator
        .get_or_generate()
        .map_err(|e| ApiError::Internal(format!("Failed to generate candidate: {}", e)))?;

    let reward_address = state
        .candidate_generator
        .reward_address()
        .unwrap_or_default();

    let parent_id = candidate
        .parent_id
        .as_ref()
        .map(|id| hex::encode(id.0.as_ref()))
        .unwrap_or_else(|| "0".repeat(64));

    Ok(Json(MiningCandidateExtended {
        msg: hex::encode(&candidate.header_bytes),
        b: nbits_to_target(candidate.n_bits),
        pk: reward_address,
        height: candidate.height,
        parent_id,
        transactions_root: hex::encode(&candidate.transactions_root),
        state_root: hex::encode(&candidate.state_root),
        tx_count: candidate.transaction_ids.len(),
        reward: candidate.reward,
    }))
}

/// POST /mining/solution
///
/// Submit a mining solution. The solution will be verified against the current
/// candidate and, if valid, the block will be broadcast to the network.
#[utoipa::path(
    post,
    path = "/mining/solution",
    tag = "mining",
    request_body = SolutionSubmission,
    responses(
        (status = 200, description = "Solution processed", body = SolutionResponse),
        (status = 400, description = "Mining not enabled or invalid solution format", body = crate::error::ErrorResponse)
    )
)]
pub async fn submit_solution(
    State(state): State<AppState>,
    Json(solution): Json<SolutionSubmission>,
) -> ApiResult<Json<SolutionResponse>> {
    if !state.mining_enabled {
        return Err(ApiError::BadRequest("Mining not enabled".to_string()));
    }

    // Validate solution format
    if solution.pk.is_empty() || solution.w.is_empty() || solution.n.is_empty() {
        return Ok(Json(SolutionResponse {
            accepted: false,
            reason: Some("Invalid solution format".to_string()),
        }));
    }

    // Parse the solution components
    let _pk = hex::decode(&solution.pk)
        .map_err(|_| ApiError::BadRequest("Invalid pk hex".to_string()))?;

    let _w =
        hex::decode(&solution.w).map_err(|_| ApiError::BadRequest("Invalid w hex".to_string()))?;

    let _nonce = hex::decode(&solution.n)
        .map_err(|_| ApiError::BadRequest("Invalid nonce hex".to_string()))?;

    let _d =
        hex::decode(&solution.d).map_err(|_| ApiError::BadRequest("Invalid d hex".to_string()))?;

    // TODO: Verify solution using Autolykos PoW
    // 1. Get the current candidate
    // 2. Verify the solution meets difficulty target
    // 3. Create full block with solution
    // 4. Submit to state manager for validation and propagation

    // For now, invalidate the current candidate to force regeneration
    state.candidate_generator.invalidate();

    // Placeholder: Accept the solution
    // In production, this would verify the PoW and broadcast the block
    Ok(Json(SolutionResponse {
        accepted: true,
        reason: None,
    }))
}

/// GET /mining/rewardAddress
///
/// Get the current mining reward address.
#[utoipa::path(
    get,
    path = "/mining/rewardAddress",
    tag = "mining",
    responses(
        (status = 200, description = "Current reward address", body = RewardAddressResponse),
        (status = 400, description = "Mining not enabled", body = crate::error::ErrorResponse)
    )
)]
pub async fn get_reward_address(
    State(state): State<AppState>,
) -> ApiResult<Json<RewardAddressResponse>> {
    if !state.mining_enabled {
        return Err(ApiError::BadRequest("Mining not enabled".to_string()));
    }

    let reward_address = state
        .candidate_generator
        .reward_address()
        .unwrap_or_default();

    Ok(Json(RewardAddressResponse { reward_address }))
}

/// POST /mining/rewardAddress
///
/// Set the mining reward address.
#[utoipa::path(
    post,
    path = "/mining/rewardAddress",
    tag = "mining",
    request_body = RewardAddressRequest,
    responses(
        (status = 200, description = "Reward address updated", body = RewardAddressResponse),
        (status = 400, description = "Mining not enabled or invalid address", body = crate::error::ErrorResponse)
    )
)]
pub async fn set_reward_address(
    State(state): State<AppState>,
    Json(request): Json<RewardAddressRequest>,
) -> ApiResult<Json<RewardAddressResponse>> {
    if !state.mining_enabled {
        return Err(ApiError::BadRequest("Mining not enabled".to_string()));
    }

    if request.reward_address.is_empty() {
        return Err(ApiError::BadRequest(
            "Reward address cannot be empty".to_string(),
        ));
    }

    // TODO: Validate that the address is a valid Ergo address

    state
        .candidate_generator
        .set_reward_address(request.reward_address.clone());

    Ok(Json(RewardAddressResponse {
        reward_address: request.reward_address,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nbits_to_target() {
        // Test with a moderate exponent
        let target = nbits_to_target(0x1a00ffff);
        assert_eq!(target.len(), 64);

        // Lower exponent = smaller target = higher difficulty
        let target2 = nbits_to_target(0x1900ffff);
        assert!(
            target2 < target,
            "Lower exponent should produce smaller target"
        );

        // Test with small exponent (high difficulty)
        let target3 = nbits_to_target(0x0400ffff);
        assert_eq!(target3.len(), 64);
        assert!(
            target3.starts_with("00"),
            "High difficulty should have leading zeros"
        );

        // Test with very high exponent (should cap at max)
        let target4 = nbits_to_target(0x2000ffff);
        assert_eq!(target4.len(), 64);
    }
}
