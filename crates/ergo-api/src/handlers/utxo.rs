//! UTXO handlers.

use crate::{ApiError, ApiResult, AppState};
use axum::{
    extract::{Path, State},
    Json,
};
use ergo_chain_types::Digest32;
use ergo_lib::ergotree_ir::chain::ergo_box::BoxId;
use ergo_lib::ergotree_ir::serialization::SigmaSerializable;
use ergo_state::BoxEntry;
use serde::{Deserialize, Serialize};

// ==================== Constants ====================

/// Maximum number of box IDs allowed in a single batch request.
/// This prevents abuse and ensures reasonable response times.
const MAX_BATCH_SIZE: usize = 100;

/// Box (UTXO) response.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoxResponse {
    pub box_id: String,
    pub value: u64,
    pub creation_height: u32,
    pub ergo_tree: String,
    pub assets: Vec<TokenAmount>,
}

/// Box with serialized bytes.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoxBinaryResponse {
    pub box_id: String,
    pub bytes: String,
}

/// Token amount.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenAmount {
    pub token_id: String,
    pub amount: u64,
}

/// Multiple box IDs request.
#[derive(Deserialize)]
pub struct BoxIdsRequest(pub Vec<String>);

/// Snapshot info response.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotsInfoResponse {
    /// Available snapshot heights.
    pub available_manifests: Vec<u32>,
}

/// Genesis boxes response.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenesisBoxesResponse {
    pub boxes: Vec<BoxResponse>,
}

/// Convert a BoxEntry to a BoxResponse.
fn box_entry_to_response(entry: &BoxEntry) -> BoxResponse {
    let ergo_box = &entry.ergo_box;

    // Get ErgoTree bytes
    let ergo_tree_bytes = ergo_box
        .ergo_tree
        .sigma_serialize_bytes()
        .unwrap_or_default();

    // Convert tokens to response format
    let assets: Vec<TokenAmount> = ergo_box
        .tokens
        .as_ref()
        .map(|tokens| {
            tokens
                .iter()
                .map(|token| TokenAmount {
                    token_id: hex::encode(token.token_id.as_ref()),
                    amount: u64::from(token.amount),
                })
                .collect()
        })
        .unwrap_or_default();

    BoxResponse {
        box_id: hex::encode(ergo_box.box_id().as_ref()),
        value: u64::from(ergo_box.value),
        creation_height: entry.creation_height,
        ergo_tree: hex::encode(&ergo_tree_bytes),
        assets,
    }
}

