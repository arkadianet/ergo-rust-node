//! Transaction building and signing for the wallet.
//!
//! This module provides a high-level API for building and signing
//! Ergo transactions using wallet-managed keys.

use ergo_lib::{
    chain::{
        ergo_box::box_builder::{ErgoBoxCandidateBuilder, ErgoBoxCandidateBuilderError},
        ergo_state_context::ErgoStateContext,
        transaction::{unsigned::UnsignedTransaction, Transaction},
    },
    ergotree_ir::{
        chain::{
            address::{Address, AddressEncoder, NetworkPrefix},
            ergo_box::{box_value::BoxValue, ErgoBox, ErgoBoxCandidate},
            token::Token,
        },
        serialization::SigmaParsingError,
    },
    wallet::{
        box_selector::{BoxSelection, BoxSelector, BoxSelectorError, SimpleBoxSelector},
        signing::{TransactionContext, TxSigningError},
        tx_builder::{TxBuilder, TxBuilderError},
        tx_context::TransactionContextError,
        Wallet as ErgoLibWallet, WalletError as ErgoLibWalletError,
    },
};
use std::convert::TryFrom;
use thiserror::Error;

/// Minimum box value in nanoERG (0.001 ERG).
pub const MIN_BOX_VALUE: u64 = 1_000_000;

/// Default transaction fee in nanoERG (0.001 ERG).
pub const DEFAULT_FEE: u64 = 1_000_000;

/// Errors during transaction building.
#[derive(Error, Debug)]
pub enum TxBuildError {
    /// Not enough funds.
    #[error("Insufficient funds: need {needed} nanoERG, have {available}")]
    InsufficientFunds { needed: u64, available: u64 },

    /// Not enough tokens.
    #[error("Insufficient tokens: {0}")]
    InsufficientTokens(String),

