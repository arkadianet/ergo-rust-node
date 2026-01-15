//! State manager coordinating UTXO state and history.

use crate::{History, StateChange, StateError, StateResult, UtxoState};
use ergo_chain_types::Header;
use ergo_consensus::{ChainParameters, FullBlock, VOTING_EPOCH_LENGTH};
use ergo_storage::Storage;
use parking_lot::RwLock;
use std::sync::Arc;
use tracing::{debug, info, instrument, warn};

/// State root verification mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateRootVerification {
    /// Skip state root verification (for initial sync or testing).
    Skip,
    /// Log warnings but don't fail on mismatch.
    WarnOnly,
    /// Enforce state root verification (production mode).
    Enforce,
}

impl Default for StateRootVerification {
    fn default() -> Self {
        // Default to WarnOnly during development; change to Enforce for production
        Self::WarnOnly
    }
}

/// Coordinates state transitions between UTXO state and history.
pub struct StateManager {
    /// UTXO state.
    pub utxo: UtxoState,
    /// Block history.
    pub history: History,
    /// State root verification mode.
    state_root_verification: StateRootVerification,
    /// Current chain parameters (updated at epoch boundaries from extensions).
    /// Wrapped in RwLock for thread-safe updates while allowing shared access.
    current_parameters: RwLock<ChainParameters>,
}

impl StateManager {
    /// Create a new state manager with the given storage.
    pub fn new(storage: Arc<dyn Storage>) -> Self {
        Self {
            utxo: UtxoState::new(Arc::clone(&storage)),
            history: History::new(storage),
            state_root_verification: StateRootVerification::default(),
            current_parameters: RwLock::new(ChainParameters::default()),
        }
    }

    /// Create a new state manager with custom verification mode.
    pub fn with_verification(
        storage: Arc<dyn Storage>,
        verification: StateRootVerification,
    ) -> Self {
        Self {
            utxo: UtxoState::new(Arc::clone(&storage)),
            history: History::new(storage),
            state_root_verification: verification,
            current_parameters: RwLock::new(ChainParameters::default()),
        }
    }

    /// Initialize from existing storage.
    pub fn init_from_storage(storage: Arc<dyn Storage>) -> StateResult<Self> {
        let utxo = UtxoState::init_from_storage(Arc::clone(&storage))?;
        let history = History::init_from_storage(storage)?;

        info!(
            utxo_height = utxo.height(),
            header_height = history.best_height(),
            full_block_height = history.best_full_block_height(),
            "State manager initialized"
        );

        let mut manager = Self {
            utxo,
            history,
            state_root_verification: StateRootVerification::default(),
            current_parameters: RwLock::new(ChainParameters::default()),
        };

        // Check UTXO state consistency and auto-recover if needed
        manager.check_and_recover_utxo_state()?;

        // Initialize chain parameters from the most recent epoch boundary
        manager.init_parameters_from_storage();

        Ok(manager)
    }

    /// Initialize chain parameters from storage by finding the most recent epoch boundary.
    ///
    /// This is called on startup to restore parameters to their correct values
    /// based on what was voted in previous epochs.
    fn init_parameters_from_storage(&self) {
        let current_height = self.utxo.height();
        if current_height == 0 {
            return; // Fresh state, use defaults
        }

        // Find the most recent epoch boundary at or before current height
        // Epoch boundaries are at heights divisible by 1024
        let epoch_length = VOTING_EPOCH_LENGTH;
        let most_recent_epoch = (current_height / epoch_length) * epoch_length;

        if most_recent_epoch == 0 {
            return; // Before first epoch, use defaults
        }

        info!(
            current_height,
            most_recent_epoch, "Initializing chain parameters from most recent epoch boundary"
        );

        // We need to scan backwards through epoch boundaries to build up the parameter state
        // since parameters are inherited from previous epochs if not changed
        let mut params = ChainParameters::default();
        let mut epoch = epoch_length; // Start from first epoch boundary

        while epoch <= most_recent_epoch {
            if let Ok(Some(header)) = self.history.headers.get_by_height(epoch) {
                if let Ok(Some(block)) = self.history.get_full_block(&header.id) {
                    match block.extension.parse_parameters() {
                        Ok(parsed) if !parsed.is_empty() => {
                            params = ChainParameters::from_extension(epoch, &parsed, &params);
                            debug!(
                                epoch,
                                max_block_cost = params.max_block_cost,
                                "Loaded parameters from epoch boundary"
                            );
                        }
                        Ok(_) => {
                            // No parameters in this extension, keep previous values
                        }
                        Err(e) => {
                            warn!(epoch, error = %e, "Failed to parse parameters from epoch");
                        }
                    }
                }
            }
            epoch += epoch_length;
        }

        // Update current parameters
        *self.current_parameters.write() = params.clone();

        info!(
            max_block_cost = params.max_block_cost,
            storage_fee_factor = params.storage_fee_factor,
            input_cost = params.input_cost,
            output_cost = params.output_cost,
            "Chain parameters initialized from storage"
        );
    }

