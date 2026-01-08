//! State manager coordinating UTXO state and history.

use crate::{History, StateChange, StateError, StateResult, UtxoState};
use ergo_chain_types::Header;
use ergo_consensus::FullBlock;
use ergo_storage::Storage;
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
}

impl StateManager {
    /// Create a new state manager with the given storage.
    pub fn new(storage: Arc<dyn Storage>) -> Self {
        Self {
            utxo: UtxoState::new(Arc::clone(&storage)),
            history: History::new(storage),
            state_root_verification: StateRootVerification::default(),
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
        }
    }

    /// Initialize from existing storage.
    pub fn init_from_storage(storage: Arc<dyn Storage>) -> StateResult<Self> {
        let utxo = UtxoState::init_from_storage(Arc::clone(&storage))?;
        let history = History::init_from_storage(storage)?;

        info!(
            utxo_height = utxo.height(),
            header_height = history.best_height(),
            "State manager initialized"
        );

        Ok(Self {
            utxo,
            history,
            state_root_verification: StateRootVerification::default(),
        })
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
    pub fn is_synced(&self) -> bool {
        self.utxo.height() == self.history.best_full_block_height()
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
