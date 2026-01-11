//! Web panel handlers for the node dashboard UI.

use crate::AppState;
use axum::{
    body::Body,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::{header, Response, StatusCode},
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use rust_embed::Embed;
use std::time::Duration;

/// Embedded panel assets compiled into the binary.
#[derive(Embed)]
#[folder = "assets/panel/"]
struct PanelAssets;

/// GET /panel - Serve the main panel HTML.
pub async fn panel_index() -> impl IntoResponse {
    serve_panel_file("index.html")
}

/// GET /panel/*path - Serve static panel assets (CSS, JS, etc).
pub async fn panel_assets(Path(path): Path<String>) -> impl IntoResponse {
    serve_panel_file(&path)
}

/// Serve an embedded panel file.
fn serve_panel_file(path: &str) -> Response<Body> {
    match PanelAssets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            Response::builder()
                .header(header::CONTENT_TYPE, mime.as_ref())
                .header(header::CACHE_CONTROL, "public, max-age=3600")
                .body(Body::from(content.data.into_owned()))
                .unwrap()
        }
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("Not found"))
            .unwrap(),
    }
}

/// GET /panel/ws - WebSocket endpoint for real-time updates.
pub async fn panel_websocket(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_websocket(socket, state))
}

/// Handle WebSocket connection for panel updates.
async fn handle_websocket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    // Spawn a task to send periodic status updates
    let state_clone = state.clone();
    let send_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(2));

        loop {
            interval.tick().await;

            let update = build_status_update(&state_clone);
            if sender.send(Message::Text(update.into())).await.is_err() {
                break;
            }
        }
    });

    // Handle incoming messages (for future use, like requesting specific data)
    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(data)) => {
                // Pong is handled automatically by axum
                let _ = data;
            }
            Err(_) => break,
            _ => {}
        }
    }

    // Clean up
    send_task.abort();
}

/// Build a JSON status update message.
fn build_status_update(state: &AppState) -> String {
    let (utxo_height, header_height) = state.state.heights();
    let mempool_stats = state.mempool.stats();
    let peer_count = state.peers.connected_count();
    let is_synced = state.state.is_synced();

    // Get recent mempool transactions for animation
    let txs: Vec<serde_json::Value> = state
        .mempool
        .get_by_weight(50)
        .into_iter()
        .map(|tx| {
            serde_json::json!({
                "id": hex::encode(&tx.id),
                "fee": tx.fee,
                "size": tx.bytes.len(),
                "arrivalTime": tx.arrival_time
            })
        })
        .collect();

    serde_json::json!({
        "type": "status",
        "fullHeight": utxo_height,
        "headersHeight": header_height,
        "isSynced": is_synced,
        "peerCount": peer_count,
        "mempool": {
            "count": mempool_stats.tx_count,
            "size": mempool_stats.total_size,
            "transactions": txs
        },
        "isMining": state.mining_enabled,
        "nodeName": state.node_name
    })
    .to_string()
}
