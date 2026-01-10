# GAP_06: DNS Seed Discovery

## Priority: LOW
## Effort: Small
## Category: Network

---

## Description

The Rust Ergo node currently relies on manual peer configuration. It lacks DNS seed discovery, which allows nodes to bootstrap their peer list by querying DNS servers that return IP addresses of known nodes. This is a standard bootstrapping mechanism in blockchain networks.

---

## Current State (Rust)

### What Exists

Peer discovery in `crates/ergo-network/src/discovery.rs`:
- Manual peer configuration via config file
- Peer exchange protocol (get peers from connected peers)
- No DNS seed lookup

### Configuration

```toml
[network]
known_peers = [
    "213.239.193.208:9030",
    "159.65.11.55:9030",
    # ... manually specified peers
]
```

### What's Missing

- DNS TXT record lookup for seed nodes
- Fallback to DNS when no peers available
- Configurable DNS seeds per network

---

## Scala Reference

### DNS Seeds Configuration

```scala
// File: src/main/resources/application.conf

scorex {
  network {
    # DNS seeds for mainnet
    knownPeers = []
    
    # DNS seed domains
    dnsSeeds = [
      "ergo.network",
      "ergo-mainnet.duckdns.org"
    ]
  }
}
```

### DNS Resolution

```scala
// File: src/main/scala/scorex/core/network/PeerManager.scala

private def resolveDnsSeeds(): Seq[InetSocketAddress] = {
  settings.network.dnsSeeds.flatMap { seed =>
    Try {
      val records = InetAddress.getAllByName(seed)
      records.map(r => new InetSocketAddress(r, settings.network.bindAddress.getPort))
    }.getOrElse(Seq.empty)
  }
}

def bootstrap(): Unit = {
  if (knownPeers.isEmpty) {
    val seedPeers = resolveDnsSeeds()
    seedPeers.foreach(connect)
  }
}
```

---

## Impact

### Why DNS Seeds Matter

1. **Zero Configuration**: New nodes can find peers automatically
2. **Decentralization**: Multiple DNS seeds prevent single points of failure
3. **Freshness**: DNS can be updated with active nodes
4. **Fallback**: Works when peer exchange fails

### Current Workaround

Users must manually configure known peers, which:
- Requires knowledge of active node IPs
- May use outdated/offline nodes
- Increases setup complexity

---

## Implementation Plan

### Phase 1: DNS Resolution (0.5 days)

1. **Add DNS lookup** in `crates/ergo-network/src/discovery.rs`:
   ```rust
   use trust_dns_resolver::TokioAsyncResolver;
   use trust_dns_resolver::config::*;
   
   pub struct DnsDiscovery {
       resolver: TokioAsyncResolver,
       seeds: Vec<String>,
       default_port: u16,
   }
   
   impl DnsDiscovery {
       pub fn new(seeds: Vec<String>, default_port: u16) -> Self {
           let resolver = TokioAsyncResolver::tokio(
               ResolverConfig::default(),
               ResolverOpts::default(),
           ).unwrap();
           
           Self { resolver, seeds, default_port }
       }
       
       pub async fn resolve_seeds(&self) -> Vec<SocketAddr> {
           let mut peers = Vec::new();
           
           for seed in &self.seeds {
               match self.resolver.lookup_ip(seed.as_str()).await {
                   Ok(response) => {
                       for ip in response.iter() {
                           peers.push(SocketAddr::new(ip, self.default_port));
                       }
                   }
                   Err(e) => {
                       tracing::warn!("Failed to resolve DNS seed {}: {}", seed, e);
                   }
               }
           }
           
           peers
       }
   }
   ```

### Phase 2: Configuration (0.5 days)

2. **Add DNS seeds to config** in `crates/ergo-node/src/config.rs`:
   ```rust
   #[derive(Deserialize)]
   pub struct NetworkConfig {
       pub bind_address: SocketAddr,
       pub declared_address: Option<String>,
       pub known_peers: Vec<String>,
       
       /// DNS seeds for peer discovery (new)
       #[serde(default = "default_dns_seeds")]
       pub dns_seeds: Vec<String>,
       
       pub max_connections: usize,
   }
   
   fn default_dns_seeds() -> Vec<String> {
       vec![
           "seed1.ergo.network".into(),
           "seed2.ergo.network".into(),
       ]
   }
   ```

   **Config file example**:
   ```toml
   [network]
   bind_address = "0.0.0.0:9030"
   dns_seeds = [
       "seed1.ergo.network",
       "seed2.ergo.network",
   ]
   known_peers = []  # Optional fallback
   ```

### Phase 3: Integration (0.5 days)

