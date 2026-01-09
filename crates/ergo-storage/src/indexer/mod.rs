//! Extra indexer for advanced blockchain queries.
//!
//! This module provides additional indexes beyond basic blockchain storage,
//! enabling efficient queries by:
//! - Address (ErgoTree hash)
//! - Token ID
//! - Transaction global index
//! - Box global index
//!
//! The indexer tracks:
//! - `IndexedErgoBox`: Box with spending info and global index
//! - `IndexedErgoTransaction`: Transaction with inputs/outputs as global indexes
//! - `IndexedErgoAddress`: Address with balance tracking and associated boxes/txs
//! - `IndexedToken`: Token with creation info and associated boxes
//!
//! # Usage
//!
//! The indexer runs as an optional background process, indexing blocks as they
//! are applied. It can be enabled/disabled via configuration.

mod balance;
mod indexed_address;
mod indexed_box;
mod indexed_token;
mod indexed_transaction;
mod state;

pub use balance::BalanceInfo;
pub use indexed_address::IndexedErgoAddress;
pub use indexed_box::IndexedErgoBox;
pub use indexed_token::IndexedToken;
pub use indexed_transaction::IndexedErgoTransaction;
pub use state::IndexerState;

use crate::{ColumnFamily, Storage, StorageError};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

/// Extra index type identifiers (matching Scala node).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExtraIndexType {
    /// IndexedErgoBox
    Box = 5,
    /// IndexedErgoTransaction
    Transaction = 10,
    /// IndexedErgoAddress
    Address = 15,
    /// IndexedToken
    Token = 35,
}

impl TryFrom<u8> for ExtraIndexType {
    type Error = StorageError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            5 => Ok(ExtraIndexType::Box),
            10 => Ok(ExtraIndexType::Transaction),
            15 => Ok(ExtraIndexType::Address),
            35 => Ok(ExtraIndexType::Token),
            _ => Err(StorageError::Deserialization(format!(
                "Unknown extra index type: {}",
                value
            ))),
        }
    }
}

/// Configuration for the extra indexer.
#[derive(Debug, Clone)]
pub struct IndexerConfig {
    /// Whether the indexer is enabled.
    pub enabled: bool,
    /// Number of indexes to buffer before flushing to storage.
    pub save_limit: usize,
    /// Segment threshold for numeric indexes.
    pub segment_threshold: usize,
}

impl Default for IndexerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            save_limit: 500,
            segment_threshold: 512,
        }
    }
}

/// Extra indexer for blockchain data.
///
/// Tracks additional indexes for addresses, tokens, transactions, and boxes
/// to enable efficient queries for explorers and dApps.
pub struct ExtraIndexer {
    /// Configuration.
    config: IndexerConfig,
    /// Storage handle.
    storage: Arc<dyn Storage>,
    /// Current indexer state.
    state: RwLock<IndexerState>,
    /// Buffered boxes (box_id -> IndexedErgoBox).
    boxes: RwLock<HashMap<[u8; 32], IndexedErgoBox>>,
    /// Buffered addresses (tree_hash -> IndexedErgoAddress).
    addresses: RwLock<HashMap<[u8; 32], IndexedErgoAddress>>,
    /// Buffered tokens (token_id -> IndexedToken).
    tokens: RwLock<HashMap<[u8; 32], IndexedToken>>,
    /// Buffered transactions.
    transactions: RwLock<Vec<IndexedErgoTransaction>>,
}

impl ExtraIndexer {
    /// Create a new extra indexer.
    pub fn new(storage: Arc<dyn Storage>, config: IndexerConfig) -> Self {
        Self {
            config,
            storage,
            state: RwLock::new(IndexerState::default()),
            boxes: RwLock::new(HashMap::new()),
            addresses: RwLock::new(HashMap::new()),
            tokens: RwLock::new(HashMap::new()),
            transactions: RwLock::new(Vec::new()),
        }
    }

    /// Check if the indexer is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Get the current indexer state.
    pub fn state(&self) -> IndexerState {
        self.state.read().clone()
    }

    /// Get the current indexed height.
    pub fn indexed_height(&self) -> u32 {
        self.state.read().indexed_height
    }

    /// Get the global box index counter.
    pub fn global_box_index(&self) -> u64 {
        self.state.read().global_box_index
    }

