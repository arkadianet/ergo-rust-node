//! Synchronization protocol handler.
//!
//! This module implements the Ergo sync protocol:
//! 1. Exchange SyncInfo with peers to learn their chain state
//! 2. Request headers for unknown blocks
//! 3. Download and validate full blocks
//! 4. Apply blocks to state

use crate::{BlockDownloader, DownloadTask, SyncConfig, SyncError, SyncResult, Synchronizer};
use ergo_chain_types::{BlockId, Header};
use ergo_consensus::block::ModifierType;
use ergo_network::{InvData, Message, ModifierRequest, PeerId, SyncInfo};
use parking_lot::RwLock;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, trace, warn};

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
        response_tx: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    /// Request to apply a validated block.
    ApplyBlock {
        block_id: Vec<u8>,
        block_data: Vec<u8>,
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
    /// Periodic tick for housekeeping.
    Tick,
}

/// Header chain segment for tracking sync progress.
#[derive(Debug, Clone)]
struct HeaderChain {
    /// Headers we've received but not yet have blocks for.
    headers: VecDeque<Header>,
    /// Block IDs we need to download.
    missing_blocks: VecDeque<Vec<u8>>,
}

impl HeaderChain {
    fn new() -> Self {
        Self {
            headers: VecDeque::new(),
            missing_blocks: VecDeque::new(),
        }
    }
}

/// Pending header waiting for its parent.
#[derive(Debug, Clone)]
struct PendingHeader {
    /// The header itself.
    header: Header,
    /// Parent ID we're waiting for.
    parent_id: Vec<u8>,
}

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
    /// Peers we're syncing from.
    sync_peers: RwLock<HashMap<PeerId, PeerSyncState>>,
    /// Headers pending storage (waiting for parent).
    pending_headers: RwLock<HashMap<Vec<u8>, PendingHeader>>,
    /// Headers we've successfully stored (by ID).
    stored_headers: RwLock<std::collections::HashSet<Vec<u8>>>,
    /// Command sender.
    command_tx: mpsc::Sender<SyncCommand>,
    /// Pending header IDs from Inv that we haven't requested yet.
    pending_header_inv: RwLock<VecDeque<Vec<u8>>>,
    /// Header IDs we've requested but not yet received (in-flight).
    in_flight_headers: RwLock<std::collections::HashSet<Vec<u8>>>,
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
}

