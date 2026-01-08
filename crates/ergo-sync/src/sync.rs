//! Synchronization state machine.

use crate::{SyncError, SyncResult, HEADERS_BATCH_SIZE};
use ergo_network::PeerId;
use parking_lot::RwLock;
use std::collections::{HashMap, VecDeque};
use tracing::{debug, info, warn};

/// Synchronization state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncState {
    /// Not syncing.
    Idle,
    /// Syncing headers.
    SyncingHeaders {
        /// Current height.
        from: u32,
        /// Target height.
        to: u32,
    },
    /// Downloading blocks.
    SyncingBlocks {
        /// Pending block IDs.
        pending_count: usize,
    },
    /// Fully synchronized.
    Synchronized,
}

/// Sync configuration.
#[derive(Debug, Clone)]
pub struct SyncConfig {
    /// Headers batch size.
    pub headers_batch: usize,
    /// Parallel block downloads.
    pub parallel_downloads: usize,
    /// Sync timeout in seconds.
    pub sync_timeout_secs: u64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            headers_batch: HEADERS_BATCH_SIZE,
            parallel_downloads: 16,
            sync_timeout_secs: 60,
        }
    }
}

/// Block synchronization coordinator.
pub struct Synchronizer {
    /// Configuration.
    config: SyncConfig,
    /// Current state.
    state: RwLock<SyncState>,
    /// Peer heights.
    peer_heights: RwLock<HashMap<PeerId, u32>>,
    /// Headers to download.
    header_queue: RwLock<VecDeque<Vec<u8>>>,
    /// Blocks to download.
    block_queue: RwLock<VecDeque<Vec<u8>>>,
    /// Pending requests (id -> peer).
    pending: RwLock<HashMap<Vec<u8>, PeerId>>,
    /// Our current height.
    our_height: RwLock<u32>,
}

impl Synchronizer {
    /// Create a new synchronizer.
    pub fn new(config: SyncConfig) -> Self {
        Self {
            config,
            state: RwLock::new(SyncState::Idle),
            peer_heights: RwLock::new(HashMap::new()),
            header_queue: RwLock::new(VecDeque::new()),
            block_queue: RwLock::new(VecDeque::new()),
            pending: RwLock::new(HashMap::new()),
            our_height: RwLock::new(0),
        }
    }

    /// Get current sync state.
    pub fn state(&self) -> SyncState {
        self.state.read().clone()
    }

    /// Check if synchronized.
    pub fn is_synced(&self) -> bool {
        matches!(self.state(), SyncState::Synchronized)
    }

    /// Get our current height.
    pub fn our_height(&self) -> u32 {
        *self.our_height.read()
    }

    /// Update our height.
    pub fn set_height(&self, height: u32) {
        *self.our_height.write() = height;
        self.check_sync_status();
    }

    /// Update a peer's height.
    pub fn set_peer_height(&self, peer: PeerId, height: u32) {
        self.peer_heights.write().insert(peer, height);
        self.check_sync_status();
    }

    /// Remove a peer.
    pub fn remove_peer(&self, peer: &PeerId) {
        self.peer_heights.write().remove(peer);
    }

    /// Get the best peer height.
    pub fn best_peer_height(&self) -> Option<u32> {
        self.peer_heights.read().values().max().copied()
    }

    /// Check if we need to sync.
    fn check_sync_status(&self) {
        let our = *self.our_height.read();
        let best = self.best_peer_height().unwrap_or(0);

        let mut state = self.state.write();

        if our >= best {
            if !matches!(*state, SyncState::Synchronized) {
                info!(height = our, "Synchronized with network");
                *state = SyncState::Synchronized;
            }
        } else if matches!(*state, SyncState::Idle | SyncState::Synchronized) {
            info!(our_height = our, peer_height = best, "Starting sync");
            *state = SyncState::SyncingHeaders {
                from: our,
                to: best,
            };
        }
    }

    /// Start header sync from a specific height.
    pub fn start_header_sync(&self, from: u32, to: u32) {
        info!(from, to, "Starting header sync");
        *self.state.write() = SyncState::SyncingHeaders { from, to };
    }

    /// Queue headers for download.
    pub fn queue_headers(&self, header_ids: Vec<Vec<u8>>) {
        let mut queue = self.header_queue.write();
        for id in header_ids {
            queue.push_back(id);
        }
        debug!(queued = queue.len(), "Headers queued");
    }

    /// Queue blocks for download.
    pub fn queue_blocks(&self, block_ids: Vec<Vec<u8>>) {
        let mut queue = self.block_queue.write();
        for id in block_ids {
            queue.push_back(id);
        }

        let count = queue.len();
        *self.state.write() = SyncState::SyncingBlocks {
            pending_count: count,
        };
        debug!(queued = count, "Blocks queued");
    }

    /// Get next items to download.
    pub fn get_download_batch(&self, count: usize) -> Vec<Vec<u8>> {
        let mut queue = self.block_queue.write();
        let mut batch = Vec::with_capacity(count);

        for _ in 0..count {
            if let Some(id) = queue.pop_front() {
                batch.push(id);
            } else {
                break;
            }
        }

        batch
    }

    /// Mark a block as downloaded.
    pub fn block_received(&self, block_id: &[u8]) {
        self.pending.write().remove(block_id);

        let pending = self.pending.read().len();
        let queue = self.block_queue.read().len();

        if pending == 0 && queue == 0 {
            self.check_sync_status();
        } else {
            *self.state.write() = SyncState::SyncingBlocks {
                pending_count: pending + queue,
            };
        }
    }

    /// Handle sync failure.
    pub fn handle_failure(&self, peer: &PeerId, error: &str) {
        warn!(peer = %peer, error, "Sync failure");

        // Remove pending requests for this peer
        let mut pending = self.pending.write();
        let failed: Vec<_> = pending
            .iter()
            .filter(|(_, p)| *p == peer)
            .map(|(id, _)| id.clone())
            .collect();

        for id in &failed {
            pending.remove(id);
            // Re-queue for download
            self.block_queue.write().push_back(id.clone());
        }
    }

    /// Get sync progress (0.0 - 1.0).
    pub fn progress(&self) -> f64 {
        let our = *self.our_height.read() as f64;
        let best = self.best_peer_height().unwrap_or(0) as f64;

        if best == 0.0 {
            1.0
        } else {
            (our / best).min(1.0)
        }
    }
}

impl Default for Synchronizer {
    fn default() -> Self {
        Self::new(SyncConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_state_transitions() {
        let sync = Synchronizer::default();

        assert!(matches!(sync.state(), SyncState::Idle));

        // Add a peer with higher height
        let peer = PeerId::from_bytes(vec![1, 2, 3]);
        sync.set_peer_height(peer.clone(), 1000);
        sync.set_height(0);

        assert!(matches!(sync.state(), SyncState::SyncingHeaders { .. }));

        // Catch up
        sync.set_height(1000);
        assert!(matches!(sync.state(), SyncState::Synchronized));
    }

    #[test]
    fn test_progress() {
        let sync = Synchronizer::default();

        let peer = PeerId::from_bytes(vec![1, 2, 3]);
        sync.set_peer_height(peer, 1000);
        sync.set_height(500);

        assert!((sync.progress() - 0.5).abs() < 0.01);
    }
}
