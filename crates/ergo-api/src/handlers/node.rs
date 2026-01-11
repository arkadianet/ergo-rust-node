//! Node management handlers.

use crate::{ApiError, ApiResult, AppState};
use axum::{extract::State, http::HeaderMap, Json};
use serde::Serialize;
use utoipa::ToSchema;

/// Shutdown response.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ShutdownResponse {
    /// Whether the shutdown was initiated successfully.
    pub success: bool,
    /// Message describing the shutdown status.
    #[schema(example = "Shutdown initiated")]
    pub message: String,
}

/// Extract API key from request headers.
fn extract_api_key(headers: &HeaderMap) -> Option<&str> {
    headers.get("api_key").and_then(|v| v.to_str().ok())
}

/// POST /node/shutdown
///
/// Initiate a graceful node shutdown. This endpoint ALWAYS requires API key
/// authentication for security. The node will complete in-progress operations
/// before shutting down.
#[utoipa::path(
    post,
    path = "/node/shutdown",
    tag = "node",
    responses(
        (status = 200, description = "Shutdown initiated successfully", body = ShutdownResponse),
        (status = 400, description = "API key not configured", body = crate::error::ErrorResponse),
        (status = 401, description = "Unauthorized - invalid or missing API key", body = crate::error::ErrorResponse),
        (status = 501, description = "Shutdown signal not configured", body = crate::error::ErrorResponse)
    ),
    params(
        ("api_key" = String, Header, description = "API key for authentication")
    )
)]
pub async fn shutdown(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<ShutdownResponse>> {
    // Security: Always require API key for shutdown endpoint
    let provided_key = extract_api_key(&headers);

    // If no API key is configured on the node, reject shutdown requests entirely
    // This prevents accidental exposure of shutdown capability
    if state.api_key.is_none() {
        return Err(ApiError::BadRequest(
            "Shutdown endpoint requires API key to be configured on the node".to_string(),
        ));
    }

    // Validate the provided API key
    if !state.check_api_key(provided_key) {
        return Err(ApiError::Unauthorized);
    }

    // Check if shutdown signal is configured
    if let Some(ref shutdown_tx) = state.shutdown_signal {
        // Send shutdown signal via broadcast channel
        match shutdown_tx.send(()) {
            Ok(_) => Ok(Json(ShutdownResponse {
                success: true,
                message: "Shutdown initiated".to_string(),
            })),
            Err(_) => Err(ApiError::Internal(
                "Failed to send shutdown signal".to_string(),
            )),
        }
    } else {
        Err(ApiError::NotImplemented(
            "Shutdown signal not configured for this node".to_string(),
        ))
    }
}