/// Minimum interval between SyncInfo messages to the same peer (20 seconds like Scala node).
const MIN_SYNC_INTERVAL: std::time::Duration = std::time::Duration::from_secs(20);

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
        }
    }

    /// Check if we can send SyncInfo to this peer (respecting MIN_SYNC_INTERVAL).
    fn can_send_sync(&self) -> bool {
        self.last_sync_sent.elapsed() >= MIN_SYNC_INTERVAL
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

        let mut stored = std::collections::HashSet::new();
        // Consider genesis parent as "stored" so height-1 headers can be applied
        stored.insert(genesis_parent_id);

        Self {
            synchronizer: Arc::new(Synchronizer::new(config)),
            downloader: Arc::new(BlockDownloader::default()),
            our_best_headers: RwLock::new(Vec::new()),
            header_chain: RwLock::new(HeaderChain::new()),
            sync_peers: RwLock::new(HashMap::new()),
            pending_headers: RwLock::new(HashMap::new()),
            stored_headers: RwLock::new(stored),
            command_tx,
            pending_header_inv: RwLock::new(VecDeque::new()),
            in_flight_headers: RwLock::new(std::collections::HashSet::new()),
        }
    }

    /// Get the synchronizer.
    pub fn synchronizer(&self) -> &Synchronizer {
        &self.synchronizer
    }

    /// Initialize with stored headers from database.
    /// This should be called before starting sync if we have previously synced headers.
    pub fn init_from_stored_headers(&self, header_ids: Vec<Vec<u8>>) {
        if header_ids.is_empty() {
            return;
        }

        info!(
            "Initializing sync protocol with {} stored headers",
            header_ids.len()
        );

        // Add all header IDs to stored set
        {
            let mut stored = self.stored_headers.write();
            for id in &header_ids {
                stored.insert(id.clone());
            }
        }

        // Set our best headers (last N for SyncInfo)
        // Take the most recent ones (they should be in order from genesis to tip)
        let best_headers: Vec<Vec<u8>> = header_ids
            .into_iter()
            .rev()
            .take(MAX_SYNC_HEADER_IDS)
            .collect();

        *self.our_best_headers.write() = best_headers;
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
            SyncEvent::Tick => {
                self.on_tick().await?;
            }
        }
        Ok(())
    }

    /// Handle new peer connection.
    async fn on_peer_connected(&self, peer: PeerId) -> SyncResult<()> {
        info!(peer = %peer, "Peer connected, sending SyncInfo");

        // Add to sync peers with initial state
        let mut state = PeerSyncState::new();
        // Mark that we're about to send SyncInfo
        state.last_sync_sent = std::time::Instant::now();

        self.sync_peers.write().insert(peer.clone(), state);

        // Send our SyncInfo (V1 format with header IDs)
        // Limit to MAX_SYNC_HEADER_IDS to avoid overwhelming the peer
        // Scala node typically sends ~100 IDs sampled from the chain
        let our_headers: Vec<Vec<u8>> = self
            .our_best_headers
            .read()
            .iter()
            .rev() // Most recent first
            .take(MAX_SYNC_HEADER_IDS)
            .cloned()
            .collect();

        // Always send SyncInfo - if we have no headers, send genesis ID
        // This tells the peer "I'm starting from genesis, please send me headers"
        let sync_headers = if !our_headers.is_empty() {
            debug!(
                "Initial SyncInfo: count={}, first_len={}",
                our_headers.len(),
                our_headers.first().map(|v| v.len()).unwrap_or(0)
            );
            our_headers
        } else {
            // Fresh node - send genesis block ID (all zeros) to signal we need headers from the start
            // The genesis block at height=0 has ID = 0000...0000
            let genesis_id = vec![0u8; 32];
            info!("Fresh node - sending genesis ID (all zeros) in SyncInfo to request headers");
            vec![genesis_id]
        };

        let sync_info = SyncInfo::v1(sync_headers);
        self.send_to_peer(&peer, Message::SyncInfo(sync_info))
            .await?;
        Ok(())
    }

    /// Handle peer disconnection.
    fn on_peer_disconnected(&self, peer: &PeerId) {
        info!(peer = %peer, "Peer disconnected");
        self.sync_peers.write().remove(peer);
        self.synchronizer.remove_peer(peer);
    }

    /// Handle received SyncInfo.
    async fn on_sync_info(&self, peer: PeerId, info: SyncInfo) -> SyncResult<()> {
        // Get header IDs and peer height from either V1 or V2 format
        let (header_ids, peer_height): (Vec<Vec<u8>>, Option<u32>) = if info.is_v2() {
            // V2 format contains full serialized headers - parse them to get height
            info!(peer = %peer, headers = info.last_headers.len(), "Received SyncInfo V2");
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
            info!(peer = %peer, header_ids = info.last_header_ids.len(), "Received SyncInfo V1");
            // V1 format only has header IDs, no height info
            // We'll need to estimate or get it from other sources
            (info.last_header_ids.clone(), None)
        };

        // Determine peer height - use parsed height from V2, or keep existing for V1
        // For V1, we can't know the exact height from just IDs, so we preserve
        // any previously known height (which gets updated as we receive headers)
        let estimated_height = peer_height.unwrap_or_else(|| {
            // For V1, check if we already have a height for this peer
            let existing_height = self
                .sync_peers
                .read()
                .get(&peer)
                .map(|s| s.height)
                .unwrap_or(0);

            if existing_height > 0 {
                existing_height
            } else if !header_ids.is_empty() {
                // Peer has headers but we don't know height yet
                // Use 0 - will be updated when we receive actual headers
                0
            } else {
                0
            }
        });

        if let Some(state) = self.sync_peers.write().get_mut(&peer) {
            state.best_headers = header_ids.clone();
            state.last_activity = std::time::Instant::now();
            state.height = estimated_height;
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

        // Respond with our SyncInfo so the peer knows what headers we have
        // This allows them to send us an Inv with the headers we need
        let our_headers: Vec<Vec<u8>> = self
            .our_best_headers
            .read()
            .iter()
            .take(MAX_SYNC_HEADER_IDS)
            .cloned()
            .collect();

        // Always respond - use genesis ID if we have no headers
        let response_headers = if !our_headers.is_empty() {
            debug!(
                "Responding to SyncInfo with our {} headers",
                our_headers.len()
            );
            our_headers
        } else {
            // Fresh node - send genesis block ID (all zeros)
            let genesis_id = vec![0u8; 32];
            debug!("Responding to SyncInfo with genesis ID (fresh node)");
            vec![genesis_id]
        };

        let sync_info = SyncInfo::v1(response_headers);
        self.send_to_peer(&peer, Message::SyncInfo(sync_info))
            .await?;

        Ok(())
    }

    /// Handle inventory announcement.
    async fn on_inv(&self, peer: PeerId, inv: InvData) -> SyncResult<()> {
        info!(
            peer = %peer,
            type_id = inv.type_id,
            count = inv.ids.len(),
            "Received Inv, requesting modifiers"
        );

        // Request modifiers in batches
        // Scala node uses desiredInvObjects = 400, so we can request up to 400 at once
        const MAX_REQUEST_SIZE: usize = 400;

        // Only handle header Inv specially (type_id 101)
        let is_header_inv = inv.type_id == ModifierType::Header.to_byte();

        if !inv.ids.is_empty() {
            // Log first few IDs for debugging
            for (i, id) in inv.ids.iter().take(3).enumerate() {
                debug!(index = i, id = %hex::encode(id), "Inv item");
            }

            // Filter out already-stored headers
            let filtered_ids: Vec<Vec<u8>> = {
                let stored = self.stored_headers.read();
                inv.ids
                    .into_iter()
                    .filter(|id| !stored.contains(id))
                    .collect()
            };

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
                    pending.extend(remaining);
                }
            }

            // Only send request if we have IDs to request
            // Sending empty RequestModifier causes "empty inv list" error on Scala node
            if !ids_to_request.is_empty() {
                // Track these as in-flight before sending request
                if is_header_inv {
                    let mut in_flight = self.in_flight_headers.write();
                    for id in &ids_to_request {
                        in_flight.insert(id.clone());
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
            Some(ModifierType::Header) => {
                self.on_header_received(&peer, id, data).await?;
            }
            Some(ModifierType::BlockTransactions) => {
                self.on_block_received(&peer, id, data).await?;
            }
            Some(ModifierType::Extension) => {
                // Handle extension
                debug!(id = hex::encode(&id), "Received extension");
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
        // Remove from in-flight tracking
        self.in_flight_headers.write().remove(&id);

        // Parse header using sigma-rust
        let header = match Header::scorex_parse_bytes(&data) {
            Ok(h) => h,
            Err(e) => {
                warn!("Failed to parse header: {}", e);
                return Err(SyncError::InvalidData(format!(
                    "Failed to parse header: {}",
                    e
                )));
            }
        };

        // Update peer height if this header is higher than what we've seen
        // This gives us a more accurate peer height than the V1 SyncInfo estimate
        let update_height = {
            let mut peers = self.sync_peers.write();
            if let Some(state) = peers.get_mut(peer) {
                if header.height > state.height {
                    state.height = header.height;
                    Some(header.height)
                } else {
                    None
                }
            } else {
                None
            }
        };
        if let Some(height) = update_height {
            self.synchronizer.set_peer_height(peer.clone(), height);
        }

        let parent_id = header.parent_id.0.as_ref().to_vec();

        // Check if we already have this header
        let already_stored = self.stored_headers.read().contains(&id);
        if already_stored {
            trace!(height = header.height, "Header already stored, skipping");
            return Ok(());
        }

        // Check if parent is available (either genesis or already stored)
        let parent_available = self.stored_headers.read().contains(&parent_id);

        if parent_available {
            // Parent exists, we can store this header
            self.store_header_and_descendants(header).await?;
        } else {
            // Parent not available yet, buffer this header
            debug!(
                height = header.height,
                parent = hex::encode(&parent_id),
                "Buffering header - parent not available"
            );
            self.pending_headers
                .write()
                .insert(id.clone(), PendingHeader { header, parent_id });
        }

        Ok(())
    }

    /// Store a header and any pending descendants that can now be applied.
    async fn store_header_and_descendants(&self, header: Header) -> SyncResult<()> {
        let header_id = header.id.0.as_ref().to_vec();
        let height = header.height;

        info!(id = hex::encode(&header_id), height, "Storing header");

        // Create response channel for synchronous storage confirmation
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();

        // Send command to store the header
        let cmd = SyncCommand::StoreHeader {
            header: header.clone(),
            response_tx,
        };
        self.command_tx
            .send(cmd)
            .await
            .map_err(|e| SyncError::Internal(format!("Failed to send store command: {}", e)))?;

        // Wait for storage confirmation
        match response_rx.await {
            Ok(Ok(())) => {
                debug!(height, "Header storage confirmed");
            }
            Ok(Err(e)) => {
                return Err(SyncError::Internal(format!(
                    "Failed to store header: {}",
                    e
                )));
            }
            Err(_) => {
                return Err(SyncError::Internal(
                    "Storage response channel closed".to_string(),
                ));
            }
        }

        // Mark as stored (only after confirmation)
        self.stored_headers.write().insert(header_id.clone());

        // Add to our header chain tracking
        {
            let mut chain = self.header_chain.write();
            chain.headers.push_back(header);
            chain.missing_blocks.push_back(header_id.clone());
        }

        // Update synchronizer height
        self.synchronizer.set_height(height);

        // Check if any pending headers can now be applied
        self.try_apply_pending_headers(&header_id).await?;

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
        // Find headers waiting for this parent
        let waiting: Vec<_> = {
            let pending = self.pending_headers.read();
            pending
                .iter()
                .filter(|(_, ph)| ph.parent_id == parent_id)
                .map(|(id, ph)| (id.clone(), ph.header.clone()))
                .collect()
        };

        // Apply each waiting header
        for (id, header) in waiting {
            // Remove from pending
            self.pending_headers.write().remove(&id);

            // Recursively store (this will check for its own descendants)
            Box::pin(self.store_header_and_descendants(header)).await?;
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

        // Find the corresponding header for validation
        let header = {
            let chain = self.header_chain.read();
            chain
                .headers
                .iter()
                .find(|h| h.id.0.as_ref() == id.as_slice())
                .cloned()
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
            // We don't have the header - this shouldn't happen in normal sync
            // but we can still proceed and let the state manager validate
            warn!(
                id = hex::encode(&id),
                "Received block without corresponding header in cache"
            );
        }

        // Mark as downloaded
        self.downloader.complete(&id);
        self.synchronizer.block_received(&id);

        // Send command to apply the block
        let cmd = SyncCommand::ApplyBlock {
            block_id: id,
            block_data: data,
        };
        self.command_tx
            .send(cmd)
            .await
            .map_err(|e| SyncError::Internal(format!("Failed to send command: {}", e)))?;

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

        // Remove from header chain
        let mut chain = self.header_chain.write();
        chain.missing_blocks.retain(|id| id != block_id);
    }

    /// Handle failed block application.
    fn on_block_failed(&self, block_id: &[u8], error: &str) {
        warn!(
            id = hex::encode(block_id),
            error, "Block application failed"
        );

        // Could re-request from different peer or mark as invalid
    }

    /// Periodic tick handler.
    async fn on_tick(&self) -> SyncResult<()> {
        // Check for timeouts
        let timed_out = self.downloader.check_timeouts();
        for (id, peer) in timed_out {
            warn!(id = hex::encode(&id), peer = %peer, "Download timed out");
            self.downloader.fail(&id, &peer);
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
        let missing_blocks = self.header_chain.read().missing_blocks.len();

        // Check if we have peers with headers (even if we don't know their height)
        let has_peers_with_headers = self
            .sync_peers
            .read()
            .values()
            .any(|s| !s.best_headers.is_empty());

        if our_height < peer_height
            || has_peers_with_headers
            || stats.pending > 0
            || stats.in_flight > 0
            || missing_blocks > 0
        {
            // Indicate sync mode in log
            // If peer_height is 0 but we have peers with headers, we're still syncing headers
            let sync_mode = if peer_height == 0 || our_height + 1000 < peer_height {
                "headers"
            } else {
                "blocks"
            };
            info!(
                our_height,
                peer_height,
                sync_mode,
                pending = stats.pending,
                in_flight = stats.in_flight,
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

        // During initial header sync, don't download blocks yet
        // Only start downloading blocks when we're within 1000 headers of the tip
        // This implements header-first sync strategy
        if peer_height > 0 && our_height + 1000 < peer_height {
            // Still syncing headers, skip block downloads for now
            return;
        }

        let stats = self.downloader.stats();

        // Don't queue more if we already have enough pending
        if stats.pending + stats.in_flight >= 16 {
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

        debug!(
            our_height,
            peer_height,
            chain_len,
            pending_inv_count,
            in_flight_count,
            "request_more_headers check"
        );

        // Don't request more if we still have headers in flight
        if in_flight_count > 0 {
            debug!(in_flight_count, "Waiting for in-flight headers to complete");
            return Ok(());
        }

        // If we have pending headers, check if we should clear them
        // (they might be stuck because we received non-consecutive headers)
        let pending_count = self.pending_headers.read().len();
        if pending_count > 0 {
            // Clear pending headers if they've been stuck
            // (they were probably from non-consecutive SyncInfo samples)
            if pending_count > 0 && self.header_chain.read().headers.is_empty() {
                // We have pending but no applied headers - clear the pending ones
                warn!(pending_count, "Clearing stuck pending headers");
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

                    // Track as in-flight
                    {
                        let mut in_flight = self.in_flight_headers.write();
                        for id in &pending_ids {
                            in_flight.insert(id.clone());
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

        // If we're behind the network and have no pending Inv, send SyncInfo
        // Also send if peer_height is 0 but peer has announced headers (V1 SyncInfo case)
        let has_peers_with_headers = self
            .sync_peers
            .read()
            .values()
            .any(|s| !s.best_headers.is_empty());

        if our_height < peer_height || (peer_height == 0 && has_peers_with_headers) {
            // Find a peer that we can send SyncInfo to (respecting rate limits)
            if let Some(peer) = self.find_peer_for_sync() {
                // Get our best header IDs for SyncInfo
                let our_best_ids: Vec<Vec<u8>> = {
                    let chain = self.header_chain.read();
                    chain
                        .headers
                        .iter()
                        .rev()
                        .take(10)
                        .map(|h| h.id.0.as_ref().to_vec())
                        .collect()
                };

                // Use genesis ID if we have no headers yet
                let sync_ids = if !our_best_ids.is_empty() {
                    our_best_ids
                } else {
                    // Fresh node - send genesis block ID (all zeros)
                    let genesis_id = vec![0u8; 32];
                    info!("Fresh node tick - sending genesis ID (all zeros) in SyncInfo");
                    vec![genesis_id]
                };

                // Log the IDs we're sending for debugging
                debug!(
                    "SyncInfo IDs: count={}, first_len={}, first_hex={}",
                    sync_ids.len(),
                    sync_ids.first().map(|v| v.len()).unwrap_or(0),
                    sync_ids.first().map(|v| hex::encode(v)).unwrap_or_default()
                );
                info!(
                    our_height,
                    peer_height, chain_len, "Sending SyncInfo to request more headers"
                );

                // Update last sync time
                if let Some(state) = self.sync_peers.write().get_mut(&peer) {
                    state.last_sync_sent = std::time::Instant::now();
                }

                // Update our tracked best headers
                *self.our_best_headers.write() = sync_ids.clone();

                // Send SyncInfo to trigger the peer to send us Inv with more headers
                let sync_info = SyncInfo::v1(sync_ids);
                self.send_to_peer(&peer, Message::SyncInfo(sync_info))
                    .await?;
            }
        }

        Ok(())
    }

    /// Dispatch pending downloads to peers.
    async fn dispatch_downloads(&self) -> SyncResult<()> {
        let tasks = self.downloader.get_ready_tasks(16);
        if tasks.is_empty() {
            return Ok(());
        }

        // Find a peer that can receive requests
        let peer = match self.find_peer_for_request() {
            Some(p) => p,
            None => {
                debug!("No peer available for download dispatch (rate limited)");
                return Ok(());
            }
        };

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
        if let Some(state) = self.sync_peers.write().get_mut(&peer) {
            state.in_flight += tasks.len() as u32;
            state.last_request_sent = std::time::Instant::now();
        }

        // Send one batched request per type
        for (type_id, ids) in by_type {
            let request = ModifierRequest { type_id, ids };
            self.send_to_peer(&peer, Message::RequestModifier(request))
                .await?;
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
}
