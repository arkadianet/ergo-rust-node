//! Blockchain indexer API endpoints.
//!
//! Provides API endpoints for querying indexed blockchain data:
//! - Transactions by ID, index, or address
//! - Boxes by ID, index, address, token, or ErgoTree
//! - Token information
//! - Address balances

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use ergo_storage::IndexedErgoBox;
use serde::{Deserialize, Serialize};

use crate::AppState;

/// Pagination parameters.
#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    5
}

/// Sort direction.
#[derive(Debug, Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum SortDirection {
    Asc,
    #[default]
    Desc,
}

/// Indexed height response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexedHeightResponse {
    pub indexed_height: u32,
    pub full_height: u32,
}

/// Transaction response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionResponse {
    pub id: String,
    pub tx_index: u16,
    pub height: u32,
    pub size: u32,
    pub global_index: u64,
    pub inputs: Vec<u64>,
    pub outputs: Vec<u64>,
    pub data_inputs: Vec<String>,
}

/// Box response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoxResponse {
    pub box_id: String,
    pub inclusion_height: u32,
    pub global_index: u64,
    pub value: u64,
    pub ergo_tree_hash: String,
    pub spent: bool,
    pub spending_tx_id: Option<String>,
    pub spending_height: Option<u32>,
    pub tokens: Vec<TokenAmount>,
}

/// Token amount in a box.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenAmount {
    pub token_id: String,
    pub amount: u64,
}

/// Token info response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenInfoResponse {
    pub token_id: String,
    pub creation_box_id: Option<String>,
    pub amount: Option<u64>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub decimals: Option<u8>,
    pub box_count: usize,
}

/// Address balance response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddressBalanceResponse {
    pub address_hash: String,
    pub nano_ergs: i64,
    pub tokens: Vec<TokenBalance>,
    pub tx_count: usize,
    pub box_count: usize,
    pub unspent_box_count: usize,
}

/// Token balance.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenBalance {
    pub token_id: String,
    pub amount: i64,
}

/// Paginated response wrapper.
#[derive(Debug, Serialize)]
pub struct PaginatedResponse<T> {
    pub items: Vec<T>,
    pub total: u64,
}

/// Error response.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Maximum items per request.
const MAX_ITEMS: usize = 16384;

/// Check if indexer is enabled and return error if not.
fn check_indexer_enabled(state: &AppState) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if let Some(ref indexer) = state.indexer {
        if indexer.is_enabled() {
            return Ok(());
        }
    }
    Err((
        StatusCode::SERVICE_UNAVAILABLE,
        Json(ErrorResponse {
            error: "Extra indexing is not enabled".to_string(),
        }),
    ))
}

/// GET /blockchain/indexedHeight
///
/// Get the current indexed height and full blockchain height.
pub async fn get_indexed_height(
    State(state): State<AppState>,
) -> Result<Json<IndexedHeightResponse>, (StatusCode, Json<ErrorResponse>)> {
    check_indexer_enabled(&state)?;

    let indexer = state.indexer.as_ref().unwrap();
    let indexed_height = indexer.indexed_height();

    // Get full height from state
    let (full_height, _) = state.state.heights();

    Ok(Json(IndexedHeightResponse {
        indexed_height,
        full_height,
    }))
}

