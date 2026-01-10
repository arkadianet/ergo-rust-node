# GAP_07: Advanced Peer Scoring

## Priority: LOW
## Effort: Medium
## Category: Network

---

## Description

The Rust node has basic peer management but lacks the sophisticated peer scoring and penalty system implemented in the Scala node. Advanced peer scoring helps identify reliable peers, penalize misbehaving ones, and protect against various network attacks.

---

## Current State (Rust)

### What Exists

In `crates/ergo-network/src/peer.rs`:
```rust
pub struct PeerInfo {
    pub addr: SocketAddr,
    pub version: Option<Version>,
    pub last_seen: Instant,
    pub connection_state: ConnectionState,
}
```

Basic tracking:
- Connection state (Connected, Disconnected)
- Last seen timestamp
- Protocol version

### What's Missing

1. **Peer scoring** based on behavior
2. **Penalty system** for protocol violations
3. **Decline table** for spam protection
4. **Reputation persistence** across restarts
5. **Peer prioritization** for connections

---

## Scala Reference

### Peer Scoring System

```scala
// File: src/main/scala/scorex/core/network/peer/PeerInfo.scala

case class PeerInfo(
  peerSpec: PeerSpec,
  lastHandshake: Long,
  connectionType: Option[ConnectionType],
  // Scoring fields
  penaltyScore: Int = 0,
  lastPenaltyTime: Long = 0,
  responseTime: Long = 0,
  deliveredModifiers: Long = 0,
  requestedModifiers: Long = 0
)

object PeerInfo {
  val MaxPenalty = 1000
  val PenaltyDecayPerMinute = 10
  
  def updatePenalty(info: PeerInfo, penalty: Int): PeerInfo = {
    val decayedPenalty = calculateDecay(info.penaltyScore, info.lastPenaltyTime)
    info.copy(
      penaltyScore = math.min(decayedPenalty + penalty, MaxPenalty),
      lastPenaltyTime = System.currentTimeMillis()
    )
  }
}
```

### Penalty Categories

```scala
// File: src/main/scala/org/ergoplatform/network/PeerPenalties.scala

object PeerPenalties {
  // Minor violations
  val InvalidMessage = 5
  val SlowResponse = 2
  val MissingResponse = 10
  
  // Moderate violations
  val InvalidModifier = 50
  val DuplicateModifier = 20
  val UnexpectedMessage = 30
  
  // Severe violations
  val InvalidBlock = 100
  val InvalidTransaction = 80
  val SpamDetected = 150
  
  // Critical violations (instant ban)
  val MaliciousBehavior = 1000
  val ProtocolViolation = 500
}
```

### Decline Table (Spam Protection)

```scala
// File: src/main/scala/scorex/core/network/DeclineTable.scala

class DeclineTable(maxSize: Int, ttl: Duration) {
  private val declined = new ConcurrentHashMap[ModifierId, Long]()
  
  def decline(modifierId: ModifierId): Unit = {
    declined.put(modifierId, System.currentTimeMillis())
    cleanup()
  }
  
  def isDeclined(modifierId: ModifierId): Boolean = {
    Option(declined.get(modifierId))
      .exists(time => System.currentTimeMillis() - time < ttl.toMillis)
  }
  
  private def cleanup(): Unit = {
    val now = System.currentTimeMillis()
    declined.entrySet().removeIf(e => now - e.getValue > ttl.toMillis)
  }
}
```

### Peer Selection Priority

```scala
// File: src/main/scala/scorex/core/network/PeerManager.scala

def selectPeersForSync(): Seq[ConnectedPeer] = {
  connectedPeers
    .filter(_.penaltyScore < MaxPenaltyForSync)
    .sortBy { peer =>
      // Priority scoring
      val responsiveness = 1.0 / (peer.responseTime + 1)
      val reliability = peer.deliveredModifiers.toDouble / (peer.requestedModifiers + 1)
      val freshness = 1.0 / (System.currentTimeMillis() - peer.lastHandshake + 1)
      
      -(responsiveness * 0.4 + reliability * 0.4 + freshness * 0.2)
    }
    .take(maxSyncPeers)
}
```

---

## Impact

### Why Advanced Scoring Matters

1. **Attack Resistance**: Identify and isolate malicious peers
2. **Performance**: Prioritize reliable, fast peers for sync
3. **Spam Protection**: Prevent resource exhaustion attacks
4. **Network Health**: Maintain quality peer connections

### Current Risks

- No protection against slow-drip attacks
- Cannot identify unreliable peers
- No spam filtering for invalid modifiers
- Equal treatment of good and bad peers

---

## Implementation Plan

### Phase 1: Peer Score Structure (1 day)

