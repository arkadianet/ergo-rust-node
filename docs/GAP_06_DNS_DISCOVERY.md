# GAP_06: DNS Seed Discovery

## Status: IMPLEMENTED
## Priority: LOW
## Effort: Small
## Category: Network

---

## Implementation Summary (January 2025)

DNS seed discovery has been fully implemented in `crates/ergo-network/src/discovery.rs`.

### Components Implemented

**Constants**:
- `MAINNET_DNS_SEEDS` - Default Ergo mainnet DNS seeds
- `TESTNET_DNS_SEEDS` - Default Ergo testnet DNS seeds
- `MAINNET_KNOWN_PEERS` - Fallback peer addresses for mainnet
- `TESTNET_KNOWN_PEERS` - Fallback peer addresses for testnet

**Types**:
- `NetworkType` - Enum with `Mainnet` and `Testnet` variants
- `PeerDiscovery` - Service for discovering peers

**PeerDiscovery Methods**:
```rust
impl PeerDiscovery {
    pub fn new(network: NetworkType) -> Self;
    pub fn with_timeout(self, timeout: Duration) -> Self;
    pub async fn discover_from_dns(&self) -> Vec<SocketAddr>;
    pub fn get_known_peers(&self) -> Vec<SocketAddr>;
    pub async fn discover_all(&self) -> Vec<SocketAddr>;
}
```

**NetworkType Methods**:
```rust
impl NetworkType {
    pub fn dns_seeds(&self) -> &'static [&'static str];
    pub fn known_peers(&self) -> &'static [&'static str];
    pub fn default_port(&self) -> u16;
}
```

**Utility Functions**:
- `parse_peer_address(addr: &str, default_port: u16) -> Option<SocketAddr>`

### Tests

- `test_network_type` - Network type properties
- `test_parse_peer_address` - Address parsing with/without port
- `test_known_peers_parse` - Known peers validation
- `test_discover_fallback` - Fallback when DNS fails (async)

### Exports (lib.rs)

```rust
pub use discovery::{NetworkType, PeerDiscovery, MAINNET_DNS_SEEDS, TESTNET_DNS_SEEDS};
```

### Key Features

1. **Async DNS Resolution** - Uses `tokio::task::spawn_blocking` for DNS lookups
2. **Timeout Support** - Configurable DNS resolution timeout (default 10s)
3. **Fallback Mechanism** - Falls back to known peers if DNS fails
4. **Deduplication** - Results are sorted and deduplicated
5. **Logging** - Uses tracing for info/warn/debug messages

---

## Description

DNS seed discovery allows nodes to bootstrap their peer list by resolving DNS names that return IP addresses of known nodes. This is a standard bootstrapping mechanism in blockchain networks.

---

## Scala File References

| Feature | Scala File |
|---------|------------|
| DNS resolution | `scorex-core: src/main/scala/scorex/core/network/PeerManager.scala` |
| Configuration | `src/main/resources/application.conf` |
| Peer discovery | `src/main/scala/scorex/core/network/NetworkController.scala` |