    /// Check UTXO state consistency and rollback if corrupted.
    /// This handles the case where the node crashed mid-write, leaving the
    /// UTXO height updated but boxes not written.
    fn check_and_recover_utxo_state(&mut self) -> StateResult<()> {
        let utxo_height = self.utxo.height();
        if utxo_height == 0 {
            return Ok(()); // Fresh state, nothing to check
        }

        // Try to find a consistent UTXO state by checking progressively older blocks
        // We'll check the current height, then go back in larger steps if needed
        let check_heights = [
            utxo_height,
            utxo_height.saturating_sub(100),
            utxo_height.saturating_sub(500),
            utxo_height.saturating_sub(1000),
            utxo_height.saturating_sub(5000),
            utxo_height.saturating_sub(10000),
        ];

        let mut last_good_height: Option<u32> = None;

        for &check_height in &check_heights {
            if check_height == 0 {
                continue;
            }

            // Get the header at this height
            let header = match self.history.headers.get_by_height(check_height)? {
                Some(h) => h,
                None => continue,
            };

            // Try to get the block transactions
            let block_txs = match self.history.blocks.get_transactions(&header.id)? {
                Some(txs) => txs,
                None => {
                    // No block transactions - can't verify this height
                    continue;
                }
            };

            // Check if coinbase outputs exist in UTXO
            if let Some(coinbase) = block_txs.txs.first() {
                let mut found_any = false;
                for output in &coinbase.outputs {
                    let box_id = output.box_id();
                    if self.utxo.get_box(&box_id)?.is_some() {
                        found_any = true;
                        break;
                    }
                }

                if found_any {
                    // Found a consistent height
                    last_good_height = Some(check_height);
                    break;
                }
            }
        }

        // Determine if we need to rollback
        let current_height = self.utxo.height();

        match last_good_height {
            Some(good_height) if good_height < current_height => {
                // We found corruption - rollback to slightly before the last good height
                // (in case there are issues at the boundary)
                let target_height = good_height.saturating_sub(10);
                warn!(
                    current_height,
                    last_good_height = good_height,
                    target_height,
                    "UTXO state corruption detected, rolling back"
                );

                self.utxo.rollback(target_height)?;

                info!(
                    new_height = self.utxo.height(),
                    "UTXO state recovered via rollback"
                );
            }
            None => {
                // Couldn't find any consistent state - severe corruption
                // Rollback to very early height or genesis
                let target_height = 1; // Near genesis
                warn!(
                    current_height,
                    "Severe UTXO state corruption - no consistent state found. Rolling back to near genesis."
                );

                self.utxo.rollback(target_height)?;

                info!(
                    new_height = self.utxo.height(),
                    "UTXO state recovered via rollback to near genesis"
                );
            }
            Some(good_height) if good_height == current_height => {
                // Current state is consistent
                debug!(current_height, "UTXO state consistency check passed");
            }
            _ => {}
        }

        Ok(())
    }

    /// Set the state root verification mode.
    pub fn set_verification_mode(&mut self, mode: StateRootVerification) {
        self.state_root_verification = mode;
    }

    /// Get current state heights.
    pub fn heights(&self) -> (u32, u32) {
        (self.utxo.height(), self.history.best_height())
    }

    /// Check if state is synchronized.
    /// We consider ourselves synced if:
    /// 1. We have headers (header height > 0)
    /// 2. Our full block height matches our header height (all blocks downloaded)
    /// 3. We have at least some blocks (full block height > 0)
    /// During initial header sync (no blocks yet), we report as NOT synced.
    pub fn is_synced(&self) -> bool {
        let header_height = self.history.best_height();
        let full_block_height = self.history.best_full_block_height();

        // Must have blocks and headers must match full blocks
        full_block_height > 0 && full_block_height == header_height
    }

    /// Get the gap between headers and full blocks.
    pub fn chain_gap(&self) -> u32 {
        self.history.chain_gap()
    }

    /// Apply a validated header to the chain.
    #[instrument(skip(self, header), fields(height = header.height))]
    pub fn apply_header(&self, header: Header) -> StateResult<crate::ChainSelection> {
        self.history.append_header(header)
    }

