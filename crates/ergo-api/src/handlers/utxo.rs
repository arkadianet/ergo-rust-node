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
use serde::Serialize;

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

/// Token amount.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenAmount {
    pub token_id: String,
    pub amount: u64,
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
