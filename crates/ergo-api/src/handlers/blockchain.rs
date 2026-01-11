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
use utoipa::{IntoParams, ToSchema};

use crate::AppState;

/// Pagination parameters for blockchain queries.
#[derive(Debug, Deserialize, IntoParams)]
pub struct BlockchainPagination {
    /// Offset for pagination.
    #[serde(default)]
    pub offset: usize,
    /// Limit (default: 5).
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    5
}

/// Sort direction.
#[derive(Debug, Deserialize, Default, Clone, Copy, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SortDirection {
    /// Ascending order.
    Asc,
    /// Descending order.
    #[default]
    Desc,
}

/// Indexed height response.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IndexedHeightResponse {
    /// Current indexed height.
    #[schema(example = 1000000)]
    pub indexed_height: u32,
    /// Full blockchain height.
    #[schema(example = 1000100)]
    pub full_height: u32,
}

/// Indexed transaction response.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IndexedTransactionResponse {
    /// Transaction ID (hex).
    #[schema(example = "0000000000000000000000000000000000000000000000000000000000000000")]
    pub id: String,
    /// Transaction index within block.
    #[schema(example = 0)]
    pub tx_index: u16,
    /// Block height.
    #[schema(example = 1000000)]
    pub height: u32,
    /// Transaction size in bytes.
    #[schema(example = 300)]
    pub size: u32,
    /// Global transaction index.
    #[schema(example = 5000000)]
    pub global_index: u64,
    /// Input box indexes.
    pub inputs: Vec<u64>,
    /// Output box indexes.
    pub outputs: Vec<u64>,
    /// Data input box IDs.
    pub data_inputs: Vec<String>,
}

/// Indexed box response.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IndexedBoxResponse {
    /// Box ID (hex).
    #[schema(example = "0000000000000000000000000000000000000000000000000000000000000000")]
    pub box_id: String,
    /// Block height where box was created.
    #[schema(example = 1000000)]
    pub inclusion_height: u32,
    /// Global box index.
    #[schema(example = 10000000)]
    pub global_index: u64,
    /// Box value in nanoERG.
    #[schema(example = 1000000000)]
    pub value: u64,
    /// ErgoTree hash (hex).
    #[schema(example = "0000000000000000000000000000000000000000000000000000000000000000")]
    pub ergo_tree_hash: String,
    /// Whether box is spent.
    pub spent: bool,
    /// Spending transaction ID (if spent).
    pub spending_tx_id: Option<String>,
    /// Spending height (if spent).
    pub spending_height: Option<u32>,
    /// Tokens in box.
    pub tokens: Vec<BoxTokenAmount>,
}

/// Token amount in a box.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BoxTokenAmount {
    /// Token ID (hex).
    #[schema(example = "0000000000000000000000000000000000000000000000000000000000000000")]
    pub token_id: String,
    /// Token amount.
    #[schema(example = 1000)]
    pub amount: u64,
}

/// Token info response.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TokenInfoResponse {
    /// Token ID (hex).
    #[schema(example = "0000000000000000000000000000000000000000000000000000000000000000")]
    pub token_id: String,
    /// Box ID where token was created.
    pub creation_box_id: Option<String>,
    /// Total token amount.
    pub amount: Option<u64>,
    /// Token name.
    #[schema(example = "MyToken")]
    pub name: Option<String>,
    /// Token description.
    pub description: Option<String>,
    /// Token decimals.
    #[schema(example = 8)]
    pub decimals: Option<u8>,
    /// Number of boxes containing this token.
    #[schema(example = 100)]
    pub box_count: usize,
}

/// Address balance response.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddressBalanceResponse {
    /// Address hash (ErgoTree hash, hex).
    #[schema(example = "0000000000000000000000000000000000000000000000000000000000000000")]
    pub address_hash: String,
    /// Balance in nanoERG.
    #[schema(example = 1000000000)]
    pub nano_ergs: i64,
    /// Token balances.
    pub tokens: Vec<IndexedTokenBalance>,
    /// Number of transactions.
    #[schema(example = 50)]
    pub tx_count: usize,
    /// Total box count.
    #[schema(example = 100)]
    pub box_count: usize,
    /// Unspent box count.
    #[schema(example = 10)]
    pub unspent_box_count: usize,
}

/// Indexed token balance.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IndexedTokenBalance {
    /// Token ID (hex).
    #[schema(example = "0000000000000000000000000000000000000000000000000000000000000000")]
    pub token_id: String,
    /// Token balance (can be negative for spent).
    #[schema(example = 1000)]
    pub amount: i64,
}

/// Paginated box response.
#[derive(Debug, Serialize, ToSchema)]
pub struct PaginatedBoxResponse {
    /// Response items.
    pub items: Vec<IndexedBoxResponse>,
    /// Total count.
    #[schema(example = 100)]
    pub total: u64,
}