1. **Extend PeerInfo** in `crates/ergo-network/src/peer.rs`:
   ```rust
   use std::time::{Duration, Instant};
   
   pub struct PeerScore {
       /// Accumulated penalty points (0-1000)
       pub penalty: u32,
       /// Last time penalty was updated
       pub last_penalty_time: Instant,
       /// Average response time in milliseconds
       pub avg_response_time: u64,
       /// Modifiers successfully delivered
       pub delivered_count: u64,
       /// Modifiers requested but not delivered
       pub failed_count: u64,
       /// Number of invalid messages received
       pub invalid_messages: u32,
   }
   
   impl PeerScore {
       pub const MAX_PENALTY: u32 = 1000;
       pub const DECAY_PER_MINUTE: u32 = 10;
       pub const BAN_THRESHOLD: u32 = 500;
       
       pub fn new() -> Self {
           Self {
               penalty: 0,
               last_penalty_time: Instant::now(),
               avg_response_time: 0,
               delivered_count: 0,
               failed_count: 0,
               invalid_messages: 0,
           }
       }
       
       /// Apply penalty with automatic decay
       pub fn apply_penalty(&mut self, amount: u32) {
           let decayed = self.calculate_decayed_penalty();
           self.penalty = (decayed + amount).min(Self::MAX_PENALTY);
           self.last_penalty_time = Instant::now();
       }
       
       fn calculate_decayed_penalty(&self) -> u32 {
           let minutes = self.last_penalty_time.elapsed().as_secs() / 60;
           self.penalty.saturating_sub((minutes as u32) * Self::DECAY_PER_MINUTE)
       }
       
       /// Should this peer be banned?
       pub fn should_ban(&self) -> bool {
           self.calculate_decayed_penalty() >= Self::BAN_THRESHOLD
       }
       
       /// Calculate peer priority (higher is better)
       pub fn priority(&self) -> f64 {
           let responsiveness = 1.0 / (self.avg_response_time as f64 + 1.0);
           let reliability = self.delivered_count as f64 / (self.failed_count as f64 + 1.0);
           let penalty_factor = 1.0 - (self.calculate_decayed_penalty() as f64 / 1000.0);
           
           responsiveness * 0.3 + reliability * 0.4 + penalty_factor * 0.3
       }
   }
   ```

### Phase 2: Penalty Definitions (0.5 days)

2. **Create penalty module** in `crates/ergo-network/src/penalties.rs`:
   ```rust
   /// Penalty amounts for various peer misbehaviors
   pub struct Penalties;
   
   impl Penalties {
       // Minor violations (1-10 points)
       pub const SLOW_RESPONSE: u32 = 2;
       pub const MISSING_RESPONSE: u32 = 10;
       pub const DUPLICATE_MESSAGE: u32 = 5;
       
       // Moderate violations (20-50 points)
       pub const INVALID_MESSAGE_FORMAT: u32 = 20;
       pub const UNEXPECTED_MESSAGE: u32 = 30;
       pub const INVALID_MODIFIER: u32 = 50;
       
       // Severe violations (80-150 points)
       pub const INVALID_TRANSACTION: u32 = 80;
       pub const INVALID_BLOCK: u32 = 100;
       pub const SPAM_DETECTED: u32 = 150;
       
       // Critical violations (instant ban)
       pub const PROTOCOL_VIOLATION: u32 = 500;
       pub const MALICIOUS_BEHAVIOR: u32 = 1000;
   }
   ```

### Phase 3: Decline Table (1 day)

3. **Implement decline table** in `crates/ergo-network/src/decline.rs`:
   ```rust
   use dashmap::DashMap;
   use std::time::{Duration, Instant};
   
   pub struct DeclineTable {
       declined: DashMap<ModifierId, Instant>,
       ttl: Duration,
       max_size: usize,
   }
   
   impl DeclineTable {
       pub fn new(max_size: usize, ttl: Duration) -> Self {
           Self {
               declined: DashMap::new(),
               ttl,
               max_size,
           }
       }
       
       /// Mark a modifier as declined (invalid)
       pub fn decline(&self, id: ModifierId) {
           if self.declined.len() >= self.max_size {
               self.cleanup();
           }
           self.declined.insert(id, Instant::now());
       }
       
       /// Check if a modifier was previously declined
       pub fn is_declined(&self, id: &ModifierId) -> bool {
           self.declined.get(id)
               .map(|time| time.elapsed() < self.ttl)
               .unwrap_or(false)
       }
       
       /// Remove expired entries
       pub fn cleanup(&self) {
           self.declined.retain(|_, time| time.elapsed() < self.ttl);
       }
   }
   ```

### Phase 4: Scoring Integration (1.5 days)

4. **Integrate scoring into network service**:
   ```rust
   // crates/ergo-network/src/service.rs
   
   impl NetworkService {
       /// Apply penalty to a peer
       pub fn penalize_peer(&self, peer: &SocketAddr, penalty: u32, reason: &str) {
           if let Some(mut info) = self.peers.get_mut(peer) {
               info.score.apply_penalty(penalty);
               tracing::warn!(
                   peer = %peer,
                   penalty = penalty,
                   total = info.score.penalty,
                   "Penalized peer: {}",
                   reason
               );
               
               if info.score.should_ban() {
                   self.ban_peer(peer);
               }
           }
       }
       
       /// Record successful delivery
       pub fn record_delivery(&self, peer: &SocketAddr, count: u64, response_time: Duration) {
           if let Some(mut info) = self.peers.get_mut(peer) {
               info.score.delivered_count += count;
               // Exponential moving average for response time
               let new_time = response_time.as_millis() as u64;
               info.score.avg_response_time = 
                   (info.score.avg_response_time * 7 + new_time) / 8;
           }
       }
       
       /// Record failed delivery
       pub fn record_failure(&self, peer: &SocketAddr, count: u64) {
           if let Some(mut info) = self.peers.get_mut(peer) {
               info.score.failed_count += count;
           }
       }
       
       /// Get peers sorted by priority for sync
       pub fn get_sync_peers(&self, max_count: usize) -> Vec<SocketAddr> {
           let mut peers: Vec<_> = self.peers.iter()
               .filter(|p| !p.score.should_ban())
               .map(|p| (*p.key(), p.score.priority()))
               .collect();
           
           peers.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
           peers.into_iter().take(max_count).map(|(addr, _)| addr).collect()
       }
   }
   ```