/// GET /blockchain/transaction/byId/{id}
///
/// Get transaction by its ID.
pub async fn get_transaction_by_id(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Option<TransactionResponse>>, (StatusCode, Json<ErrorResponse>)> {
    check_indexer_enabled(&state)?;

    let tx_id = parse_hex_id(&id)?;
    let indexer = state.indexer.as_ref().unwrap();

    let tx = indexer
        .get_transaction(&tx_id)
        .map(|itx| TransactionResponse {
            id: hex::encode(itx.tx_id),
            tx_index: itx.tx_index,
            height: itx.height,
            size: itx.size,
            global_index: itx.global_index,
            inputs: itx.input_indexes,
            outputs: itx.output_indexes,
            data_inputs: itx.data_inputs.iter().map(hex::encode).collect(),
        });

    Ok(Json(tx))
}

/// GET /blockchain/transaction/byIndex/{index}
///
/// Get transaction by its global index.
pub async fn get_transaction_by_index(
    State(state): State<AppState>,
    Path(index): Path<u64>,
) -> Result<Json<Option<TransactionResponse>>, (StatusCode, Json<ErrorResponse>)> {
    check_indexer_enabled(&state)?;

    let indexer = state.indexer.as_ref().unwrap();

    // Look up tx ID by global index, then get full tx
    // For now, we'd need to add a get_transaction_by_index method
    // This is a placeholder - would need TxNumericIndex lookup
    let _ = indexer;
    let _ = index;

    Ok(Json(None))
}

/// GET /blockchain/box/byId/{id}
///
/// Get box by its ID.
pub async fn get_box_by_id(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Option<BoxResponse>>, (StatusCode, Json<ErrorResponse>)> {
    check_indexer_enabled(&state)?;

    let box_id = parse_hex_id(&id)?;
    let indexer = state.indexer.as_ref().unwrap();

    let response = indexer.get_box(&box_id).map(|ieb| to_box_response(&ieb));

    Ok(Json(response))
}

/// GET /blockchain/box/byIndex/{index}
///
/// Get box by its global index.
pub async fn get_box_by_index(
    State(state): State<AppState>,
    Path(index): Path<u64>,
) -> Result<Json<Option<BoxResponse>>, (StatusCode, Json<ErrorResponse>)> {
    check_indexer_enabled(&state)?;

    let indexer = state.indexer.as_ref().unwrap();

    let response = indexer
        .get_box_by_index(index)
        .map(|ieb| to_box_response(&ieb));

    Ok(Json(response))
}

/// GET /blockchain/token/byId/{id}
///
/// Get token information by ID.
pub async fn get_token_by_id(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Option<TokenInfoResponse>>, (StatusCode, Json<ErrorResponse>)> {
    check_indexer_enabled(&state)?;

    let token_id = parse_hex_id(&id)?;
    let indexer = state.indexer.as_ref().unwrap();

    let response = indexer.get_token(&token_id).map(|token| {
        let box_count = token.box_count();
        TokenInfoResponse {
            token_id: hex::encode(token.token_id),
            creation_box_id: token.creation_box_id.map(hex::encode),
            amount: token.amount,
            name: token.name,
            description: token.description,
            decimals: token.decimals,
            box_count,
        }
    });

    Ok(Json(response))
}

/// Address query params.
#[derive(Debug, Deserialize)]
pub struct AddressQuery {
    pub address: String,
}

/// GET /blockchain/balance/byAddress
///
/// Get balance for an address (by ErgoTree hash).
pub async fn get_balance_by_address(
    State(state): State<AppState>,
    Query(query): Query<AddressQuery>,
) -> Result<Json<Option<AddressBalanceResponse>>, (StatusCode, Json<ErrorResponse>)> {
    check_indexer_enabled(&state)?;

    let tree_hash = parse_hex_id(&query.address)?;
    let indexer = state.indexer.as_ref().unwrap();

    let response = indexer
        .get_address(&tree_hash)
        .map(|addr| AddressBalanceResponse {
            address_hash: hex::encode(addr.tree_hash),
            nano_ergs: addr.balance.nano_ergs,
            tokens: addr
                .balance
                .tokens
                .iter()
                .map(|(id, amount)| TokenBalance {
                    token_id: hex::encode(id),
                    amount: *amount,
                })
                .collect(),
            tx_count: addr.tx_count(),
            box_count: addr.box_count(),
            unspent_box_count: addr.unspent_box_count(),
        });

    Ok(Json(response))
}

/// Boxes by address query params.
#[derive(Debug, Deserialize)]
pub struct BoxesByAddressQuery {
    pub address: String,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub unspent_only: bool,
}

/// GET /blockchain/box/byAddress
///
/// Get boxes for an address.
pub async fn get_boxes_by_address(
    State(state): State<AppState>,
    Query(query): Query<BoxesByAddressQuery>,
) -> Result<Json<PaginatedResponse<BoxResponse>>, (StatusCode, Json<ErrorResponse>)> {
    check_indexer_enabled(&state)?;

    if query.limit > MAX_ITEMS {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("No more than {} boxes can be requested", MAX_ITEMS),
            }),
        ));
    }

    let tree_hash = parse_hex_id(&query.address)?;
    let indexer = state.indexer.as_ref().unwrap();

    let addr = indexer.get_address(&tree_hash);

    let (items, total) = match addr {
        Some(addr) => {
            let box_indexes: Vec<u64> = if query.unspent_only {
                addr.unspent_boxes()
            } else {
                addr.box_indexes
                    .iter()
                    .map(|&idx| idx.unsigned_abs())
                    .collect()
            };

            let total = box_indexes.len() as u64;
            let boxes: Vec<BoxResponse> = box_indexes
                .into_iter()
                .skip(query.offset)
                .take(query.limit)
                .filter_map(|idx| indexer.get_box_by_index(idx))
                .map(|ieb| to_box_response(&ieb))
                .collect();

            (boxes, total)
        }
        None => (Vec::new(), 0),
    };

    Ok(Json(PaginatedResponse { items, total }))
}