    /// Get the global transaction index counter.
    pub fn global_tx_index(&self) -> u64 {
        self.state.read().global_tx_index
    }

    /// Check if buffer should be flushed.
    fn should_flush(&self) -> bool {
        let boxes = self.boxes.read().len();
        let txs = self.transactions.read().len();
        boxes + txs >= self.config.save_limit
    }

    /// Clear all buffers.
    fn clear_buffers(&self) {
        self.boxes.write().clear();
        self.addresses.write().clear();
        self.tokens.write().clear();
        self.transactions.write().clear();
    }

    /// Lookup a box by its ID.
    pub fn get_box(&self, box_id: &[u8; 32]) -> Option<IndexedErgoBox> {
        // First check buffer
        if let Some(ieb) = self.boxes.read().get(box_id) {
            return Some(ieb.clone());
        }

        // Then check storage
        self.storage
            .get(ColumnFamily::ExtraIndex, box_id)
            .ok()
            .flatten()
            .and_then(|data: Vec<u8>| IndexedErgoBox::deserialize(&data).ok())
    }

    /// Lookup a box by its global index.
    pub fn get_box_by_index(&self, global_index: u64) -> Option<IndexedErgoBox> {
        let key = global_index.to_be_bytes();
        self.storage
            .get(ColumnFamily::BoxIndex, &key)
            .ok()
            .flatten()
            .and_then(|box_id: Vec<u8>| {
                if box_id.len() == 32 {
                    let mut id = [0u8; 32];
                    id.copy_from_slice(&box_id);
                    self.get_box(&id)
                } else {
                    None
                }
            })
    }

    /// Lookup a transaction by its ID.
    pub fn get_transaction(&self, tx_id: &[u8; 32]) -> Option<IndexedErgoTransaction> {
        // First check buffer
        for tx in self.transactions.read().iter() {
            if tx.tx_id == *tx_id {
                return Some(tx.clone());
            }
        }

        // Then check storage
        self.storage
            .get(ColumnFamily::ExtraIndex, tx_id)
            .ok()
            .flatten()
            .and_then(|data: Vec<u8>| IndexedErgoTransaction::deserialize(&data).ok())
    }

    /// Lookup an address by its ErgoTree hash.
    pub fn get_address(&self, tree_hash: &[u8; 32]) -> Option<IndexedErgoAddress> {
        // First check buffer
        if let Some(addr) = self.addresses.read().get(tree_hash) {
            return Some(addr.clone());
        }

        // Then check storage
        self.storage
            .get(ColumnFamily::ExtraIndex, tree_hash)
            .ok()
            .flatten()
            .and_then(|data: Vec<u8>| IndexedErgoAddress::deserialize(&data).ok())
    }

    /// Lookup a token by its ID.
    pub fn get_token(&self, token_id: &[u8; 32]) -> Option<IndexedToken> {
        // Create unique ID for token (to avoid collision with box IDs)
        let unique_id = Self::token_unique_id(token_id);

        // First check buffer
        if let Some(token) = self.tokens.read().get(&unique_id) {
            return Some(token.clone());
        }

        // Then check storage
        self.storage
            .get(ColumnFamily::ExtraIndex, &unique_id)
            .ok()
            .flatten()
            .and_then(|data: Vec<u8>| IndexedToken::deserialize(&data).ok())
    }

    /// Compute unique ID for a token (to avoid collision with box IDs).
    /// Same algorithm as Scala: hash(tokenId + "token")
    fn token_unique_id(token_id: &[u8; 32]) -> [u8; 32] {
        use blake2::{Blake2b, Digest};
        use digest::consts::U32;

        let mut hasher = Blake2b::<U32>::new();
        hasher.update(token_id);
        hasher.update(b"token");
        let result = hasher.finalize();
        let mut id = [0u8; 32];
        id.copy_from_slice(&result);
        id
    }

    /// Compute ErgoTree hash for address indexing.
    pub fn hash_ergo_tree(ergo_tree_bytes: &[u8]) -> [u8; 32] {
        use blake2::{Blake2b, Digest};
        use digest::consts::U32;

        let mut hasher = Blake2b::<U32>::new();
        hasher.update(ergo_tree_bytes);
        let result = hasher.finalize();
        let mut id = [0u8; 32];
        id.copy_from_slice(&result);
        id
    }