    /// Apply a validated header to the chain using raw bytes.
    /// This preserves correct ID computation for headers with BigInt serialization issues.
    #[instrument(skip(self, header, raw_bytes), fields(height = header.height))]
    pub fn apply_header_with_bytes(
        &self,
        header: Header,
        raw_bytes: &[u8],
    ) -> StateResult<crate::ChainSelection> {
        self.history.append_header_with_bytes(header, raw_bytes)
    }

    /// Apply multiple validated headers in a single batched write operation.
    /// This is significantly more efficient than applying headers individually during sync.
    ///
    /// Headers must be in ascending height order and form a valid chain.
    #[instrument(skip(self, headers), fields(count = headers.len()))]
    pub fn apply_headers_batched(
        &self,
        headers: Vec<(Header, Vec<u8>)>,
    ) -> StateResult<crate::ChainSelection> {
        self.history.append_headers_batched(headers)
    }

    /// Apply a validated full block to the state.
    #[instrument(skip(self, block, state_change), fields(height = block.height()))]
    pub fn apply_block(
        &self,
        block: FullBlock,
        state_change: StateChange,
    ) -> StateResult<crate::ChainSelection> {
        let height = block.height();
        let expected_state_root: Vec<u8> = block.header.state_root.0.as_ref().to_vec();

        // Verify this block extends current state
        let utxo_height = self.utxo.height();
        if height != utxo_height + 1 {
            return Err(StateError::InvalidTransition(format!(
                "Expected height {}, got {}",
                utxo_height + 1,
                height
            )));
        }

        // Store the full block (header, transactions, extension, proofs)
        let selection = self.history.append_full_block(block)?;

        // Apply state change to UTXO set
        self.utxo.apply_change(&state_change, height)?;

        // Verify state root after application (based on verification mode)
        if self.state_root_verification != StateRootVerification::Skip {
            let computed_state_root = self.utxo.state_root();

            if computed_state_root.is_empty() || computed_state_root.iter().all(|&b| b == 0) {
                // State root not yet computed - this is expected until AVL tree is fully initialized
                debug!(
                    height,
                    expected_state_root = %hex::encode(&expected_state_root),
                    "State root verification skipped (AVL tree not initialized)"
                );
            } else {
                // Compare computed state root with expected
                // ADDigest is 33 bytes (32-byte hash + 1-byte height flag)
                // Our computed root is 32 bytes, so we compare the hash portion
                let expected_hash: &[u8] = if expected_state_root.len() == 33 {
                    &expected_state_root[..32]
                } else {
                    &expected_state_root
                };

                if computed_state_root.as_slice() != expected_hash {
                    let error_msg = format!(
                        "State root mismatch at height {}: expected {}, computed {}",
                        height,
                        hex::encode(expected_hash),
                        hex::encode(&computed_state_root)
                    );

                    match self.state_root_verification {
                        StateRootVerification::Enforce => {
                            // Rollback the state change since verification failed
                            if let Err(e) = self.utxo.rollback(utxo_height) {
                                warn!(height, error = %e, "Failed to rollback after state root mismatch");
                            }
                            return Err(StateError::StateRootMismatch {
                                height,
                                expected: hex::encode(expected_hash),
                                computed: hex::encode(&computed_state_root),
                            });
                        }
                        StateRootVerification::WarnOnly => {
                            warn!(height, expected = %hex::encode(expected_hash), computed = %hex::encode(&computed_state_root), "State root mismatch (warning only)");
                        }
                        StateRootVerification::Skip => {
                            // Already checked above, but included for completeness
                        }
                    }
                } else {
                    debug!(
                        height,
                        state_root = %hex::encode(&computed_state_root),
                        "State root verified successfully"
                    );
                }
            }
        }

        info!(height, "Block applied to state");
        Ok(selection)
    }

    /// Apply state change without storing block (for validation).
    pub fn apply_state_change(&self, state_change: &StateChange, height: u32) -> StateResult<()> {
        self.utxo.apply_change(state_change, height)
    }