    /// Box selection failed.
    #[error("Box selection failed: {0}")]
    BoxSelection(#[from] BoxSelectorError),

    /// Transaction builder error.
    #[error("Transaction builder error: {0}")]
    TxBuilder(#[from] TxBuilderError),

    /// Box candidate builder error.
    #[error("Box candidate error: {0}")]
    BoxCandidate(#[from] ErgoBoxCandidateBuilderError),

    /// Signing error.
    #[error("Signing error: {0}")]
    Signing(#[from] TxSigningError),

    /// Transaction context error.
    #[error("Transaction context error: {0}")]
    TxContext(#[from] TransactionContextError),

    /// Wallet error.
    #[error("Wallet error: {0}")]
    Wallet(#[from] ErgoLibWalletError),

    /// Script parsing error.
    #[error("Script parsing error: {0}")]
    ScriptParsing(#[from] SigmaParsingError),

    /// Invalid address.
    #[error("Invalid address: {0}")]
    InvalidAddress(String),

    /// Invalid amount.
    #[error("Invalid amount: {0}")]
    InvalidAmount(String),

    /// No inputs available.
    #[error("No inputs available")]
    NoInputs,

    /// No change address.
    #[error("No change address set")]
    NoChangeAddress,
}

/// A simple payment request.
#[derive(Debug, Clone)]
pub struct PaymentRequest {
    /// Recipient address (base58 encoded).
    pub recipient: String,
    /// Amount in nanoERG.
    pub amount: u64,
    /// Optional tokens to send.
    pub tokens: Vec<Token>,
}

impl PaymentRequest {
    /// Create a new payment request.
    pub fn new(recipient: impl Into<String>, amount: u64) -> Self {
        Self {
            recipient: recipient.into(),
            amount,
            tokens: Vec::new(),
        }
    }

    /// Add tokens to the payment.
    pub fn with_tokens(mut self, tokens: Vec<Token>) -> Self {
        self.tokens = tokens;
        self
    }
}

/// Transaction builder for the wallet.
///
/// Provides a simple API for common transaction patterns.
pub struct WalletTxBuilder {
    /// Network prefix for address parsing.
    network: NetworkPrefix,
    /// Current blockchain height.
    current_height: u32,
    /// Transaction fee.
    fee: u64,
    /// Input boxes.
    inputs: Vec<ErgoBox>,
    /// Output requests.
    outputs: Vec<PaymentRequest>,
    /// Change address.
    change_address: Option<String>,
}

impl WalletTxBuilder {
    /// Create a new transaction builder.
    pub fn new(network: NetworkPrefix, current_height: u32) -> Self {
        Self {
            network,
            current_height,
            fee: DEFAULT_FEE,
            inputs: Vec::new(),
            outputs: Vec::new(),
            change_address: None,
        }
    }

    /// Set the transaction fee.
    pub fn fee(mut self, fee: u64) -> Self {
        self.fee = fee;
        self
    }

    /// Add input boxes.
    pub fn inputs(mut self, boxes: Vec<ErgoBox>) -> Self {
        self.inputs = boxes;
        self
    }

    /// Add a payment output.
    pub fn add_output(mut self, request: PaymentRequest) -> Self {
        self.outputs.push(request);
        self
    }

    /// Add multiple payment outputs.
    pub fn outputs(mut self, requests: Vec<PaymentRequest>) -> Self {
        self.outputs.extend(requests);
        self
    }

    /// Set the change address.
    pub fn change_address(mut self, address: impl Into<String>) -> Self {
        self.change_address = Some(address.into());
        self
    }

    /// Calculate total output amount (excluding fee).
    fn total_output_amount(&self) -> u64 {
        self.outputs.iter().map(|o| o.amount).sum()
    }

    /// Parse an address string.
    fn parse_address(&self, address: &str) -> Result<Address, TxBuildError> {
        let encoder = AddressEncoder::new(self.network);
        encoder
            .parse_address_from_str(address)
            .map_err(|e| TxBuildError::InvalidAddress(format!("{}: {}", address, e)))
    }

    /// Build output candidates from payment requests.
    fn build_output_candidates(&self) -> Result<Vec<ErgoBoxCandidate>, TxBuildError> {
        let mut candidates = Vec::with_capacity(self.outputs.len());

        for output in &self.outputs {
            let address = self.parse_address(&output.recipient)?;
            let value = BoxValue::try_from(output.amount)
                .map_err(|e| TxBuildError::InvalidAmount(e.to_string()))?;

            let mut builder =
                ErgoBoxCandidateBuilder::new(value, address.script()?, self.current_height);

            for token in &output.tokens {
                builder.add_token(token.clone());
            }

            candidates.push(builder.build()?);
        }

        Ok(candidates)
    }

    /// Build an unsigned transaction.
    pub fn build(self) -> Result<UnsignedTransaction, TxBuildError> {
        if self.inputs.is_empty() {
            return Err(TxBuildError::NoInputs);
        }

        let change_address_str = self
            .change_address
            .clone()
            .ok_or(TxBuildError::NoChangeAddress)?;
        let change_address = self.parse_address(&change_address_str)?;

        // Calculate total needed
        let total_output = self.total_output_amount();
        let total_needed = total_output + self.fee;

        // Collect all tokens needed (simplified - doesn't merge duplicate token IDs)
        let needed_tokens: Vec<Token> =
            self.outputs.iter().flat_map(|o| o.tokens.clone()).collect();

        // Select boxes
        let selector = SimpleBoxSelector::new();
        let target_value = BoxValue::try_from(total_needed)
            .map_err(|e| TxBuildError::InvalidAmount(e.to_string()))?;

        let box_selection: BoxSelection<ErgoBox> =
            selector.select(self.inputs.clone(), target_value, &needed_tokens)?;

        // Build output candidates
        let output_candidates = self.build_output_candidates()?;

        // Build transaction
        let fee_value =
            BoxValue::try_from(self.fee).map_err(|e| TxBuildError::InvalidAmount(e.to_string()))?;

        let tx_builder = TxBuilder::new(
            box_selection,
            output_candidates,
            self.current_height,
            fee_value,
            change_address,
        );

        let unsigned_tx = tx_builder.build()?;
        Ok(unsigned_tx)
    }
}

/// Sign an unsigned transaction using wallet secrets.
pub fn sign_tx(
    unsigned_tx: UnsignedTransaction,
    input_boxes: Vec<ErgoBox>,
    data_boxes: Vec<ErgoBox>,
    state_context: &ErgoStateContext,
    wallet: &ErgoLibWallet,
) -> Result<Transaction, TxBuildError> {
    let tx_context = TransactionContext::new(unsigned_tx, input_boxes, data_boxes)?;

    let signed_tx = wallet.sign_transaction(tx_context, state_context, None)?;
    Ok(signed_tx)
}

/// Create a simple send transaction.
///
/// This is a convenience function for the most common use case.
pub fn create_send_tx(
    from_boxes: Vec<ErgoBox>,
    to_address: &str,
    amount: u64,
    change_address: &str,
    current_height: u32,
    network: NetworkPrefix,
) -> Result<UnsignedTransaction, TxBuildError> {
    WalletTxBuilder::new(network, current_height)
        .inputs(from_boxes)
        .add_output(PaymentRequest::new(to_address, amount))
        .change_address(change_address)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_payment_request() {
        let request = PaymentRequest::new("9addr", 1_000_000_000);
        assert_eq!(request.amount, 1_000_000_000);
        assert!(request.tokens.is_empty());
    }

    #[test]
    fn test_tx_builder_no_inputs() {
        let builder = WalletTxBuilder::new(NetworkPrefix::Mainnet, 100)
            .add_output(PaymentRequest::new("9addr", 1_000_000_000))
            .change_address("9change");

        let result = builder.build();
        assert!(matches!(result, Err(TxBuildError::NoInputs)));
    }

    #[test]
    fn test_tx_builder_no_change_address() {
        let builder = WalletTxBuilder::new(NetworkPrefix::Mainnet, 100)
            .add_output(PaymentRequest::new("9addr", 1_000_000_000));

        // Would fail on build since we can't create inputs easily in tests
        // Just verify the builder pattern works
        assert!(builder.change_address.is_none());
    }

    #[test]
    fn test_total_output_amount() {
        let builder = WalletTxBuilder::new(NetworkPrefix::Mainnet, 100)
            .add_output(PaymentRequest::new("9addr1", 1_000_000_000))
            .add_output(PaymentRequest::new("9addr2", 500_000_000));

        assert_eq!(builder.total_output_amount(), 1_500_000_000);
    }
}
