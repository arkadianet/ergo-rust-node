# GAP_01: Test Coverage

## Priority: CRITICAL
## Effort: Large
## Category: Testing

---

## Description

The Rust Ergo node has approximately 87% less test coverage compared to the Scala reference implementation. While the Scala node has ~2000+ tests across 131 test files, the Rust node has only 294 tests distributed across its crates.

---

## Current State (Rust)

### Test Distribution by Crate

| Crate | Tests | Coverage Level |
|-------|-------|----------------|
| ergo-consensus | 66 | Good |
| ergo-storage | 22 | Basic |
| ergo-network | 29 | Basic |
| ergo-state | 18 | Minimal |
| ergo-sync | 15 | Minimal |
| ergo-api | 13 | Minimal |
| ergo-wallet | 23 | Basic |
| ergo-mempool | 4 | Critical Gap |
| ergo-mining | 6 | Critical Gap |
| ergo-node | 2 | Critical Gap |
| **Total** | **294** | **~15% of Scala** |

### Critical Gaps

1. **Mempool (4 tests)**: Only basic ordering tests exist
   - Missing: Double-spend detection, fee ordering edge cases, dependency tracking, expiry
   - Scala has 150+ mempool tests

2. **State (18 tests)**: Basic UTXO operations only
   - Missing: Rollback tests, state transition property tests, AVL+ tree verification
   - Scala has 200+ state tests

3. **Network (29 tests)**: Message serialization covered
   - Missing: Sync tracker, delivery tracker, peer filtering, protocol edge cases
   - Scala has 300+ network tests

4. **API (13 tests)**: Minimal handler tests
   - Missing: All route tests, error handling, JSON validation
   - Scala has 100+ API tests

5. **Storage (22 tests)**: Indexer tests only
   - Missing: Database operations, column family tests, batch writes
   - Scala has 100+ storage tests

---

## Scala Reference

### Test File Structure
```
src/test/scala/org/ergoplatform/
├── mining/
│   ├── CandidateGeneratorSpec.scala (605 lines)
│   └── ErgoMinerSpec.scala (432 lines)
├── nodeView/
│   ├── mempool/
│   │   ├── ErgoMemPoolSpec.scala (529 lines)
│   │   └── ExpirationSpecification.scala
│   ├── state/
│   │   ├── ErgoStateSpecification.scala
│   │   ├── UtxoStateSpecification.scala (553 lines)
│   │   └── DigestStateSpecification.scala
│   ├── history/
│   │   ├── VerifyADHistorySpecification.scala (502 lines)
│   │   └── HistoryTests.scala
│   └── wallet/
│       └── ErgoWalletSpec.scala (1,130 lines)
├── network/
│   ├── ErgoNodeViewSynchronizerSpecification.scala (453 lines)
│   ├── ErgoSyncInfoSpecification.scala
│   ├── ErgoSyncTrackerSpecification.scala
│   ├── DeliveryTrackerSpec.scala
│   └── PeerFilteringRuleSpecification.scala
├── http/api/routes/
│   ├── MiningApiRouteSpec.scala
│   ├── WalletApiRouteSpec.scala
│   ├── BlocksApiRouteSpec.scala
│   └── ... (10+ route specs)
└── utils/
    └── generators/ (domain-specific test data generators)
```

### Key Test Patterns in Scala

1. **Property-Based Testing**: ScalaCheck `forAll` patterns
2. **Actor Testing**: Akka TestKit for async operations
3. **Test Fixtures**: Comprehensive data generators
4. **Eventually Assertions**: 67 uses of `eventually` for async checks
5. **Test Traits**: Reusable test behaviors

---

## Impact

### Why This Matters

1. **Regression Risk**: Changes may break functionality without test coverage
2. **Consensus Safety**: Insufficient validation tests risk consensus bugs
3. **Protocol Compatibility**: Network protocol edge cases untested
4. **Production Readiness**: Cannot confidently deploy without comprehensive tests

### Specific Risks

- **Mempool**: Double-spend bugs could allow invalid transactions
- **State**: Rollback bugs could corrupt UTXO state
- **Network**: Protocol bugs could cause peer disconnections
- **API**: Invalid responses could break wallet integrations

---

## Implementation Plan

### Phase 1: Critical Path Tests (Week 1-2)

