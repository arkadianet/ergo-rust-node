//! Synchronization state machine.

use crate::HEADERS_BATCH_SIZE;
use ergo_network::PeerId;
use parking_lot::RwLock;
use rand::seq::SliceRandom;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Minimum number of peers required to have a snapshot before downloading.
pub const MIN_SNAPSHOT_PEERS: usize = 2;

/// Number of chunks to download in parallel (minimum).
pub const CHUNKS_IN_PARALLEL_MIN: usize = 16;

/// Number of chunks to request from each peer at a time.
pub const CHUNKS_PER_PEER: usize = 4;

/// Chunk delivery timeout multiplier (relative to normal delivery timeout).
pub const CHUNK_TIMEOUT_MULTIPLIER: u32 = 4;

/// Manifest request timeout in seconds.
pub const MANIFEST_TIMEOUT_SECS: u64 = 60;

/// Synchronization state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncState {
    /// Not syncing.
    Idle,
    /// Awaiting snapshots info from peers (UTXO snapshot bootstrap).
    AwaitingSnapshotsInfo {
        /// Peers we've requested from.
        requested_from: usize,
        /// Timestamp when we started requesting.
        started_at_ms: u64,
    },
    /// Awaiting manifest from a peer.
    AwaitingManifest {
        /// Manifest ID (32-byte digest).
        manifest_id: [u8; 32],
        /// Snapshot height.
        height: u32,
        /// Peer we requested from.
        peer: PeerId,
    },
    /// Downloading UTXO snapshot chunks.
    DownloadingChunks {
        /// Manifest ID (32-byte digest).
        manifest_id: [u8; 32],
        /// Snapshot height.
        height: u32,
        /// Total chunks to download.
        total_chunks: usize,
        /// Chunks downloaded so far.
        downloaded_chunks: usize,
    },
    /// Applying downloaded UTXO snapshot.
    ApplyingSnapshot {
        /// Snapshot height.
        height: u32,
    },
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
    /// Enable UTXO snapshot bootstrap.
    pub utxo_bootstrap: bool,
    /// Minimum number of peers for snapshot sync.
    pub min_snapshot_peers: usize,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            headers_batch: HEADERS_BATCH_SIZE,
            parallel_downloads: 16,
            sync_timeout_secs: 60,
            utxo_bootstrap: false,
            min_snapshot_peers: MIN_SNAPSHOT_PEERS,
        }
    }
}

