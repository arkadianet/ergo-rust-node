//! Shared application state.

use ergo_mempool::Mempool;
use ergo_mining::CandidateGenerator;
use ergo_network::PeerManager;
use ergo_state::StateManager;
use ergo_storage::{Database, ExtraIndexer, ScanStorage};
use ergo_wallet::{Wallet, WalletConfig};
use std::sync::Arc;
use tokio::sync::broadcast;

/// Shared application state for API handlers.
#[derive(Clone)]
pub struct AppState {
    /// State manager.
    pub state: Arc<StateManager>,
    /// Transaction mempool.
    pub mempool: Arc<Mempool>,
    /// Peer manager.
    pub peers: Arc<PeerManager>,
    /// Block candidate generator for mining.
    pub candidate_generator: Arc<CandidateGenerator>,
    /// Wallet (optional).
    pub wallet: Option<Arc<Wallet>>,
    /// Extra indexer (optional).
    pub indexer: Option<Arc<ExtraIndexer>>,
    /// Scan storage (optional).
    pub scan_storage: Option<Arc<ScanStorage<Database>>>,
    /// Node name.
    pub node_name: String,
    /// API key (if required).
    pub api_key: Option<String>,
    /// Mining enabled.
    pub mining_enabled: bool,
    /// Shutdown signal sender.
    /// When a message is sent on this channel, the node will initiate graceful shutdown.
    pub shutdown_signal: Option<broadcast::Sender<()>>,
}

impl AppState {
    /// Create a new app state.
    pub fn new(
        state: Arc<StateManager>,
        mempool: Arc<Mempool>,
        peers: Arc<PeerManager>,
        node_name: String,
    ) -> Self {
        let candidate_generator = Arc::new(CandidateGenerator::new(
            Arc::clone(&state),
            Arc::clone(&mempool),
        ));
        Self {
            state,
            mempool,
            peers,
            candidate_generator,
            wallet: None,
            indexer: None,
            scan_storage: None,
            node_name,
            api_key: None,
            mining_enabled: false,
            shutdown_signal: None,
        }
    }

    /// Enable extra indexer.
    pub fn with_indexer(mut self, indexer: Arc<ExtraIndexer>) -> Self {
        self.indexer = Some(indexer);
        self
    }

    /// Enable scan storage.
    pub fn with_scan_storage(mut self, scan_storage: Arc<ScanStorage<Database>>) -> Self {
        self.scan_storage = Some(scan_storage);
        self
    }

    /// Set the API key.
    pub fn with_api_key(mut self, key: String) -> Self {
        self.api_key = Some(key);
        self
    }

    /// Enable mining with a reward address.
    pub fn with_mining(mut self, enabled: bool, reward_address: Option<String>) -> Self {
        self.mining_enabled = enabled;
        if let Some(addr) = reward_address {
            self.candidate_generator.set_reward_address(addr);
        }
        self
    }

    /// Enable wallet with the given configuration.
    pub fn with_wallet(mut self, config: WalletConfig) -> Self {
        let wallet = Wallet::new(config, Arc::clone(&self.state));
        self.wallet = Some(Arc::new(wallet));
        self
    }

    /// Check if API key is valid.
    pub fn check_api_key(&self, provided: Option<&str>) -> bool {
        match (&self.api_key, provided) {
            (None, _) => true, // No key required
            (Some(expected), Some(provided)) => expected == provided,
            (Some(_), None) => false,
        }
    }

    /// Set the shutdown signal sender.
    ///
    /// The returned receiver can be used to listen for shutdown requests.
    /// When the `/node/shutdown` endpoint is called, a message will be sent on this channel.
    pub fn with_shutdown_signal(mut self, sender: broadcast::Sender<()>) -> Self {
        self.shutdown_signal = Some(sender);
        self
    }
}