/// Boxes by token query params.
#[derive(Debug, Deserialize)]
pub struct BoxesByTokenQuery {
    pub token_id: String,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub unspent_only: bool,
}

/// GET /blockchain/box/byTokenId
///
/// Get boxes containing a specific token.
pub async fn get_boxes_by_token_id(
    State(state): State<AppState>,
    Query(query): Query<BoxesByTokenQuery>,
) -> Result<Json<PaginatedResponse<BoxResponse>>, (StatusCode, Json<ErrorResponse>)> {
    check_indexer_enabled(&state)?;

    if query.limit > MAX_ITEMS {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("No more than {} boxes can be requested", MAX_ITEMS),
            }),
        ));
    }

    let token_id = parse_hex_id(&query.token_id)?;
    let indexer = state.indexer.as_ref().unwrap();

    let token = indexer.get_token(&token_id);

    let (items, total) = match token {
        Some(token) => {
            let box_indexes: Vec<u64> = if query.unspent_only {
                token
                    .box_indexes
                    .iter()
                    .filter(|&&idx| idx > 0)
                    .map(|&idx| idx as u64)
                    .collect()
            } else {
                token
                    .box_indexes
                    .iter()
                    .map(|&idx| idx.unsigned_abs())
                    .collect()
            };

            let total = box_indexes.len() as u64;
            let boxes: Vec<BoxResponse> = box_indexes
                .into_iter()
                .skip(query.offset)
                .take(query.limit)
                .filter_map(|idx| indexer.get_box_by_index(idx))
                .map(|ieb| to_box_response(&ieb))
                .collect();

            (boxes, total)
        }
        None => (Vec::new(), 0),
    };

    Ok(Json(PaginatedResponse { items, total }))
}

/// Parse a hex string to a 32-byte ID.
fn parse_hex_id(hex_str: &str) -> Result<[u8; 32], (StatusCode, Json<ErrorResponse>)> {
    let bytes = hex::decode(hex_str).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Invalid hex string".to_string(),
            }),
        )
    })?;

    if bytes.len() != 32 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "ID must be 32 bytes".to_string(),
            }),
        ));
    }

    let mut id = [0u8; 32];
    id.copy_from_slice(&bytes);
    Ok(id)
}

/// Convert IndexedErgoBox to BoxResponse.
fn to_box_response(ieb: &IndexedErgoBox) -> BoxResponse {
    BoxResponse {
        box_id: hex::encode(ieb.box_id),
        inclusion_height: ieb.inclusion_height,
        global_index: ieb.global_index,
        value: ieb.value,
        ergo_tree_hash: hex::encode(ieb.ergo_tree_hash),
        spent: ieb.is_spent(),
        spending_tx_id: ieb.spending_tx_id.map(hex::encode),
        spending_height: ieb.spending_height,
        tokens: ieb
            .tokens
            .iter()
            .map(|(id, amount)| TokenAmount {
                token_id: hex::encode(id),
                amount: *amount,
            })
            .collect(),
    }
}