/// Generic paginated response wrapper (internal use).
#[derive(Debug, Serialize)]
pub struct PaginatedResponse<T> {
    /// Response items.
    pub items: Vec<T>,
    /// Total count.
    pub total: u64,
}

/// Blockchain error response.
#[derive(Debug, Serialize, ToSchema)]
pub struct BlockchainErrorResponse {
    /// Error message.
    #[schema(example = "Extra indexing is not enabled")]
    pub error: String,
}

/// Maximum items per request.
const MAX_ITEMS: usize = 16384;

/// Check if indexer is enabled and return error if not.
fn check_indexer_enabled(
    state: &AppState,
) -> Result<(), (StatusCode, Json<BlockchainErrorResponse>)> {
    if let Some(ref indexer) = state.indexer {
        if indexer.is_enabled() {
            return Ok(());
        }
    }
    Err((
        StatusCode::SERVICE_UNAVAILABLE,
        Json(BlockchainErrorResponse {
            error: "Extra indexing is not enabled".to_string(),
        }),
    ))
}

/// GET /blockchain/indexedHeight
///
/// Get the current indexed height and full blockchain height.
#[utoipa::path(
    get,
    path = "/blockchain/indexedHeight",
    tag = "blockchain",
    responses(
        (status = 200, description = "Indexed height info", body = IndexedHeightResponse),
        (status = 503, description = "Indexer not enabled", body = BlockchainErrorResponse)
    )
)]
pub async fn get_indexed_height(
    State(state): State<AppState>,
) -> Result<Json<IndexedHeightResponse>, (StatusCode, Json<BlockchainErrorResponse>)> {
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
#[utoipa::path(
    get,
    path = "/blockchain/transaction/byId/{id}",
    tag = "blockchain",
    params(("id" = String, Path, description = "Transaction ID (hex)")),
    responses(
        (status = 200, description = "Transaction found", body = Option<IndexedTransactionResponse>),
        (status = 503, description = "Indexer not enabled", body = BlockchainErrorResponse)
    )
)]
pub async fn get_transaction_by_id(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Option<IndexedTransactionResponse>>, (StatusCode, Json<BlockchainErrorResponse>)> {
    check_indexer_enabled(&state)?;

    let tx_id = parse_hex_id(&id)?;
    let indexer = state.indexer.as_ref().unwrap();

    let tx = indexer
        .get_transaction(&tx_id)
        .map(|itx| IndexedTransactionResponse {
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
#[utoipa::path(
    get,
    path = "/blockchain/transaction/byIndex/{index}",
    tag = "blockchain",
    params(("index" = u64, Path, description = "Global transaction index")),
    responses(
        (status = 200, description = "Transaction found", body = Option<IndexedTransactionResponse>),
        (status = 503, description = "Indexer not enabled", body = BlockchainErrorResponse)
    )
)]
pub async fn get_transaction_by_index(
    State(state): State<AppState>,
    Path(index): Path<u64>,
) -> Result<Json<Option<IndexedTransactionResponse>>, (StatusCode, Json<BlockchainErrorResponse>)> {
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
#[utoipa::path(
    get,
    path = "/blockchain/box/byId/{id}",
    tag = "blockchain",
    params(("id" = String, Path, description = "Box ID (hex)")),
    responses(
        (status = 200, description = "Box found", body = Option<IndexedBoxResponse>),
        (status = 503, description = "Indexer not enabled", body = BlockchainErrorResponse)
    )
)]
pub async fn get_box_by_id(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Option<IndexedBoxResponse>>, (StatusCode, Json<BlockchainErrorResponse>)> {
    check_indexer_enabled(&state)?;

    let box_id = parse_hex_id(&id)?;
    let indexer = state.indexer.as_ref().unwrap();

    let response = indexer.get_box(&box_id).map(|ieb| to_box_response(&ieb));

    Ok(Json(response))
}

/// GET /blockchain/box/byIndex/{index}
///
/// Get box by its global index.
#[utoipa::path(
    get,
    path = "/blockchain/box/byIndex/{index}",
    tag = "blockchain",
    params(("index" = u64, Path, description = "Global box index")),
    responses(
        (status = 200, description = "Box found", body = Option<IndexedBoxResponse>),
        (status = 503, description = "Indexer not enabled", body = BlockchainErrorResponse)
    )
)]
pub async fn get_box_by_index(
    State(state): State<AppState>,
    Path(index): Path<u64>,
) -> Result<Json<Option<IndexedBoxResponse>>, (StatusCode, Json<BlockchainErrorResponse>)> {
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
#[utoipa::path(
    get,
    path = "/blockchain/token/byId/{id}",
    tag = "blockchain",
    params(("id" = String, Path, description = "Token ID (hex)")),
    responses(
        (status = 200, description = "Token info", body = Option<TokenInfoResponse>),
        (status = 503, description = "Indexer not enabled", body = BlockchainErrorResponse)
    )
)]
pub async fn get_token_by_id(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Option<TokenInfoResponse>>, (StatusCode, Json<BlockchainErrorResponse>)> {
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
#[derive(Debug, Deserialize, IntoParams)]
pub struct AddressQuery {
    /// Address (ErgoTree hash, hex).
    pub address: String,
}

/// GET /blockchain/balance/byAddress
///
/// Get balance for an address (by ErgoTree hash).
#[utoipa::path(
    get,
    path = "/blockchain/balance/byAddress",
    tag = "blockchain",
    params(AddressQuery),
    responses(
        (status = 200, description = "Address balance", body = Option<AddressBalanceResponse>),
        (status = 503, description = "Indexer not enabled", body = BlockchainErrorResponse)
    )
)]
pub async fn get_balance_by_address(
    State(state): State<AppState>,
    Query(query): Query<AddressQuery>,
) -> Result<Json<Option<AddressBalanceResponse>>, (StatusCode, Json<BlockchainErrorResponse>)> {
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
                .map(|(id, amount)| IndexedTokenBalance {
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
#[derive(Debug, Deserialize, IntoParams)]
pub struct BoxesByAddressQuery {
    /// Address (ErgoTree hash, hex).
    pub address: String,
    /// Offset for pagination.
    #[serde(default)]
    pub offset: usize,
    /// Limit (default: 5).
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Only return unspent boxes.
    #[serde(default)]
    pub unspent_only: bool,
}

/// GET /blockchain/box/byAddress
///
/// Get boxes for an address.
#[utoipa::path(
    get,
    path = "/blockchain/box/byAddress",
    tag = "blockchain",
    params(BoxesByAddressQuery),
    responses(
        (status = 200, description = "Paginated boxes", body = PaginatedBoxResponse),
        (status = 400, description = "Invalid request", body = BlockchainErrorResponse),
        (status = 503, description = "Indexer not enabled", body = BlockchainErrorResponse)
    )
)]
pub async fn get_boxes_by_address(
    State(state): State<AppState>,
    Query(query): Query<BoxesByAddressQuery>,
) -> Result<Json<PaginatedResponse<IndexedBoxResponse>>, (StatusCode, Json<BlockchainErrorResponse>)>
{
    check_indexer_enabled(&state)?;

    if query.limit > MAX_ITEMS {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(BlockchainErrorResponse {
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
            let boxes: Vec<IndexedBoxResponse> = box_indexes
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
#[derive(Debug, Deserialize, IntoParams)]
pub struct BoxesByTokenQuery {
    /// Token ID (hex).
    pub token_id: String,
    /// Offset for pagination.
    #[serde(default)]
    pub offset: usize,
    /// Limit (default: 5).
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Only return unspent boxes.
    #[serde(default)]
    pub unspent_only: bool,
}

/// GET /blockchain/box/byTokenId
///
/// Get boxes containing a specific token.
#[utoipa::path(
    get,
    path = "/blockchain/box/byTokenId",
    tag = "blockchain",
    params(BoxesByTokenQuery),
    responses(
        (status = 200, description = "Paginated boxes", body = PaginatedBoxResponse),
        (status = 400, description = "Invalid request", body = BlockchainErrorResponse),
        (status = 503, description = "Indexer not enabled", body = BlockchainErrorResponse)
    )
)]
pub async fn get_boxes_by_token_id(
    State(state): State<AppState>,
    Query(query): Query<BoxesByTokenQuery>,
) -> Result<Json<PaginatedResponse<IndexedBoxResponse>>, (StatusCode, Json<BlockchainErrorResponse>)>
{
    check_indexer_enabled(&state)?;

    if query.limit > MAX_ITEMS {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(BlockchainErrorResponse {
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
            let boxes: Vec<IndexedBoxResponse> = box_indexes
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
fn parse_hex_id(hex_str: &str) -> Result<[u8; 32], (StatusCode, Json<BlockchainErrorResponse>)> {
    let bytes = hex::decode(hex_str).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(BlockchainErrorResponse {
                error: "Invalid hex string".to_string(),
            }),
        )
    })?;

    if bytes.len() != 32 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(BlockchainErrorResponse {
                error: "ID must be 32 bytes".to_string(),
            }),
        ));
    }

    let mut id = [0u8; 32];
    id.copy_from_slice(&bytes);
    Ok(id)
}

/// Convert IndexedErgoBox to IndexedBoxResponse.
fn to_box_response(ieb: &IndexedErgoBox) -> IndexedBoxResponse {
    IndexedBoxResponse {
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
            .map(|(id, amount)| BoxTokenAmount {
                token_id: hex::encode(id),
                amount: *amount,
            })
            .collect(),
    }
}