    /// Apply multiple blocks in a single batched write operation.
    /// This is significantly more efficient than applying blocks individually during sync.
    ///
    /// Each element is a tuple of (FullBlock, StateChange).
    /// Blocks must be in ascending height order and contiguous.
    #[instrument(skip(self, blocks), fields(count = blocks.len()))]
    pub fn apply_blocks_batched(
        &self,
        blocks: Vec<(FullBlock, StateChange)>,
    ) -> StateResult<crate::ChainSelection> {
        if blocks.is_empty() {
            return Ok(crate::ChainSelection::Ignored);
        }

        let first_height = blocks[0].0.height();
        let last_height = blocks[blocks.len() - 1].0.height();
        let count = blocks.len();

        // Verify this batch extends current state
        let utxo_height = self.utxo.height();
        if first_height != utxo_height + 1 {
            return Err(StateError::InvalidTransition(format!(
                "Expected first block at height {}, got {}",
                utxo_height + 1,
                first_height
            )));
        }

        // Verify blocks are contiguous
        for (i, (block, _)) in blocks.iter().enumerate() {
            let expected_height = first_height + i as u32;
            if block.height() != expected_height {
                return Err(StateError::InvalidTransition(format!(
                    "Non-contiguous block: expected height {}, got {}",
                    expected_height,
                    block.height()
                )));
            }
        }

        info!(
            first_height,
            last_height, count, "Applying {} blocks in batched write", count
        );

        // Create a single batch for ALL blocks
        let mut batch = ergo_storage::WriteBatch::new();

        // Get starting cumulative difficulty
        let mut cumulative_difficulty = if first_height <= 2 {
            num_bigint::BigUint::from(0u32)
        } else {
            let parent_id = &blocks[0].0.header.parent_id;
            self.history
                .get_cumulative_difficulty(parent_id)?
                .unwrap_or_else(|| num_bigint::BigUint::from(0u32))
        };

        // Track boxes created across all blocks in this batch.
        // This allows spending a box created in block N within block N+1 of the same batch,
        // before the batch is committed to the database.
        //
        // NOTE: A similar tracking mechanism exists in Node::run() (see crates/ergo-node/src/node.rs)
        // for the validation phase. Both are necessary: that one for validation lookups during
        // script verification, this one for UTXO state updates during batch application.
        let mut batch_created_boxes: std::collections::HashMap<Vec<u8>, crate::utxo::BoxEntry> =
            std::collections::HashMap::new();

        // Add all blocks to the batch
        let mut final_block_id = blocks[0].0.id();
        for (block, state_change) in &blocks {
            let height = block.height();
            final_block_id = block.id();

            // Add block data to history batch
            cumulative_difficulty =
                self.history
                    .add_block_to_batch(&mut batch, block, &cumulative_difficulty)?;

            // Add UTXO changes to batch with cross-block context
            self.utxo.add_change_to_batch_with_context(
                &mut batch,
                state_change,
                height,
                &batch_created_boxes,
            )?;

            // Add boxes created in this block to the cross-block context
            // (so they can be found if spent in subsequent blocks of this batch)
            for entry in &state_change.created {
                batch_created_boxes.insert(entry.box_id_bytes(), entry.clone());
            }

            // Remove spent boxes from the cross-block context
            // (they've been deleted in the batch and shouldn't be found again)
            for box_id in &state_change.spent {
                batch_created_boxes.remove(box_id.as_ref());
            }
        }

        // Add final metadata updates
        let current_best_difficulty = self.history.best_cumulative_difficulty();
        let selection = if cumulative_difficulty > current_best_difficulty {
            batch.put(
                ergo_storage::ColumnFamily::Metadata,
                b"best_header_id",
                final_block_id.0.as_ref().to_vec(),
            );
            batch.put(
                ergo_storage::ColumnFamily::Metadata,
                b"best_height",
                last_height.to_be_bytes().to_vec(),
            );
            batch.put(
                ergo_storage::ColumnFamily::Metadata,
                b"best_cumulative_difficulty",
                cumulative_difficulty.to_bytes_be(),
            );
            crate::ChainSelection::Extended
        } else {
            crate::ChainSelection::Ignored
        };

        // Update best full block metadata
        let current_full_height = self.history.best_full_block_height();
        if last_height > current_full_height {
            batch.put(
                ergo_storage::ColumnFamily::Metadata,
                b"best_full_block_id",
                final_block_id.0.as_ref().to_vec(),
            );
            batch.put(
                ergo_storage::ColumnFamily::Metadata,
                b"best_full_block_height",
                last_height.to_be_bytes().to_vec(),
            );
        }

        // Execute single batch write for ALL blocks
        self.history.execute_batch(batch)?;

        // Update in-memory state after successful write
        self.history
            .update_in_memory_state(final_block_id, last_height, cumulative_difficulty);
        self.utxo.update_height_in_memory(last_height);

        info!(
            first_height,
            last_height, count, "Successfully applied {} blocks in single batch", count
        );

        Ok(selection)
    }

