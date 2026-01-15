//! Synchronization protocol handler.
//!
//! This module implements the Ergo sync protocol:
//! 1. Exchange SyncInfo with peers to learn their chain state
//! 2. Request headers for unknown blocks
//! 3. Download and validate full blocks
//! 4. Apply blocks to state

use crate::{
    BlockDownloader, DownloadTask, SyncConfig, SyncError, SyncResult, Synchronizer,
    PARALLEL_DOWNLOADS,
};
use blake2::digest::consts::U32;
use blake2::{Blake2b, Digest};
use ergo_chain_types::{BlockId, Digest32, Header};
use ergo_consensus::block::ModifierType;
use ergo_network::{InvData, Message, ModifierRequest, PeerId, SyncInfo};
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// Compute the Blake2b-256 hash of header bytes to get the correct header ID.
/// This is needed because sigma-rust's Header::scorex_parse_bytes() computes the ID
/// from reserialized bytes, which can differ from original bytes due to BigInt
/// serialization issues (leading 0xFF bytes being stripped).
fn compute_header_id(data: &[u8]) -> Vec<u8> {
    let mut hasher = Blake2b::<U32>::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

/// Compute the ID for a non-header block section (BlockTransactions, Extension, ADProofs).
///
/// In the Ergo protocol, block sections have a derived ID computed as:
/// `blake2b256(modifierTypeId ++ headerId ++ sectionDigest)`
///
/// This is NOT the same as the header ID! The Scala node uses this derived ID
/// when sending Inv messages and expecting RequestModifier requests.
///
/// For BlockTransactions: sectionDigest = header.transactionsRoot
/// For Extension: sectionDigest = header.extensionRoot
/// For ADProofs: sectionDigest = header.adProofsRoot
fn compute_block_section_id(modifier_type: u8, header_id: &[u8], section_digest: &[u8]) -> Vec<u8> {
    let mut hasher = Blake2b::<U32>::new();
    hasher.update(&[modifier_type]);
    hasher.update(header_id);
    hasher.update(section_digest);
    hasher.finalize().to_vec()
}

/// Messages that the sync service can send.
#[derive(Debug)]
pub enum SyncCommand {
    /// Send a message to a specific peer.
    SendToPeer { peer: PeerId, message: Message },
    /// Send a message to all connected peers.
    Broadcast { message: Message },
    /// Store a received header (with response channel for confirmation).
    StoreHeader {
        header: Header,
        /// Raw bytes from network - needed for correct ID computation.
        raw_bytes: Vec<u8>,
        response_tx: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    /// Request to apply a validated block.
    ApplyBlock {
        block_id: Vec<u8>,
        block_data: Vec<u8>,
    },
    /// Store a received extension.
    StoreExtension {
        /// Header ID this extension belongs to.
        header_id: Vec<u8>,
        /// Raw extension data.
        extension_data: Vec<u8>,
    },
    /// Add a transaction to the mempool.
    AddTransaction {
        tx_id: Vec<u8>,
        tx_data: Vec<u8>,
        /// Peer that sent this transaction (for rebroadcast exclusion).
        from_peer: PeerId,
    },
    /// Store multiple headers in a single batched write operation.
    /// This is significantly more efficient than storing headers individually.
    StoreHeadersBatch {
        /// Headers with their raw bytes, in ascending height order.
        headers: Vec<(Header, Vec<u8>)>,
        /// Response channel for confirmation.
        response_tx: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
}

/// Events that the sync service receives.
#[derive(Debug, Clone)]
pub enum SyncEvent {
    /// A new peer connected.
    PeerConnected { peer: PeerId },
    /// A peer disconnected.
    PeerDisconnected { peer: PeerId },
    /// Received SyncInfo from a peer.
    SyncInfoReceived { peer: PeerId, info: SyncInfo },
    /// Received inventory announcement.
    InvReceived { peer: PeerId, inv: InvData },
    /// Received modifier (header or block).
    ModifierReceived {
        peer: PeerId,
        type_id: u8,
        id: Vec<u8>,
        data: Vec<u8>,
    },
    /// A block was successfully applied.
    BlockApplied { block_id: Vec<u8>, height: u32 },
    /// Block application failed.
    BlockFailed { block_id: Vec<u8>, error: String },
    /// Request to download blocks for these headers.
    /// The node sends this with headers for blocks it needs (based on UTXO height).
    RequestBlocks { headers: Vec<Header> },
    /// Periodic tick for housekeeping.
    Tick,
}

/// Maximum number of headers to keep in memory.
/// Headers older than this are dropped to save memory.
/// With ~500 bytes per header, 50000 headers = ~25MB.
/// Increased from 10k to reduce header cache misses during fast sync.
const MAX_CACHED_HEADERS: usize = 50_000;

/// Maximum number of block IDs to track for download.
/// This limits memory used by missing_blocks and tx_id_to_header_id.
const MAX_PENDING_BLOCKS: usize = 10_000;

/// Header chain segment for tracking sync progress.
#[derive(Debug, Clone)]
struct HeaderChain {
    /// Headers we've received but not yet have blocks for.
    /// Limited to MAX_CACHED_HEADERS to prevent unbounded memory growth.
    headers: VecDeque<Header>,
    /// BlockTransactions IDs we need to download.
    /// These are DERIVED IDs: blake2b256(typeId ++ headerId ++ transactionsRoot)
    /// NOT the same as header IDs!
    missing_blocks: VecDeque<Vec<u8>>,
    /// Mapping from transactionsId -> headerId for matching received blocks.
    /// When we receive a block, we need to find the corresponding header.
    tx_id_to_header_id: HashMap<Vec<u8>, Vec<u8>>,
    /// Reverse mapping from headerId -> transactionsId for O(1) lookup.
    /// Used in on_block_applied/on_block_failed to quickly find the tx_id.
    header_id_to_tx_id: HashMap<Vec<u8>, Vec<u8>>,
    /// Headers indexed by ID for O(1) lookup.
    /// Used in on_block_received to quickly find the header for validation.
    headers_by_id: HashMap<Vec<u8>, Header>,
}

impl HeaderChain {
    fn new() -> Self {
        Self {
            headers: VecDeque::new(),
            missing_blocks: VecDeque::new(),
            tx_id_to_header_id: HashMap::new(),
            header_id_to_tx_id: HashMap::new(),
            headers_by_id: HashMap::new(),
        }
    }

    /// Add a header and its block ID, enforcing memory limits.
    fn add_header(&mut self, header: Header, transactions_id: Vec<u8>, header_id: Vec<u8>) {
        // Add the new entries
        self.headers_by_id.insert(header_id.clone(), header.clone());
        self.headers.push_back(header);
        // NOTE: Do NOT add to missing_blocks here!
        // Headers are buffered before being written to the database.
        // If we add to missing_blocks now, queue_block_downloads() may request blocks
        // before the headers are in the database, causing "Header not found" errors.
        // Block downloads are properly handled by block_sync_interval in node.rs,
        // which only requests blocks for headers that are already stored in the database.
        self.header_id_to_tx_id
            .insert(header_id.clone(), transactions_id.clone());
        self.tx_id_to_header_id.insert(transactions_id, header_id);

        // Trim old entries if we exceed limits
        self.enforce_limits();
    }

    /// Add header tracking for O(1) lookups without adding to missing_blocks.
    /// Used by on_request_blocks which manages downloads via the downloader directly.
    /// Note: Does NOT call enforce_limits() - caller should call it after batch operations.
    fn track_header(&mut self, header: Header, transactions_id: Vec<u8>, header_id: Vec<u8>) {
        // Add to lookup indexes only - don't add to missing_blocks since
        // on_request_blocks queues downloads directly via the downloader
        self.headers_by_id.insert(header_id.clone(), header.clone());
        self.headers.push_back(header);
        self.header_id_to_tx_id
            .insert(header_id.clone(), transactions_id.clone());
        self.tx_id_to_header_id.insert(transactions_id, header_id);
        // Note: enforce_limits() is NOT called here to avoid overhead during batch inserts.
        // The caller (on_request_blocks) should call it once after adding all headers.
    }

    /// Remove a header and its mappings when a block is applied.
    fn remove_header(&mut self, header_id: &[u8]) {
        // Remove from headers_by_id
        self.headers_by_id.remove(header_id);

        // Remove from reverse index and get the tx_id
        if let Some(tx_id) = self.header_id_to_tx_id.remove(header_id) {
            // Remove from tx_id_to_header_id
            self.tx_id_to_header_id.remove(&tx_id);
            // Note: We do NOT remove from missing_blocks here as it would be O(n).
            // missing_blocks is only used by queue_block_downloads() which drains from the front,
            // and stale entries (already downloaded) are simply skipped by the downloader.
            // The enforce_limits() method will clean up old entries from the front.
        }

        // Note: We don't remove from the headers VecDeque here because it would be O(n).
        // Instead, stale entries in headers VecDeque are cleaned up by enforce_limits().
        // The headers_by_id HashMap is the authoritative source for lookups.
    }

    /// Get a header by its ID in O(1) time.
    fn get_header(&self, header_id: &[u8]) -> Option<&Header> {
        self.headers_by_id.get(header_id)
    }

    /// Get the tx_id for a header_id in O(1) time.
    fn get_tx_id(&self, header_id: &[u8]) -> Option<&Vec<u8>> {
        self.header_id_to_tx_id.get(header_id)
    }

    /// Remove old entries to enforce memory limits.
    fn enforce_limits(&mut self) {
        // Trim headers and their mappings
        while self.headers.len() > MAX_CACHED_HEADERS {
            if let Some(old_header) = self.headers.pop_front() {
                let old_header_id = old_header.id.0.as_ref();
                // Clean up associated mappings
                self.headers_by_id.remove(old_header_id);
                if let Some(tx_id) = self.header_id_to_tx_id.remove(old_header_id) {
                    self.tx_id_to_header_id.remove(&tx_id);
                }
            }
        }

        // Trim missing_blocks and corresponding mappings
        while self.missing_blocks.len() > MAX_PENDING_BLOCKS {
            if let Some(old_tx_id) = self.missing_blocks.pop_front() {
                if let Some(header_id) = self.tx_id_to_header_id.remove(&old_tx_id) {
                    self.header_id_to_tx_id.remove(&header_id);
                    self.headers_by_id.remove(&header_id);
                }
            }
        }
    }

    /// Get memory usage statistics.
    fn memory_stats(&self) -> (usize, usize, usize) {
        (
            self.headers.len(),
            self.missing_blocks.len(),
            self.tx_id_to_header_id.len(),
        )
    }
}

/// Pending header waiting for its parent.
#[derive(Debug, Clone)]
struct PendingHeader {
    /// The header itself.
    header: Header,
    /// Raw bytes from network - needed for correct ID computation on storage.
    raw_bytes: Vec<u8>,
    /// Parent ID we're waiting for.
    parent_id: Vec<u8>,
}

/// Efficient container for pending headers with O(1) lookup and O(log n) eviction.
struct PendingHeadersMap {
    /// Main storage: header ID -> pending header data.
    by_id: HashMap<Vec<u8>, PendingHeader>,
    /// Index: parent ID -> set of child header IDs waiting for this parent.
    by_parent: HashMap<Vec<u8>, HashSet<Vec<u8>>>,
    /// Sorted set for efficient min-height eviction: (height, header_id).
    by_height: std::collections::BTreeSet<(u32, Vec<u8>)>,
}

impl PendingHeadersMap {
    fn new() -> Self {
        Self {
            by_id: HashMap::new(),
            by_parent: HashMap::new(),
            by_height: std::collections::BTreeSet::new(),
        }
    }

    fn len(&self) -> usize {
        self.by_id.len()
    }

    fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// Insert a pending header. If at capacity, evicts the oldest (lowest height) entry first.
    fn insert(&mut self, id: Vec<u8>, pending: PendingHeader, max_size: usize) -> Option<u32> {
        let mut evicted_height = None;

        // Evict oldest if at capacity
        if self.by_id.len() >= max_size {
            if let Some(&(height, ref evict_id)) = self.by_height.iter().next() {
                let evict_id = evict_id.clone();
                self.remove(&evict_id);
                evicted_height = Some(height);
            }
        }

        let height = pending.header.height;
        let parent_id = pending.parent_id.clone();

        // Insert into main storage
        self.by_id.insert(id.clone(), pending);

        // Update parent index
        self.by_parent
            .entry(parent_id)
            .or_default()
            .insert(id.clone());

        // Update height index
        self.by_height.insert((height, id));

        evicted_height
    }

    /// Remove a pending header by ID.
    fn remove(&mut self, id: &[u8]) -> Option<PendingHeader> {
        let id_vec = id.to_vec();
        if let Some(pending) = self.by_id.remove(&id_vec) {
            // Remove from parent index
            if let Some(children) = self.by_parent.get_mut(&pending.parent_id) {
                children.remove(&id_vec);
                if children.is_empty() {
                    self.by_parent.remove(&pending.parent_id);
                }
            }
            // Remove from height index
            self.by_height.remove(&(pending.header.height, id_vec));
            Some(pending)
        } else {
            None
        }
    }

    /// Get headers waiting for a specific parent.
    fn get_by_parent(&self, parent_id: &[u8]) -> Vec<(Vec<u8>, PendingHeader)> {
        if let Some(child_ids) = self.by_parent.get(parent_id) {
            let mut result = Vec::new();
            for id in child_ids {
                if let Some(ph) = self.by_id.get(id) {
                    result.push((id.clone(), ph.clone()));
                }
            }
            result
        } else {
            Vec::new()
        }
    }

    /// Get a sample of pending headers for debugging.
    fn sample(&self, count: usize) -> Vec<(u32, String)> {
        self.by_id
            .values()
            .take(count)
            .map(|ph| (ph.header.height, hex::encode(&ph.parent_id)))
            .collect()
    }

    /// Clear all pending headers.
    fn clear(&mut self) {
        self.by_id.clear();
        self.by_parent.clear();
        self.by_height.clear();
    }
}

/// Maximum number of headers to include in V2 SyncInfo.
/// The Scala node uses `ErgoSyncInfo.MaxBlockIds = 10` for V2.
const MAX_V2_HEADERS: usize = 10;

/// V2 SyncInfo offsets from best height (matches Scala node's FullV2SyncOffsets).
const V2_SYNC_OFFSETS: [u32; 4] = [0, 16, 128, 512];

/// Maximum number of headers to buffer before flushing to storage.
/// Increased from 200 to amortize storage overhead during fast sync.
const HEADER_BATCH_SIZE: usize = 500;

/// Maximum size of the stored_headers LRU cache.
/// 100,000 entries covers ~4 days of blocks at 2min/block, plenty for active sync window.
/// Increased to reduce cache misses during fast sync.
const STORED_HEADERS_CACHE_SIZE: usize = 100_000;

/// Maximum size of the pending_headers map (headers waiting for their parent).
const MAX_PENDING_HEADERS: usize = 10_000;

/// Maximum size of the pending_header_inv queue.
const MAX_PENDING_HEADER_INV: usize = 10_000;

/// Maximum time to buffer headers before flushing (milliseconds).
const HEADER_BATCH_TIMEOUT_MS: u64 = 500;

/// Sync protocol handler.
pub struct SyncProtocol {
    /// Synchronizer state.
    synchronizer: Arc<Synchronizer>,
    /// Block downloader.
    downloader: Arc<BlockDownloader>,
    /// Our current best header IDs (for SyncInfo).
    our_best_headers: RwLock<Vec<Vec<u8>>>,
    /// Headers we're tracking.
    header_chain: RwLock<HeaderChain>,
    /// Headers for V2 SyncInfo at offsets [0, 16, 128, 512] from best height.
    /// Stored in newest-first order (best header at index 0).
    v2_sync_headers: RwLock<Vec<Header>>,
    /// Peers we're syncing from.
    sync_peers: RwLock<HashMap<PeerId, PeerSyncState>>,
    /// Headers pending storage (waiting for parent).
    /// Uses efficient data structures for O(1) lookup and O(log n) eviction.
    pending_headers: RwLock<PendingHeadersMap>,
    /// Headers we've successfully stored (by ID).
    /// Uses LRU cache to bound memory - old entries are evicted automatically.
    /// Uses RwLock to allow parallel reads (using peek() which doesn't update LRU order).
    stored_headers: RwLock<lru::LruCache<Vec<u8>, ()>>,
    /// Command sender.
    command_tx: mpsc::Sender<SyncCommand>,
    /// Pending header IDs from Inv that we haven't requested yet.
    pending_header_inv: RwLock<VecDeque<Vec<u8>>>,
    /// Header IDs we've requested but not yet received (in-flight), with request timestamp.
    in_flight_headers: RwLock<HashMap<Vec<u8>, std::time::Instant>>,
    /// Header request timeout duration.
    header_timeout: std::time::Duration,
    /// Transaction IDs we've already processed (to avoid re-requesting).
    /// Uses LRU cache to automatically evict oldest entries when capacity is exceeded.
    known_transactions: parking_lot::Mutex<lru::LruCache<Vec<u8>, ()>>,
    /// Buffer for headers pending batched write to storage.
    /// Headers are accumulated here and flushed in batches for efficiency.
    pending_header_writes: RwLock<Vec<(Header, Vec<u8>)>>,
    /// Timestamp when the first header was added to pending_header_writes.
    /// Used to trigger timeout-based flushing.
    pending_header_writes_since: RwLock<Option<std::time::Instant>>,
}

/// Peer chain status relative to ours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PeerChainStatus {
    /// Unknown status (initial state).
    Unknown,
    /// Peer is on a shorter chain than us.
    Younger,
    /// Peer is on a longer chain than us.
    Older,
    /// Peer is on a forked chain.
    Fork,
    /// Peer is on the same chain as us.
    Equal,
}

/// Per-peer sync state.
#[derive(Debug, Clone)]
struct PeerSyncState {
    /// Their reported height.
    height: u32,
    /// Their best header IDs.
    best_headers: Vec<Vec<u8>>,
    /// Number of requests in flight to this peer.
    in_flight: u32,
    /// Last activity timestamp.
    last_activity: std::time::Instant,
    /// Last time we sent SyncInfo to this peer.
    last_sync_sent: std::time::Instant,
    /// Last time we sent a modifier request to this peer.
    last_request_sent: std::time::Instant,
    /// Last time we received SyncInfo from this peer.
    last_sync_received: std::time::Instant,
    /// Chain status relative to ours.
    chain_status: PeerChainStatus,
}

/// Minimum interval between SyncInfo messages to the same peer.
/// Reduced from 20s to 5s for faster header discovery during initial sync.
const MIN_SYNC_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// Minimum interval between processing SyncInfo from the same peer (100ms like Scala node).
/// This prevents resource exhaustion from spammy peers.
const PER_PEER_SYNC_LOCK_TIME: std::time::Duration = std::time::Duration::from_millis(100);

/// Minimum interval between modifier requests to the same peer.
/// The Scala node doesn't have explicit per-request throttling, but has deliveryTimeout = 10s.
/// Using 100ms to allow fast sequential requests while avoiding overwhelming peers.
const MIN_REQUEST_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

/// Maximum number of header IDs to include in SyncInfo message.
/// The Scala node samples ~100 headers from the chain for SyncInfo.
const MAX_SYNC_HEADER_IDS: usize = 100;

impl PeerSyncState {
    fn new() -> Self {
        // Set last_sync_sent to epoch so first sync is allowed immediately
        let epoch = std::time::Instant::now() - std::time::Duration::from_secs(60);
        Self {
            height: 0,
            best_headers: Vec::new(),
            in_flight: 0,
            last_activity: std::time::Instant::now(),
            last_sync_sent: epoch,
            last_request_sent: epoch,
            last_sync_received: epoch,
            chain_status: PeerChainStatus::Unknown,
        }
    }

    /// Check if we can send SyncInfo to this peer (respecting MIN_SYNC_INTERVAL).
    fn can_send_sync(&self) -> bool {
        self.last_sync_sent.elapsed() >= MIN_SYNC_INTERVAL
    }

    /// Check if we should process SyncInfo from this peer (respecting PER_PEER_SYNC_LOCK_TIME).
    /// Returns true if enough time has passed since last SyncInfo from this peer.
    fn can_process_sync(&self) -> bool {
        self.last_sync_received.elapsed() >= PER_PEER_SYNC_LOCK_TIME
    }

    /// Check if we should send SyncInfo response based on status change and conditions.
    /// Matches Scala node logic: send if status changed, peer is outdated, or we're older/fork.
    fn should_send_sync_response(
        &self,
        old_status: PeerChainStatus,
        new_status: PeerChainStatus,
    ) -> bool {
        // Send if status changed
        if old_status != new_status {
            return true;
        }
        // Send if we're older or on a fork (we need their headers)
        if new_status == PeerChainStatus::Older || new_status == PeerChainStatus::Fork {
            return true;
        }
        // Send if peer hasn't received sync from us in a while
        if self.last_sync_sent.elapsed() >= MIN_SYNC_INTERVAL {
            return true;
        }
        false
    }

    /// Check if we can send a modifier request to this peer (respecting MIN_REQUEST_INTERVAL).
    fn can_send_request(&self) -> bool {
        self.last_request_sent.elapsed() >= MIN_REQUEST_INTERVAL
    }
}

impl SyncProtocol {
    /// Create a new sync protocol handler.
    pub fn new(config: SyncConfig, command_tx: mpsc::Sender<SyncCommand>) -> Self {
        // The genesis parent ID is all zeros - this is the parent of the height=1 block.
        // The height=1 block (b0244dfc...) has parentId = 0000...0000
        // We pre-seed this so that when we receive the height=1 header, its parent is "available"
        let genesis_parent_id = vec![0u8; 32];

        // Initialize LRU cache for stored headers with genesis parent pre-seeded
        let mut stored =
            lru::LruCache::new(std::num::NonZeroUsize::new(STORED_HEADERS_CACHE_SIZE).unwrap());
        // Consider genesis parent as "stored" so height-1 headers can be applied
        stored.put(genesis_parent_id, ());

        Self {
            synchronizer: Arc::new(Synchronizer::new(config)),
            downloader: Arc::new(BlockDownloader::default()),
            our_best_headers: RwLock::new(Vec::new()),
            header_chain: RwLock::new(HeaderChain::new()),
            v2_sync_headers: RwLock::new(Vec::new()),
            sync_peers: RwLock::new(HashMap::new()),
            pending_headers: RwLock::new(PendingHeadersMap::new()),
            stored_headers: RwLock::new(stored),
            command_tx,
            pending_header_inv: RwLock::new(VecDeque::new()),
            in_flight_headers: RwLock::new(HashMap::new()),
            header_timeout: std::time::Duration::from_secs(10),
            // LRU cache for known transaction IDs - automatically evicts oldest entries
            known_transactions: parking_lot::Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(10_000).unwrap(),
            )),
            pending_header_writes: RwLock::new(Vec::new()),
            pending_header_writes_since: RwLock::new(None),
        }
    }

    /// Get the synchronizer.
    pub fn synchronizer(&self) -> &Synchronizer {
        &self.synchronizer
    }

    /// Initialize with stored headers from database.
    /// This should be called before starting sync if we have previously synced headers.
    ///
    /// # Arguments
    /// * `all_header_ids` - All stored header IDs (for tracking which headers we have).
    ///   **IMPORTANT**: Must be in ascending height order (oldest first, newest last).
    ///   Only the most recent ones (up to STORED_HEADERS_CACHE_SIZE) are loaded into the LRU cache.
    /// * `locator_ids` - Exponentially spaced header IDs for SyncInfo messages
    /// * `v2_headers` - Headers at V2 SyncInfo offsets [0, 16, 128, 512] from best height (newest first)
    ///
    /// # Panics
    /// Debug builds will panic if `v2_headers` is not in descending height order (newest first).
    pub fn init_from_stored_headers(
        &self,
        all_header_ids: Vec<Vec<u8>>,
        locator_ids: Vec<Vec<u8>>,
        v2_headers: Vec<Header>,
    ) {
        if all_header_ids.is_empty() {
            return;
        }

        // Verify v2_headers ordering assumption in debug builds
        debug_assert!(
            v2_headers.windows(2).all(|w| w[0].height >= w[1].height),
            "v2_headers must be in descending height order (newest first)"
        );

        // Only load the most recent header IDs into the LRU cache to bound memory.
        // The all_header_ids are assumed to be in ascending order, so we take the last N.
        let ids_to_load = if all_header_ids.len() > STORED_HEADERS_CACHE_SIZE {
            &all_header_ids[all_header_ids.len() - STORED_HEADERS_CACHE_SIZE..]
        } else {
            &all_header_ids[..]
        };

        info!(
            "Initializing sync protocol with {} stored headers (loading {} into cache), {} locator headers, {} V2 sync headers",
            all_header_ids.len(),
            ids_to_load.len(),
            locator_ids.len(),
            v2_headers.len()
        );

        // Add recent header IDs to stored cache (for tracking what we already have)
        {
            let mut stored = self.stored_headers.write();
            for id in ids_to_load {
                stored.put(id.clone(), ());
            }
        }

        // Set our best headers for SyncInfo (exponentially spaced locator)
        *self.our_best_headers.write() = locator_ids;

        // Store V2 sync headers (headers at offsets [0, 16, 128, 512] from best height)
        if !v2_headers.is_empty() {
            let heights: Vec<u32> = v2_headers.iter().map(|h| h.height).collect();
            *self.v2_sync_headers.write() = v2_headers;
            info!("Initialized V2 sync headers at heights: {:?}", heights);
        }
    }

    /// Get the downloader.
    pub fn downloader(&self) -> &BlockDownloader {
        &self.downloader
    }

    /// Set our best header IDs (called when our chain updates).
    pub fn set_our_headers(&self, header_ids: Vec<Vec<u8>>) {
        *self.our_best_headers.write() = header_ids;
    }

    /// Set our current height.
    pub fn set_height(&self, height: u32) {
        self.synchronizer.set_height(height);
    }

    /// Get the best header height from the in-memory chain.
    /// This may be higher than the stored database height during sync.
    pub fn best_header_height(&self) -> u32 {
        self.header_chain
            .read()
            .headers
            .back()
            .map(|h| h.height)
            .unwrap_or_else(|| self.synchronizer.our_height())
    }

    /// Handle an incoming sync event.
    pub async fn handle_event(&self, event: SyncEvent) -> SyncResult<()> {
        match event {
            SyncEvent::PeerConnected { peer } => {
                self.on_peer_connected(peer).await?;
            }
            SyncEvent::PeerDisconnected { peer } => {
                self.on_peer_disconnected(&peer);
            }
            SyncEvent::SyncInfoReceived { peer, info } => {
                self.on_sync_info(peer, info).await?;
            }
            SyncEvent::InvReceived { peer, inv } => {
                self.on_inv(peer, inv).await?;
            }
            SyncEvent::ModifierReceived {
                peer,
                type_id,
                id,
                data,
            } => {
                self.on_modifier(peer, type_id, id, data).await?;
            }
            SyncEvent::BlockApplied { block_id, height } => {
                self.on_block_applied(&block_id, height);
            }
            SyncEvent::BlockFailed { block_id, error } => {
                self.on_block_failed(&block_id, &error);
            }
            SyncEvent::RequestBlocks { headers } => {
                self.on_request_blocks(headers).await?;
            }
            SyncEvent::Tick => {
                self.on_tick().await?;
            }
        }
        Ok(())
    }

    /// Handle new peer connection.
    async fn on_peer_connected(&self, peer: PeerId) -> SyncResult<()> {
        info!(peer = %peer, "Peer connected, sending SyncInfo V2");

        // Add to sync peers with initial state
        let mut state = PeerSyncState::new();
        // Mark that we're about to send SyncInfo
        state.last_sync_sent = std::time::Instant::now();

        self.sync_peers.write().insert(peer.clone(), state);

        // Send V2 SyncInfo with serialized headers so the peer knows our height
        // and will respond with V2 SyncInfo containing their height
        //
        // V2 SyncInfo contains full serialized headers which include height.
        // The Scala node checks if we sent V2 first before responding with V2.
        let sync_info = self.build_v2_sync_info();

        if sync_info.is_v2() {
            info!(
                header_count = sync_info.last_headers.len(),
                "Sending SyncInfo V2 with serialized headers"
            );
        } else {
            info!(
                id_count = sync_info.last_header_ids.len(),
                "Sending SyncInfo V1 (no headers to serialize yet)"
            );
        }

        self.send_to_peer(&peer, Message::SyncInfo(sync_info))
            .await?;
        Ok(())
    }

    /// Build a V2 SyncInfo with serialized headers.
    /// Uses headers at offsets [0, 16, 128, 512] from best height (matching Scala node).
    /// Falls back to V1 with genesis ID if we have no headers yet.
    fn build_v2_sync_info(&self) -> SyncInfo {
        // Use V2 sync headers (at offsets [0, 16, 128, 512] from best height)
        let v2_headers = self.v2_sync_headers.read();

        if !v2_headers.is_empty() {
            // Serialize headers for V2 format
            // Headers are already in newest-first order (best header at index 0)
            let serialized_headers: Vec<Vec<u8>> = v2_headers
                .iter()
                .filter_map(|h| {
                    match h.scorex_serialize_bytes() {
                        Ok(bytes) => Some(bytes),
                        Err(e) => {
                            warn!(height = h.height, error = ?e, "Failed to serialize header for V2 SyncInfo");
                            None
                        }
                    }
                })
                .collect();

            if !serialized_headers.is_empty() {
                let heights: Vec<u32> = v2_headers.iter().map(|h| h.height).collect();
                debug!(
                    header_count = serialized_headers.len(),
                    heights = ?heights,
                    "Built V2 SyncInfo with headers at offset heights"
                );
                return SyncInfo::v2(serialized_headers);
            }
        }

        // No headers yet - send V1 with genesis ID
        // The genesis block at height=0 has ID = 0000...0000
        let genesis_id = vec![0u8; 32];
        debug!("No headers to serialize - sending V1 SyncInfo with genesis ID");
        SyncInfo::v1(vec![genesis_id])
    }

    /// Handle peer disconnection.
    fn on_peer_disconnected(&self, peer: &PeerId) {
        info!(peer = %peer, "Peer disconnected");
        self.sync_peers.write().remove(peer);
        self.synchronizer.remove_peer(peer);
    }

    /// Handle received SyncInfo.
    async fn on_sync_info(&self, peer: PeerId, info: SyncInfo) -> SyncResult<()> {
        // Rate limit: Check if we received SyncInfo from this peer too recently
        // This prevents resource exhaustion from spammy peers (matches Scala's PerPeerSyncLockTime)
        let (can_process, old_status) = {
            let peers = self.sync_peers.read();
            if let Some(state) = peers.get(&peer) {
                (state.can_process_sync(), state.chain_status)
            } else {
                // Unknown peer, allow processing
                (true, PeerChainStatus::Unknown)
            }
        };

        if !can_process {
            debug!(peer = %peer, "Ignoring spammy SyncInfo (too frequent)");
            return Ok(());
        }

        // Get header IDs and peer height from either V1 or V2 format
        let (header_ids, peer_height): (Vec<Vec<u8>>, Option<u32>) = if info.is_v2() {
            // V2 format contains full serialized headers - parse them to get height
            debug!(peer = %peer, headers = info.last_headers.len(), "Received SyncInfo V2");
            let mut ids = Vec::new();
            let mut max_height: Option<u32> = None;

            for header_bytes in &info.last_headers {
                match Header::scorex_parse_bytes(header_bytes) {
                    Ok(header) => {
                        ids.push(header.id.0.as_ref().to_vec());
                        max_height =
                            Some(max_height.map_or(header.height, |h| h.max(header.height)));
                    }
                    Err(e) => {
                        warn!(error = ?e, "Failed to parse header from SyncInfo V2");
                    }
                }
            }
            (ids, max_height)
        } else {
            debug!(peer = %peer, header_ids = info.last_header_ids.len(), "Received SyncInfo V1");
            // V1 format only has header IDs, no height info
            // We'll need to estimate or get it from other sources
            (info.last_header_ids.clone(), None)
        };

        // Determine peer height - use parsed height from V2, or keep existing for V1
        // For V1, we can't know the exact height from just IDs, so we preserve
        // any previously known height (which gets updated as we receive headers)
        let existing_height = self
            .sync_peers
            .read()
            .get(&peer)
            .map(|s| s.height)
            .unwrap_or(0);

        let estimated_height = peer_height.unwrap_or(existing_height);

        // Determine the new chain status by comparing our height with peer's
        let our_height = self
            .header_chain
            .read()
            .headers
            .back()
            .map(|h| h.height)
            .unwrap_or(0);

        let new_status = if estimated_height == 0 && our_height == 0 {
            PeerChainStatus::Unknown
        } else if estimated_height > our_height {
            PeerChainStatus::Older // We are older (behind)
        } else if estimated_height < our_height {
            PeerChainStatus::Younger // Peer is younger (behind us)
        } else {
            // Same height - check if we share headers
            let our_headers = self.our_best_headers.read();
            let shared = header_ids.iter().any(|h| our_headers.contains(h));
            if shared {
                PeerChainStatus::Equal
            } else {
                PeerChainStatus::Fork
            }
        };

        // Determine if we should send SyncInfo response
        let should_send_sync = {
            let peers = self.sync_peers.read();
            if let Some(state) = peers.get(&peer) {
                state.should_send_sync_response(old_status, new_status)
            } else {
                // New peer, always respond
                true
            }
        };

        // Update peer state
        if let Some(state) = self.sync_peers.write().get_mut(&peer) {
            state.best_headers = header_ids.clone();
            state.last_activity = std::time::Instant::now();
            state.last_sync_received = std::time::Instant::now();
            state.height = estimated_height;
            state.chain_status = new_status;
        }

        // Update synchronizer with peer height
        self.synchronizer
            .set_peer_height(peer.clone(), estimated_height);

        // Note: We don't directly request headers from SyncInfo because they are
        // chain samples (not consecutive). The peer should respond to our SyncInfo
        // with an Inv message containing the consecutive headers we need.
        //
        // If the peer has headers we don't have, they will send us an Inv.
        // We just wait for that Inv and request the headers from there.

        let unknown_count = {
            let our_headers = self.our_best_headers.read();
            header_ids
                .iter()
                .filter(|h| !our_headers.contains(h))
                .count()
        };

        if unknown_count > 0 {
            debug!(
                unknown_count,
                "Peer has headers we don't have, waiting for Inv"
            );
        }

        // Only respond with SyncInfo if necessary (matches Scala node behavior)
        // This prevents ping-pong where both nodes keep responding to each other
        if should_send_sync {
            let sync_info = self.build_v2_sync_info();

            if sync_info.is_v2() {
                debug!(
                    peer = %peer,
                    old_status = ?old_status,
                    new_status = ?new_status,
                    "Responding to SyncInfo with V2 ({} headers)",
                    sync_info.last_headers.len()
                );
            } else {
                debug!(
                    peer = %peer,
                    old_status = ?old_status,
                    new_status = ?new_status,
                    "Responding to SyncInfo with V1 (no headers yet)"
                );
            }

            // Update last_sync_sent
            if let Some(state) = self.sync_peers.write().get_mut(&peer) {
                state.last_sync_sent = std::time::Instant::now();
            }

            self.send_to_peer(&peer, Message::SyncInfo(sync_info))
                .await?;
        } else {
            debug!(
                peer = %peer,
                old_status = ?old_status,
                new_status = ?new_status,
                "Not responding to SyncInfo (no change needed)"
            );
        }

        Ok(())
    }

    /// Handle inventory announcement.
    async fn on_inv(&self, peer: PeerId, inv: InvData) -> SyncResult<()> {
        // Request modifiers in batches
        // Scala node uses desiredInvObjects = 400, so we can request up to 400 at once
        const MAX_REQUEST_SIZE: usize = 400;

        // Identify the modifier type
        let is_header_inv = inv.type_id == ModifierType::Header.to_byte();
        let is_transaction_inv = inv.type_id == ModifierType::Transaction.to_byte();
        let is_block_transactions_inv = inv.type_id == ModifierType::BlockTransactions.to_byte();

        // Ignore BlockTransactions Inv - we request blocks ourselves based on stored headers
        // This prevents requesting blocks before headers are synced, which causes failures
        if is_block_transactions_inv {
            debug!(
                peer = %peer,
                count = inv.ids.len(),
                "Ignoring BlockTransactions Inv - blocks requested via block_sync_interval"
            );
            return Ok(());
        }

        info!(
            peer = %peer,
            type_id = inv.type_id,
            count = inv.ids.len(),
            "Received Inv, requesting modifiers"
        );

        if is_transaction_inv {
            info!(
                peer = %peer,
                count = inv.ids.len(),
                "Received transaction Inv announcement"
            );
        }

        if !inv.ids.is_empty() {
            // Log first few IDs for debugging
            for (i, id) in inv.ids.iter().take(3).enumerate() {
                debug!(index = i, id = %hex::encode(id), "Inv item");
            }

            // Filter out already-known modifiers (headers or transactions)
            let original_count = inv.ids.len();
            let (filtered_ids, removed_count): (Vec<Vec<u8>>, usize) = if is_header_inv {
                // Filter out already-stored headers (read lock since contains() doesn't update LRU)
                let stored = self.stored_headers.read();
                let mut filtered = Vec::new();
                let mut removed = 0;
                for id in inv.ids {
                    if stored.contains(&id) {
                        removed += 1;
                    } else {
                        filtered.push(id);
                    }
                }
                (filtered, removed)
            } else if is_transaction_inv {
                // Filter out already-known transactions using LRU cache
                let known = self.known_transactions.lock();
                let known_count = known.len();
                let mut filtered = Vec::new();
                let mut removed = 0;
                for id in inv.ids {
                    if known.contains(&id) {
                        removed += 1;
                    } else {
                        filtered.push(id);
                    }
                }
                drop(known); // Release lock before logging
                debug!(
                    original = original_count,
                    filtered = filtered.len(),
                    removed = removed,
                    known_tx_cache_size = known_count,
                    "Transaction Inv filtering result"
                );
                (filtered, removed)
            } else {
                // For other types, don't filter
                (inv.ids, 0)
            };

            // Only log summary at info level if we actually filtered something
            if removed_count > 0 {
                debug!(
                    original = original_count,
                    filtered = filtered_ids.len(),
                    removed = removed_count,
                    type_id = inv.type_id,
                    "Filtered Inv IDs (already known)"
                );
            }

            let mut ids_iter = filtered_ids.into_iter();

            // Take the first batch to request now
            let ids_to_request: Vec<Vec<u8>> = ids_iter.by_ref().take(MAX_REQUEST_SIZE).collect();

            // Store remaining IDs for later if this is a header Inv
            if is_header_inv {
                let remaining: Vec<Vec<u8>> = ids_iter.collect();
                if !remaining.is_empty() {
                    info!(
                        remaining = remaining.len(),
                        "Storing remaining header IDs from Inv for later"
                    );
                    let mut pending = self.pending_header_inv.write();
                    // Evict oldest entries if adding these would exceed the limit
                    let space_needed = remaining.len();
                    while pending.len() + space_needed > MAX_PENDING_HEADER_INV
                        && !pending.is_empty()
                    {
                        pending.pop_front();
                    }
                    pending.extend(remaining);
                }
            }

            // Only send request if we have IDs to request
            // Sending empty RequestModifier causes "empty inv list" error on Scala node
            if !ids_to_request.is_empty() {
                // Track these as in-flight before sending request
                if is_header_inv {
                    let now = std::time::Instant::now();
                    let mut in_flight = self.in_flight_headers.write();
                    for id in &ids_to_request {
                        in_flight.insert(id.clone(), now);
                    }
                    drop(in_flight);
                }

                let request = ModifierRequest {
                    type_id: inv.type_id,
                    ids: ids_to_request,
                };

                info!(peer = %peer, type_id = request.type_id, count = request.ids.len(), "Sending RequestModifier");

                // Update last request time for rate limiting
                if let Some(state) = self.sync_peers.write().get_mut(&peer) {
                    state.last_request_sent = std::time::Instant::now();
                }

                self.send_to_peer(&peer, Message::RequestModifier(request))
                    .await?;
            } else {
                debug!(peer = %peer, "All IDs in Inv already stored, skipping request");
            }
        }

        Ok(())
    }

    /// Handle received modifier (header or block).
    async fn on_modifier(
        &self,
        peer: PeerId,
        type_id: u8,
        id: Vec<u8>,
        data: Vec<u8>,
    ) -> SyncResult<()> {
        info!(
            peer = %peer,
            type_id,
            id = hex::encode(&id),
            size = data.len(),
            "Received modifier"
        );

        // Update peer activity
        if let Some(state) = self.sync_peers.write().get_mut(&peer) {
            state.last_activity = std::time::Instant::now();
            state.in_flight = state.in_flight.saturating_sub(1);
        }

        match ModifierType::from_byte(type_id) {
            Some(ModifierType::Transaction) => {
                info!(
                    tx_id = hex::encode(&id),
                    size = data.len(),
                    "Dispatching to on_transaction_received"
                );
                self.on_transaction_received(peer, id, data).await?;
            }
            Some(ModifierType::Header) => {
                self.on_header_received(&peer, id, data).await?;
            }
            Some(ModifierType::BlockTransactions) => {
                self.on_block_received(&peer, id, data).await?;
            }
            Some(ModifierType::Extension) => {
                self.on_extension_received(&peer, id, data).await?;
            }
            Some(ModifierType::ADProofs) => {
                // Handle AD proofs
                debug!(id = hex::encode(&id), "Received AD proofs");
            }
            _ => {
                warn!(type_id, "Unknown modifier type");
            }
        }

        Ok(())
    }

    /// Handle received header.
    async fn on_header_received(
        &self,
        peer: &PeerId,
        id: Vec<u8>,
        data: Vec<u8>,
    ) -> SyncResult<()> {
        info!(
            id = hex::encode(&id),
            data_len = data.len(),
            "on_header_received called"
        );

        // Remove from in-flight tracking
        self.in_flight_headers.write().remove(&id);

        // Compute the correct header ID from the raw bytes.
        // This is necessary because sigma-rust's Header::scorex_parse_bytes() computes
        // the ID from reserialized bytes, which can differ from original bytes due to
        // BigInt serialization issues in Autolykos v1 headers (leading 0xFF sign bytes).
        let correct_id = compute_header_id(&data);

        // Parse header using sigma-rust
        let mut header = match Header::scorex_parse_bytes(&data) {
            Ok(h) => h,
            Err(e) => {
                warn!("Failed to parse header: {}", e);
                return Err(SyncError::InvalidData(format!(
                    "Failed to parse header: {}",
                    e
                )));
            }
        };

        // Fix the header ID if it differs from the correct one
        let sigma_id = header.id.0.as_ref().to_vec();
        if sigma_id != correct_id {
            debug!(
                height = header.height,
                sigma_id = hex::encode(&sigma_id),
                correct_id = hex::encode(&correct_id),
                "Fixing header ID (sigma-rust BigInt serialization issue)"
            );
            // Create a new BlockId with the correct hash
            let correct_digest: [u8; 32] = correct_id.clone().try_into().expect("hash is 32 bytes");
            header.id = BlockId(Digest32::from(correct_digest));
        }

        info!(
            height = header.height,
            id = hex::encode(&correct_id),
            "Parsed header"
        );

        // Note: We intentionally do NOT update peer_height from received headers.
        // With V1 SyncInfo, we don't know the peer's actual height - they could have
        // millions more headers. Setting peer_height to received header height would
        // make us think we're caught up when we're not. Instead, we keep sending
        // SyncInfo periodically to discover more headers.

        let parent_id = header.parent_id.0.as_ref().to_vec();

        // Check header status in a single lock acquisition to reduce contention
        let (already_stored, parent_available, stored_count) = {
            let stored = self.stored_headers.read();
            (
                stored.contains(&correct_id),
                stored.contains(&parent_id),
                stored.len(),
            )
        };

        if already_stored {
            // Log at info level for first few headers to diagnose restart behavior
            if header.height <= 5 || header.height % 100 == 0 {
                info!(
                    height = header.height,
                    "Header already in stored_headers, skipping"
                );
            }
            return Ok(());
        }

        // Log for first few headers to diagnose, or for any header around the sync point
        if header.height <= 5 || (header.height >= 3130 && header.height <= 3140) {
            info!(
                height = header.height,
                parent = hex::encode(&parent_id),
                parent_available,
                stored_count,
                "Processing header - checking parent"
            );
        }

        if parent_available {
            // Parent exists, we can store this header
            info!(height = header.height, "Storing header - parent available");
            self.store_header_and_descendants(header, data).await?;
        } else {
            // Parent not available yet, buffer this header
            // Use correct_id for pending tracking
            let pending_count = {
                let mut pending = self.pending_headers.write();

                // Insert with automatic eviction of oldest (lowest height) if at capacity
                let evicted = pending.insert(
                    correct_id.clone(),
                    PendingHeader {
                        header: header.clone(),
                        raw_bytes: data,
                        parent_id: parent_id.clone(),
                    },
                    MAX_PENDING_HEADERS,
                );

                if let Some(evicted_height) = evicted {
                    debug!(
                        evicted_height,
                        new_height = header.height,
                        "Evicted oldest pending header to make room"
                    );
                }

                pending.len()
            };

            // Log periodically to show buffering is happening
            if pending_count % 50 == 1 || header.height <= 5 {
                info!(
                    height = header.height,
                    parent = hex::encode(&parent_id),
                    pending_count,
                    stored_count = self.stored_headers.read().len(),
                    "Buffering header - parent not available"
                );
            }
        }

        Ok(())
    }

    /// Buffer a header for batched storage.
    /// Headers are accumulated and flushed in batches for efficiency.
    fn buffer_header_for_storage(&self, header: &Header, raw_bytes: &[u8]) {
        let mut pending = self.pending_header_writes.write();
        let mut since = self.pending_header_writes_since.write();

        // Set timestamp if this is the first header in the batch
        if pending.is_empty() {
            *since = Some(std::time::Instant::now());
        }

        pending.push((header.clone(), raw_bytes.to_vec()));
    }

    /// Check if we should flush buffered headers (size or timeout).
    fn should_flush_headers(&self) -> bool {
        let pending = self.pending_header_writes.read();
        if pending.is_empty() {
            return false;
        }

        // Flush if we've reached batch size
        if pending.len() >= HEADER_BATCH_SIZE {
            return true;
        }

        // Flush if timeout has elapsed
        let since = self.pending_header_writes_since.read();
        if let Some(start) = *since {
            if start.elapsed().as_millis() as u64 >= HEADER_BATCH_TIMEOUT_MS {
                return true;
            }
        }

        false
    }

    /// Flush buffered headers to storage in a single batch.
    /// Returns the list of header IDs that were stored.
    async fn flush_pending_headers_to_storage(&self) -> SyncResult<Vec<Vec<u8>>> {
        // Take the buffered headers
        let headers: Vec<(Header, Vec<u8>)> = {
            let mut pending = self.pending_header_writes.write();
            let mut since = self.pending_header_writes_since.write();
            *since = None;
            std::mem::take(&mut *pending)
        };

        if headers.is_empty() {
            return Ok(Vec::new());
        }

        let count = headers.len();
        let first_height = headers[0].0.height;
        let last_height = headers[count - 1].0.height;

        info!(
            first_height,
            last_height, count, "Flushing {} buffered headers to storage", count
        );

        // Collect header IDs before sending (we need them for tracking)
        let header_ids: Vec<Vec<u8>> = headers
            .iter()
            .map(|(h, _)| h.id.0.as_ref().to_vec())
            .collect();

        // Create response channel
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();

        // Send batched store command
        let cmd = SyncCommand::StoreHeadersBatch {
            headers,
            response_tx,
        };
        self.command_tx.send(cmd).await.map_err(|e| {
            SyncError::Internal(format!("Failed to send batch store command: {}", e))
        })?;

        // Wait for storage confirmation
        match response_rx.await {
            Ok(Ok(())) => {
                info!(
                    first_height,
                    last_height, count, "Batch of {} headers stored successfully", count
                );
            }
            Ok(Err(e)) => {
                return Err(SyncError::Internal(format!(
                    "Failed to store header batch: {}",
                    e
                )));
            }
            Err(_) => {
                return Err(SyncError::Internal(
                    "Batch storage response channel closed".to_string(),
                ));
            }
        }

        Ok(header_ids)
    }

    /// Store a header and any pending descendants that can now be applied.
    /// Headers are buffered and written in batches for efficiency.
    async fn store_header_and_descendants(
        &self,
        header: Header,
        raw_bytes: Vec<u8>,
    ) -> SyncResult<()> {
        let header_id = header.id.0.as_ref().to_vec();
        let height = header.height;

        debug!(
            id = hex::encode(&header_id),
            height, "Buffering header for storage"
        );

        // Buffer the header for batched storage
        self.buffer_header_for_storage(&header, &raw_bytes);

        // Optimistically mark as stored so descendants can be processed
        // This is safe because if the batch write fails, we'll error out anyway
        self.stored_headers.write().put(header_id.clone(), ());

        // Compute the BlockTransactions ID (derived from header ID + transactions root)
        // This is NOT the same as the header ID!
        // Formula: blake2b256(modifierTypeId ++ headerId ++ transactionsRoot)
        let transactions_id = compute_block_section_id(
            ModifierType::BlockTransactions.to_byte(),
            &header_id,
            header.transaction_root.0.as_ref(),
        );

        debug!(
            height = header.height,
            header_id = hex::encode(&header_id),
            transactions_id = hex::encode(&transactions_id),
            "Computed BlockTransactions ID for block download"
        );

        // Add to our header chain tracking (with memory limits enforced)
        {
            let mut chain = self.header_chain.write();
            chain.add_header(header.clone(), transactions_id, header_id.clone());
        }

        // Update V2 sync headers - the new header becomes the best (offset 0)
        // This ensures our SyncInfo always includes our current best header
        {
            let mut v2_headers = self.v2_sync_headers.write();
            if v2_headers.is_empty() {
                // First header - just add it
                v2_headers.push(header);
            } else {
                // Replace the best header (index 0) with the new one
                v2_headers[0] = header;
            }
        }

        // Update synchronizer height
        self.synchronizer.set_height(height);

        // Check if any pending headers can now be applied
        self.try_apply_pending_headers(&header_id).await?;

        // Check if we should flush buffered headers to storage
        if self.should_flush_headers() {
            self.flush_pending_headers_to_storage().await?;
        }

        // Note: We don't request more headers here anymore.
        // The tick handler will request more when appropriate.
        // This avoids spamming the peer with too many requests.

        Ok(())
    }

    /// Find a peer that we can send a modifier request to (respecting rate limits).
    fn find_peer_for_request(&self) -> Option<PeerId> {
        let peers = self.sync_peers.read();
        peers
            .iter()
            .find(|(_, state)| state.can_send_request())
            .map(|(id, _)| id.clone())
    }

    /// Find a peer that we can send SyncInfo to (respecting rate limits).
    fn find_peer_for_sync(&self) -> Option<PeerId> {
        let peers = self.sync_peers.read();
        peers
            .iter()
            .find(|(_, state)| state.can_send_sync())
            .map(|(id, _)| id.clone())
    }

    /// Try to apply pending headers that were waiting for the given parent.
    async fn try_apply_pending_headers(&self, parent_id: &[u8]) -> SyncResult<()> {
        // Find headers waiting for this parent using the efficient parent index (O(1) lookup)
        let waiting: Vec<_> = {
            let pending = self.pending_headers.read();
            let waiting = pending
                .get_by_parent(parent_id)
                .into_iter()
                .map(|(id, ph)| (id, ph.header.clone(), ph.raw_bytes.clone()))
                .collect::<Vec<_>>();

            if waiting.is_empty() && !pending.is_empty() {
                // Log for debugging - show first few pending parents
                let sample_parents = pending.sample(3);
                info!(
                    parent_id = hex::encode(parent_id),
                    pending_count = pending.len(),
                    ?sample_parents,
                    "No pending headers waiting for this parent"
                );
            }

            waiting
        };

        if !waiting.is_empty() {
            info!(
                parent_id = hex::encode(parent_id),
                waiting_count = waiting.len(),
                "Found pending headers to apply"
            );
        }

        // Apply each waiting header
        for (id, header, raw_bytes) in waiting {
            // Remove from pending
            self.pending_headers.write().remove(&id);

            // Recursively store (this will check for its own descendants)
            Box::pin(self.store_header_and_descendants(header, raw_bytes)).await?;
        }

        Ok(())
    }

    /// Handle received block.
    async fn on_block_received(
        &self,
        _peer: &PeerId,
        id: Vec<u8>,
        data: Vec<u8>,
    ) -> SyncResult<()> {
        use ergo_consensus::block::BlockTransactions;

        info!(
            id = hex::encode(&id),
            size = data.len(),
            "Processing block transactions"
        );

        // Parse BlockTransactions
        let block_txs = match BlockTransactions::parse(&data) {
            Ok(txs) => {
                info!(
                    header_id = hex::encode(txs.header_id.0.as_ref()),
                    tx_count = txs.txs.len(),
                    block_version = txs.block_version,
                    "Parsed BlockTransactions"
                );
                txs
            }
            Err(e) => {
                warn!(id = hex::encode(&id), error = ?e, "Failed to parse BlockTransactions");
                self.downloader.fail(&id, &PeerId::from_bytes(vec![]));
                return Err(SyncError::InvalidData(format!(
                    "Failed to parse BlockTransactions: {:?}",
                    e
                )));
            }
        };

        // The `id` we received is the transactionsId (derived ID), not the header ID.
        // We need to find the corresponding header. We can either:
        // 1. Use the header_id from the parsed BlockTransactions
        // 2. Look up our tx_id_to_header_id mapping
        // Using the parsed header_id is more reliable since it comes from the data itself.
        let header_id_from_block = block_txs.header_id.0.as_ref().to_vec();

        // Find the corresponding header for validation using O(1) HashMap lookup
        let header = {
            let chain = self.header_chain.read();
            chain.get_header(&header_id_from_block).cloned()
        };

        // Validate block transactions against header if we have it
        if let Some(ref header) = header {
            match block_txs.validate_against_header(header) {
                Ok(()) => {
                    info!(
                        height = header.height,
                        "Block transactions validated successfully"
                    );
                }
                Err(e) => {
                    warn!(
                        id = hex::encode(&id),
                        error = %e,
                        "Block transactions validation failed"
                    );
                    self.downloader.fail(&id, &PeerId::from_bytes(vec![]));
                    return Err(SyncError::InvalidData(format!(
                        "Block validation failed: {}",
                        e
                    )));
                }
            }
        } else {
            // Header not in local cache - this can happen during fast sync when
            // headers are loaded from database via block_sync_interval but may not
            // be in the in-memory cache. This is benign - the state manager will
            // validate the block against the header from the database.
            debug!(
                id = hex::encode(&id),
                "Block header not in cache, skipping local validation"
            );
        }

        // Mark as downloaded
        // Mark as downloaded using the transactionsId (which is what we requested)
        self.downloader.complete(&id);
        self.synchronizer.block_received(&id);

        // Send command to apply the block
        // Use the header_id from the BlockTransactions, not the transactionsId
        let cmd = SyncCommand::ApplyBlock {
            block_id: header_id_from_block.clone(),
            block_data: data,
        };
        self.command_tx
            .send(cmd)
            .await
            .map_err(|e| SyncError::Internal(format!("Failed to send command: {}", e)))?;

        // Clean up the tx_id_to_header_id mapping
        {
            let mut chain = self.header_chain.write();
            chain.tx_id_to_header_id.remove(&id);
        }

        Ok(())
    }

    /// Handle received extension.
    ///
    /// Extensions contain protocol parameters at epoch boundaries (every 1024 blocks).
    /// We need to store them so parameters can be loaded on node restart.
    async fn on_extension_received(
        &self,
        _peer: &PeerId,
        id: Vec<u8>,
        data: Vec<u8>,
    ) -> SyncResult<()> {
        use ergo_consensus::block::Extension;

        debug!(
            id = hex::encode(&id),
            size = data.len(),
            "Processing extension"
        );

        // Parse the extension to extract the header ID
        let extension = match Extension::parse(&data) {
            Ok(ext) => ext,
            Err(e) => {
                warn!(id = hex::encode(&id), error = ?e, "Failed to parse Extension");
                // Mark as completed anyway to avoid re-requesting
                self.downloader.complete(&id);
                return Ok(());
            }
        };

        let header_id = extension.header_id.0.as_ref().to_vec();

        debug!(
            extension_id = hex::encode(&id),
            header_id = hex::encode(&header_id),
            fields = extension.fields.len(),
            "Parsed Extension"
        );

        // Mark as downloaded
        self.downloader.complete(&id);

        // Send command to store the extension
        let cmd = SyncCommand::StoreExtension {
            header_id,
            extension_data: data,
        };
        self.command_tx
            .send(cmd)
            .await
            .map_err(|e| SyncError::Internal(format!("Failed to send command: {}", e)))?;

        Ok(())
    }

    /// Handle received transaction.
    async fn on_transaction_received(
        &self,
        peer: PeerId,
        id: Vec<u8>,
        data: Vec<u8>,
    ) -> SyncResult<()> {
        debug!(
            peer = %peer,
            tx_id = hex::encode(&id),
            size = data.len(),
            "Received unconfirmed transaction"
        );

        // Mark as known to avoid re-requesting.
        // LRU cache automatically evicts oldest entries when capacity is exceeded.
        self.known_transactions.lock().put(id.clone(), ());

        // Forward to the node for mempool insertion and validation.
        // The node will validate the transaction (check inputs exist, verify scripts,
        // check conservation rules) before adding to mempool.
        let cmd = SyncCommand::AddTransaction {
            tx_id: id,
            tx_data: data,
            from_peer: peer,
        };

        self.command_tx.send(cmd).await.map_err(|e| {
            SyncError::Internal(format!("Failed to send AddTransaction command: {}", e))
        })?;

        Ok(())
    }

    /// Handle successful block application.
    fn on_block_applied(&self, block_id: &[u8], height: u32) {
        debug!(
            id = hex::encode(block_id),
            height, "Block applied successfully"
        );

        // Update our height
        self.synchronizer.set_height(height);

        // Remove from header chain using O(1) lookup - note: block_id here is the header_id
        let mut chain = self.header_chain.write();

        // Use the reverse index for O(1) lookup of tx_id from header_id
        let tx_id_to_remove = chain.get_tx_id(block_id).cloned();

        if let Some(ref tx_id) = tx_id_to_remove {
            // Also remove from downloader's completed set to free memory
            // and allow re-downloading if ever needed (e.g., after rollback)
            self.downloader.uncomplete(tx_id);
        }

        // Remove header and all its mappings using the new O(1) method
        chain.remove_header(block_id);

        // Also remove the header_id itself from completed (in case it was tracked that way)
        self.downloader.uncomplete(block_id);

        // Log memory stats periodically (every 1000 blocks)
        if height % 1000 == 0 {
            let (headers, pending, mappings) = chain.memory_stats();
            info!(
                height,
                cached_headers = headers,
                pending_blocks = pending,
                id_mappings = mappings,
                "Sync memory stats"
            );
        }
    }

    /// Handle failed block application.
    fn on_block_failed(&self, block_id: &[u8], error: &str) {
        warn!(
            id = hex::encode(block_id),
            error, "Block application failed"
        );

        // Remove from completed set so it can be re-downloaded if needed
        // This handles the case where validation fails due to transient issues
        // (e.g., missing UTXO data that becomes available after other blocks are applied)
        self.downloader.uncomplete(block_id);

        // Use O(1) reverse index lookup to find the transactionsId
        let chain = self.header_chain.read();
        if let Some(tx_id) = chain.get_tx_id(block_id) {
            self.downloader.uncomplete(tx_id);
        }
    }

    /// Handle request to download blocks for specific headers.
    /// The node sends this with headers for blocks it needs (starting from UTXO height + 1).
    async fn on_request_blocks(&self, headers: Vec<Header>) -> SyncResult<()> {
        if headers.is_empty() {
            return Ok(());
        }

        // Collect download tasks, checking for duplicates
        let mut tasks = Vec::new();
        let stats = self.downloader.stats();

        // Collect all header info first, then batch update under single lock
        let header_infos: Vec<_> = headers
            .iter()
            .map(|header| {
                let header_id = header.id.0.as_ref().to_vec();
                let transactions_id = compute_block_section_id(
                    ModifierType::BlockTransactions.to_byte(),
                    &header_id,
                    header.transaction_root.0.as_ref(),
                );
                // Compute extension ID from header ID and extension root
                let extension_id = compute_block_section_id(
                    ModifierType::Extension.to_byte(),
                    &header_id,
                    header.extension_root.0.as_ref(),
                );
                (header.clone(), transactions_id, extension_id, header_id)
            })
            .collect();

        // Batch update header chain under single lock
        {
            let mut chain = self.header_chain.write();
            for (header, transactions_id, _extension_id, header_id) in &header_infos {
                if !chain.tx_id_to_header_id.contains_key(transactions_id) {
                    // Use track_header which populates lookup indexes but NOT missing_blocks
                    // since we queue downloads directly via the downloader below
                    chain.track_header(header.clone(), transactions_id.clone(), header_id.clone());
                }
            }
            // NOTE: Do NOT call enforce_limits() here!
            // These headers are for blocks we're about to download - evicting them
            // immediately would cause "header not in cache" warnings when blocks arrive.
            // Headers will be removed via remove_header() when blocks are applied.
        }

        // Create download tasks for both block transactions and extensions
        for (_, transactions_id, extension_id, _) in header_infos {
            // Request block transactions
            let tx_task =
                DownloadTask::new(transactions_id, ModifierType::BlockTransactions.to_byte());
            tasks.push(tx_task);

            // Request extension (contains protocol parameters at epoch boundaries)
            let ext_task = DownloadTask::new(extension_id, ModifierType::Extension.to_byte());
            tasks.push(ext_task);
        }

        // Queue all tasks at once
        // Use queue() instead of queue_force() to avoid re-queuing blocks that are
        // already completed but waiting to be applied. This is called every second,
        // so we don't want to repeatedly re-request the same blocks.
        if !tasks.is_empty() {
            let task_count = tasks.len();
            self.downloader.queue(tasks);

            // Log current state
            let new_stats = self.downloader.stats();
            let added = new_stats.pending.saturating_sub(stats.pending);

            // Log differently based on whether tasks were actually added
            if added > 0 {
                info!(
                    requested = headers.len(),
                    first_height = headers.first().map(|h| h.height),
                    last_height = headers.last().map(|h| h.height),
                    added,
                    pending = new_stats.pending,
                    in_flight = new_stats.in_flight,
                    completed = new_stats.completed,
                    "Queued block downloads"
                );
            } else {
                // All tasks were skipped - likely already completed or in-flight
                debug!(
                    requested = headers.len(),
                    first_height = headers.first().map(|h| h.height),
                    last_height = headers.last().map(|h| h.height),
                    task_count,
                    pending = new_stats.pending,
                    in_flight = new_stats.in_flight,
                    completed = new_stats.completed,
                    "Block download request - all tasks skipped (already completed/pending/in-flight)"
                );
            }
        }

        // Dispatch the downloads immediately
        self.dispatch_downloads().await?;

        Ok(())
    }

    /// Periodic tick handler.
    async fn on_tick(&self) -> SyncResult<()> {
        // Flush any buffered headers that have timed out
        if self.should_flush_headers() {
            self.flush_pending_headers_to_storage().await?;
        }

        // Check for block download timeouts
        let timed_out = self.downloader.check_timeouts();
        for (id, peer) in timed_out {
            warn!(id = hex::encode(&id), peer = %peer, "Block download timed out");
            self.downloader.fail(&id, &peer);
        }

        // Check for header request timeouts and re-queue them
        let header_timeout = self.header_timeout;
        let timed_out_headers: Vec<Vec<u8>> = {
            let in_flight = self.in_flight_headers.read();
            in_flight
                .iter()
                .filter(|(_, requested_at)| requested_at.elapsed() > header_timeout)
                .map(|(id, _)| id.clone())
                .collect()
        };

        if !timed_out_headers.is_empty() {
            warn!(
                count = timed_out_headers.len(),
                "Header requests timed out, re-queuing for retry"
            );
            // Remove from in-flight and add back to pending queue for retry
            let mut in_flight = self.in_flight_headers.write();
            let mut pending = self.pending_header_inv.write();
            for id in timed_out_headers {
                in_flight.remove(&id);
                // Evict from back if at capacity before pushing to front
                if pending.len() >= MAX_PENDING_HEADER_INV {
                    pending.pop_back();
                }
                // Re-queue at front for faster retry
                pending.push_front(id);
            }
        }

        // Queue block downloads if we have headers but no pending block downloads
        self.queue_block_downloads();

        // Dispatch pending downloads
        self.dispatch_downloads().await?;

        // Check if we need to request more headers
        self.request_more_headers_if_needed().await?;

        // Log progress
        let our_height = self.synchronizer.our_height();
        let peer_height = self.synchronizer.best_peer_height().unwrap_or(0);
        let stats = self.downloader.stats();
        let chain = self.header_chain.read();
        let missing_blocks = chain.missing_blocks.len();
        let our_header_height = chain.headers.back().map(|h| h.height).unwrap_or(0);
        drop(chain);

        // Use best available network height estimate
        let estimated_network_height = if peer_height > 0 {
            peer_height
        } else {
            our_header_height
        };

        // Check if we have peers with headers (even if we don't know their height)
        let has_peers_with_headers = self
            .sync_peers
            .read()
            .values()
            .any(|s| !s.best_headers.is_empty());

        if our_height < estimated_network_height
            || has_peers_with_headers
            || stats.pending > 0
            || stats.in_flight > 0
            || missing_blocks > 0
        {
            // Indicate sync mode in log
            let sync_mode =
                if estimated_network_height > 0 && our_height + 1000 < estimated_network_height {
                    "headers"
                } else {
                    "blocks"
                };
            info!(
                our_height,
                peer_height,
                header_height = our_header_height,
                sync_mode,
                pending = stats.pending,
                in_flight = stats.in_flight,
                completed = stats.completed,
                failed = stats.failed,
                "Sync progress"
            );
        }

        Ok(())
    }

    /// Queue block downloads from headers we've received.
    /// During header-first sync, we skip block downloads until we're close to the tip.
    fn queue_block_downloads(&self) {
        let our_height = self.synchronizer.our_height();
        let peer_height = self.synchronizer.best_peer_height().unwrap_or(0);

        // Get our best header height from the chain - this is more reliable than peer_height
        // when peers are using V1 SyncInfo (which doesn't include height)
        let our_header_height = self
            .header_chain
            .read()
            .headers
            .back()
            .map(|h| h.height)
            .unwrap_or(0);

        // Use the best available estimate of network height:
        // - peer_height if known (from V2 SyncInfo)
        // - our_header_height as fallback (we've synced headers to this point)
        let estimated_network_height = if peer_height > 0 {
            peer_height
        } else {
            our_header_height
        };

        // During initial header sync, don't download blocks yet
        // Only start downloading blocks when we're within 1000 headers of the tip
        // This implements header-first sync strategy
        //
        // Skip only if we have a valid network height estimate and are far behind
        if estimated_network_height > 0 && our_height + 1000 < estimated_network_height {
            // Still syncing headers, skip block downloads for now
            return;
        }

        let stats = self.downloader.stats();

        // Don't queue more if we already have enough pending
        // Increased from 16 to match PARALLEL_DOWNLOADS for full parallelism
        if stats.pending + stats.in_flight >= PARALLEL_DOWNLOADS {
            return;
        }

        // Take blocks from missing_blocks queue
        let blocks_to_queue: Vec<Vec<u8>> = {
            let mut chain = self.header_chain.write();
            let count = (16 - stats.pending - stats.in_flight).min(chain.missing_blocks.len());
            chain.missing_blocks.drain(..count).collect()
        };

        if blocks_to_queue.is_empty() {
            return;
        }

        // Create download tasks for BlockTransactions
        let tasks: Vec<DownloadTask> = blocks_to_queue
            .into_iter()
            .map(|id| DownloadTask::new(id, ModifierType::BlockTransactions.to_byte()))
            .collect();

        info!(count = tasks.len(), "Queuing block downloads");
        self.downloader.queue(tasks);
    }

    /// Request more headers if we're behind and not currently waiting for any.
    /// Respects rate limits to avoid being banned by peers.
    async fn request_more_headers_if_needed(&self) -> SyncResult<()> {
        let our_height = self.synchronizer.our_height();
        let peer_height = self.synchronizer.best_peer_height().unwrap_or(0);
        let chain_len = self.header_chain.read().headers.len();
        let pending_inv_count = self.pending_header_inv.read().len();
        let in_flight_count = self.in_flight_headers.read().len();

        let has_peers_with_headers_check = self
            .sync_peers
            .read()
            .values()
            .any(|s| !s.best_headers.is_empty());

        // Tick counter for debugging (static so it persists across calls)
        static REQ_TICK_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let tick_num = REQ_TICK_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        info!(
            tick_num,
            our_height,
            peer_height,
            chain_len,
            pending_inv_count,
            in_flight_count,
            pending_headers = self.pending_headers.read().len(),
            has_peers_with_headers = has_peers_with_headers_check,
            "request_more_headers tick"
        );

        // Don't request more headers if we have enough in flight
        // (timeouts are handled separately in on_tick via header_timeout)
        // Increased from 50 to 300 to allow more concurrent header requests
        if in_flight_count > 300 {
            debug!(
                in_flight_count,
                "Enough headers in flight, waiting for responses"
            );
            return Ok(());
        }

        // If we have pending headers, check if we should clear them
        // (they might be stuck because we received non-consecutive headers)
        // But DON'T clear them if we have stored headers (restart case)
        let pending_count = self.pending_headers.read().len();
        let stored_count = self.stored_headers.read().len();
        if pending_count > 0 && stored_count == 0 {
            // Clear pending headers only if we're truly a fresh node with no stored headers
            // (they were probably from non-consecutive SyncInfo samples)
            if self.header_chain.read().headers.is_empty() {
                // We have pending but no applied headers and no stored headers - clear the pending ones
                warn!(pending_count, "Clearing stuck pending headers (fresh node)");
                self.pending_headers.write().clear();
            }
        }

        // If we have pending Inv IDs, try to request them (respecting rate limits)
        if pending_inv_count > 0 {
            if let Some(peer) = self.find_peer_for_request() {
                const MAX_REQUEST_SIZE: usize = 400;

                // Filter out already-stored headers to avoid requesting duplicates
                let pending_ids: Vec<Vec<u8>> = {
                    let stored = self.stored_headers.read();
                    let mut pending = self.pending_header_inv.write();
                    let mut ids = Vec::new();
                    while ids.len() < MAX_REQUEST_SIZE && !pending.is_empty() {
                        if let Some(id) = pending.pop_front() {
                            if !stored.contains(&id) {
                                ids.push(id);
                            }
                        }
                    }
                    ids
                };

                if !pending_ids.is_empty() {
                    info!(
                        our_height,
                        pending = pending_ids.len(),
                        remaining = self.pending_header_inv.read().len(),
                        "Requesting pending headers from Inv queue (tick)"
                    );

                    // Track as in-flight with timestamp
                    {
                        let now = std::time::Instant::now();
                        let mut in_flight = self.in_flight_headers.write();
                        for id in &pending_ids {
                            in_flight.insert(id.clone(), now);
                        }
                    }

                    // Update last request time
                    if let Some(state) = self.sync_peers.write().get_mut(&peer) {
                        state.last_request_sent = std::time::Instant::now();
                    }

                    let request = ModifierRequest {
                        type_id: ModifierType::Header.to_byte(),
                        ids: pending_ids,
                    };
                    self.send_to_peer(&peer, Message::RequestModifier(request))
                        .await?;
                }
            }
            return Ok(());
        }

        // Periodically send SyncInfo to discover more headers
        // We need to keep sending even when our_height == peer_height because:
        // 1. peer_height is just an estimate based on headers we received
        // 2. The peer may have many more headers we haven't discovered yet
        // 3. With V1 SyncInfo, we don't get the actual peer height
        let has_peers = !self.sync_peers.read().is_empty();

        // Send SyncInfo if we have peers and either:
        // - We're behind the peer (our_height < peer_height)
        // - We have no pending work (to discover more headers)
        // - Every 10 ticks as a heartbeat to check for new headers
        static SYNC_TICK_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sync_tick = SYNC_TICK_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let should_send_sync = has_peers
            && (our_height < peer_height
                || peer_height == 0
                || (pending_inv_count == 0 && in_flight_count == 0 && sync_tick % 5 == 0));

        if should_send_sync {
            // Find a peer that we can send SyncInfo to (respecting rate limits)
            if let Some(peer) = self.find_peer_for_sync() {
                info!(
                    our_height,
                    peer_height, chain_len, "Sending SyncInfo V2 to request more headers"
                );

                // Update last sync time
                if let Some(state) = self.sync_peers.write().get_mut(&peer) {
                    state.last_sync_sent = std::time::Instant::now();
                }

                // Send V2 SyncInfo with serialized headers
                // This ensures peer responds with V2 containing their height
                let sync_info = self.build_v2_sync_info();
                self.send_to_peer(&peer, Message::SyncInfo(sync_info))
                    .await?;
            }
        }

        Ok(())
    }

    /// Dispatch pending downloads to peers.
    /// Distributes blocks across ALL available peers for parallel downloading.
    async fn dispatch_downloads(&self) -> SyncResult<()> {
        // Check if we already have too many requests in flight
        // This prevents request flooding when blocks aren't being received
        let stats = self.downloader.stats();
        if stats.in_flight >= PARALLEL_DOWNLOADS {
            debug!(
                in_flight = stats.in_flight,
                "Too many requests in flight, waiting for responses"
            );
            return Ok(());
        }

        // Get ALL available peers that can receive requests (respecting rate limits)
        let available_peers: Vec<PeerId> = {
            let peers = self.sync_peers.read();
            peers
                .iter()
                .filter(|(_, state)| state.can_send_request())
                .map(|(id, _)| id.clone())
                .collect()
        };

        if available_peers.is_empty() {
            debug!("No peers available for download dispatch (all rate limited)");
            return Ok(());
        }

        // Check for stuck tasks first
        {
            let all_peers: Vec<_> = self.sync_peers.read().keys().cloned().collect();
            let stuck = self.downloader.get_stuck_tasks(&all_peers);
            if !stuck.is_empty() {
                warn!(
                    stuck_count = stuck.len(),
                    peer_count = all_peers.len(),
                    "Some blocks stuck - all peers failed to deliver them"
                );
                // Clear the failed_peers for stuck tasks so they can be retried
                for id in &stuck {
                    self.downloader.clear_failed_peers(id);
                }
            }
        }

        // Distribute tasks across peers: up to 32 blocks per peer
        // Use up to 16 peers for maximum parallelism during initial sync
        // With 16 peers × 32 blocks = 512 blocks in-flight for fast sync
        const MAX_BLOCKS_PER_PEER: usize = 32;
        const MAX_PEERS_TO_USE: usize = 16;

        let mut total_dispatched = 0;
        let peers_to_use = available_peers.len().min(MAX_PEERS_TO_USE);

        // Log dispatch diagnostics periodically
        static DISPATCH_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dispatch_num = DISPATCH_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if dispatch_num % 100 == 0 {
            info!(
                dispatch_num,
                available_peers = available_peers.len(),
                peers_to_use,
                pending = stats.pending,
                in_flight = stats.in_flight,
                "Dispatch diagnostics"
            );
        }

        for peer in available_peers.iter().take(peers_to_use) {
            // Get tasks that can be dispatched to THIS peer
            let tasks = self
                .downloader
                .get_ready_tasks_for_peer(MAX_BLOCKS_PER_PEER, peer);
            if tasks.is_empty() {
                continue;
            }

            // Group tasks by type and batch them
            let mut by_type: std::collections::HashMap<u8, Vec<Vec<u8>>> =
                std::collections::HashMap::new();
            for task in &tasks {
                by_type
                    .entry(task.type_id)
                    .or_default()
                    .push(task.id.clone());
                self.downloader.dispatch(&task.id, peer.clone());
            }

            // Update peer state
            if let Some(state) = self.sync_peers.write().get_mut(peer) {
                state.in_flight += tasks.len() as u32;
                state.last_request_sent = std::time::Instant::now();
            }

            // Send one batched request per type
            for (type_id, ids) in by_type {
                debug!(
                    peer = %peer,
                    type_id,
                    count = ids.len(),
                    "Dispatching block download request"
                );
                let request = ModifierRequest { type_id, ids };
                self.send_to_peer(peer, Message::RequestModifier(request))
                    .await?;
            }

            total_dispatched += tasks.len();
        }

        if total_dispatched > 0 {
            debug!(
                peers = available_peers.len(),
                blocks = total_dispatched,
                "Multi-peer block dispatch complete"
            );
        }

        Ok(())
    }

    /// Send a message to a peer.
    async fn send_to_peer(&self, peer: &PeerId, message: Message) -> SyncResult<()> {
        let cmd = SyncCommand::SendToPeer {
            peer: peer.clone(),
            message,
        };
        self.command_tx
            .send(cmd)
            .await
            .map_err(|e| SyncError::Internal(format!("Failed to send: {}", e)))?;
        Ok(())
    }
}

