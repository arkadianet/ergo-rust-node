//! Node info handler.

use crate::{ApiResult, AppState, NODE_VERSION};
use axum::{extract::State, Json};
use serde::Serialize;
use utoipa::ToSchema;

/// Node info response.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NodeInfo {
    /// Node name.
    #[schema(example = "ergo-rust-node")]
    pub name: String,
    /// App version.
    #[schema(example = "0.1.0")]
    pub app_version: String,
    /// Full height (block height with full data).
    #[schema(example = 1234567)]
    pub full_height: u32,
    /// Headers height.
    #[schema(example = 1234567)]
    pub headers_height: u32,
    /// Best header ID (hex-encoded).
    #[schema(example = "0000000000000000000000000000000000000000000000000000000000000000")]
    pub best_header_id: String,
    /// Best full block ID (hex-encoded).
    #[schema(example = "0000000000000000000000000000000000000000000000000000000000000000")]
    pub best_full_block_id: String,
    /// State root (hex-encoded).
    #[schema(example = "0000000000000000000000000000000000000000000000000000000000000000")]
    pub state_root: String,
    /// Is mining enabled.
    pub is_mining: bool,
    /// Number of connected peers.
    #[schema(example = 10)]
    pub peer_count: usize,
    /// Number of unconfirmed transactions in mempool.
    #[schema(example = 5)]
    pub unconfirmed_count: usize,
    /// State type (utxo or digest).
    #[schema(example = "utxo")]
    pub state_type: String,
    /// Is node synchronized with network.
    pub is_synced: bool,
}

/// GET /info
///
/// Get basic node information including sync status, peer count, and version.
#[utoipa::path(
    get,
    path = "/info",
    tag = "info",
    responses(
        (status = 200, description = "Node information retrieved successfully", body = NodeInfo)
    )
)]
pub async fn get_info(State(state): State<AppState>) -> ApiResult<Json<NodeInfo>> {
    let (utxo_height, header_height) = state.state.heights();

    let best_header_id = state
        .state
        .history
        .best_header_id()
        .map(|id| hex::encode(id.0.as_ref()))
        .unwrap_or_default();
    let state_root = state.state.utxo.state_root();
    let mempool_stats = state.mempool.stats();
    let peer_count = state.peers.connected_count();

    let info = NodeInfo {
        name: state.node_name.clone(),
        app_version: NODE_VERSION.to_string(),
        full_height: utxo_height,
        headers_height: header_height,
        best_header_id: best_header_id.clone(),
        best_full_block_id: best_header_id, // Same for now
        state_root: hex::encode(&state_root),
        is_mining: state.mining_enabled,
        peer_count,
        unconfirmed_count: mempool_stats.tx_count,
        state_type: "utxo".to_string(),
        is_synced: state.state.is_synced(),
    };

    Ok(Json(info))
}
