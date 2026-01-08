//! Node implementation.

use crate::config::NodeConfig;
use anyhow::Result;
use ergo_api::AppState;
use ergo_consensus::block::{BlockId, BlockTransactions, Digest32, Extension, FullBlock};
use ergo_mempool::Mempool;
use ergo_mining::{CandidateGenerator, Miner, MinerConfig};
use ergo_network::{
    Message, NetworkCommand, NetworkConfig, NetworkEvent, NetworkService, PeerId, PeerManager,
    MAINNET_MAGIC, TESTNET_MAGIC,
};
use ergo_state::{StateChange, StateManager};
use ergo_storage::Database;
use ergo_sync::{SyncCommand, SyncConfig, SyncEvent, SyncProtocol};
use ergo_wallet::{Wallet, WalletConfig as WalletCfg};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};

/// Tracks a peer that we want to reconnect to.
#[derive(Debug, Clone)]
struct ReconnectInfo {
    /// Socket address to reconnect to.
    addr: SocketAddr,
    /// Number of consecutive failed connection attempts.
    attempts: u32,
    /// Last attempt time.
    last_attempt: Instant,
    /// Next allowed attempt time (with exponential backoff).
    next_attempt: Instant,
}

impl ReconnectInfo {
    fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            attempts: 0,
            last_attempt: Instant::now(),
            next_attempt: Instant::now(),
        }
    }

    /// Calculate backoff duration based on attempt count.
    /// Uses exponential backoff: 5s, 10s, 20s, 40s, 80s, capped at 5 minutes.
    fn backoff_duration(&self) -> Duration {
        let base_secs = 5u64;
        let max_secs = 300u64; // 5 minutes
        let secs = base_secs.saturating_mul(1 << self.attempts.min(6));
        Duration::from_secs(secs.min(max_secs))
    }

    /// Mark a connection attempt as failed.
    fn mark_failed(&mut self) {
        self.attempts += 1;
        self.last_attempt = Instant::now();
        self.next_attempt = Instant::now() + self.backoff_duration();
    }

    /// Check if we can attempt reconnection now.
    fn can_attempt(&self) -> bool {
        Instant::now() >= self.next_attempt
    }

    /// Reset on successful connection.
    fn reset(&mut self) {
        self.attempts = 0;
    }
}

/// Maximum number of reconnection attempts before giving up on a peer.
const MAX_RECONNECT_ATTEMPTS: u32 = 10;

/// The main node struct coordinating all components.
pub struct Node {
    /// Node configuration.
    config: NodeConfig,
    /// Storage database.
    storage: Arc<Database>,
    /// State manager.
    state: Arc<StateManager>,
    /// Transaction mempool.
    mempool: Arc<Mempool>,
    /// Peer manager.
    peers: Arc<PeerManager>,
    /// Miner (optional).
    miner: Option<Arc<Miner>>,
    /// Wallet (optional).
    wallet: Option<Arc<Wallet>>,
    /// Shutdown flag.
    shutdown: Arc<AtomicBool>,
    /// API server handle.
    api_handle: RwLock<Option<tokio::task::JoinHandle<()>>>,
    /// Network command sender.
    network_cmd_tx: Option<mpsc::Sender<NetworkCommand>>,
    /// Sync command sender.
    sync_cmd_tx: Option<mpsc::Sender<SyncCommand>>,
}

