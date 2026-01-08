//! Peer handlers.

use crate::{ApiResult, AppState};
use axum::{
    extract::State,
    Json,
};
use serde::{Deserialize, Serialize};

/// Peer info response.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerResponse {
    pub address: String,
    pub name: Option<String>,
    pub connection_type: String,
    pub last_seen: u64,
}

/// Connect request.
#[derive(Deserialize)]
pub struct ConnectRequest {
    pub address: String,
}

/// GET /peers/all
pub async fn get_all_peers(
    State(_state): State<AppState>,
) -> ApiResult<Json<Vec<PeerResponse>>> {
    // Would return all known peers
    Ok(Json(Vec::new()))
}

/// GET /peers/connected
pub async fn get_connected_peers(
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<PeerResponse>>> {
    let peers: Vec<PeerResponse> = state.peers.get_connected()
        .into_iter()
        .map(|p| PeerResponse {
            address: p.addr.to_string(),
            name: p.agent,
            connection_type: if p.outbound { "Outgoing" } else { "Incoming" }.to_string(),
            last_seen: 0, // Would calculate from p.last_seen
        })
        .collect();

    Ok(Json(peers))
}

/// GET /peers/blacklisted
pub async fn get_blacklisted(
    State(_state): State<AppState>,
) -> ApiResult<Json<Vec<String>>> {
    // Would return banned peers
    Ok(Json(Vec::new()))
}

/// POST /peers/connect
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
