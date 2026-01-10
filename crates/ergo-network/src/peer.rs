//! Peer management with advanced scoring.

use crate::penalties::PenaltyReason;
use crate::scoring::PeerScore;
use dashmap::DashMap;
use parking_lot::RwLock;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

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

/// Peer information with advanced scoring.
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
    /// Advanced peer score.
    pub score: PeerScore,
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
            score: PeerScore::new(),
            last_seen: Instant::now(),
            outbound,
        }
    }

    /// Get the peer's priority score for sync operations.
    pub fn priority(&self) -> f64 {
        self.score.priority()
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
    /// Minimum peers to maintain for sync.
    pub min_sync_peers: usize,
}

impl Default for PeerManagerConfig {
    fn default() -> Self {
        Self {
            max_connections: 30,
            max_outbound: 10,
            ban_duration: Duration::from_secs(3600), // 1 hour
            min_sync_peers: 5,
        }
    }
}

/// Manages peer connections with advanced scoring.
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

    /// Apply a penalty to a peer.
    ///
    /// # Arguments
    /// * `id` - Peer identifier
    /// * `reason` - The reason for the penalty
    ///
    /// # Returns
    /// `true` if the peer was banned as a result
    pub fn penalize(&self, id: &PeerId, reason: PenaltyReason) -> bool {
        if let Some(mut peer) = self.peers.get_mut(id) {
            let should_ban = peer.score.apply_penalty(reason);

            if should_ban {
                warn!(
                    peer = %id,
                    reason = %reason,
                    penalty = peer.score.current_penalty(),
                    "Banning peer"
                );
                peer.state = PeerState::Banned;
                self.connected.write().remove(id);
                self.banned
                    .insert(peer.addr, Instant::now() + self.config.ban_duration);
                return true;
            } else {
                debug!(
                    peer = %id,
                    reason = %reason,
                    penalty = peer.score.current_penalty(),
                    "Penalized peer"
                );
            }
        }
        false
    }

    /// Apply a penalty with a custom amount.
    pub fn penalize_amount(&self, id: &PeerId, amount: u32, reason: &str) -> bool {
        if let Some(mut peer) = self.peers.get_mut(id) {
            let should_ban = peer.score.apply_penalty_amount(amount);

            if should_ban {
                warn!(
                    peer = %id,
                    reason = reason,
                    penalty = peer.score.current_penalty(),
                    "Banning peer"
                );
                peer.state = PeerState::Banned;
                self.connected.write().remove(id);
                self.banned
                    .insert(peer.addr, Instant::now() + self.config.ban_duration);
                return true;
            }
        }
        false
    }

    /// Reward a peer for good behavior.
    pub fn reward(&self, id: &PeerId, reward: i32) {
        if let Some(mut peer) = self.peers.get_mut(id) {
            peer.score.apply_reward(reward);
        }
    }

    /// Record a successful delivery from a peer.
    pub fn record_delivery(&self, id: &PeerId) {
        if let Some(mut peer) = self.peers.get_mut(id) {
            peer.score.record_delivery();
        }
    }

    /// Record a failed delivery from a peer.
    pub fn record_failure(&self, id: &PeerId) {
        if let Some(mut peer) = self.peers.get_mut(id) {
            peer.score.record_failure();
        }
    }

    /// Record response time from a peer.
    pub fn record_response_time(&self, id: &PeerId, duration: Duration) {
        if let Some(mut peer) = self.peers.get_mut(id) {
            peer.score.record_response_time(duration);
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

    /// Unban a peer.
    pub fn unban(&self, addr: &SocketAddr) {
        self.banned.remove(addr);

        // Also reset the peer's score if we have info for them
        for mut peer in self.peers.iter_mut() {
            if peer.addr == *addr {
                peer.score.reset();
                if peer.state == PeerState::Banned {
                    peer.state = PeerState::Disconnected;
                }
                break;
            }
        }

        info!(addr = %addr, "Unbanned peer");
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

    /// Get peers to connect to (not connected, not banned).
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

    /// Get peers sorted by priority for sync operations.
    ///
    /// Returns peers in order of priority (best first), excluding
    /// peers that should be banned.
    pub fn get_sync_peers(&self, max_count: usize) -> Vec<PeerInfo> {
        let mut peers: Vec<_> = self
            .get_connected()
            .into_iter()
            .filter(|p| !p.score.should_ban())
            .collect();

        // Sort by priority (highest first)
        peers.sort_by(|a, b| {
            b.priority()
                .partial_cmp(&a.priority())
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        peers.truncate(max_count);
        peers
    }

    /// Get peers with height greater than specified.
    pub fn get_peers_with_height(&self, min_height: u32) -> Vec<PeerInfo> {
        self.get_connected()
            .into_iter()
            .filter(|p| p.height > min_height)
            .collect()
    }

    /// Get peers sorted by height (highest first).
    pub fn get_peers_by_height(&self, max_count: usize) -> Vec<PeerInfo> {
        let mut peers = self.get_connected();
        peers.sort_by(|a, b| b.height.cmp(&a.height));
        peers.truncate(max_count);
        peers
    }

    /// Get statistics about peer scores.
    pub fn score_stats(&self) -> PeerScoreStats {
        let connected = self.get_connected();

        if connected.is_empty() {
            return PeerScoreStats::default();
        }

        let penalties: Vec<_> = connected
            .iter()
            .map(|p| p.score.current_penalty())
            .collect();
        let priorities: Vec<_> = connected.iter().map(|p| p.priority()).collect();
        let response_times: Vec<_> = connected
            .iter()
            .map(|p| p.score.avg_response_time())
            .collect();

        PeerScoreStats {
            peer_count: connected.len(),
            avg_penalty: penalties.iter().sum::<u32>() / penalties.len() as u32,
            max_penalty: *penalties.iter().max().unwrap_or(&0),
            avg_priority: priorities.iter().sum::<f64>() / priorities.len() as f64,
            avg_response_time_ms: if response_times.iter().any(|&t| t > 0) {
                response_times.iter().filter(|&&t| t > 0).sum::<u64>()
                    / response_times.iter().filter(|&&t| t > 0).count().max(1) as u64
            } else {
                0
            },
            banned_count: self.banned.len(),
        }
    }
}

impl Default for PeerManager {
    fn default() -> Self {
        Self::new(PeerManagerConfig::default())
    }
}

/// Statistics about peer scores.
#[derive(Debug, Clone, Default)]
pub struct PeerScoreStats {
    /// Number of connected peers.
    pub peer_count: usize,
    /// Average penalty across peers.
    pub avg_penalty: u32,
    /// Maximum penalty among peers.
    pub max_penalty: u32,
    /// Average priority score.
    pub avg_priority: f64,
    /// Average response time in milliseconds.
    pub avg_response_time_ms: u64,
    /// Number of banned addresses.
    pub banned_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_addr(port: u16) -> SocketAddr {
        format!("127.0.0.1:{}", port).parse().unwrap()
    }

    #[test]
    fn test_peer_management() {
        let pm = PeerManager::default();

        let addr = test_addr(9030);
        let info = PeerInfo::new(addr, true);
        let id = info.id.clone();

        pm.add_peer(info);
        pm.set_state(&id, PeerState::Connected);

        assert_eq!(pm.connected_count(), 1);
        assert_eq!(pm.outbound_count(), 1);

        pm.set_state(&id, PeerState::Disconnected);
        assert_eq!(pm.connected_count(), 0);
    }

    #[test]
    fn test_penalty_and_ban() {
        let pm = PeerManager::default();

        let addr = test_addr(9030);
        let info = PeerInfo::new(addr, true);
        let id = info.id.clone();

        pm.add_peer(info);
        pm.set_state(&id, PeerState::Connected);

        // Apply a critical penalty
        let banned = pm.penalize(&id, PenaltyReason::MaliciousBehavior);
        assert!(banned);
        assert!(pm.is_banned(&addr));

        // Peer should be disconnected
        assert_eq!(pm.connected_count(), 0);
    }

    #[test]
    fn test_sync_peer_priority() {
        let pm = PeerManager::default();

        // Add a good peer
        let good_addr = test_addr(9030);
        let mut good_info = PeerInfo::new(good_addr, true);
        good_info
            .score
            .record_response_time(Duration::from_millis(50));
        for _ in 0..10 {
            good_info.score.record_delivery();
        }
        let good_id = good_info.id.clone();
        pm.add_peer(good_info);
        pm.set_state(&good_id, PeerState::Connected);

        // Add a bad peer
        let bad_addr = test_addr(9031);
        let mut bad_info = PeerInfo::new(bad_addr, true);
        bad_info
            .score
            .record_response_time(Duration::from_millis(5000));
        for _ in 0..10 {
            bad_info.score.record_failure();
        }
        bad_info.score.apply_penalty_amount(100);
        let bad_id = bad_info.id.clone();
        pm.add_peer(bad_info);
        pm.set_state(&bad_id, PeerState::Connected);

        // Get sync peers - good peer should be first
        let sync_peers = pm.get_sync_peers(10);
        assert_eq!(sync_peers.len(), 2);
        assert_eq!(sync_peers[0].id, good_id);
        assert_eq!(sync_peers[1].id, bad_id);
    }

    #[test]
    fn test_delivery_tracking() {
        let pm = PeerManager::default();

        let addr = test_addr(9030);
        let info = PeerInfo::new(addr, true);
        let id = info.id.clone();

        pm.add_peer(info);
        pm.set_state(&id, PeerState::Connected);

        pm.record_delivery(&id);
        pm.record_delivery(&id);
        pm.record_failure(&id);

        let peer = pm.get_peer(&id).unwrap();
        assert_eq!(peer.score.delivered_count(), 2);
        assert_eq!(peer.score.failed_count(), 1);
    }

    #[test]
    fn test_response_time_tracking() {
        let pm = PeerManager::default();

        let addr = test_addr(9030);
        let info = PeerInfo::new(addr, true);
        let id = info.id.clone();

        pm.add_peer(info);

        pm.record_response_time(&id, Duration::from_millis(100));
        pm.record_response_time(&id, Duration::from_millis(200));

        let peer = pm.get_peer(&id).unwrap();
        assert!(peer.score.avg_response_time() > 0);
    }

    #[test]
    fn test_unban() {
        let pm = PeerManager::default();

        let addr = test_addr(9030);
        let info = PeerInfo::new(addr, true);
        let id = info.id.clone();

        pm.add_peer(info);
        pm.set_state(&id, PeerState::Connected);

        pm.penalize(&id, PenaltyReason::MaliciousBehavior);
        assert!(pm.is_banned(&addr));

        pm.unban(&addr);
        assert!(!pm.is_banned(&addr));

        let peer = pm.get_peer(&id).unwrap();
        assert_eq!(peer.score.current_penalty(), 0);
    }

    #[test]
    fn test_score_stats() {
        let pm = PeerManager::default();

        // Add some peers
        for port in 9030..9035 {
            let addr = test_addr(port);
            let info = PeerInfo::new(addr, true);
            let id = info.id.clone();
            pm.add_peer(info);
            pm.set_state(&id, PeerState::Connected);
        }

        let stats = pm.score_stats();
        assert_eq!(stats.peer_count, 5);
        assert_eq!(stats.avg_penalty, 0);
    }
}