impl Node {
    /// Create a new node.
    pub async fn new(config: NodeConfig) -> Result<Arc<Self>> {
        // Create data directory
        std::fs::create_dir_all(&config.data_dir)?;

        // Open database
        let db_path = config.data_dir.join("db");
        info!("Opening database at {:?}", db_path);
        let storage = Arc::new(Database::open(&db_path)?);

        // Initialize state manager
        let state = Arc::new(StateManager::init_from_storage(
            Arc::clone(&storage) as Arc<dyn ergo_storage::Storage>
        )?);

        // Initialize mempool
        let mempool = Arc::new(Mempool::with_defaults());

        // Initialize peer manager
        let peers = Arc::new(PeerManager::default());

        // Add known peers
        for peer_addr in &config.network_config.known_peers {
            if let Ok(addr) = peer_addr.parse::<SocketAddr>() {
                let info = ergo_network::PeerInfo::new(addr, true);
                peers.add_peer(info);
            }
        }

        // Initialize miner if enabled
        let miner = if config.mining.enabled {
            let candidate_gen = Arc::new(CandidateGenerator::new(
                Arc::clone(&state),
                Arc::clone(&mempool),
            ));

            let miner_config = MinerConfig {
                internal_mining: false,
                external_mining: config.mining.external,
                reward_address: config.mining.reward_address.clone().unwrap_or_default(),
                threads: 1,
            };

            let miner = Arc::new(Miner::new(miner_config, candidate_gen));
            miner.start();
            Some(miner)
        } else {
            None
        };

        // Initialize wallet if enabled
        let wallet = if config.wallet.enabled {
            let wallet_cfg = WalletCfg {
                data_dir: config.data_dir.join(&config.wallet.data_dir),
                secret_file: config.wallet.secret_file.clone(),
                ..Default::default()
            };

            let wallet = Arc::new(Wallet::new(wallet_cfg, Arc::clone(&state)));

            // Try to load existing wallet
            if wallet.load().is_ok() {
                info!("Loaded existing wallet");
            }

            Some(wallet)
        } else {
            None
        };

        let node = Arc::new(Self {
            config,
            storage,
            state,
            mempool,
            peers,
            miner,
            wallet,
            shutdown: Arc::new(AtomicBool::new(false)),
            api_handle: RwLock::new(None),
            network_cmd_tx: None,
            sync_cmd_tx: None,
        });

        Ok(node)
    }

    /// Run the node.
    pub async fn run(self: &Arc<Self>) -> Result<()> {
        info!("Starting node services...");

        // Start API server
        self.start_api().await?;

        // Start P2P networking
        self.start_networking().await?;

        // Start synchronization
        self.start_sync().await?;

        // Main loop
        while !self.shutdown.load(Ordering::SeqCst) {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

            // Periodic tasks
            self.tick().await;
        }

        info!("Node main loop stopped");
        Ok(())
    }

