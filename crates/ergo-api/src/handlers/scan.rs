//! Scan API handlers (EIP-0001).
//!
//! Provides endpoints for registering external scans, tracking boxes, and retrieving
//! boxes matching scan predicates.

use crate::{ApiError, ApiResult, AppState};
use axum::{
    extract::{Path, Query, State},
    Json,
};
use ergo_storage::{Scan, ScanBox, ScanId, ScanRequest, ScanWalletInteraction, ScanningPredicate};
use serde::{Deserialize, Serialize};

/// Scan ID wrapper for JSON responses.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanIdWrapper {
    pub scan_id: ScanId,
}

/// Scan ID and Box ID for stop tracking requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanIdBoxId {
    pub scan_id: ScanId,
    pub box_id: String, // hex-encoded
}

/// Box with scan IDs for add box requests.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoxWithScanIds {
    /// Serialized box data (hex-encoded).
    pub box_bytes: String,
    /// Scan IDs to associate with this box.
    pub scan_ids: Vec<ScanId>,
}

/// Query parameters for box listing.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoxQueryParams {
    /// Minimum confirmations (-1 for unconfirmed).
    #[serde(default = "default_min_confirmations")]
    pub min_confirmations: i32,
    /// Maximum confirmations.
    #[serde(default = "default_max_confirmations")]
    pub max_confirmations: i32,
    /// Minimum inclusion height.
    #[serde(default)]
    pub min_inclusion_height: u32,
    /// Maximum inclusion height.
    #[serde(default = "default_max_height")]
    pub max_inclusion_height: u32,
    /// Maximum number of boxes to return.
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Offset for pagination.
    #[serde(default)]
    pub offset: usize,
}

fn default_min_confirmations() -> i32 {
    0
}

fn default_max_confirmations() -> i32 {
    i32::MAX
}

fn default_max_height() -> u32 {
    u32::MAX
}

fn default_limit() -> usize {
    50
}

/// Box response matching Scala node format.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletBoxResponse {
    /// Box ID (hex).
    pub box_id: String,
    /// Inclusion height.
    pub inclusion_height: u32,
    /// Number of confirmations.
    pub confirmations_num: u32,
    /// Whether the box is spent.
    pub spent: bool,
    /// Spending transaction ID (if spent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spending_transaction: Option<String>,
    /// Spending height (if spent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spending_height: Option<u32>,
    /// Associated scan IDs.
    pub scan_ids: Vec<ScanId>,
}

impl From<ScanBox> for WalletBoxResponse {
    fn from(b: ScanBox) -> Self {
        Self {
            box_id: hex::encode(&b.box_id),
            inclusion_height: b.inclusion_height,
            confirmations_num: b.confirmations_num,
            spent: b.spent,
            spending_transaction: b.spending_tx_id.map(|id| hex::encode(&id)),
            spending_height: b.spending_height,
            scan_ids: b.scan_ids,
        }
    }
}

/// Scan response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResponse {
    pub scan_id: ScanId,
    pub scan_name: String,
    pub tracking_rule: ScanningPredicate,
    pub wallet_interaction: ScanWalletInteraction,
    pub remove_offchain: bool,
}

impl From<Scan> for ScanResponse {
    fn from(s: Scan) -> Self {
        Self {
            scan_id: s.scan_id,
            scan_name: s.scan_name,
            tracking_rule: s.tracking_rule,
            wallet_interaction: s.wallet_interaction,
            remove_offchain: s.remove_offchain,
        }
    }
}

/// Register a new scan.
///
/// POST /scan/register
pub async fn register(
    State(state): State<AppState>,
    Json(request): Json<ScanRequest>,
) -> ApiResult<Json<ScanIdWrapper>> {
    let scan_storage = state
        .scan_storage
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("Scan storage not enabled".to_string()))?;

    let scan = scan_storage
        .register(request)
        .map_err(|e| ApiError::BadRequest(e))?;

    Ok(Json(ScanIdWrapper {
        scan_id: scan.scan_id,
    }))
}

/// Deregister (remove) a scan.
///
/// POST /scan/deregister
pub async fn deregister(
    State(state): State<AppState>,
    Json(wrapper): Json<ScanIdWrapper>,
) -> ApiResult<Json<ScanIdWrapper>> {
    let scan_storage = state
        .scan_storage
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("Scan storage not enabled".to_string()))?;

    scan_storage
        .deregister(wrapper.scan_id)
        .map_err(|e| ApiError::BadRequest(e))?;

    Ok(Json(wrapper))
}

/// List all registered scans.
///
/// GET /scan/listAll
pub async fn list_all(State(state): State<AppState>) -> ApiResult<Json<Vec<ScanResponse>>> {
    let scan_storage = state
        .scan_storage
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("Scan storage not enabled".to_string()))?;

    let scans: Vec<ScanResponse> = scan_storage
        .list_all()
        .into_iter()
        .map(Into::into)
        .collect();

    Ok(Json(scans))
}

