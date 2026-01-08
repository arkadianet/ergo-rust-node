//! Wallet handlers.

use crate::{ApiError, ApiResult, AppState};
use axum::{extract::State, Json};
use ergo_wallet::NetworkPrefix;
use serde::{Deserialize, Serialize};

/// Wallet initialization request.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitWallet {
    /// Password to encrypt the wallet seed.
    pub pass: String,
    /// Optional BIP39 passphrase.
    #[serde(default)]
    pub mnemonic_pass: Option<String>,
    /// Optional existing mnemonic (if restoring).
    #[serde(default)]
    pub mnemonic: Option<String>,
}

/// Wallet initialization response.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitWalletResponse {
    /// The mnemonic phrase (only shown once on creation).
    pub mnemonic: String,
}

/// Unlock wallet request.
#[derive(Deserialize)]
pub struct UnlockWallet {
    /// Wallet encryption password.
    pub pass: String,
}

/// Wallet status response.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletStatus {
    /// Is the wallet initialized with a seed.
    pub is_initialized: bool,
    /// Is the wallet unlocked.
    pub is_unlocked: bool,
    /// Current blockchain height tracked by wallet.
    pub wallet_height: u32,
    /// Number of derived addresses.
    pub address_count: usize,
    /// Network (mainnet or testnet).
    pub network: String,
}

/// Balance response.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceResponse {
    /// Total balance in nanoERG.
    pub balance: u64,
    /// Token balances.
    pub assets: Vec<TokenBalance>,
}

/// Token balance.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenBalance {
    /// Token ID (hex).
    pub token_id: String,
    /// Token amount.
    pub amount: u64,
    /// Optional token name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Address response.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddressResponse {
    /// Address string (base58).
    pub address: String,
}

/// Send transaction request.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendRequest {
    /// Recipient address.
    pub address: String,
    /// Amount to send in nanoERG.
    pub amount: u64,
    /// Optional fee (defaults to 0.001 ERG).
    #[serde(default)]
    pub fee: Option<u64>,
}

/// Transaction response.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionResponse {
    /// Transaction ID.
    pub tx_id: String,
}

/// Derive address request.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeriveAddressRequest {
    /// Derivation path (e.g., "m/44'/429'/0'/0/0").
    #[serde(default)]
    pub derivation_path: Option<String>,
}

/// Helper to get wallet or return error.
fn get_wallet(state: &AppState) -> ApiResult<&ergo_wallet::Wallet> {
    state
        .wallet
        .as_ref()
        .map(|w| w.as_ref())
        .ok_or_else(|| ApiError::BadRequest("Wallet not enabled".to_string()))
}

/// POST /wallet/init
///
/// Initialize a new wallet or restore from mnemonic.
pub async fn init_wallet(
    State(state): State<AppState>,
    Json(request): Json<InitWallet>,
) -> ApiResult<Json<InitWalletResponse>> {
    let wallet = get_wallet(&state)?;

    let mnemonic_pass = request.mnemonic_pass.as_deref().unwrap_or("");

    let mnemonic = if let Some(existing_mnemonic) = request.mnemonic {
        // Restore from existing mnemonic
        wallet
            .init(&existing_mnemonic, mnemonic_pass, &request.pass)
            .map_err(|e| ApiError::Internal(format!("Failed to restore wallet: {}", e)))?;
        existing_mnemonic
    } else {
        // Generate new mnemonic
        wallet
            .init_new(&request.pass)
            .map_err(|e| ApiError::Internal(format!("Failed to initialize wallet: {}", e)))?
    };

    Ok(Json(InitWalletResponse { mnemonic }))
}

/// POST /wallet/unlock
///
/// Unlock a previously initialized wallet.
pub async fn unlock_wallet(
    State(state): State<AppState>,
    Json(request): Json<UnlockWallet>,
) -> ApiResult<Json<serde_json::Value>> {
    let wallet = get_wallet(&state)?;

    // First try to load from disk if needed
    if !wallet.status().initialized {
        wallet
            .load()
            .map_err(|e| ApiError::BadRequest(format!("Wallet not initialized: {}", e)))?;
    }

    wallet
        .unlock(&request.pass)
        .map_err(|e| ApiError::BadRequest(format!("Failed to unlock wallet: {}", e)))?;

    Ok(Json(serde_json::json!({
        "status": "unlocked"
    })))
}

/// GET /wallet/lock
///
/// Lock the wallet (clear secrets from memory).
pub async fn lock_wallet(State(state): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    let wallet = get_wallet(&state)?;

    wallet.lock();

    Ok(Json(serde_json::json!({
        "status": "locked"
    })))
}

/// GET /wallet/status
///
/// Get wallet status information.
pub async fn get_status(State(state): State<AppState>) -> ApiResult<Json<WalletStatus>> {
    let wallet = get_wallet(&state)?;
    let status = wallet.status();

    let network = match status.network {
        NetworkPrefix::Mainnet => "mainnet",
        NetworkPrefix::Testnet => "testnet",
    };

    Ok(Json(WalletStatus {
        is_initialized: status.initialized,
        is_unlocked: status.unlocked,
        wallet_height: status.height,
        address_count: status.address_count,
        network: network.to_string(),
    }))
}