    /// Index a new output box.
    ///
    /// This adds the box to the buffer and updates the associated address and tokens.
    pub fn index_output(
        &self,
        box_id: [u8; 32],
        height: u32,
        box_bytes: Vec<u8>,
        value: u64,
        ergo_tree_bytes: &[u8],
        tokens: &[([u8; 32], u64)],
    ) -> u64 {
        let mut state = self.state.write();
        let global_index = state.global_box_index;

        let ergo_tree_hash = Self::hash_ergo_tree(ergo_tree_bytes);

        // Create indexed box
        let ieb = IndexedErgoBox::new(
            box_id,
            height,
            global_index,
            box_bytes,
            value,
            ergo_tree_hash,
            tokens.to_vec(),
        );

        // Add to box buffer
        self.boxes.write().insert(box_id, ieb.clone());

        // Update address
        {
            let mut addresses = self.addresses.write();
            let addr = addresses
                .entry(ergo_tree_hash)
                .or_insert_with(|| IndexedErgoAddress::new(ergo_tree_hash));
            addr.add_box(global_index, value, tokens);
        }

        // Update tokens
        {
            let mut token_map = self.tokens.write();
            for (token_id, _amount) in tokens {
                let unique_id = Self::token_unique_id(token_id);
                let token = token_map
                    .entry(unique_id)
                    .or_insert_with(|| IndexedToken::new(*token_id));
                token.add_box(global_index);
            }
        }

        // Increment global box index
        state.global_box_index += 1;

        global_index
    }

    /// Mark a box as spent.
    ///
    /// This updates the box's spending info and the associated address balance.
    pub fn spend_box(
        &self,
        box_id: &[u8; 32],
        spending_tx_id: [u8; 32],
        spending_height: u32,
    ) -> Option<u64> {
        // Find the box (in buffer or storage)
        let ieb = self.get_box(box_id)?;
        let global_index = ieb.global_index;

        // Update box with spending info
        let mut updated_box = ieb.clone();
        updated_box.mark_spent(spending_tx_id, spending_height);
        self.boxes.write().insert(*box_id, updated_box);

        // Update address balance
        {
            let mut addresses = self.addresses.write();
            if let Some(addr) = addresses.get_mut(&ieb.ergo_tree_hash) {
                addr.spend_box(global_index, ieb.value, &ieb.tokens);
            } else {
                // Load from storage and update
                if let Some(mut addr) = self.get_address(&ieb.ergo_tree_hash) {
                    addr.spend_box(global_index, ieb.value, &ieb.tokens);
                    addresses.insert(ieb.ergo_tree_hash, addr);
                }
            }
        }

        // Update tokens
        {
            let mut token_map = self.tokens.write();
            for (token_id, _) in &ieb.tokens {
                let unique_id = Self::token_unique_id(token_id);
                if let Some(token) = token_map.get_mut(&unique_id) {
                    token.spend_box(global_index);
                } else {
                    // Load from storage and update
                    if let Some(mut token) = self.get_token(token_id) {
                        token.spend_box(global_index);
                        token_map.insert(unique_id, token);
                    }
                }
            }
        }

        Some(global_index)
    }

    /// Index a transaction.
    ///
    /// Returns the global transaction index.
    pub fn index_transaction(
        &self,
        tx_id: [u8; 32],
        tx_index: u16,
        height: u32,
        size: u32,
        input_indexes: Vec<u64>,
        output_indexes: Vec<u64>,
        data_inputs: Vec<[u8; 32]>,
    ) -> u64 {
        let mut state = self.state.write();
        let global_index = state.global_tx_index;

        let itx = IndexedErgoTransaction::new(
            tx_id,
            tx_index,
            height,
            size,
            global_index,
            input_indexes,
            output_indexes,
            data_inputs,
        );

        self.transactions.write().push(itx);

        // Update addresses with transaction
        // Note: This would need the ErgoTree hashes from inputs/outputs
        // which should be passed in or looked up

        state.global_tx_index += 1;

        global_index
    }