1. **Mempool Tests** (`crates/ergo-mempool/src/pool.rs`)
   - Double-spend detection tests
   - Fee ordering property tests
   - Transaction dependency tests
   - Expiry mechanism tests
   - Size limit tests

2. **State Rollback Tests** (`crates/ergo-state/src/manager.rs`)
   - Single block rollback
   - Multi-block rollback
   - Rollback after fork
   - State consistency after rollback

### Phase 2: Validation Tests (Week 3-4)

3. **Transaction Validation** (`crates/ergo-consensus/src/tx_validation.rs`)
   - Input/output conservation
   - Token minting/burning
   - Script execution edge cases
   - Invalid transaction rejection

4. **Block Validation** (`crates/ergo-consensus/src/block_validation.rs`)
   - Invalid header rejection
   - Invalid PoW rejection
   - Merkle root verification
   - Extension validation

### Phase 3: Network Tests (Week 5-6)

5. **Sync Protocol** (`crates/ergo-sync/src/protocol.rs`)
   - Header sync scenarios
   - Block download coordination
   - Chain reorganization
   - Peer timeout handling

6. **Network Protocol** (`crates/ergo-network/src/`)
   - Message serialization round-trips
   - Handshake edge cases
   - Peer scoring updates

### Phase 4: API Tests (Week 7-8)

7. **Route Tests** (`crates/ergo-api/src/handlers/`)
   - Each endpoint: valid request, invalid request, error response
   - JSON serialization/deserialization
   - Authentication where required

---

## Files to Modify/Create

### New Test Modules

```
crates/ergo-mempool/src/
├── pool.rs (add #[cfg(test)] mod tests)
├── ordering.rs (expand existing tests)
└── test_fixtures.rs (new - test data generators)

crates/ergo-state/src/
├── manager.rs (add rollback tests)
├── utxo.rs (add state transition tests)
└── test_fixtures.rs (new)

crates/ergo-consensus/src/
├── tx_validation.rs (expand tests)
├── block_validation.rs (expand tests)
└── test_generators.rs (new - property test generators)

crates/ergo-network/src/
├── message.rs (add round-trip tests)
├── handshake.rs (add edge case tests)
└── service.rs (add peer management tests)

crates/ergo-api/src/handlers/
├── blocks.rs (add route tests)
├── transactions.rs (add route tests)
├── utxo.rs (add route tests)
├── wallet.rs (add route tests)
└── mining.rs (add route tests)
```

---

## Estimated Effort

| Area | Tests Needed | Effort |
|------|--------------|--------|
| Mempool | ~50 tests | 3 days |
| State | ~50 tests | 4 days |
| Consensus | ~30 tests | 3 days |
| Network | ~40 tests | 3 days |
| API | ~50 tests | 4 days |
| **Total** | **~220 tests** | **~17 days** |

---

## Dependencies

- GAP_04 (Property-Based Testing) should be implemented first for efficient test generation
- GAP_03 (Integration Tests) provides the framework for multi-component tests

---

## Test Strategy

### Unit Test Pattern
```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_specific_behavior() {
        // Arrange
        let input = create_test_input();
        
        // Act
        let result = function_under_test(input);
        
        // Assert
        assert_eq!(result, expected_value);
    }
}
```

### Property Test Pattern (after GAP_04)
```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn property_holds_for_all_inputs(input in arb_transaction()) {
        let result = validate(input);
        prop_assert!(result.is_valid() || result.is_invalid_for_known_reason());
    }
}
```

### Async Test Pattern
```rust
#[tokio::test]
async fn test_async_operation() {
    let component = TestComponent::new();
    let result = component.async_operation().await;
    assert!(result.is_ok());
}
```

---

## Success Metrics

1. **Test Count**: Increase from 294 to 500+ tests
2. **Code Coverage**: Target 80% line coverage
3. **Critical Path**: 100% coverage for consensus and validation
4. **All Tests Pass**: `cargo test --workspace` succeeds
5. **No Regressions**: Existing functionality preserved

---

## Scala Test References

Use these Scala tests as reference implementations:

| Rust Area | Scala Reference |
|-----------|-----------------|
| Mempool | `ErgoMemPoolSpec.scala` |
| State | `UtxoStateSpecification.scala` |
| Validation | `VerifyADHistorySpecification.scala` |
| Network | `ErgoNodeViewSynchronizerSpecification.scala` |
| Wallet | `ErgoWalletSpec.scala` |
| Mining | `CandidateGeneratorSpec.scala` |
