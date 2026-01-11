//! Peer handlers.

use crate::{ApiResult, AppState};
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Peer info response.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PeerInfo {
    /// Peer address (IP:port).
    #[schema(example = "192.168.1.1:9030")]
    pub address: String,
    /// Peer agent name (if known).
    #[schema(example = "ergo-mainnet-5.0.0")]
    pub name: Option<String>,
    /// Connection type (Incoming or Outgoing).
    #[schema(example = "Outgoing")]
    pub connection_type: String,
    /// Unix timestamp of last message from peer.
    #[schema(example = 1704067200)]
    pub last_seen: u64,
}

/// Connect request.
#[derive(Deserialize, ToSchema)]
pub struct ConnectRequest {
    /// Peer address to connect to (IP:port).
    #[schema(example = "192.168.1.1:9030")]
    pub address: String,
}

/// GET /peers/all
///
/// Get all known peers (connected and previously seen).
#[utoipa::path(
    get,
    path = "/peers/all",
    tag = "peers",
    responses(
        (status = 200, description = "List of all known peers", body = Vec<PeerInfo>)
    )
)]
pub async fn get_all_peers(State(_state): State<AppState>) -> ApiResult<Json<Vec<PeerInfo>>> {
    // Would return all known peers
    Ok(Json(Vec::new()))
}

/// GET /peers/connected
///
/// Get currently connected peers.
#[utoipa::path(
    get,
    path = "/peers/connected",
    tag = "peers",
    responses(
        (status = 200, description = "List of connected peers", body = Vec<PeerInfo>)
    )
)]
pub async fn get_connected_peers(State(state): State<AppState>) -> ApiResult<Json<Vec<PeerInfo>>> {
    let peers: Vec<PeerInfo> = state
        .peers
        .get_connected()
        .into_iter()
        .map(|p| PeerInfo {
            address: p.addr.to_string(),
            name: p.agent,
            connection_type: if p.outbound { "Outgoing" } else { "Incoming" }.to_string(),
            last_seen: 0, // Would calculate from p.last_seen
        })
        .collect();

    Ok(Json(peers))
}

/// GET /peers/blacklisted
///
/// Get list of blacklisted peer addresses.
#[utoipa::path(
    get,
    path = "/peers/blacklisted",
    tag = "peers",
    responses(
        (status = 200, description = "List of blacklisted peer addresses", body = Vec<String>)
    )
)]
pub async fn get_blacklisted(State(_state): State<AppState>) -> ApiResult<Json<Vec<String>>> {
    // Would return banned peers
    Ok(Json(Vec::new()))
}

/// POST /peers/connect
///
/// Initiate connection to a peer.
#[utoipa::path(
    post,
    path = "/peers/connect",
    tag = "peers",
    request_body = ConnectRequest,
    responses(
        (status = 200, description = "Connection initiated")
    )
)]
pub async fn connect_peer(
    State(_state): State<AppState>,
    Json(request): Json<ConnectRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    // Would initiate connection to peer
    Ok(Json(serde_json::json!({
        "address": request.address,
        "status": "connecting"
    })))
}