// Need to import ScorexSerializable for Header parsing
use sigma_ser::ScorexSerializable;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sync_protocol_creation() {
        let (tx, _rx) = mpsc::channel(100);
        let protocol = SyncProtocol::new(SyncConfig::default(), tx);

        assert!(!protocol.synchronizer().is_synced());
    }

    #[test]
    fn test_compute_block_section_id() {
        // Test that compute_block_section_id produces correct derived IDs
        // Formula: blake2b256(typeId ++ headerId ++ sectionDigest)

        let type_id = ModifierType::BlockTransactions.to_byte(); // 102
        let header_id = vec![0x01; 32]; // dummy header ID
        let transactions_root = vec![0x02; 32]; // dummy transactions root

        let result = compute_block_section_id(type_id, &header_id, &transactions_root);

        // Verify the result is 32 bytes (blake2b-256 output)
        assert_eq!(result.len(), 32);

        // Verify it's different from both inputs (it's a hash)
        assert_ne!(result, header_id);
        assert_ne!(result, transactions_root);

        // Verify determinism - same inputs produce same output
        let result2 = compute_block_section_id(type_id, &header_id, &transactions_root);
        assert_eq!(result, result2);

        // Verify different type_id produces different result
        let result_different_type = compute_block_section_id(
            ModifierType::Extension.to_byte(),
            &header_id,
            &transactions_root,
        );
        assert_ne!(result, result_different_type);
    }

    #[test]
    fn test_compute_header_id() {
        // Test that compute_header_id produces blake2b-256 hash
        let data = vec![0x01, 0x02, 0x03, 0x04];

        let result = compute_header_id(&data);

        // Verify the result is 32 bytes
        assert_eq!(result.len(), 32);

        // Verify determinism
        let result2 = compute_header_id(&data);
        assert_eq!(result, result2);

        // Verify different data produces different hash
        let data2 = vec![0x01, 0x02, 0x03, 0x05];
        let result3 = compute_header_id(&data2);
        assert_ne!(result, result3);
    }
}
