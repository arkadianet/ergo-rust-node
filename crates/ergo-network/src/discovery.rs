//! Peer discovery mechanisms.
//!
//! This module provides DNS seed-based peer discovery for bootstrapping
//! the node's peer list.

use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;
use tokio::time::timeout;
use tracing::{debug, info, warn};

/// Default Ergo mainnet DNS seeds.
/// These are well-known nodes that can be used to bootstrap the network.
pub const MAINNET_DNS_SEEDS: &[&str] = &[
    "seed1.ergoplatform.com:9030",
    "seed2.ergoplatform.com:9030",
    "seed3.ergoplatform.com:9030",
];

/// Default Ergo testnet DNS seeds.
pub const TESTNET_DNS_SEEDS: &[&str] = &["testnet.ergoplatform.com:9020"];

/// Well-known mainnet peer addresses (fallback if DNS fails).
pub const MAINNET_KNOWN_PEERS: &[&str] = &[
    "213.239.193.208:9030", // EU node
    "159.65.11.55:9030",    // US node
    "165.227.26.175:9030",  // US node
    "159.89.116.15:9030",   // EU node
    "136.244.110.145:9030", // EU node
    "94.130.108.35:9030",   // EU node
    "51.75.147.1:9030",     // EU node
];

/// Well-known testnet peer addresses.
pub const TESTNET_KNOWN_PEERS: &[&str] = &["213.239.193.208:9020"];

/// Network type for discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkType {
    Mainnet,
    Testnet,
}

impl NetworkType {
    /// Get DNS seeds for this network.
    pub fn dns_seeds(&self) -> &'static [&'static str] {
        match self {
            NetworkType::Mainnet => MAINNET_DNS_SEEDS,
            NetworkType::Testnet => TESTNET_DNS_SEEDS,
        }
    }

    /// Get fallback known peers for this network.
    pub fn known_peers(&self) -> &'static [&'static str] {
        match self {
            NetworkType::Mainnet => MAINNET_KNOWN_PEERS,
            NetworkType::Testnet => TESTNET_KNOWN_PEERS,
        }
    }

    /// Get default port for this network.
    pub fn default_port(&self) -> u16 {
        match self {
            NetworkType::Mainnet => 9030,
            NetworkType::Testnet => 9020,
        }
    }
}

/// Peer discovery service.
pub struct PeerDiscovery {
    /// Network type.
    network: NetworkType,
    /// DNS resolution timeout.
    dns_timeout: Duration,
}

impl PeerDiscovery {
    /// Create a new peer discovery service.
    pub fn new(network: NetworkType) -> Self {
        Self {
            network,
            dns_timeout: Duration::from_secs(10),
        }
    }

    /// Set DNS resolution timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.dns_timeout = timeout;
        self
    }

    /// Discover peers from DNS seeds.
    pub async fn discover_from_dns(&self) -> Vec<SocketAddr> {
        let mut peers = Vec::new();

        for seed in self.network.dns_seeds() {
            match self.resolve_seed(seed).await {
                Ok(addrs) => {
                    info!(seed = %seed, count = addrs.len(), "Resolved DNS seed");
                    peers.extend(addrs);
                }
                Err(e) => {
                    warn!(seed = %seed, error = %e, "Failed to resolve DNS seed");
                }
            }
        }

        peers
    }

    /// Resolve a single DNS seed.
    async fn resolve_seed(&self, seed: &str) -> Result<Vec<SocketAddr>, std::io::Error> {
        let seed = seed.to_string();
        let dns_timeout = self.dns_timeout;

        // Spawn blocking DNS resolution
        let result = timeout(
            dns_timeout,
            tokio::task::spawn_blocking(move || {
                seed.to_socket_addrs().map(|iter| iter.collect::<Vec<_>>())
            }),
        )
        .await;

        match result {
            Ok(Ok(Ok(addrs))) => Ok(addrs),
            Ok(Ok(Err(e))) => Err(e),
            Ok(Err(e)) => Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Task join error: {}", e),
            )),
            Err(_) => Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "DNS resolution timed out",
            )),
        }
    }

    /// Get fallback known peers.
    pub fn get_known_peers(&self) -> Vec<SocketAddr> {
        self.network
            .known_peers()
            .iter()
            .filter_map(|addr| addr.parse().ok())
            .collect()
    }

    /// Discover all available peers (DNS + fallback).
    pub async fn discover_all(&self) -> Vec<SocketAddr> {
        let mut peers = self.discover_from_dns().await;

        if peers.is_empty() {
            debug!("No peers from DNS, using fallback known peers");
            peers = self.get_known_peers();
        }

        // Deduplicate
        peers.sort();
        peers.dedup();

        info!(count = peers.len(), "Discovered peers");
        peers
    }
}

/// Parse a peer address string (with optional port).
pub fn parse_peer_address(addr: &str, default_port: u16) -> Option<SocketAddr> {
    // Try direct parse first
    if let Ok(socket_addr) = addr.parse::<SocketAddr>() {
        return Some(socket_addr);
    }

    // Try adding default port
    if !addr.contains(':') {
        if let Ok(socket_addr) = format!("{}:{}", addr, default_port).parse::<SocketAddr>() {
            return Some(socket_addr);
        }
    }

    // Try DNS resolution (blocking)
    addr.to_socket_addrs().ok()?.next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_type() {
        assert_eq!(NetworkType::Mainnet.default_port(), 9030);
        assert_eq!(NetworkType::Testnet.default_port(), 9020);

        assert!(!NetworkType::Mainnet.dns_seeds().is_empty());
        assert!(!NetworkType::Mainnet.known_peers().is_empty());
    }

    #[test]
    fn test_parse_peer_address() {
        // With port
        let addr = parse_peer_address("127.0.0.1:9030", 9030);
        assert!(addr.is_some());
        assert_eq!(addr.unwrap().port(), 9030);

        // Without port (uses default)
        let addr = parse_peer_address("127.0.0.1", 9030);
        assert!(addr.is_some());
        assert_eq!(addr.unwrap().port(), 9030);
    }

    #[test]
    fn test_known_peers_parse() {
        let discovery = PeerDiscovery::new(NetworkType::Mainnet);
        let peers = discovery.get_known_peers();

        assert!(!peers.is_empty());
        for peer in &peers {
            assert_eq!(peer.port(), 9030);
        }
    }

    #[tokio::test]
    async fn test_discover_fallback() {
        let discovery =
            PeerDiscovery::new(NetworkType::Mainnet).with_timeout(Duration::from_millis(100)); // Short timeout to fail DNS

        // Even if DNS fails, we should get fallback peers
        let peers = discovery.discover_all().await;
        assert!(!peers.is_empty());
    }
}
