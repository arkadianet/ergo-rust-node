//! Transaction handlers.

use crate::{ApiError, ApiResult, AppState};
use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};

// ==================== Constants ====================

/// Minimum fee in nanoERG (0.001 ERG).
const MIN_FEE_NANO_ERG: u64 = 1_000_000;

/// Base block time in milliseconds (~2 minutes).
const BASE_BLOCK_TIME_MS: u64 = 120_000;

/// Minimum transaction size in bytes.
const MIN_TX_SIZE: usize = 100;

/// Maximum transaction size in bytes (~1MB).
const MAX_TX_SIZE: usize = 1_000_000;

/// Transaction submission request.
#[derive(Deserialize)]
pub struct SubmitTx {
    /// Serialized transaction (hex or base64).
    pub bytes: String,
}

/// Transaction response.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TxResponse {
    pub id: String,
}

/// Unconfirmed transaction info.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnconfirmedTx {
    pub id: String,
    pub fee: u64,
    pub size: usize,
}

/// Pagination parameters.
#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    50
}

/// Fee histogram parameters.
#[derive(Debug, Deserialize)]
pub struct FeeHistogramParams {
    /// Number of bins for the histogram.
    #[serde(default = "default_bins")]
    pub bins: usize,
    /// Maximum time in milliseconds.
    #[serde(default = "default_max_time")]
    pub maxtime: u64,
}

fn default_bins() -> usize {
    10
}

fn default_max_time() -> u64 {
    60000 // 1 minute
}

/// Fee request parameters.
#[derive(Debug, Deserialize)]
pub struct FeeRequestParams {
    /// Expected wait time in milliseconds.
    #[serde(default = "default_wait_time")]
    pub wait_time: u64,
    /// Transaction size in bytes.
    #[serde(default = "default_tx_size")]
    pub tx_size: usize,
}

fn default_wait_time() -> u64 {
    1000
}

fn default_tx_size() -> usize {
    300
}

/// Wait time request parameters.
#[derive(Debug, Deserialize)]
pub struct WaitTimeParams {
    /// Fee in nanoERG.
    pub fee: u64,
    /// Transaction size in bytes.
    #[serde(default = "default_tx_size")]
    pub tx_size: usize,
}

/// Fee histogram entry.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeeHistogramEntry {
    /// Number of transactions in this bin.
    pub n_txns: usize,
    /// Total size of transactions in this bin.
    pub total_size: usize,
}

/// Recommended fee response.
#[derive(Debug, Serialize)]
pub struct RecommendedFee {
    /// Recommended fee in nanoERG.
    pub fee: u64,
}

/// Wait time response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitTimeResponse {
    /// Expected wait time in milliseconds.
    pub wait_time: u64,
}

/// Transaction check response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckTxResponse {
    /// Whether the transaction is valid.
    pub success: bool,
    /// Error message if invalid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// POST /transactions
pub async fn submit_transaction(
    State(state): State<AppState>,
    Json(request): Json<SubmitTx>,
) -> ApiResult<Json<TxResponse>> {
    let bytes = hex::decode(&request.bytes)
        .map_err(|_| ApiError::BadRequest("Invalid transaction bytes".to_string()))?;

    // Parse and validate transaction
    // For now, just generate an ID
    let id = blake2b_hash(&bytes);

    let pooled = ergo_mempool::PooledTransaction {
        id: id.clone(),
        bytes,
        fee: 0,              // Would extract from transaction
        inputs: Vec::new(),  // Would extract from transaction
        outputs: Vec::new(), // Would extract from transaction
        arrival_time: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64,
    };

    state.mempool.add(pooled)?;

    Ok(Json(TxResponse {
        id: hex::encode(&id),
    }))
}