/// Available manifest tracking: manifest_id -> (height, peers that have it).
pub type AvailableManifests = HashMap<[u8; 32], (u32, Vec<PeerId>)>;

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
    /// Whether UTXO snapshot has been applied in this session.
    utxo_snapshot_applied: RwLock<bool>,
    /// Available manifests from peers: manifest_id -> (height, peers).
    available_manifests: RwLock<AvailableManifests>,
    /// Peers that support UTXO snapshots.
    snapshot_peers: RwLock<HashSet<PeerId>>,
    /// Chunks currently being downloaded: chunk_id -> (peer, request_time).
    downloading_chunks: RwLock<HashMap<[u8; 32], (PeerId, Instant)>>,
    /// Downloaded chunk IDs.
    downloaded_chunks: RwLock<HashSet<[u8; 32]>>,
    /// Time when manifest request was started (for timeout detection).
    manifest_request_time: RwLock<Option<Instant>>,
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
            utxo_snapshot_applied: RwLock::new(false),
            available_manifests: RwLock::new(HashMap::new()),
            snapshot_peers: RwLock::new(HashSet::new()),
            downloading_chunks: RwLock::new(HashMap::new()),
            downloaded_chunks: RwLock::new(HashSet::new()),
            manifest_request_time: RwLock::new(None),
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
        } else if matches!(
            *state,
            SyncState::Idle | SyncState::Synchronized | SyncState::ApplyingSnapshot { .. }
        ) {
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

    // ==================== UTXO Snapshot Sync Methods ====================

    /// Check if UTXO snapshot bootstrap is enabled.
    pub fn is_utxo_bootstrap_enabled(&self) -> bool {
        self.config.utxo_bootstrap
    }

    /// Check if UTXO snapshot has been applied.
    pub fn is_utxo_snapshot_applied(&self) -> bool {
        *self.utxo_snapshot_applied.read()
    }

    /// Mark UTXO snapshot as applied.
    pub fn set_utxo_snapshot_applied(&self) {
        *self.utxo_snapshot_applied.write() = true;
        info!("UTXO snapshot marked as applied");
    }

    /// Check if we should use snapshot sync.
    ///
    /// Returns true if:
    /// - UTXO bootstrap is enabled
    /// - No snapshot has been applied yet
    /// - We have no blocks (height 0)
    pub fn should_use_snapshot_sync(&self) -> bool {
        self.config.utxo_bootstrap
            && !self.is_utxo_snapshot_applied()
            && *self.our_height.read() == 0
    }

    /// Register a peer as supporting UTXO snapshots.
    pub fn register_snapshot_peer(&self, peer: PeerId) {
        debug!(peer = %peer, "Registered snapshot peer");
        self.snapshot_peers.write().insert(peer);
    }

    /// Unregister a peer from snapshot support.
    pub fn unregister_snapshot_peer(&self, peer: &PeerId) {
        self.snapshot_peers.write().remove(peer);
        self.available_manifests.write().retain(|_, (_, peers)| {
            peers.retain(|p| p != peer);
            !peers.is_empty()
        });
    }

    /// Get peers that support UTXO snapshots.
    pub fn snapshot_peers(&self) -> Vec<PeerId> {
        self.snapshot_peers.read().iter().cloned().collect()
    }

    /// Get number of snapshot-supporting peers.
    pub fn snapshot_peer_count(&self) -> usize {
        self.snapshot_peers.read().len()
    }

    /// Start awaiting snapshots info from peers.
    pub fn start_snapshot_info_request(&self, peer_count: usize) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        *self.state.write() = SyncState::AwaitingSnapshotsInfo {
            requested_from: peer_count,
            started_at_ms: now_ms,
        };
        info!(peers = peer_count, "Requesting snapshots info from peers");
    }

    /// Process snapshots info received from a peer.
    ///
    /// Returns the manifest ID and height if we should request a manifest.
    pub fn process_snapshots_info(
        &self,
        peer: PeerId,
        manifests: Vec<(u32, [u8; 32])>,
    ) -> Option<([u8; 32], u32)> {
        if manifests.is_empty() {
            debug!(peer = %peer, "Peer has no snapshots");
            return None;
        }

        {
            let mut available = self.available_manifests.write();

            for (height, manifest_id) in manifests {
                debug!(
                    peer = %peer,
                    height,
                    manifest_id = hex::encode(manifest_id),
                    "Discovered snapshot manifest"
                );

                available
                    .entry(manifest_id)
                    .and_modify(|(_, peers)| {
                        if !peers.contains(&peer) {
                            peers.push(peer.clone());
                        }
                    })
                    .or_insert((height, vec![peer.clone()]));
            }
        } // Drop write lock before calling check_manifest_availability

        // Check if any manifest has enough peers
        self.check_manifest_availability()
    }

    /// Check if any manifest has enough peers to start downloading.
    ///
    /// Returns the best manifest (highest height) with enough peers.
    fn check_manifest_availability(&self) -> Option<([u8; 32], u32)> {
        let available = self.available_manifests.read();
        let min_peers = self.config.min_snapshot_peers;

        // Find manifests with enough peers
        let valid: Vec<_> = available
            .iter()
            .filter(|(_, (_, peers))| peers.len() >= min_peers)
            .collect();

        if valid.is_empty() {
            return None;
        }

        // Select the one with highest height
        valid
            .into_iter()
            .max_by_key(|(_, (height, _))| *height)
            .map(|(id, (height, _))| (*id, *height))
    }

    /// Start awaiting manifest from a peer.
    pub fn start_manifest_request(&self, manifest_id: [u8; 32], height: u32, peer: PeerId) {
        *self.state.write() = SyncState::AwaitingManifest {
            manifest_id,
            height,
            peer: peer.clone(),
        };
        *self.manifest_request_time.write() = Some(Instant::now());
        info!(
            height,
            manifest_id = hex::encode(manifest_id),
            peer = %peer,
            "Requesting manifest"
        );
    }

    /// Check if manifest request has timed out.
    ///
    /// Returns the peer if timed out, None otherwise.
    pub fn check_manifest_timeout(&self, timeout: Duration) -> Option<PeerId> {
        let state = self.state.read();
        if let SyncState::AwaitingManifest { peer, .. } = &*state {
            if let Some(start_time) = *self.manifest_request_time.read() {
                if start_time.elapsed() > timeout {
                    return Some(peer.clone());
                }
            }
        }
        None
    }

    /// Clear manifest request timeout tracking (called when manifest received).
    pub fn clear_manifest_request(&self) {
        *self.manifest_request_time.write() = None;
    }

    /// Get a random peer that has a specific manifest.
    pub fn get_manifest_peer(&self, manifest_id: &[u8; 32]) -> Option<PeerId> {
        let available = self.available_manifests.read();
        available
            .get(manifest_id)
            .and_then(|(_, peers)| peers.choose(&mut rand::thread_rng()).cloned())
    }

    /// Start chunk download phase.
    pub fn start_chunk_download(&self, manifest_id: [u8; 32], height: u32, total_chunks: usize) {
        *self.state.write() = SyncState::DownloadingChunks {
            manifest_id,
            height,
            total_chunks,
            downloaded_chunks: 0,
        };
        self.downloaded_chunks.write().clear();
        self.downloading_chunks.write().clear();
        info!(
            height,
            total_chunks,
            manifest_id = hex::encode(manifest_id),
            "Starting chunk download"
        );
    }

    /// Get chunks to download (up to count).
    ///
    /// Returns chunk IDs that aren't already downloaded or being downloaded.
    pub fn get_chunks_to_request(
        &self,
        expected_chunks: &[[u8; 32]],
        count: usize,
    ) -> Vec<[u8; 32]> {
        let downloaded = self.downloaded_chunks.read();
        let downloading = self.downloading_chunks.read();

        expected_chunks
            .iter()
            .filter(|id| !downloaded.contains(*id) && !downloading.contains_key(*id))
            .take(count)
            .copied()
            .collect()
    }

    /// Mark a chunk as being downloaded from a peer.
    pub fn mark_chunk_downloading(&self, chunk_id: [u8; 32], peer: PeerId) {
        self.downloading_chunks
            .write()
            .insert(chunk_id, (peer, Instant::now()));
    }

    /// Mark a chunk as downloaded.
    ///
    /// Returns true if all chunks are now downloaded.
    pub fn mark_chunk_downloaded(&self, chunk_id: [u8; 32]) -> bool {
        self.downloading_chunks.write().remove(&chunk_id);
        self.downloaded_chunks.write().insert(chunk_id);

        let downloaded_count = self.downloaded_chunks.read().len();

        // Update state
        let mut state = self.state.write();
        if let SyncState::DownloadingChunks {
            manifest_id,
            height,
            total_chunks,
            ..
        } = *state
        {
            *state = SyncState::DownloadingChunks {
                manifest_id,
                height,
                total_chunks,
                downloaded_chunks: downloaded_count,
            };

            if downloaded_count >= total_chunks {
                info!(height, total_chunks, "All snapshot chunks downloaded");
                return true;
            }
        }

        false
    }

    /// Mark a chunk download as failed (for retry).
    pub fn mark_chunk_failed(&self, chunk_id: &[u8; 32]) {
        self.downloading_chunks.write().remove(chunk_id);
    }

    /// Get chunks that have timed out.
    pub fn get_timed_out_chunks(&self, timeout: Duration) -> Vec<([u8; 32], PeerId)> {
        let downloading = self.downloading_chunks.read();
        let now = Instant::now();

        downloading
            .iter()
            .filter(|(_, (_, start_time))| now.duration_since(*start_time) > timeout)
            .map(|(id, (peer, _))| (*id, peer.clone()))
            .collect()
    }

    /// Get number of chunks currently being downloaded.
    pub fn downloading_chunk_count(&self) -> usize {
        self.downloading_chunks.read().len()
    }

    /// Get number of downloaded chunks.
    pub fn downloaded_chunk_count(&self) -> usize {
        self.downloaded_chunks.read().len()
    }

    /// Check if we need more chunk downloads.
    pub fn needs_more_chunk_downloads(&self) -> bool {
        self.downloading_chunks.read().len() < CHUNKS_IN_PARALLEL_MIN
    }

    /// Get a random peer for chunk download.
    pub fn get_random_chunk_peer(&self) -> Option<PeerId> {
        let peers: Vec<_> = self.snapshot_peers.read().iter().cloned().collect();
        peers.choose(&mut rand::thread_rng()).cloned()
    }

    /// Start applying snapshot.
    pub fn start_snapshot_application(&self, height: u32) {
        *self.state.write() = SyncState::ApplyingSnapshot { height };
        info!(height, "Applying UTXO snapshot");
    }

    /// Complete snapshot application and transition to normal sync.
    pub fn complete_snapshot_application(&self, height: u32) {
        self.set_utxo_snapshot_applied();
        *self.our_height.write() = height;

        // Clear snapshot state
        self.available_manifests.write().clear();
        self.downloading_chunks.write().clear();
        self.downloaded_chunks.write().clear();

        info!(height, "UTXO snapshot applied, resuming normal sync");

        // Transition to header sync if needed
        self.check_sync_status();
    }

    /// Get snapshot sync progress (0.0 - 1.0).
    pub fn snapshot_progress(&self) -> Option<f64> {
        match self.state() {
            SyncState::DownloadingChunks {
                total_chunks,
                downloaded_chunks,
                ..
            } => {
                if total_chunks == 0 {
                    Some(1.0)
                } else {
                    Some(downloaded_chunks as f64 / total_chunks as f64)
                }
            }
            SyncState::ApplyingSnapshot { .. } => Some(1.0),
            _ => None,
        }
    }

    /// Get available manifests.
    pub fn available_manifests(&self) -> AvailableManifests {
        self.available_manifests.read().clone()
    }

    /// Clear all snapshot sync state (for restart).
    pub fn clear_snapshot_state(&self) {
        self.available_manifests.write().clear();
        self.downloading_chunks.write().clear();
        self.downloaded_chunks.write().clear();
        // Don't clear utxo_snapshot_applied - that persists for the session
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

    #[test]
    fn test_snapshot_sync_disabled_by_default() {
        let sync = Synchronizer::default();
        assert!(!sync.is_utxo_bootstrap_enabled());
        assert!(!sync.should_use_snapshot_sync());
    }

    #[test]
    fn test_snapshot_sync_enabled() {
        let config = SyncConfig {
            utxo_bootstrap: true,
            ..Default::default()
        };
        let sync = Synchronizer::new(config);

        assert!(sync.is_utxo_bootstrap_enabled());
        assert!(sync.should_use_snapshot_sync());

        // After snapshot applied, should not use snapshot sync
        sync.set_utxo_snapshot_applied();
        assert!(!sync.should_use_snapshot_sync());
    }

    #[test]
    fn test_snapshot_peer_registration() {
        let sync = Synchronizer::default();

        let peer1 = PeerId::from_bytes(vec![1, 2, 3]);
        let peer2 = PeerId::from_bytes(vec![4, 5, 6]);

        sync.register_snapshot_peer(peer1.clone());
        sync.register_snapshot_peer(peer2.clone());

        assert_eq!(sync.snapshot_peer_count(), 2);

        sync.unregister_snapshot_peer(&peer1);
        assert_eq!(sync.snapshot_peer_count(), 1);
    }

    #[test]
    fn test_process_snapshots_info() {
        let config = SyncConfig {
            utxo_bootstrap: true,
            min_snapshot_peers: 2,
            ..Default::default()
        };
        let sync = Synchronizer::new(config);

        let peer1 = PeerId::from_bytes(vec![1, 2, 3]);
        let peer2 = PeerId::from_bytes(vec![4, 5, 6]);
        let manifest_id = [1u8; 32];

        // First peer reports manifest - not enough peers yet
        let result = sync.process_snapshots_info(peer1, vec![(100000, manifest_id)]);
        assert!(result.is_none());

        // Second peer reports same manifest - now we have enough
        let result = sync.process_snapshots_info(peer2, vec![(100000, manifest_id)]);
        assert!(result.is_some());

        let (id, height) = result.unwrap();
        assert_eq!(id, manifest_id);
        assert_eq!(height, 100000);
    }

    #[test]
    fn test_chunk_download_tracking() {
        let sync = Synchronizer::default();
        let manifest_id = [1u8; 32];
        let chunk1 = [2u8; 32];
        let chunk2 = [3u8; 32];
        let chunk3 = [4u8; 32];
        let peer = PeerId::from_bytes(vec![1, 2, 3]);

        // Start chunk download
        sync.start_chunk_download(manifest_id, 100000, 3);

        assert!(matches!(
            sync.state(),
            SyncState::DownloadingChunks {
                total_chunks: 3,
                downloaded_chunks: 0,
                ..
            }
        ));

        // Get chunks to request
        let expected = vec![chunk1, chunk2, chunk3];
        let to_request = sync.get_chunks_to_request(&expected, 2);
        assert_eq!(to_request.len(), 2);

        // Mark chunks as downloading
        sync.mark_chunk_downloading(chunk1, peer.clone());
        sync.mark_chunk_downloading(chunk2, peer.clone());
        assert_eq!(sync.downloading_chunk_count(), 2);

        // Mark first as downloaded
        let all_done = sync.mark_chunk_downloaded(chunk1);
        assert!(!all_done);
        assert_eq!(sync.downloaded_chunk_count(), 1);
        assert_eq!(sync.downloading_chunk_count(), 1);

        // Mark second as downloaded
        let all_done = sync.mark_chunk_downloaded(chunk2);
        assert!(!all_done);

        // Mark third as downloading and downloaded
        sync.mark_chunk_downloading(chunk3, peer);
        let all_done = sync.mark_chunk_downloaded(chunk3);
        assert!(all_done);

        assert!(matches!(
            sync.state(),
            SyncState::DownloadingChunks {
                total_chunks: 3,
                downloaded_chunks: 3,
                ..
            }
        ));
    }

    #[test]
    fn test_snapshot_progress() {
        let sync = Synchronizer::default();

        // No progress when not in snapshot sync
        assert!(sync.snapshot_progress().is_none());

        // Start chunk download
        sync.start_chunk_download([1u8; 32], 100000, 4);

        assert_eq!(sync.snapshot_progress(), Some(0.0));

        // Download 2 of 4 chunks
        sync.mark_chunk_downloaded([2u8; 32]);
        sync.mark_chunk_downloaded([3u8; 32]);

        assert_eq!(sync.snapshot_progress(), Some(0.5));
    }

    #[test]
    fn test_complete_snapshot_application() {
        let config = SyncConfig {
            utxo_bootstrap: true,
            ..Default::default()
        };
        let sync = Synchronizer::new(config);

        // Add peer with higher height BEFORE completing snapshot
        let peer = PeerId::from_bytes(vec![1, 2, 3]);
        sync.peer_heights.write().insert(peer, 200000);

        // Start snapshot application
        sync.start_snapshot_application(100000);
        assert!(matches!(
            sync.state(),
            SyncState::ApplyingSnapshot { height: 100000 }
        ));

        // Complete application
        sync.complete_snapshot_application(100000);

        assert!(sync.is_utxo_snapshot_applied());
        assert_eq!(sync.our_height(), 100000);

        // Should transition to header sync since peers have higher height (200000 > 100000)
        match sync.state() {
            SyncState::SyncingHeaders { from, to } => {
                assert_eq!(from, 100000);
                assert_eq!(to, 200000);
            }
            other => panic!("Expected SyncingHeaders, got {:?}", other),
        }
    }
}
