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
use utoipa::ToSchema;

// ==================== Constants ====================

/// Maximum number of box IDs allowed in a single batch request.
/// This prevents abuse and ensures reasonable response times.
const MAX_BATCH_SIZE: usize = 100;

/// Box (UTXO) response.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BoxResponse {
    /// Box ID (hex-encoded 32 bytes).
    #[schema(example = "0000000000000000000000000000000000000000000000000000000000000000")]
    pub box_id: String,
    /// Box value in nanoERG.
    #[schema(example = 1000000000_u64)]
    pub value: u64,
    /// Block height when the box was created.
    #[schema(example = 1234567)]
    pub creation_height: u32,
    /// ErgoTree bytes (hex-encoded).
    #[schema(example = "0008cd...")]
    pub ergo_tree: String,
    /// Token assets in the box.
    pub assets: Vec<TokenAmount>,
}

/// Box with serialized bytes.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BoxBinaryResponse {
    /// Box ID (hex-encoded).
    #[schema(example = "0000000000000000000000000000000000000000000000000000000000000000")]
    pub box_id: String,
    /// Serialized box bytes (hex-encoded).
    #[schema(example = "80a0...")]
    pub bytes: String,
}

/// Token amount.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TokenAmount {
    /// Token ID (hex-encoded 32 bytes).
    #[schema(example = "0000000000000000000000000000000000000000000000000000000000000000")]
    pub token_id: String,
    /// Token amount.
    #[schema(example = 1000)]
    pub amount: u64,
}

/// Multiple box IDs request.
#[derive(Deserialize, ToSchema)]
pub struct BoxIdsRequest(
    /// List of box IDs to query (max 100).
    pub Vec<String>,
);

/// Snapshot info response.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotsInfoResponse {
    /// Available snapshot heights.
    pub available_manifests: Vec<u32>,
}

/// Genesis box response.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GenesisBoxResponse {
    /// List of genesis boxes.
    pub boxes: Vec<BoxResponse>,
}

/// Binary proof request (same as BoxIdsRequest).
#[derive(Deserialize, ToSchema)]
pub struct BinaryProofRequest(
    /// List of box IDs to generate proof for.
    pub Vec<String>,
);

/// Binary proof response.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BinaryProofResponse {
    /// Hex-encoded Merkle proof bytes.
    #[schema(example = "0a0b0c...")]
    pub proof: String,
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
///
/// Get a box (UTXO) by its ID from the confirmed UTXO set.
#[utoipa::path(
    get,
    path = "/utxo/byId/{id}",
    tag = "utxo",
    params(
        ("id" = String, Path, description = "Box ID (hex-encoded 32 bytes)")
    ),
    responses(
        (status = 200, description = "Box found", body = BoxResponse),
        (status = 400, description = "Invalid box ID", body = crate::error::ErrorResponse),
        (status = 404, description = "Box not found", body = crate::error::ErrorResponse)
    )
)]
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
///
/// Get boxes by address (or ErgoTree hash).
#[utoipa::path(
    get,
    path = "/utxo/byAddress/{address}",
    tag = "utxo",
    params(
        ("address" = String, Path, description = "Address or ErgoTree hash (hex-encoded)")
    ),
    responses(
        (status = 200, description = "Boxes found", body = Vec<BoxResponse>)
    )
)]
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
///
/// Get box as serialized bytes.
#[utoipa::path(
    get,
    path = "/utxo/byIdBinary/{id}",
    tag = "utxo",
    params(
        ("id" = String, Path, description = "Box ID (hex-encoded 32 bytes)")
    ),
    responses(
        (status = 200, description = "Box bytes", body = BoxBinaryResponse),
        (status = 400, description = "Invalid box ID", body = crate::error::ErrorResponse),
        (status = 404, description = "Box not found", body = crate::error::ErrorResponse)
    )
)]
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
///
/// Get box from UTXO set or mempool outputs.
#[utoipa::path(
    get,
    path = "/utxo/withPool/byId/{id}",
    tag = "utxo",
    params(
        ("id" = String, Path, description = "Box ID (hex-encoded 32 bytes)")
    ),
    responses(
        (status = 200, description = "Box found", body = BoxResponse),
        (status = 400, description = "Invalid box ID", body = crate::error::ErrorResponse),
        (status = 404, description = "Box not found", body = crate::error::ErrorResponse)
    )
)]
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
///
/// Get multiple boxes from UTXO set or mempool. Rate limited to 100 box IDs.
#[utoipa::path(
    post,
    path = "/utxo/withPool/byIds",
    tag = "utxo",
    request_body = BoxIdsRequest,
    responses(
        (status = 200, description = "Boxes found", body = Vec<BoxResponse>),
        (status = 400, description = "Too many box IDs", body = crate::error::ErrorResponse)
    )
)]
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
///
/// Get genesis boxes.
#[utoipa::path(
    get,
    path = "/utxo/genesis",
    tag = "utxo",
    responses(
        (status = 200, description = "Genesis boxes", body = GenesisBoxResponse)
    )
)]
pub async fn get_genesis_boxes(
    State(_state): State<AppState>,
) -> ApiResult<Json<GenesisBoxResponse>> {
    // Genesis boxes are hardcoded in the protocol
    // For now, return empty - would need to load from config/genesis block
    Ok(Json(GenesisBoxResponse { boxes: vec![] }))
}