    /// Start the API server.
    async fn start_api(&self) -> Result<()> {
        let bind_addr: SocketAddr = self.config.api.bind_address.parse()?;

        let app_state = AppState::new(
            Arc::clone(&self.state),
            Arc::clone(&self.mempool),
            Arc::clone(&self.peers),
            self.config.node_name.clone(),
        )
        .with_mining(
            self.config.mining.enabled,
            self.config.mining.reward_address.clone(),
        );

        let app_state = if let Some(ref key) = self.config.api.api_key {
            app_state.with_api_key(key.clone())
        } else {
            app_state
        };

        let router = ergo_api::build_api(app_state);

        info!("Starting API server on {}", bind_addr);

        let listener = tokio::net::TcpListener::bind(bind_addr).await?;

        let handle = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, router).await {
                warn!("API server error: {}", e);
            }
        });

        *self.api_handle.write().await = Some(handle);

        Ok(())
    }

    /// Start P2P networking and synchronization.
    async fn start_networking(&self) -> Result<()> {
        let bind_addr: SocketAddr = self.config.network_config.bind_address.parse()?;

        let magic = if self.config.network == "testnet" {
            TESTNET_MAGIC
        } else {
            MAINNET_MAGIC
        };

        let network_config = NetworkConfig {
            listen_addr: bind_addr,
            magic,
            agent_name: "ergo-rust-node".to_string(),
            node_name: self.config.node_name.clone(),
            max_connections: self.config.network_config.max_connections,
            ..Default::default()
        };

        info!("Starting P2P networking on {}", bind_addr);

        // Create network service
        let (network_service, mut network_event_rx, network_cmd_tx) =
            NetworkService::new(network_config, Arc::clone(&self.peers));

        // Create sync protocol
        let (sync_cmd_tx, mut sync_cmd_rx) = mpsc::channel::<SyncCommand>(100);
        let (sync_event_tx, mut sync_event_rx) = mpsc::channel::<SyncEvent>(100);
        let sync_protocol = SyncProtocol::new(SyncConfig::default(), sync_cmd_tx.clone());

        // Initialize sync protocol with stored headers from database
        let (_, header_height) = self.state.heights();
        info!("Initializing sync with header_height={}", header_height);
        if header_height > 0 {
            // Try to get header locator (exponentially spaced headers)
            match self.state.get_header_locator() {
                Ok(locator) if !locator.is_empty() => {
                    info!("Got {} headers from locator", locator.len());
                    let header_ids: Vec<Vec<u8>> =
                        locator.iter().map(|id| id.0.as_ref().to_vec()).collect();
                    sync_protocol.init_from_stored_headers(header_ids);
                }
                Ok(_) => {
                    // Locator empty, try getting headers by range
                    match self.state.get_headers(1, header_height) {
                        Ok(headers) if !headers.is_empty() => {
                            info!("Got {} headers from range query", headers.len());
                            let header_ids: Vec<Vec<u8>> =
                                headers.iter().map(|h| h.id.0.as_ref().to_vec()).collect();
                            sync_protocol.init_from_stored_headers(header_ids);
                        }
                        Ok(_) => {
                            warn!(
                                "No headers found in storage despite header_height={}",
                                header_height
                            );
                        }
                        Err(e) => {
                            warn!("Failed to get headers from storage: {}", e);
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to get header locator: {}", e);
                }
            }
        }

        let network_cmd_tx_clone = network_cmd_tx.clone();
        let shutdown = Arc::clone(&self.shutdown);
        let mempool = Arc::clone(&self.mempool);

        // Spawn network service
        tokio::spawn(async move {
            if let Err(e) = network_service.run().await {
                error!("Network service error: {}", e);
            }
        });

        // Spawn sync protocol event handler
        let sync_protocol = Arc::new(sync_protocol);
        let sync_protocol_clone = Arc::clone(&sync_protocol);
        let shutdown_for_sync = Arc::clone(&self.shutdown);
        tokio::spawn(async move {
            while !shutdown_for_sync.load(Ordering::SeqCst) {
                match sync_event_rx.recv().await {
                    Some(event) => {
                        if let Err(e) = sync_protocol_clone.handle_event(event).await {
                            warn!("Sync event error: {}", e);
                        }
                    }
                    None => break,
                }
            }
            info!("Sync protocol stopped");
        });

        // Connect to known peers
        for peer_addr in &self.config.network_config.known_peers {
            if let Ok(addr) = peer_addr.parse::<SocketAddr>() {
                let _ = network_cmd_tx.send(NetworkCommand::Connect { addr }).await;
            }
        }

        // Spawn event router - bridges network events to sync protocol
        let network_cmd_tx_for_router = network_cmd_tx_clone.clone();
        let shutdown_for_router = shutdown.clone();
        let sync_event_tx_clone = sync_event_tx.clone();
        let state_for_router = Arc::clone(&self.state);

        // Track peer addresses for reconnection
        let mut peer_addresses: HashMap<PeerId, SocketAddr> = HashMap::new();
        // Track peers we want to reconnect to
        let mut reconnect_queue: HashMap<SocketAddr, ReconnectInfo> = HashMap::new();
        // Desired minimum number of connections
        let min_connections: usize = 3;

        tokio::spawn(async move {
            // Tick interval for sync protocol housekeeping (every 1 second)
            // This needs to be fast enough to request pending headers before peer disconnects
            let mut tick_interval = tokio::time::interval(std::time::Duration::from_secs(1));
            tick_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            // Reconnection check interval (every 10 seconds)
            let mut reconnect_interval = tokio::time::interval(std::time::Duration::from_secs(10));
            reconnect_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            while !shutdown_for_router.load(Ordering::SeqCst) {
                tokio::select! {
                    // Periodic tick for sync protocol
                    _ = tick_interval.tick() => {
                        let _ = sync_event_tx_clone.send(SyncEvent::Tick).await;
                    }
                    // Periodic reconnection attempts
                    _ = reconnect_interval.tick() => {
                        let connected_count = peer_addresses.len();

                        // Try to reconnect to peers if we're below minimum
                        if connected_count < min_connections {
                            let mut to_remove = Vec::new();

                            for (addr, info) in reconnect_queue.iter_mut() {
                                if info.attempts >= MAX_RECONNECT_ATTEMPTS {
                                    info!(addr = %addr, attempts = info.attempts, "Giving up on peer after max attempts");
                                    to_remove.push(*addr);
                                    continue;
                                }

                                if info.can_attempt() {
                                    info!(
                                        addr = %addr,
                                        attempt = info.attempts + 1,
                                        backoff_secs = info.backoff_duration().as_secs(),
                                        "Attempting to reconnect to peer"
                                    );
                                    let _ = network_cmd_tx_for_router.send(NetworkCommand::Connect { addr: *addr }).await;
                                    // Don't mark as failed yet - wait for ConnectionFailed event
                                }
                            }

                            // Remove peers we've given up on
                            for addr in to_remove {
                                reconnect_queue.remove(&addr);
                            }
                        }
                    }
                    // Handle network events - forward to sync protocol
                    Some(event) = network_event_rx.recv() => {
                        match event {
                            NetworkEvent::PeerConnected { peer_id, addr, handshake } => {
                                info!(peer = %peer_id, addr = %addr, agent = %handshake.agent_name, "Peer connected");

                                // Track the peer's address for potential reconnection later
                                peer_addresses.insert(peer_id.clone(), addr);

                                // Remove from reconnect queue on successful connection
                                if let Some(info) = reconnect_queue.get_mut(&addr) {
                                    info.reset();
                                }
                                reconnect_queue.remove(&addr);

                                let _ = sync_event_tx_clone.send(SyncEvent::PeerConnected {
                                    peer: peer_id,
                                }).await;
                            }
                            NetworkEvent::PeerDisconnected { peer_id } => {
                                info!(peer = %peer_id, "Peer disconnected");

                                // Get the address before removing
                                if let Some(addr) = peer_addresses.remove(&peer_id) {
                                    // Add to reconnect queue if not already there
                                    if !reconnect_queue.contains_key(&addr) {
                                        info!(addr = %addr, "Adding peer to reconnection queue");
                                        reconnect_queue.insert(addr, ReconnectInfo::new(addr));
                                    }
                                }

                                let _ = sync_event_tx_clone.send(SyncEvent::PeerDisconnected {
                                    peer: peer_id,
                                }).await;
                            }
                            NetworkEvent::MessageReceived { peer_id, message } => {
                                debug!(peer = %peer_id, msg = ?message.message_type(), "Message received");
                                Self::handle_message(
                                    &peer_id,
                                    message,
                                    &mempool,
                                    &sync_event_tx_clone,
                                    &peer_addresses,
                                    &network_cmd_tx_for_router,
                                ).await;
                            }
                            NetworkEvent::ConnectionFailed { addr, error } => {
                                warn!(addr = %addr, error = %error, "Connection failed");

                                // Update reconnect info with failed attempt
                                if let Some(info) = reconnect_queue.get_mut(&addr) {
                                    info.mark_failed();
                                    info!(
                                        addr = %addr,
                                        attempts = info.attempts,
                                        next_attempt_secs = info.backoff_duration().as_secs(),
                                        "Will retry connection with backoff"
                                    );
                                } else {
                                    // Add to reconnect queue for first-time failures (e.g., initial connection)
                                    let mut new_info = ReconnectInfo::new(addr);
                                    new_info.mark_failed();
                                    info!(
                                        addr = %addr,
                                        next_attempt_secs = new_info.backoff_duration().as_secs(),
                                        "Adding failed peer to reconnection queue"
                                    );
                                    reconnect_queue.insert(addr, new_info);
                                }
                            }
                        }
                    }
                    // Handle sync commands that need network actions or state updates
                    Some(cmd) = sync_cmd_rx.recv() => {
                        match cmd {
                            SyncCommand::SendToPeer { peer, message } => {
                                let _ = network_cmd_tx_for_router.send(NetworkCommand::SendMessage {
                                    peer_id: peer,
                                    message,
                                }).await;
                            }
                            SyncCommand::Broadcast { message } => {
                                let _ = network_cmd_tx_for_router.send(NetworkCommand::Broadcast {
                                    message,
                                }).await;
                            }
                            SyncCommand::StoreHeader { header, response_tx } => {
                                let height = header.height;
                                let result = match state_for_router.apply_header(header) {
                                    Ok(_) => {
                                        debug!(height, "Header stored");
                                        Ok(())
                                    }
                                    Err(e) => {
                                        warn!(height, error = %e, "Failed to store header");
                                        Err(e.to_string())
                                    }
                                };
                                // Send response back to sync protocol
                                let _ = response_tx.send(result);
                            }
                            SyncCommand::ApplyBlock { block_id, block_data } => {
                                debug!(block_id = %hex::encode(&block_id), size = block_data.len(), "Applying block");

                                // Parse BlockTransactions from raw data
                                let block_txs = match BlockTransactions::parse(&block_data) {
                                    Ok(txs) => txs,
                                    Err(e) => {
                                        warn!(block_id = %hex::encode(&block_id), error = ?e, "Failed to parse BlockTransactions");
                                        continue;
                                    }
                                };

                                // Get the header from state manager
                                let header_id = BlockId(
                                    Digest32::from(
                                        <[u8; 32]>::try_from(block_id.as_slice()).unwrap_or([0u8; 32])
                                    )
                                );

                                let header = match state_for_router.get_header(&header_id) {
                                    Ok(Some(h)) => h,
                                    Ok(None) => {
                                        warn!(block_id = %hex::encode(&block_id), "Header not found for block");
                                        continue;
                                    }
                                    Err(e) => {
                                        warn!(block_id = %hex::encode(&block_id), error = %e, "Failed to get header");
                                        continue;
                                    }
                                };

                                let height = header.height;

                                // Create StateChange from block transactions
                                let state_change = StateChange::from_block_transactions(&block_txs, height);

                                info!(
                                    height,
                                    spent = state_change.spent.len(),
                                    created = state_change.created.len(),
                                    "Applying block to UTXO state"
                                );

                                // Create FullBlock (with empty extension for now - we'd need to fetch it separately)
                                let extension = Extension::empty(header_id.clone());
                                let full_block = FullBlock::new(header, block_txs, extension, None);

                                // Apply block to state
                                match state_for_router.apply_block(full_block, state_change) {
                                    Ok(_) => {
                                        info!(height, block_id = %hex::encode(&block_id), "Block applied successfully");
                                        // Notify sync protocol
                                        let _ = sync_event_tx_clone.send(SyncEvent::BlockApplied {
                                            block_id,
                                            height,
                                        }).await;
                                    }
                                    Err(e) => {
                                        warn!(height, block_id = %hex::encode(&block_id), error = %e, "Failed to apply block");
                                        let _ = sync_event_tx_clone.send(SyncEvent::BlockFailed {
                                            block_id,
                                            error: e.to_string(),
                                        }).await;
                                    }
                                }
                            }
                        }
                    }
                    else => break,
                }
            }
            info!("Event router stopped");
        });

        Ok(())
    }

    /// Handle an incoming P2P message.
    async fn handle_message(
        peer_id: &ergo_network::PeerId,
        message: Message,
        _mempool: &Arc<Mempool>,
        sync_event_tx: &mpsc::Sender<SyncEvent>,
        peer_addresses: &HashMap<PeerId, SocketAddr>,
        network_cmd_tx: &mpsc::Sender<NetworkCommand>,
    ) {
        match message {
            Message::SyncInfo(sync_info) => {
                debug!(peer = %peer_id, height = sync_info.last_headers.len(), "SyncInfo received");
                let _ = sync_event_tx
                    .send(SyncEvent::SyncInfoReceived {
                        peer: peer_id.clone(),
                        info: sync_info,
                    })
                    .await;
            }
            Message::Inv(inv) => {
                debug!(peer = %peer_id, count = inv.ids.len(), "Inv received");
                let _ = sync_event_tx
                    .send(SyncEvent::InvReceived {
                        peer: peer_id.clone(),
                        inv,
                    })
                    .await;
            }
            Message::Modifier(modifier) => {
                debug!(peer = %peer_id, type_id = modifier.type_id, count = modifier.modifiers.len(), "Modifier received");
                // Send an event for each modifier in the batch
                for item in modifier.modifiers {
                    let _ = sync_event_tx
                        .send(SyncEvent::ModifierReceived {
                            peer: peer_id.clone(),
                            type_id: modifier.type_id,
                            id: item.id,
                            data: item.data,
                        })
                        .await;
                }
            }
            Message::RequestModifier(request) => {
                debug!(peer = %peer_id, type_id = request.type_id, count = request.ids.len(), "RequestModifier received");
                // We would respond with requested modifiers if we have them
            }
            Message::GetPeers => {
                debug!(peer = %peer_id, "GetPeers request - sending known peers");

                // Collect peer addresses to share (excluding the requesting peer)
                let peer_addrs: Vec<String> = peer_addresses
                    .iter()
                    .filter(|(id, _)| *id != peer_id)
                    .map(|(_, addr)| addr.to_string())
                    .take(20) // Limit to 20 peers
                    .collect();

                if !peer_addrs.is_empty() {
                    info!(peer = %peer_id, count = peer_addrs.len(), "Responding with peers");
                    let response = Message::Peers(peer_addrs);
                    let _ = network_cmd_tx
                        .send(NetworkCommand::SendMessage {
                            peer_id: peer_id.clone(),
                            message: response,
                        })
                        .await;
                } else {
                    debug!(peer = %peer_id, "No peers to share");
                }
            }
            Message::Peers(peers) => {
                debug!(peer = %peer_id, count = peers.len(), "Peers received");
                // Log the received peers - they can be used for future connections
                for peer_addr in &peers {
                    debug!(peer_addr = %peer_addr, "Received peer address");
                }
                // TODO: Add received peers to PeerManager for future connections
            }
            Message::Handshake(_) => {
                // Handshake already handled by NetworkService
            }
        }
    }

    /// Start synchronization.
    async fn start_sync(&self) -> Result<()> {
        let (utxo_height, header_height) = self.state.heights();
        info!(
            "Current state: UTXO height={}, header height={}",
            utxo_height, header_height
        );

        // Sync is started as part of start_networking
        // The SyncProtocol will handle header and block synchronization

        Ok(())
    }

    /// Periodic tick.
    async fn tick(&self) {
        // Log stats periodically
        static COUNTER: AtomicBool = AtomicBool::new(false);

        if !COUNTER.swap(true, Ordering::SeqCst) {
            let (utxo_height, header_height) = self.state.heights();
            let mempool_stats = self.mempool.stats();
            let peer_count = self.peers.connected_count();

            info!(
                utxo_height,
                header_height,
                mempool_txs = mempool_stats.tx_count,
                peers = peer_count,
                "Node status"
            );
        }
    }

    /// Shutdown the node.
    pub async fn shutdown(&self) {
        info!("Shutting down node...");
        self.shutdown.store(true, Ordering::SeqCst);

        // Stop miner
        if let Some(ref miner) = self.miner {
            miner.stop();
        }

        // Cancel API server
        if let Some(handle) = self.api_handle.write().await.take() {
            handle.abort();
        }

        // Flush database
        if let Err(e) = self.storage.flush() {
            warn!("Error flushing database: {}", e);
        }

        info!("Node shutdown complete");
    }
}

impl Clone for Node {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            storage: Arc::clone(&self.storage),
            state: Arc::clone(&self.state),
            mempool: Arc::clone(&self.mempool),
            peers: Arc::clone(&self.peers),
            miner: self.miner.clone(),
            wallet: self.wallet.clone(),
            shutdown: Arc::clone(&self.shutdown),
            api_handle: RwLock::new(None), // Don't clone the handle
            network_cmd_tx: self.network_cmd_tx.clone(),
            sync_cmd_tx: self.sync_cmd_tx.clone(),
        }
    }
}
