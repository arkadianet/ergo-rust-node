//! Peer management.

use dashmap::DashMap;
use parking_lot::RwLock;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tracing::warn;

/// Unique peer identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PeerId(pub Vec<u8>);

impl PeerId {
    /// Create from bytes.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Create from socket address.
    pub fn from_addr(addr: &SocketAddr) -> Self {
        Self(format!("{}", addr).into_bytes())
    }
}

impl std::fmt::Display for PeerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", hex::encode(&self.0))
    }
}

/// Peer connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerState {
    /// Not connected.
    Disconnected,
    /// Connection in progress.
    Connecting,
    /// Connected and handshaking.
    Handshaking,
    /// Fully connected.
    Connected,
    /// Banned.
    Banned,
}

/// Peer information.
#[derive(Debug, Clone)]
pub struct PeerInfo {
    /// Peer identifier.
    pub id: PeerId,
    /// Socket address.
    pub addr: SocketAddr,
    /// Current state.
    pub state: PeerState,
    /// Protocol version.
    pub version: Option<(u8, u8, u8)>,
    /// Agent name.
    pub agent: Option<String>,
    /// Best known height.
    pub height: u32,
    /// Connection score.
    pub score: i32,
    /// Last seen time.
    pub last_seen: Instant,
    /// Connection direction (true = outbound).
    pub outbound: bool,
}

impl PeerInfo {
    /// Create new peer info.
    pub fn new(addr: SocketAddr, outbound: bool) -> Self {
        Self {
            id: PeerId::from_addr(&addr),
            addr,
            state: PeerState::Disconnected,
            version: None,
            agent: None,
            height: 0,
            score: 0,
            last_seen: Instant::now(),
            outbound,
        }
    }
}

/// Peer manager configuration.
#[derive(Debug, Clone)]
pub struct PeerManagerConfig {
    /// Maximum number of connections.
    pub max_connections: usize,
    /// Maximum number of outbound connections.
    pub max_outbound: usize,
    /// Ban duration.
    pub ban_duration: Duration,
    /// Penalty threshold for banning.
    pub ban_threshold: i32,
}

impl Default for PeerManagerConfig {
    fn default() -> Self {
        Self {
            max_connections: 30,
            max_outbound: 10,
            ban_duration: Duration::from_secs(3600), // 1 hour
            ban_threshold: -100,
        }
    }
}

/// Manages peer connections.
pub struct PeerManager {
    /// Configuration.
    config: PeerManagerConfig,
    /// Known peers.
    peers: DashMap<PeerId, PeerInfo>,
    /// Connected peer IDs.
    connected: RwLock<HashSet<PeerId>>,
    /// Banned peers (addr -> unban time).
    banned: DashMap<SocketAddr, Instant>,
}

impl PeerManager {
    /// Create a new peer manager.
    pub fn new(config: PeerManagerConfig) -> Self {
        Self {
            config,
            peers: DashMap::new(),
            connected: RwLock::new(HashSet::new()),
            banned: DashMap::new(),
        }
    }

    /// Add a known peer.
    pub fn add_peer(&self, info: PeerInfo) {
        let id = info.id.clone();
        self.peers.insert(id, info);
    }

    /// Get peer info.
    pub fn get_peer(&self, id: &PeerId) -> Option<PeerInfo> {
        self.peers.get(id).map(|r| r.clone())
    }

    /// Update peer state.
    pub fn set_state(&self, id: &PeerId, state: PeerState) {
        if let Some(mut peer) = self.peers.get_mut(id) {
            peer.state = state;
            peer.last_seen = Instant::now();

            match state {
                PeerState::Connected => {
                    self.connected.write().insert(id.clone());
                }
                PeerState::Disconnected | PeerState::Banned => {
                    self.connected.write().remove(id);
                }
                _ => {}
            }
        }
    }

    /// Update peer height.
    pub fn set_height(&self, id: &PeerId, height: u32) {
        if let Some(mut peer) = self.peers.get_mut(id) {
            peer.height = height;
            peer.last_seen = Instant::now();
        }
    }

    /// Apply penalty to peer.
    pub fn penalize(&self, id: &PeerId, penalty: i32) {
        if let Some(mut peer) = self.peers.get_mut(id) {
            peer.score -= penalty;

            if peer.score <= self.config.ban_threshold {
                warn!(peer = %id, score = peer.score, "Banning peer");
                peer.state = PeerState::Banned;
                self.connected.write().remove(id);
                self.banned
                    .insert(peer.addr, Instant::now() + self.config.ban_duration);
            }
        }
    }

    /// Reward peer.
    pub fn reward(&self, id: &PeerId, reward: i32) {
        if let Some(mut peer) = self.peers.get_mut(id) {
            peer.score = (peer.score + reward).min(100);
        }
    }

    /// Check if address is banned.
    pub fn is_banned(&self, addr: &SocketAddr) -> bool {
        if let Some(unban_time) = self.banned.get(addr) {
            if Instant::now() < *unban_time {
                return true;
            }
            self.banned.remove(addr);
        }
        false
    }

    /// Get connected peer count.
    pub fn connected_count(&self) -> usize {
        self.connected.read().len()
    }

    /// Get outbound peer count.
    pub fn outbound_count(&self) -> usize {
        self.connected
            .read()
            .iter()
            .filter(|id| self.peers.get(*id).map(|p| p.outbound).unwrap_or(false))
            .count()
    }

    /// Can accept new connection?
    pub fn can_accept(&self) -> bool {
        self.connected_count() < self.config.max_connections
    }

    /// Can make new outbound connection?
    pub fn can_connect_outbound(&self) -> bool {
        self.outbound_count() < self.config.max_outbound
    }

    /// Get peers to connect to.
    pub fn get_peers_to_connect(&self, count: usize) -> Vec<SocketAddr> {
        self.peers
            .iter()
            .filter(|r| r.state == PeerState::Disconnected)
            .filter(|r| !self.is_banned(&r.addr))
            .take(count)
            .map(|r| r.addr)
            .collect()
    }

    /// Get connected peers.
    pub fn get_connected(&self) -> Vec<PeerInfo> {
        self.connected
            .read()
            .iter()
            .filter_map(|id| self.get_peer(id))
            .collect()
    }

    /// Get best peer by height.
    pub fn get_best_peer(&self) -> Option<PeerInfo> {
        self.get_connected().into_iter().max_by_key(|p| p.height)
    }
}

impl Default for PeerManager {
    fn default() -> Self {
        Self::new(PeerManagerConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_peer_management() {
        let pm = PeerManager::default();

        let addr: SocketAddr = "127.0.0.1:9030".parse().unwrap();
        let info = PeerInfo::new(addr, true);
        let id = info.id.clone();

        pm.add_peer(info);
        pm.set_state(&id, PeerState::Connected);

        assert_eq!(pm.connected_count(), 1);
        assert_eq!(pm.outbound_count(), 1);

        pm.set_state(&id, PeerState::Disconnected);
        assert_eq!(pm.connected_count(), 0);
    }
}