    /// Register a new token creation.
    pub fn register_token(
        &self,
        token_id: [u8; 32],
        creation_box_id: [u8; 32],
        amount: u64,
        name: String,
        description: String,
        decimals: u8,
    ) {
        let unique_id = Self::token_unique_id(&token_id);
        let mut tokens = self.tokens.write();

        if let Some(existing) = tokens.get_mut(&unique_id) {
            // Token already exists (created in multiple boxes), add emission
            existing.add_emission(amount);
        } else {
            // New token
            let token = IndexedToken::with_creation_info(
                token_id,
                creation_box_id,
                amount,
                name,
                description,
                decimals,
            );
            tokens.insert(unique_id, token);
        }
    }

    /// Flush buffered data to storage.
    pub fn flush(&self) -> Result<(), StorageError> {
        use crate::WriteBatch;

        let mut batch = WriteBatch::new();
        let state = self.state.read().clone();

        // Save state
        let state_key = b"indexer_state".to_vec();
        batch.put(ColumnFamily::Metadata, state_key, state.serialize());

        // Save boxes
        for (box_id, ieb) in self.boxes.read().iter() {
            batch.put(ColumnFamily::ExtraIndex, box_id.to_vec(), ieb.serialize());
            // Also save numeric index
            let idx_key = ieb.global_index.to_be_bytes().to_vec();
            batch.put(ColumnFamily::BoxIndex, idx_key, box_id.to_vec());
        }

        // Save transactions
        for itx in self.transactions.read().iter() {
            batch.put(
                ColumnFamily::ExtraIndex,
                itx.tx_id.to_vec(),
                itx.serialize(),
            );
            // Also save numeric index
            let idx_key = itx.global_index.to_be_bytes().to_vec();
            batch.put(ColumnFamily::TxNumericIndex, idx_key, itx.tx_id.to_vec());
        }

        // Save addresses
        for (tree_hash, addr) in self.addresses.read().iter() {
            batch.put(
                ColumnFamily::ExtraIndex,
                tree_hash.to_vec(),
                addr.serialize(),
            );
        }

        // Save tokens
        for (unique_id, token) in self.tokens.read().iter() {
            batch.put(
                ColumnFamily::ExtraIndex,
                unique_id.to_vec(),
                token.serialize(),
            );
        }

        // Write batch
        self.storage.write_batch(batch)?;

        // Clear buffers
        self.clear_buffers();

        Ok(())
    }

    /// Update the indexed height after processing a block.
    pub fn set_indexed_height(&self, height: u32) {
        self.state.write().indexed_height = height;
    }

    /// Load indexer state from storage.
    pub fn load_state(&self) -> Result<(), StorageError> {
        let state_key = b"indexer_state";
        if let Some(data) = self.storage.get(ColumnFamily::Metadata, state_key)? {
            if let Ok(state) = IndexerState::deserialize(&data) {
                *self.state.write() = state;
            }
        }
        Ok(())
    }

    /// Get the number of buffered items.
    pub fn buffer_count(&self) -> usize {
        self.boxes.read().len()
            + self.transactions.read().len()
            + self.addresses.read().len()
            + self.tokens.read().len()
    }

    /// Check if flush is needed and perform it.
    pub fn maybe_flush(&self) -> Result<bool, StorageError> {
        if self.should_flush() {
            self.flush()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_indexer_config_default() {
        let config = IndexerConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.save_limit, 500);
        assert_eq!(config.segment_threshold, 512);
    }

    #[test]
    fn test_extra_index_type() {
        assert_eq!(ExtraIndexType::try_from(5).unwrap(), ExtraIndexType::Box);
        assert_eq!(
            ExtraIndexType::try_from(10).unwrap(),
            ExtraIndexType::Transaction
        );
        assert_eq!(
            ExtraIndexType::try_from(15).unwrap(),
            ExtraIndexType::Address
        );
        assert_eq!(ExtraIndexType::try_from(35).unwrap(), ExtraIndexType::Token);
        assert!(ExtraIndexType::try_from(99).is_err());
    }

    #[test]
    fn test_token_unique_id() {
        let token_id = [1u8; 32];
        let unique_id = ExtraIndexer::token_unique_id(&token_id);
        // Unique ID should be different from original
        assert_ne!(unique_id, token_id);
        // Same input should produce same output
        assert_eq!(unique_id, ExtraIndexer::token_unique_id(&token_id));
    }
}
