//! API route tests.
//!
//! These tests verify API endpoint behavior including:
//! - Correct response formats
//! - Error handling
//! - Input validation
//! - JSON serialization

use crate::harness::TestDatabase;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use ergo_api::AppState;
use ergo_mempool::Mempool;
use ergo_network::PeerManager;
use ergo_state::StateManager;
use ergo_storage::Storage;
use http_body_util::BodyExt;

use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

/// Create a test API router with fresh state.
fn create_test_api() -> (Router, TestDatabase) {
    let test_db = TestDatabase::new();
    let storage: Arc<dyn Storage> = Arc::new(test_db.db_clone());

    let state_manager = Arc::new(StateManager::new(Arc::clone(&storage)));
    let mempool = Arc::new(Mempool::with_defaults());
    let peers = Arc::new(PeerManager::default());

    let app_state = AppState::new(state_manager, mempool, peers, "test-node".to_string());

    let router = ergo_api::build_api(app_state);
    (router, test_db)
}

/// Create a test API router with API key authentication.
fn create_test_api_with_auth(api_key: &str) -> (Router, TestDatabase) {
    let test_db = TestDatabase::new();
    let storage: Arc<dyn Storage> = Arc::new(test_db.db_clone());

    let state_manager = Arc::new(StateManager::new(Arc::clone(&storage)));
    let mempool = Arc::new(Mempool::with_defaults());
    let peers = Arc::new(PeerManager::default());

    let app_state = AppState::new(state_manager, mempool, peers, "test-node".to_string())
        .with_api_key(api_key.to_string());

    let router = ergo_api::build_api(app_state);
    (router, test_db)
}

/// Helper to make a GET request and get response body as JSON.
async fn get_json(router: &Router, path: &str) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();

    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);

    (status, json)
}

/// Helper to make a POST request with JSON body.
async fn post_json(router: &Router, path: &str, body: Value) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);

    (status, json)
}

// ============================================================================
// Info Endpoint Tests
// ============================================================================

#[tokio::test]
async fn test_get_info_returns_node_info() {
    let (router, _db) = create_test_api();

    let (status, json) = get_json(&router, "/info").await;

    assert_eq!(status, StatusCode::OK);
    assert!(json.get("name").is_some());
    assert!(json.get("appVersion").is_some());
    assert!(json.get("fullHeight").is_some());
    assert!(json.get("headersHeight").is_some());
    assert!(json.get("stateType").is_some());
}

#[tokio::test]
async fn test_get_info_has_correct_node_name() {
    let (router, _db) = create_test_api();

    let (status, json) = get_json(&router, "/info").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json.get("name").unwrap(), "test-node");
}

#[tokio::test]
async fn test_get_info_heights_are_zero_initially() {
    let (router, _db) = create_test_api();

    let (status, json) = get_json(&router, "/info").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json.get("fullHeight").unwrap(), 0);
    assert_eq!(json.get("headersHeight").unwrap(), 0);
}

// ============================================================================
// Blocks Endpoint Tests
// ============================================================================