### Phase 5: Protocol Integration (1 day)

5. **Apply penalties during message handling**:
   ```rust
   // crates/ergo-sync/src/protocol.rs
   
   impl SyncProtocol {
       async fn handle_modifier(&mut self, peer: SocketAddr, modifier: Modifier) {
           // Check decline table first
           if self.decline_table.is_declined(&modifier.id()) {
               self.network.penalize_peer(&peer, Penalties::SPAM_DETECTED, "sent declined modifier");
               return;
           }
           
           match self.validate_modifier(&modifier) {
               Ok(()) => {
                   self.network.record_delivery(&peer, 1, response_time);
                   self.process_modifier(modifier).await;
               }
               Err(ValidationError::InvalidPoW) => {
                   self.decline_table.decline(modifier.id());
                   self.network.penalize_peer(&peer, Penalties::INVALID_BLOCK, "invalid PoW");
               }
               Err(ValidationError::InvalidTransaction(e)) => {
                   self.decline_table.decline(modifier.id());
                   self.network.penalize_peer(&peer, Penalties::INVALID_TRANSACTION, &e.to_string());
               }
               Err(e) => {
                   self.network.penalize_peer(&peer, Penalties::INVALID_MODIFIER, &e.to_string());
               }
           }
       }
   }
   ```

---

## Files to Create/Modify

### New Files
```
crates/ergo-network/src/penalties.rs  # Penalty definitions
crates/ergo-network/src/decline.rs    # Decline table
crates/ergo-network/src/scoring.rs    # PeerScore implementation
```

### Modified Files
```
crates/ergo-network/src/peer.rs       # Add score field
crates/ergo-network/src/service.rs    # Integrate scoring
crates/ergo-network/src/mod.rs        # Export new modules
crates/ergo-sync/src/protocol.rs      # Apply penalties
```

---

## Estimated Effort

| Task | Time |
|------|------|
| Peer score structure | 1 day |
| Penalty definitions | 0.5 days |
| Decline table | 1 day |
| Scoring integration | 1.5 days |
| Protocol integration | 1 day |
| Testing | 1 day |
| **Total** | **6 days** |

---

## Dependencies

- Basic peer management (already exists)
- Modifier validation (already exists)

---

## Test Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_penalty_decay() {
        let mut score = PeerScore::new();
        score.apply_penalty(100);
        
        // Simulate 5 minutes passing
        score.last_penalty_time = Instant::now() - Duration::from_secs(300);
        
        // Should have decayed by 50 points
        assert_eq!(score.calculate_decayed_penalty(), 50);
    }
    
    #[test]
    fn test_ban_threshold() {
        let mut score = PeerScore::new();
        
        score.apply_penalty(499);
        assert!(!score.should_ban());
        
        score.apply_penalty(1);
        assert!(score.should_ban());
    }
    
    #[test]
    fn test_peer_priority() {
        let mut good_peer = PeerScore::new();
        good_peer.delivered_count = 100;
        good_peer.avg_response_time = 50;
        
        let mut bad_peer = PeerScore::new();
        bad_peer.failed_count = 50;
        bad_peer.avg_response_time = 500;
        bad_peer.penalty = 200;
        
        assert!(good_peer.priority() > bad_peer.priority());
    }
    
    #[test]
    fn test_decline_table() {
        let table = DeclineTable::new(1000, Duration::from_secs(60));
        let id = ModifierId::from([0u8; 32]);
        
        assert!(!table.is_declined(&id));
        
        table.decline(id.clone());
        assert!(table.is_declined(&id));
    }
}
```

---

## Success Metrics

1. Misbehaving peers are penalized appropriately
2. Penalties decay over time (rehabilitation)
3. Severe violations result in bans
4. Sync prefers high-priority peers
5. Decline table prevents spam processing

---

## Scala File References

| Feature | Scala File |
|---------|------------|
| Peer scoring | `scorex-core: src/main/scala/scorex/core/network/peer/PeerInfo.scala` |
| Penalties | `src/main/scala/org/ergoplatform/network/PeerPenalties.scala` |
| Decline table | `scorex-core: src/main/scala/scorex/core/network/DeclineTable.scala` |
| Peer selection | `scorex-core: src/main/scala/scorex/core/network/PeerManager.scala` |