3. **Integrate into network service** in `crates/ergo-network/src/service.rs`:
   ```rust
   impl NetworkService {
       pub async fn bootstrap(&mut self) -> Result<()> {
           // Try known peers first
           if !self.config.known_peers.is_empty() {
               for peer in &self.config.known_peers {
                   self.connect(peer).await?;
               }
           }
           
           // If no connections, try DNS seeds
           if self.connected_peers().is_empty() {
               tracing::info!("No peers from config, trying DNS seeds");
               
               let dns = DnsDiscovery::new(
                   self.config.dns_seeds.clone(),
                   self.config.bind_address.port(),
               );
               
               let seed_peers = dns.resolve_seeds().await;
               tracing::info!("Resolved {} peers from DNS", seed_peers.len());
               
               for peer in seed_peers {
                   if let Err(e) = self.connect(&peer.to_string()).await {
                       tracing::debug!("Failed to connect to seed peer {}: {}", peer, e);
                   }
               }
           }
           
           Ok(())
       }
   }
   ```

### Phase 4: Periodic Re-resolution (0.5 days)

4. **Add background DNS refresh**:
   ```rust
   impl NetworkService {
       async fn dns_refresh_task(&self) {
           let mut interval = tokio::time::interval(Duration::from_secs(3600)); // 1 hour
           
           loop {
               interval.tick().await;
               
               // Only refresh if low on peers
               if self.connected_peers().len() < self.config.min_connections {
                   let dns = DnsDiscovery::new(
                       self.config.dns_seeds.clone(),
                       self.config.bind_address.port(),
                   );
                   
                   for peer in dns.resolve_seeds().await {
                       if !self.is_connected(&peer) {
                           let _ = self.connect(&peer.to_string()).await;
                       }
                   }
               }
           }
       }
   }
   ```

---

## Files to Modify/Create

### Modified Files
```
crates/ergo-network/src/discovery.rs  # Add DnsDiscovery
crates/ergo-network/src/service.rs    # Integrate DNS lookup
crates/ergo-node/src/config.rs        # Add dns_seeds config
```

### Dependencies to Add
```toml
# In crates/ergo-network/Cargo.toml
[dependencies]
trust-dns-resolver = { version = "0.23", features = ["tokio-runtime"] }
```

---

## Estimated Effort

| Task | Time |
|------|------|
| DNS resolution | 0.5 days |
| Configuration | 0.5 days |
| Service integration | 0.5 days |
| Periodic refresh | 0.5 days |
| Testing | 0.5 days |
| **Total** | **2.5 days** |

---

## Dependencies

- None - standalone feature

---

## Test Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_dns_resolution() {
        let dns = DnsDiscovery::new(
            vec!["localhost".into()],
            9030,
        );
        
        let peers = dns.resolve_seeds().await;
        assert!(!peers.is_empty());
        assert_eq!(peers[0].port(), 9030);
    }
    
    #[tokio::test]
    async fn test_dns_failure_handling() {
        let dns = DnsDiscovery::new(
            vec!["nonexistent.invalid.domain".into()],
            9030,
        );
        
        // Should not panic, just return empty
        let peers = dns.resolve_seeds().await;
        assert!(peers.is_empty());
    }
}
```

### Integration Tests

```rust
#[tokio::test]
async fn test_bootstrap_with_dns() {
    let config = NetworkConfig {
        dns_seeds: vec!["seed1.ergo.network".into()],
        known_peers: vec![],
        ..Default::default()
    };
    
    let mut service = NetworkService::new(config);
    service.bootstrap().await.unwrap();
    
    // Should have attempted connections
    assert!(service.connection_attempts() > 0);
}
```

---

## Success Metrics

1. Node can bootstrap without manual peer configuration
2. DNS failures are handled gracefully (fallback to known peers)
3. Multiple DNS seeds are tried in sequence
4. Background refresh maintains peer count

---

## DNS Seed Setup Notes

### For Node Operators

DNS seeds should be configured as A records pointing to stable, long-running nodes:

```
seed1.ergo.network.  300  IN  A  213.239.193.208
seed2.ergo.network.  300  IN  A  159.65.11.55
```

### For Different Networks

```toml
# Mainnet
dns_seeds = ["mainnet-seed.ergo.network"]

# Testnet
dns_seeds = ["testnet-seed.ergo.network"]
```

---

## Scala File References

| Feature | Scala File |
|---------|------------|
| DNS resolution | `scorex-core: src/main/scala/scorex/core/network/PeerManager.scala` |
| Configuration | `src/main/resources/application.conf` |
| Peer discovery | `src/main/scala/scorex/core/network/NetworkController.scala` |