/// Get unspent boxes for a scan.
///
/// GET /scan/unspentBoxes/{scanId}
pub async fn unspent_boxes(
    State(state): State<AppState>,
    Path(scan_id): Path<i16>,
    Query(params): Query<BoxQueryParams>,
) -> ApiResult<Json<Vec<WalletBoxResponse>>> {
    let scan_storage = state
        .scan_storage
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("Scan storage not enabled".to_string()))?;

    let boxes = scan_storage
        .get_unspent_boxes(scan_id)
        .map_err(|e| ApiError::Internal(format!("Failed to get unspent boxes: {}", e)))?;

    // Apply filters
    let current_height = state.state.utxo.height();
    let filtered: Vec<WalletBoxResponse> = boxes
        .into_iter()
        .filter(|b| {
            let confirmations = current_height.saturating_sub(b.inclusion_height) as i32;
            confirmations >= params.min_confirmations
                && confirmations <= params.max_confirmations
                && b.inclusion_height >= params.min_inclusion_height
                && b.inclusion_height <= params.max_inclusion_height
        })
        .skip(params.offset)
        .take(params.limit)
        .map(Into::into)
        .collect();

    Ok(Json(filtered))
}

/// Get spent boxes for a scan.
///
/// GET /scan/spentBoxes/{scanId}
pub async fn spent_boxes(
    State(state): State<AppState>,
    Path(scan_id): Path<i16>,
    Query(params): Query<BoxQueryParams>,
) -> ApiResult<Json<Vec<WalletBoxResponse>>> {
    let scan_storage = state
        .scan_storage
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("Scan storage not enabled".to_string()))?;

    let boxes = scan_storage
        .get_spent_boxes(scan_id)
        .map_err(|e| ApiError::Internal(format!("Failed to get spent boxes: {}", e)))?;

    // Apply filters
    let current_height = state.state.utxo.height();
    let filtered: Vec<WalletBoxResponse> = boxes
        .into_iter()
        .filter(|b| {
            let confirmations = current_height.saturating_sub(b.inclusion_height) as i32;
            confirmations >= params.min_confirmations
                && confirmations <= params.max_confirmations
                && b.inclusion_height >= params.min_inclusion_height
                && b.inclusion_height <= params.max_inclusion_height
        })
        .skip(params.offset)
        .take(params.limit)
        .map(Into::into)
        .collect();

    Ok(Json(filtered))
}

/// Stop tracking a box for a scan.
///
/// POST /scan/stopTracking
pub async fn stop_tracking(
    State(state): State<AppState>,
    Json(request): Json<ScanIdBoxId>,
) -> ApiResult<Json<ScanIdBoxId>> {
    let scan_storage = state
        .scan_storage
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("Scan storage not enabled".to_string()))?;

    let box_id = hex::decode(&request.box_id)
        .map_err(|e| ApiError::BadRequest(format!("Invalid box ID: {}", e)))?;

    scan_storage
        .stop_tracking(request.scan_id, &box_id)
        .map_err(|e| ApiError::Internal(format!("Failed to stop tracking: {}", e)))?;

    Ok(Json(request))
}

/// Manually add a box to scans.
///
/// POST /scan/addBox
pub async fn add_box(
    State(state): State<AppState>,
    Json(request): Json<BoxWithScanIds>,
) -> ApiResult<Json<String>> {
    let scan_storage = state
        .scan_storage
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("Scan storage not enabled".to_string()))?;

    let box_bytes = hex::decode(&request.box_bytes)
        .map_err(|e| ApiError::BadRequest(format!("Invalid box bytes: {}", e)))?;

    // Extract box ID from the box bytes (first 32 bytes after deserialization)
    // For now, we'll use a hash of the box bytes as the ID
    use blake2::digest::consts::U32;
    use blake2::{Blake2b, Digest};
    let mut hasher = Blake2b::<U32>::new();
    hasher.update(&box_bytes);
    let box_id = hasher.finalize().to_vec();

    let current_height = state.state.utxo.height();

    scan_storage
        .add_box_to_scans(&box_id, request.scan_ids, current_height, 0, box_bytes)
        .map_err(|e| ApiError::Internal(format!("Failed to add box: {}", e)))?;

    Ok(Json(hex::encode(&box_id)))
}

/// P2S address tracking rule helper.
///
/// Creates a scan that tracks boxes with a specific P2S address.
///
/// POST /scan/p2sRule
pub async fn p2s_rule(
    State(state): State<AppState>,
    Json(address): Json<String>,
) -> ApiResult<Json<ScanIdWrapper>> {
    let scan_storage = state
        .scan_storage
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("Scan storage not enabled".to_string()))?;

    // Parse the address and extract the ErgoTree bytes
    // The address string should be a base58-encoded Ergo address
    // For P2S addresses, we track boxes where R1 (propositionBytes) equals the script bytes

    // For now, we'll use a simple equals predicate on R1
    // In a full implementation, we'd parse the address and extract the ErgoTree

    // R1 is the propositionBytes register (index 1)
    let address_bytes = address.as_bytes().to_vec();

    let tracking_rule = ScanningPredicate::Equals {
        reg_id: 1, // R1 = propositionBytes
        value: address_bytes,
    };

    let request = ScanRequest {
        scan_name: address.clone(),
        tracking_rule,
        wallet_interaction: Some(ScanWalletInteraction::Off),
        remove_offchain: Some(true),
    };

    let scan = scan_storage
        .register(request)
        .map_err(|e| ApiError::BadRequest(e))?;

    Ok(Json(ScanIdWrapper {
        scan_id: scan.scan_id,
    }))
}