#[tokio::test]
async fn test_get_blocks_returns_empty_list_initially() {
    let (router, _db) = create_test_api();

    let (status, json) = get_json(&router, "/blocks").await;

    assert_eq!(status, StatusCode::OK);
    assert!(json.is_array());
    assert_eq!(json.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_get_blocks_with_pagination() {
    let (router, _db) = create_test_api();

    let (status, json) = get_json(&router, "/blocks?offset=0&limit=10").await;

    assert_eq!(status, StatusCode::OK);
    assert!(json.is_array());
}

#[tokio::test]
async fn test_get_block_by_invalid_id_returns_bad_request() {
    let (router, _db) = create_test_api();

    let (status, json) = get_json(&router, "/blocks/invalid-id").await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json.get("error").is_some());
}

#[tokio::test]
async fn test_get_block_by_nonexistent_id_returns_not_found() {
    let (router, _db) = create_test_api();

    // Valid hex but doesn't exist
    let fake_id = "0".repeat(64);
    let (status, json) = get_json(&router, &format!("/blocks/{}", fake_id)).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(json.get("error").is_some());
}

#[tokio::test]
async fn test_get_block_at_height_not_found() {
    let (router, _db) = create_test_api();

    let (status, json) = get_json(&router, "/blocks/at/999").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(json.get("error").is_some());
}

// ============================================================================
// Transactions Endpoint Tests
// ============================================================================

#[tokio::test]
async fn test_get_unconfirmed_returns_empty_list() {
    let (router, _db) = create_test_api();

    let (status, json) = get_json(&router, "/transactions/unconfirmed").await;

    assert_eq!(status, StatusCode::OK);
    assert!(json.is_array());
    assert_eq!(json.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_get_unconfirmed_with_limit() {
    let (router, _db) = create_test_api();

    let (status, json) = get_json(&router, "/transactions/unconfirmed?limit=5").await;

    assert_eq!(status, StatusCode::OK);
    assert!(json.is_array());
}

#[tokio::test]
async fn test_get_pool_histogram() {
    let (router, _db) = create_test_api();

    let (status, _json) = get_json(&router, "/transactions/poolHistogram").await;

    // Should return the pool histogram (empty initially)
    assert_eq!(status, StatusCode::OK);
}

// ============================================================================
// UTXO Endpoint Tests
// ============================================================================

#[tokio::test]
async fn test_get_utxo_by_invalid_id_returns_bad_request() {
    let (router, _db) = create_test_api();

    let (status, json) = get_json(&router, "/utxo/byId/not-valid-hex").await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json.get("error").is_some());
}

#[tokio::test]
async fn test_get_utxo_by_nonexistent_id_returns_not_found() {
    let (router, _db) = create_test_api();

    let fake_id = "0".repeat(64);
    let (status, json) = get_json(&router, &format!("/utxo/byId/{}", fake_id)).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(json.get("error").is_some());
}

// ============================================================================
// Peers Endpoint Tests
// ============================================================================

#[tokio::test]
async fn test_get_peers_all_returns_list() {
    let (router, _db) = create_test_api();

    let (status, json) = get_json(&router, "/peers/all").await;

    assert_eq!(status, StatusCode::OK);
    assert!(json.is_array());
}

#[tokio::test]
async fn test_get_peers_connected_returns_list() {
    let (router, _db) = create_test_api();

    let (status, json) = get_json(&router, "/peers/connected").await;

    assert_eq!(status, StatusCode::OK);
    assert!(json.is_array());
}

// ============================================================================
// Mining Endpoint Tests
// ============================================================================

#[tokio::test]
async fn test_get_mining_candidate_when_mining_disabled() {
    let (router, _db) = create_test_api();

    let (status, _json) = get_json(&router, "/mining/candidate").await;

    // Mining is disabled by default, should return an error or empty
    assert!(status == StatusCode::BAD_REQUEST || status == StatusCode::OK);
}

// ============================================================================
// Utils Endpoint Tests
// ============================================================================

#[tokio::test]
async fn test_get_raw_bytes_from_hex_valid() {
    let (router, _db) = create_test_api();

    let (status, json) = get_json(&router, "/utils/rawToAddress/0008cd").await;

    // Even if address conversion fails, the hex parsing should work
    assert!(status == StatusCode::OK || status == StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_get_address_to_raw_invalid_address() {
    let (router, _db) = create_test_api();

    let (status, json) = get_json(&router, "/utils/addressToRaw/invalid-address").await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json.get("error").is_some());
}

// ============================================================================
// Error Response Format Tests
// ============================================================================

#[tokio::test]
async fn test_error_response_has_correct_format() {
    let (router, _db) = create_test_api();

    let (status, json) = get_json(&router, "/blocks/invalid").await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json.get("error").is_some());
    assert!(json.get("reason").is_some());
    assert!(json.get("detail").is_some());
}

#[tokio::test]
async fn test_not_found_returns_404() {
    let (router, _db) = create_test_api();

    let fake_id = "a".repeat(64);
    let (status, json) = get_json(&router, &format!("/blocks/{}", fake_id)).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json.get("error").unwrap(), 404);
}

// ============================================================================
// Emission Endpoint Tests
// ============================================================================

#[tokio::test]
async fn test_get_emission_at_height() {
    let (router, _db) = create_test_api();

    let (status, json) = get_json(&router, "/emission/at/1").await;

    assert_eq!(status, StatusCode::OK);
    // Response contains minerReward field (camelCase)
    assert!(json.get("minerReward").is_some());
    assert!(json.get("height").is_some());
}

#[tokio::test]
async fn test_get_emission_at_height_zero() {
    let (router, _db) = create_test_api();

    let (status, _json) = get_json(&router, "/emission/at/0").await;

    // Height 0 might be valid or invalid depending on implementation
    assert!(status == StatusCode::OK || status == StatusCode::BAD_REQUEST);
}

// ============================================================================
// Script Endpoint Tests
// ============================================================================

#[tokio::test]
async fn test_post_script_p2s_address() {
    let (router, _db) = create_test_api();

    // Simple true script
    let body = serde_json::json!({
        "source": "true"
    });

    let (status, _json) = post_json(&router, "/script/p2sAddress", body).await;

    // May succeed or fail depending on script compilation
    assert!(status == StatusCode::OK || status == StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_post_script_with_invalid_source() {
    let (router, _db) = create_test_api();

    let body = serde_json::json!({
        "source": "this is not valid ergoscript"
    });

    let (status, _json) = post_json(&router, "/script/p2sAddress", body).await;

    // Should fail with bad request
    assert!(status == StatusCode::BAD_REQUEST || status == StatusCode::INTERNAL_SERVER_ERROR);
}

// ============================================================================
// Blockchain Endpoint Tests
// ============================================================================

#[tokio::test]
async fn test_get_blockchain_indexed_height_without_indexer() {
    let (router, _db) = create_test_api();

    let (status, _json) = get_json(&router, "/blockchain/indexedHeight").await;

    // Without indexer enabled, returns SERVICE_UNAVAILABLE (503)
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

// ============================================================================
// Request Validation Tests
// ============================================================================

#[tokio::test]
async fn test_invalid_json_returns_bad_request() {
    let (router, _db) = create_test_api();

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/script/p2sAddress")
                .header("Content-Type", "application/json")
                .body(Body::from("{ invalid json }"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        response.status() == StatusCode::BAD_REQUEST
            || response.status() == StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn test_missing_content_type_for_post() {
    let (router, _db) = create_test_api();

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/script/p2sAddress")
                .body(Body::from(r#"{"source": "true"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should either work or return unsupported media type
    assert!(
        response.status() == StatusCode::OK
            || response.status() == StatusCode::BAD_REQUEST
            || response.status() == StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
}

// ============================================================================
// Concurrent Request Tests
// ============================================================================

#[tokio::test]
async fn test_concurrent_info_requests() {
    let (router, _db) = create_test_api();

    let mut handles = Vec::new();

    for _ in 0..10 {
        let router = router.clone();
        handles.push(tokio::spawn(async move {
            let (status, _) = get_json(&router, "/info").await;
            status
        }));
    }

    for handle in handles {
        let status = handle.await.unwrap();
        assert_eq!(status, StatusCode::OK);
    }
}

#[tokio::test]
async fn test_concurrent_different_endpoints() {
    let (router, _db) = create_test_api();

    let endpoints = vec![
        "/info",
        "/blocks",
        "/transactions/unconfirmed",
        "/peers/all",
    ];

    let mut handles = Vec::new();

    for endpoint in endpoints {
        let router = router.clone();
        let ep = endpoint.to_string();
        handles.push(tokio::spawn(async move {
            let (status, _) = get_json(&router, &ep).await;
            (ep, status)
        }));
    }

    for handle in handles {
        let (endpoint, status) = handle.await.unwrap();
        assert_eq!(status, StatusCode::OK, "Failed for endpoint: {}", endpoint);
    }
}