/// GET /utxo/byId/:id
pub async fn get_box_by_id(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<BoxResponse>> {
    let id_bytes =
        hex::decode(&id).map_err(|_| ApiError::BadRequest("Invalid box ID".to_string()))?;

    // Convert bytes to BoxId
    if id_bytes.len() != 32 {
        return Err(ApiError::BadRequest("Box ID must be 32 bytes".to_string()));
    }

    let digest: [u8; 32] = id_bytes.try_into().unwrap();
    let box_id = BoxId::from(Digest32::from(digest));

    let entry = state
        .state
        .utxo
        .get_box(&box_id)?
        .ok_or_else(|| ApiError::NotFound(format!("Box {} not found", id)))?;

    Ok(Json(box_entry_to_response(&entry)))
}

/// GET /utxo/byAddress/:address
pub async fn get_boxes_by_address(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> ApiResult<Json<Vec<BoxResponse>>> {
    // Decode address to get ErgoTree hash for lookup
    // For now, use ErgoTree-based lookup (requires index)
    let ergo_tree_hash = hex::decode(&address).unwrap_or_else(|_| address.as_bytes().to_vec());

    let boxes = state.state.utxo.get_boxes_by_ergo_tree(&ergo_tree_hash)?;

    let response: Vec<BoxResponse> = boxes.iter().map(box_entry_to_response).collect();

    Ok(Json(response))
}

/// GET /utxo/byIdBinary/:id
/// Get box as serialized bytes.
pub async fn get_box_binary_by_id(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<BoxBinaryResponse>> {
    let id_bytes =
        hex::decode(&id).map_err(|_| ApiError::BadRequest("Invalid box ID".to_string()))?;

    if id_bytes.len() != 32 {
        return Err(ApiError::BadRequest("Box ID must be 32 bytes".to_string()));
    }

    let digest: [u8; 32] = id_bytes.try_into().unwrap();
    let box_id = BoxId::from(Digest32::from(digest));

    let entry = state
        .state
        .utxo
        .get_box(&box_id)?
        .ok_or_else(|| ApiError::NotFound(format!("Box {} not found", id)))?;

    // Serialize box to bytes
    let bytes = entry
        .ergo_box
        .sigma_serialize_bytes()
        .map_err(|e| ApiError::Internal(format!("Failed to serialize box: {}", e)))?;

    Ok(Json(BoxBinaryResponse {
        box_id: id,
        bytes: hex::encode(&bytes),
    }))
}

/// GET /utxo/withPool/byId/:id
/// Get box from UTXO set or mempool outputs.
pub async fn get_box_with_pool(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<BoxResponse>> {
    let id_bytes =
        hex::decode(&id).map_err(|_| ApiError::BadRequest("Invalid box ID".to_string()))?;

    if id_bytes.len() != 32 {
        return Err(ApiError::BadRequest("Box ID must be 32 bytes".to_string()));
    }

    let digest: [u8; 32] = id_bytes.try_into().unwrap();
    let box_id = BoxId::from(Digest32::from(digest));

    // Try UTXO set first
    if let Some(entry) = state.state.utxo.get_box(&box_id)? {
        return Ok(Json(box_entry_to_response(&entry)));
    }

    // Try mempool outputs
    // TODO: implement mempool output lookup

    Err(ApiError::NotFound(format!("Box {} not found", id)))
}

/// POST /utxo/withPool/byIds
/// Get multiple boxes from UTXO set or mempool.
/// Rate limited to MAX_BATCH_SIZE box IDs per request.
pub async fn get_boxes_with_pool(
    State(state): State<AppState>,
    Json(request): Json<BoxIdsRequest>,
) -> ApiResult<Json<Vec<BoxResponse>>> {
    // Rate limiting: reject requests with too many IDs
    if request.0.len() > MAX_BATCH_SIZE {
        return Err(ApiError::BadRequest(format!(
            "Too many box IDs. Maximum allowed: {}, received: {}",
            MAX_BATCH_SIZE,
            request.0.len()
        )));
    }

    let mut results = Vec::with_capacity(request.0.len());

    for id in request.0 {
        let id_bytes = match hex::decode(&id) {
            Ok(b) if b.len() == 32 => b,
            _ => continue,
        };

        let digest: [u8; 32] = id_bytes.try_into().unwrap();
        let box_id = BoxId::from(Digest32::from(digest));

        if let Ok(Some(entry)) = state.state.utxo.get_box(&box_id) {
            results.push(box_entry_to_response(&entry));
        }
    }

    Ok(Json(results))
}

/// GET /utxo/genesis
/// Get genesis boxes.
pub async fn get_genesis_boxes(
    State(_state): State<AppState>,
) -> ApiResult<Json<GenesisBoxesResponse>> {
    // Genesis boxes are hardcoded in the protocol
    // For now, return empty - would need to load from config/genesis block
    Ok(Json(GenesisBoxesResponse { boxes: vec![] }))
}

/// GET /utxo/getSnapshotsInfo
/// Get UTXO snapshot information.
pub async fn get_snapshots_info(
    State(_state): State<AppState>,
) -> ApiResult<Json<SnapshotsInfoResponse>> {
    // Return available snapshot heights
    // Would query snapshot storage for available manifests
    Ok(Json(SnapshotsInfoResponse {
        available_manifests: vec![],
    }))
}