    /// Rollback state to a previous height.
    #[instrument(skip(self))]
    pub fn rollback_to(&self, height: u32) -> StateResult<()> {
        // Rollback UTXO state
        self.utxo.rollback(height)?;

        info!(height, "State rolled back");
        Ok(())
    }

    /// Get headers for sync (header locator).
    pub fn get_header_locator(&self) -> StateResult<Vec<ergo_chain_types::BlockId>> {
        self.history.get_header_locator()
    }

    /// Get headers in a range.
    pub fn get_headers(&self, from_height: u32, count: u32) -> StateResult<Vec<Header>> {
        self.history.headers.get_range(from_height, count)
    }

    /// Get header IDs in a range without loading full headers.
    /// This is much more memory efficient for bulk operations.
    pub fn get_header_ids(
        &self,
        from_height: u32,
        count: u32,
    ) -> StateResult<Vec<ergo_chain_types::BlockId>> {
        self.history.headers.get_ids_range(from_height, count)
    }

    /// Check if we have a header.
    pub fn has_header(&self, id: &ergo_chain_types::BlockId) -> StateResult<bool> {
        self.history.headers.contains(id)
    }

    /// Get a header by ID.
    pub fn get_header(&self, id: &ergo_chain_types::BlockId) -> StateResult<Option<Header>> {
        self.history.headers.get(id)
    }

    /// Get a full block by ID.
    pub fn get_full_block(&self, id: &ergo_chain_types::BlockId) -> StateResult<Option<FullBlock>> {
        self.history.get_full_block(id)
    }

    /// Update chain parameters from a block's extension if at an epoch boundary.
    ///
    /// Parameters are stored in block extensions and updated via miner voting
    /// at epoch boundaries (every 1024 blocks). This method should be called
    /// after each block is applied to check for parameter updates.
    ///
    /// This method is safe to call from multiple threads due to interior mutability.
    pub fn update_parameters_from_block(&self, block: &FullBlock) {
        let height = block.height();

        // Only check for parameter updates at epoch boundaries
        if !ChainParameters::is_epoch_boundary(height) {
            return;
        }

        // Parse parameters from extension
        match block.extension.parse_parameters() {
            Ok(parsed) if !parsed.is_empty() => {
                let mut params = self.current_parameters.write();
                let old_max_block_cost = params.max_block_cost;
                *params = ChainParameters::from_extension(height, &parsed, &params);

                info!(
                    height,
                    max_block_cost = params.max_block_cost,
                    storage_fee_factor = params.storage_fee_factor,
                    input_cost = params.input_cost,
                    output_cost = params.output_cost,
                    "Updated chain parameters at epoch boundary"
                );

                // Log if max_block_cost changed (important for transaction validation)
                if params.max_block_cost != old_max_block_cost {
                    info!(
                        height,
                        old_max_block_cost,
                        new_max_block_cost = params.max_block_cost,
                        "max_block_cost parameter changed via voting"
                    );
                }
            }
            Ok(_) => {
                // Empty extension or no system parameters - keep existing values
                debug!(
                    height,
                    "Epoch boundary reached but no parameters in extension"
                );
            }
            Err(e) => {
                warn!(
                    height,
                    error = %e,
                    "Failed to parse extension parameters at epoch boundary"
                );
            }
        }
    }

    /// Get the current chain parameters.
    /// Returns a clone of the parameters to allow safe concurrent access.
    pub fn get_parameters(&self) -> ChainParameters {
        self.current_parameters.read().clone()
    }

    /// Update chain parameters if any blocks in the given height range crossed an epoch boundary.
    ///
    /// This should be called after applying a batch of blocks. It checks for epoch boundaries
    /// and fetches extensions from storage to update parameters as needed.
    pub fn update_parameters_for_height_range(&self, first_height: u32, last_height: u32) {
        // Find all epoch boundaries in this range
        for height in first_height..=last_height {
            if ChainParameters::is_epoch_boundary(height) {
                // Try to get the block at this height and update parameters from its extension
                if let Ok(Some(header)) = self.history.headers.get_by_height(height) {
                    if let Ok(Some(block)) = self.history.get_full_block(&header.id) {
                        self.update_parameters_from_block(&block);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ergo_storage::Database;
    use tempfile::TempDir;

    #[test]
    fn test_state_manager_init() {
        let tmp = TempDir::new().unwrap();
        let db = Database::open(tmp.path()).unwrap();
        let manager = StateManager::new(Arc::new(db));

        assert_eq!(manager.heights(), (0, 0));
    }
}