/// GET /transactions/:id
pub async fn get_transaction(
    State(_state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    // Would look up in chain + mempool
    Err(ApiError::NotFound(format!("Transaction {} not found", id)))
}

/// GET /transactions/unconfirmed
pub async fn get_unconfirmed(State(state): State<AppState>) -> ApiResult<Json<Vec<UnconfirmedTx>>> {
    let txs: Vec<UnconfirmedTx> = state
        .mempool
        .get_all_ids()
        .into_iter()
        .filter_map(|id| {
            state.mempool.get(&id).map(|tx| UnconfirmedTx {
                id: hex::encode(&tx.id),
                fee: tx.fee,
                size: tx.bytes.len(),
            })
        })
        .collect();

    Ok(Json(txs))
}

/// GET /transactions/unconfirmed/:id
pub async fn get_unconfirmed_by_id(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<UnconfirmedTx>> {
    let id_bytes =
        hex::decode(&id).map_err(|_| ApiError::BadRequest("Invalid transaction ID".to_string()))?;

    let tx = state
        .mempool
        .get(&id_bytes)
        .ok_or_else(|| ApiError::NotFound(format!("Unconfirmed tx {} not found", id)))?;

    Ok(Json(UnconfirmedTx {
        id: hex::encode(&tx.id),
        fee: tx.fee,
        size: tx.bytes.len(),
    }))
}

/// POST /transactions/check
/// Validate a transaction without submitting it.
pub async fn check_transaction(
    State(_state): State<AppState>,
    Json(request): Json<SubmitTx>,
) -> ApiResult<Json<CheckTxResponse>> {
    let bytes = hex::decode(&request.bytes)
        .map_err(|_| ApiError::BadRequest("Invalid transaction bytes".to_string()))?;

    // Basic validation - in full implementation would use tx_validation
    if bytes.len() < MIN_TX_SIZE {
        return Ok(Json(CheckTxResponse {
            success: false,
            error: Some("Transaction too small".to_string()),
        }));
    }

    // Check transaction structure (basic header validation)
    // Full validation would parse inputs/outputs and check against UTXO set
    if bytes.len() > MAX_TX_SIZE {
        return Ok(Json(CheckTxResponse {
            success: false,
            error: Some("Transaction too large".to_string()),
        }));
    }

    // For now, return success for valid-looking transactions
    // TODO: Full transaction parsing and UTXO validation
    Ok(Json(CheckTxResponse {
        success: true,
        error: None,
    }))
}

/// GET /transactions/poolHistogram
/// Get fee histogram of mempool transactions.
pub async fn get_fee_histogram(
    State(state): State<AppState>,
    Query(params): Query<FeeHistogramParams>,
) -> ApiResult<Json<Vec<FeeHistogramEntry>>> {
    let txs: Vec<_> = state
        .mempool
        .get_all_ids()
        .into_iter()
        .filter_map(|id| state.mempool.get(&id))
        .collect();

    if txs.is_empty() {
        return Ok(Json(vec![]));
    }

    // Find fee range
    let max_fee = txs.iter().map(|tx| tx.fee).max().unwrap_or(0);
    let min_fee = txs.iter().map(|tx| tx.fee).min().unwrap_or(0);
    let bin_size = if max_fee > min_fee {
        (max_fee - min_fee) / params.bins as u64 + 1
    } else {
        1
    };

    // Build histogram
    let mut histogram = vec![
        FeeHistogramEntry {
            n_txns: 0,
            total_size: 0
        };
        params.bins
    ];

    for tx in txs {
        let bin = ((tx.fee - min_fee) / bin_size) as usize;
        let bin = bin.min(params.bins - 1);
        histogram[bin].n_txns += 1;
        histogram[bin].total_size += tx.bytes.len();
    }

    Ok(Json(histogram))
}

/// GET /transactions/getFee
/// Get recommended fee for a transaction.
pub async fn get_recommended_fee(
    State(state): State<AppState>,
    Query(params): Query<FeeRequestParams>,
) -> ApiResult<Json<RecommendedFee>> {
    // Simple fee estimation based on mempool state
    let tx_count = state.mempool.len();

    // Congestion multiplier as percentage (100 = 1x, 150 = 1.5x, 200 = 2x)
    // This avoids the float-to-integer cast bug where `1.5 as u64` becomes 1
    let congestion_percent: u64 = if tx_count > 100 {
        200 // 2x multiplier for high congestion
    } else if tx_count > 50 {
        150 // 1.5x multiplier for medium congestion
    } else {
        100 // 1x multiplier for low congestion
    };

    // Scale by transaction size (fee per byte)
    let size_multiplier = (params.tx_size as u64 / 100).max(1);

    // Calculate fee: base * congestion% / 100 * size
    let fee = MIN_FEE_NANO_ERG * congestion_percent / 100 * size_multiplier;

    Ok(Json(RecommendedFee { fee }))
}

/// GET /transactions/waitTime
/// Get expected wait time for a transaction with given fee.
pub async fn get_wait_time(
    State(state): State<AppState>,
    Query(params): Query<WaitTimeParams>,
) -> ApiResult<Json<WaitTimeResponse>> {
    let tx_count = state.mempool.len();

    // Lower fee = longer wait
    let fee_factor = if params.fee >= MIN_FEE_NANO_ERG * 2 {
        1 // High fee, quick confirmation
    } else if params.fee >= MIN_FEE_NANO_ERG {
        2 // Normal fee
    } else {
        5 // Low fee, longer wait
    };

    // More transactions = longer wait
    let congestion_factor = (tx_count as u64 / 10).max(1);

    let wait_time = BASE_BLOCK_TIME_MS * fee_factor * congestion_factor;

    Ok(Json(WaitTimeResponse { wait_time }))
}

/// GET /transactions/unconfirmed with pagination
pub async fn get_unconfirmed_paged(
    State(state): State<AppState>,
    Query(params): Query<PaginationParams>,
) -> ApiResult<Json<Vec<UnconfirmedTx>>> {
    let txs: Vec<UnconfirmedTx> = state
        .mempool
        .get_all_ids()
        .into_iter()
        .skip(params.offset)
        .take(params.limit)
        .filter_map(|id| {
            state.mempool.get(&id).map(|tx| UnconfirmedTx {
                id: hex::encode(&tx.id),
                fee: tx.fee,
                size: tx.bytes.len(),
            })
        })
        .collect();

    Ok(Json(txs))
}

/// Simple BLAKE2b hash for generating IDs.
fn blake2b_hash(data: &[u8]) -> Vec<u8> {
    use blake2::digest::consts::U32;
    use blake2::{Blake2b, Digest};

    let mut hasher = Blake2b::<U32>::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}
