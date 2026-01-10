# GAP_07: Advanced Peer Scoring

## Status: IMPLEMENTED
## Priority: LOW
## Effort: Medium
## Category: Network

---

## Implementation Summary (January 2025)

Advanced peer scoring has been fully implemented in `crates/ergo-network/`.

### Files Created

**penalties.rs** - Penalty definitions for peer misbehaviors:
- `Penalties` struct with constants for all violation types
- `Rewards` struct for good behavior rewards
- `PenaltyReason` enum for type-safe penalty application

**decline.rs** - Decline table for spam protection:
- `DeclineTable` - Thread-safe table for tracking invalid modifiers
- TTL-based expiration
- Maximum size with automatic cleanup

**scoring.rs** - Advanced peer scoring:
- `PeerScore` - Comprehensive scoring with decay
- Response time tracking (exponential moving average)
- Delivery success/failure tracking
- Priority calculation for sync operations

### Files Modified

**peer.rs** - Enhanced peer management:
- `PeerInfo` now uses `PeerScore` instead of simple `i32` score
- `PeerManager` with new methods:
  - `penalize(id, PenaltyReason)` - Apply typed penalty
  - `record_delivery(id)` / `record_failure(id)` - Track deliveries
  - `record_response_time(id, Duration)` - Track response times
  - `get_sync_peers(count)` - Priority-based peer selection
  - `unban(addr)` - Rehabilitate peers
  - `score_stats()` - Get scoring statistics
- `PeerScoreStats` for monitoring

**lib.rs** - New exports:
```rust
pub use decline::{DeclineTable, ModifierId};
pub use penalties::{Penalties, PenaltyReason, Rewards};
pub use scoring::PeerScore;
pub use peer::{PeerScoreStats, ...};
```

### Penalty Categories

| Category | Penalty | Examples |
|----------|---------|----------|
| Minor (1-10) | SLOW_RESPONSE=2, MISSING_RESPONSE=10 | Timeouts, delays |
| Moderate (20-50) | INVALID_MESSAGE_FORMAT=20, INVALID_MODIFIER=50 | Protocol errors |
| Severe (80-150) | INVALID_BLOCK=100, SPAM_DETECTED=150 | Validation failures |
| Critical (500+) | PROTOCOL_VIOLATION=500, MALICIOUS_BEHAVIOR=1000 | Instant ban |

### Scoring Features

1. **Penalty Decay**: Penalties decay at 10 points/minute
2. **Ban Threshold**: 500 points triggers temporary ban
3. **Response Time EMA**: 7/8 old + 1/8 new for smoothing
4. **Priority Calculation**: Weighted combination of:
   - Reliability (35%) - delivery success rate
   - Response time (30%) - faster is better
   - Penalty (20%) - lower is better
   - Rewards (15%) - accumulated good behavior

### Tests Added

- `penalties::tests` (3 tests)
- `decline::tests` (5 tests)
- `scoring::tests` (12 tests)
- `peer::tests` (8 tests) - enhanced

**Total: 55 tests pass in ergo-network**

---

## Description

Advanced peer scoring helps identify reliable peers, penalize misbehaving ones, and protect against various network attacks.

---

## Key Features

### Penalty Decay

Penalties decay automatically over time, allowing peers to be rehabilitated:

```rust
// After 50 minutes, a 500-point penalty decays to 0
let decay = minutes_elapsed * DECAY_PER_MINUTE; // 10 points/min
current_penalty = penalty.saturating_sub(decay);
```

### Decline Table

Prevents repeated validation of known-invalid modifiers:

```rust
let table = DeclineTable::with_defaults(); // 10k entries, 10min TTL

// When validation fails
table.decline(modifier_id, "invalid PoW");

// Before validating
if table.contains(&modifier_id) {
    penalize(peer, PenaltyReason::DeclinedModifier);
    return; // Skip expensive validation
}
```

### Priority-Based Peer Selection

Sync operations prefer reliable, fast peers:

```rust
let sync_peers = peer_manager.get_sync_peers(10);
// Returns peers sorted by priority (best first)
// Excludes peers that should be banned
```

### Response Time Tracking

Uses exponential moving average for stability:

```rust
peer_score.record_response_time(Duration::from_millis(100));
// EMA: new_avg = old_avg * 0.875 + new_value * 0.125
```

---

## Usage Examples

### Penalizing a Peer

```rust
// Type-safe penalty
let banned = peer_manager.penalize(&peer_id, PenaltyReason::InvalidBlock);
if banned {
    // Peer was banned
}

// Custom amount
peer_manager.penalize_amount(&peer_id, 50, "custom reason");
```

### Tracking Deliveries

```rust
// When modifier is received
peer_manager.record_delivery(&peer_id);
peer_manager.record_response_time(&peer_id, elapsed);

// When request times out
peer_manager.record_failure(&peer_id);
```

### Using Decline Table

```rust
use ergo_network::{DeclineTable, ModifierId};

let decline_table = DeclineTable::with_defaults();

// Check before processing
if let Some(reason) = decline_table.is_declined(&modifier_id) {
    warn!("Modifier was declined: {}", reason);
    peer_manager.penalize(&peer_id, PenaltyReason::DeclinedModifier);
    return;
}

// After validation failure
decline_table.decline(modifier_id, "invalid merkle root");
```

---

## Scala File References

| Feature | Scala File |
|---------|------------|
| Peer scoring | `scorex-core: src/main/scala/scorex/core/network/peer/PeerInfo.scala` |
| Penalties | `src/main/scala/org/ergoplatform/network/PeerPenalties.scala` |
| Decline table | `scorex-core: src/main/scala/scorex/core/network/DeclineTable.scala` |
| Peer selection | `scorex-core: src/main/scala/scorex/core/network/PeerManager.scala` |