/// GET /utxo/getSnapshotsInfo
///
/// Get UTXO snapshot information.
#[utoipa::path(
    get,
    path = "/utxo/getSnapshotsInfo",
    tag = "utxo",
    responses(
        (status = 200, description = "Snapshot information", body = SnapshotsInfoResponse)
    )
)]
pub async fn get_snapshots_info(
    State(_state): State<AppState>,
) -> ApiResult<Json<SnapshotsInfoResponse>> {
    // Return available snapshot heights
    // Would query snapshot storage for available manifests
    Ok(Json(SnapshotsInfoResponse {
        available_manifests: vec![],
    }))
}

/// POST /utxo/getBoxesBinaryProof
///
/// Get binary (Merkle) proof for the existence of boxes in the UTXO set.
/// This endpoint generates a cryptographic proof that can be verified
/// independently to confirm box existence without requiring full state.
#[utoipa::path(
    post,
    path = "/utxo/getBoxesBinaryProof",
    tag = "utxo",
    request_body = BinaryProofRequest,
    responses(
        (status = 200, description = "Binary proof", body = BinaryProofResponse),
        (status = 400, description = "Invalid request", body = crate::error::ErrorResponse),
        (status = 501, description = "Not implemented", body = crate::error::ErrorResponse)
    )
)]
pub async fn get_boxes_binary_proof(
    State(_state): State<AppState>,
    Json(request): Json<BinaryProofRequest>,
) -> ApiResult<Json<BinaryProofResponse>> {
    // Rate limiting: reject requests with too many IDs
    if request.0.len() > MAX_BATCH_SIZE {
        return Err(ApiError::BadRequest(format!(
            "Too many box IDs. Maximum allowed: {}, received: {}",
            MAX_BATCH_SIZE,
            request.0.len()
        )));
    }

    // Validate box IDs
    for id in &request.0 {
        let id_bytes =
            hex::decode(id).map_err(|_| ApiError::BadRequest(format!("Invalid box ID: {}", id)))?;
        if id_bytes.len() != 32 {
            return Err(ApiError::BadRequest(format!(
                "Box ID must be 32 bytes: {}",
                id
            )));
        }
    }

    // TODO: Implement actual proof generation using AVL+ tree
    // This requires:
    // 1. Loading the AVL+ tree state
    // 2. Looking up each box in the tree
    // 3. Generating a batch proof for all lookups
    //
    // For now, return NotImplemented error
    Err(ApiError::NotImplemented(
        "Binary proof generation requires AVL+ tree integration".to_string(),
    ))
}