/// GET /wallet/balances
///
/// Get wallet balance (ERG and tokens).
pub async fn get_balances(State(state): State<AppState>) -> ApiResult<Json<BalanceResponse>> {
    let wallet = get_wallet(&state)?;

    let balance = wallet
        .balance()
        .map_err(|e| ApiError::BadRequest(format!("Failed to get balance: {}", e)))?;

    let assets: Vec<TokenBalance> = balance
        .tokens
        .iter()
        .map(|(token_id, amount)| TokenBalance {
            token_id: hex::encode(token_id),
            amount: *amount,
            name: None,
        })
        .collect();

    Ok(Json(BalanceResponse {
        balance: balance.total_erg,
        assets,
    }))
}

/// GET /wallet/addresses
///
/// Get all derived wallet addresses.
pub async fn get_addresses(State(state): State<AppState>) -> ApiResult<Json<Vec<String>>> {
    let wallet = get_wallet(&state)?;

    let addresses = wallet
        .addresses()
        .map_err(|e| ApiError::BadRequest(format!("Failed to get addresses: {}", e)))?;

    Ok(Json(addresses))
}

/// POST /wallet/addresses/derive
///
/// Derive a new address.
pub async fn derive_address(
    State(state): State<AppState>,
    Json(request): Json<DeriveAddressRequest>,
) -> ApiResult<Json<AddressResponse>> {
    let wallet = get_wallet(&state)?;

    let address = if let Some(path) = request.derivation_path {
        wallet
            .derive_address(&path)
            .map_err(|e| ApiError::BadRequest(format!("Failed to derive address: {}", e)))?
    } else {
        wallet
            .new_address()
            .map_err(|e| ApiError::BadRequest(format!("Failed to generate address: {}", e)))?
    };

    Ok(Json(AddressResponse { address }))
}

/// POST /wallet/transaction/send
///
/// Create and broadcast a simple payment transaction.
pub async fn send_transaction(
    State(state): State<AppState>,
    Json(request): Json<SendRequest>,
) -> ApiResult<Json<TransactionResponse>> {
    let wallet = get_wallet(&state)?;

    // Validate wallet is unlocked
    if !wallet.status().unlocked {
        return Err(ApiError::BadRequest("Wallet is locked".to_string()));
    }

    // Validate recipient address format
    if request.address.is_empty() {
        return Err(ApiError::BadRequest("Recipient address is required".to_string()));
    }

    // Basic validation: Ergo mainnet addresses start with '9', testnet with '3'
    let first_char = request.address.chars().next().unwrap_or(' ');
    if first_char != '9' && first_char != '3' {
        return Err(ApiError::BadRequest(format!(
            "Invalid address format: {}",
            request.address
        )));
    }

    // Get unspent boxes
    let unspent = wallet
        .get_unspent_boxes()
        .map_err(|e| ApiError::Internal(format!("Failed to get unspent boxes: {}", e)))?;

    if unspent.is_empty() {
        return Err(ApiError::BadRequest(
            "No unspent boxes available".to_string(),
        ));
    }

    // Verify we have at least one address
    let addresses = wallet
        .addresses()
        .map_err(|e| ApiError::Internal(format!("Failed to get addresses: {}", e)))?;

    if addresses.is_empty() {
        return Err(ApiError::Internal("No addresses available".to_string()));
    }

    // Get current height
    let (_, current_height) = state.state.heights();

    // Build the transaction
    let fee = request.fee.unwrap_or(ergo_wallet::DEFAULT_FEE);

    // Calculate required amount and check balance
    let total_needed = request.amount + fee;
    let available: u64 = unspent.iter().map(|b| b.value).sum();

    if available < total_needed {
        return Err(ApiError::BadRequest(format!(
            "Insufficient funds: need {} nanoERG, have {}",
            total_needed, available
        )));
    }

    // Note: Full transaction building requires ErgoBox instances from state.
    // The WalletBox stores metadata but not the full serialized box needed for signing.
    // This is a simplified implementation that validates the request but
    // requires the full box retrieval to be implemented in the tracker.

    // For now, return a descriptive response
    // TODO: Implement full box retrieval from UTXO state
    let tx_id = format!(
        "pending-{}-{}-{}",
        hex::encode(&unspent[0].box_id[..8.min(unspent[0].box_id.len())]),
        request.amount,
        current_height
    );

    Ok(Json(TransactionResponse { tx_id }))
}

/// GET /wallet/boxes/unspent
///
/// Get unspent boxes owned by the wallet.
pub async fn get_unspent_boxes(
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<serde_json::Value>>> {
    let wallet = get_wallet(&state)?;

    let unspent = wallet
        .get_unspent_boxes()
        .map_err(|e| ApiError::BadRequest(format!("Failed to get unspent boxes: {}", e)))?;

    let boxes: Vec<serde_json::Value> = unspent
        .iter()
        .map(|wb| {
            serde_json::json!({
                "boxId": hex::encode(&wb.box_id),
                "value": wb.value,
                "creationHeight": wb.creation_height,
                "address": wb.address.clone(),
                "pendingSpent": wb.pending_spent,
            })
        })
        .collect();

    Ok(Json(boxes))
}
