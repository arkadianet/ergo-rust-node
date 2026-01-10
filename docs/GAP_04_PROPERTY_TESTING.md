# GAP_04: Property-Based Testing

## Priority: HIGH
## Effort: Medium
## Category: Testing
## Status: IMPLEMENTED

---

## Implementation Summary

**Completed on:** January 2025

Property-based testing infrastructure was added to the `ergo-tests` crate using `proptest`:

### Files Created/Modified
- `crates/ergo-tests/src/property_tests.rs` - 15 property tests
- Added `proptest = "1"` to workspace dependencies

### Property Tests Implemented

| Test | Description |
|------|-------------|
| `header_serialization_deterministic` | Header serialization produces consistent bytes |
| `header_roundtrip` | Headers survive serialize/deserialize cycle |
| `difficulty_always_positive` | Difficulty values are always > 0 |
| `nbits_encodes_valid_difficulty` | nBits encoding is valid |
| `erg_value_within_bounds` | ERG values don't exceed max supply |
| `token_amount_positive` | Token amounts are positive |
| `chain_heights_monotonic` | Chain heights increase monotonically |
| `chain_parent_links_valid` | Parent-child links are correct |
| `generated_ids_unique` | Generated IDs are unique |
| `storage_key_valid` | Storage keys are valid bytes |
| `storage_value_preserves_binary` | Binary data round-trips through storage |
| `batch_operations_atomic` | Batch writes are atomic |
| `fork_diverges_correctly` | Fork generation creates valid diverging chains |

### Proptest Strategies Created
- `arb_id_32()` - Arbitrary 32-byte arrays
- `arb_difficulty()` - Valid difficulty values
- `arb_height()` - Block heights
- `arb_timestamp()` - Timestamps in valid range
- `arb_nbits()` - Compact difficulty encoding
- `arb_version()` - Protocol versions
- `arb_erg_value()` - ERG amounts within supply limits
- `arb_token_amount()` - Token amounts

---

## Description

The Rust Ergo node lacks property-based testing infrastructure. The Scala node uses ScalaCheck extensively with 645+ uses of `forAll` and `property` patterns, along with domain-specific generators for complex types. Property-based testing is crucial for finding edge cases in consensus-critical code.

---

## Current State (Rust)

### What Exists
- Standard `#[test]` unit tests with hardcoded inputs
- No `proptest` or `quickcheck` dependency
- No type generators for complex domain types
- No shrinking for failing test cases

### What's Missing
- Property-based testing framework
- Generators for: Transaction, Block, Box, Header, Address
- Property tests for consensus rules
- Serialization round-trip tests
- Invariant verification tests

---

## Scala Reference

### Generator Infrastructure

```scala
// File: ergo-core/src/test/scala/org/ergoplatform/utils/generators/ErgoCoreTransactionGenerators.scala

trait ErgoCoreTransactionGenerators {
  
  lazy val boxIdGen: Gen[BoxId] = Gen.listOfN(32, Arbitrary.arbByte.arbitrary).map(BoxId @@ _)
  
  lazy val ergoBoxGen: Gen[ErgoBox] = for {
    value <- Gen.chooseNum(1L, Long.MaxValue / 10)
    tokens <- tokensGen
    registers <- registersGen
    creationHeight <- Gen.chooseNum(0, Int.MaxValue)
    ergoTree <- ergoTreeGen
  } yield new ErgoBox(value, ergoTree, tokens, registers, creationHeight)
  
  lazy val transactionGen: Gen[ErgoTransaction] = for {
    inputs <- Gen.listOfN(Gen.chooseNum(1, 5).sample.get, inputGen)
    dataInputs <- Gen.listOfN(Gen.chooseNum(0, 3).sample.get, dataInputGen)
    outputs <- Gen.listOfN(Gen.chooseNum(1, 10).sample.get, ergoBoxGen)
  } yield new ErgoTransaction(inputs, dataInputs, outputs)
}
```

### Property Tests

```scala
// File: src/test/scala/org/ergoplatform/nodeView/mempool/ErgoMemPoolSpec.scala

property("mempool should preserve fee ordering") {
  forAll(Gen.listOf(transactionGen)) { txs =>
    val mempool = new ErgoMemPool()
    txs.foreach(mempool.put)
    
    val ordered = mempool.getAllPrioritized
    ordered.sliding(2).forall {
      case Seq(a, b) => a.weightedFee >= b.weightedFee
      case _ => true
    }
  }
}

property("transaction validation is deterministic") {
  forAll(transactionGen) { tx =>
    val result1 = validator.validate(tx)
    val result2 = validator.validate(tx)
    result1 == result2
  }
}
```

### Shrinking

ScalaCheck automatically shrinks failing inputs to minimal counterexamples:
```scala
// If a test fails with a large transaction, ScalaCheck will try:
// - Fewer inputs
// - Fewer outputs
// - Smaller values
// Until it finds the minimal failing case
```

---

## Impact

### Why Property Testing Matters

1. **Edge Case Discovery**: Finds inputs humans wouldn't think of
2. **Consensus Safety**: Validates invariants across all inputs
3. **Regression Prevention**: Catches subtle bugs that unit tests miss
4. **Documentation**: Properties serve as executable specifications

### Specific Risks Without Property Testing

- **Difficulty adjustment**: Edge cases at epoch boundaries
- **Token conservation**: Overflow/underflow in complex transactions
- **State transitions**: Invariant violations with unusual input combinations
- **Serialization**: Malformed data that causes panics

---

## Implementation Plan

### Phase 1: Add proptest Dependency (Day 1)

1. **Add to Cargo.toml**:
   ```toml
   [dev-dependencies]
   proptest = "1.4"
   proptest-derive = "0.4"
   ```

### Phase 2: Create Core Generators (Week 1)

2. **Create generator module** in `crates/ergo-consensus/src/test_generators.rs`:

   ```rust
   use proptest::prelude::*;
   use ergo_lib::ergotree_ir::chain::ergo_box::ErgoBox;
   use ergo_lib::chain::transaction::{Transaction, Input, DataInput};
   
   // Box ID generator
   pub fn arb_box_id() -> impl Strategy<Value = BoxId> {
       prop::array::uniform32(any::<u8>()).prop_map(BoxId::from)
   }
   
   // Token generator
   pub fn arb_token() -> impl Strategy<Value = Token> {
       (arb_token_id(), 1u64..1_000_000_000u64)
           .prop_map(|(id, amount)| Token { id, amount })
   }
   
   // ErgoBox generator
   pub fn arb_ergo_box() -> impl Strategy<Value = ErgoBox> {
       (
           1u64..1_000_000_000_000u64,  // value
           prop::collection::vec(arb_token(), 0..5),  // tokens
           arb_ergo_tree(),  // script
           0i32..1_000_000i32,  // creation height
       ).prop_map(|(value, tokens, tree, height)| {
           ErgoBox::new(value, tree, tokens, NonMandatoryRegisters::empty(), height, BoxId::zero())
               .unwrap()
       })
   }
   
   // Transaction generator
   pub fn arb_transaction() -> impl Strategy<Value = Transaction> {
       (
           prop::collection::vec(arb_input(), 1..5),
           prop::collection::vec(arb_data_input(), 0..3),
           prop::collection::vec(arb_ergo_box(), 1..10),
       ).prop_map(|(inputs, data_inputs, outputs)| {
           Transaction::new(inputs, data_inputs, outputs).unwrap()
       })
   }
   
   // Block header generator
   pub fn arb_header() -> impl Strategy<Value = Header> {
       (
           any::<u8>(),  // version
           arb_block_id(),  // parent
           arb_digest32(),  // transactions root
           arb_digest32(),  // state root
           any::<u64>(),  // timestamp
           any::<u32>(),  // nBits
           any::<u32>(),  // height
           arb_autolykos_solution(),
       ).prop_map(|(ver, parent, tx_root, state_root, ts, bits, h, sol)| {
           Header::new(ver, parent, tx_root, state_root, ts, bits, h, sol)
       })
   }
   ```

### Phase 3: Consensus Property Tests (Week 2)

3. **Add difficulty adjustment properties**:
   ```rust
   // crates/ergo-consensus/src/difficulty.rs
   
   #[cfg(test)]
   mod property_tests {
       use super::*;
       use proptest::prelude::*;
       
       proptest! {
           #[test]
           fn difficulty_never_exceeds_2x_change(
               prev_difficulty in 1u64..u64::MAX/2,
               actual_time in 1u64..1_000_000u64,
               target_time in 1u64..1_000_000u64,
           ) {
               let new_diff = adjust_difficulty(prev_difficulty, actual_time, target_time);
               
               // Difficulty can at most double
               prop_assert!(new_diff <= prev_difficulty * 2);
               // Difficulty can at most halve
               prop_assert!(new_diff >= prev_difficulty / 2);
           }
           
           #[test]
           fn difficulty_adjustment_is_monotonic(
               epochs in prop::collection::vec(arb_epoch_data(), 1..16),
           ) {
               let result = calculate_difficulty(&epochs);
               
               // If blocks are faster than target, difficulty increases
               if epochs.iter().all(|e| e.actual_time < e.target_time) {
                   prop_assert!(result > epochs.last().unwrap().difficulty);
               }
           }
       }
   }
   ```

4. **Add transaction validation properties**:
   ```rust
   // crates/ergo-consensus/src/tx_validation.rs
   
   #[cfg(test)]
   mod property_tests {
       use super::*;
       use crate::test_generators::*;
       use proptest::prelude::*;
       
       proptest! {
           #[test]
           fn erg_conservation_is_verified(
               inputs in prop::collection::vec(arb_ergo_box(), 1..5),
               fee in 1_000_000u64..10_000_000u64,
           ) {
               let total_input: u64 = inputs.iter().map(|b| b.value()).sum();
               let tx = create_tx_with_outputs(&inputs, total_input - fee);
               
               let result = validate_erg_conservation(&tx, &inputs);
               prop_assert!(result.is_ok());
           }
           
           #[test]
           fn token_conservation_is_verified(
               inputs in prop::collection::vec(arb_box_with_tokens(), 1..3),
           ) {
               let tokens = aggregate_tokens(&inputs);
               let tx = create_tx_preserving_tokens(&inputs, &tokens);
               
               let result = validate_token_conservation(&tx, &inputs);
               prop_assert!(result.is_ok());
           }
           
           #[test]
           fn validation_is_deterministic(tx in arb_valid_transaction()) {
               let ctx = create_test_context();
               let result1 = validate_transaction(&tx, &ctx);
               let result2 = validate_transaction(&tx, &ctx);
               
               prop_assert_eq!(result1.is_ok(), result2.is_ok());
           }
       }
   }
   ```

### Phase 4: Serialization Round-Trip Tests (Week 3)

5. **Add serialization property tests**:
   ```rust
   // crates/ergo-network/src/message.rs
   
   #[cfg(test)]
   mod property_tests {
       use super::*;
       use proptest::prelude::*;
       
       proptest! {
           #[test]
           fn message_roundtrip(msg in arb_message()) {
               let bytes = msg.serialize();
               let parsed = Message::deserialize(&bytes)?;
               
               prop_assert_eq!(msg, parsed);
           }
           
           #[test]
           fn header_roundtrip(header in arb_header()) {
               let bytes = header.sigma_serialize_bytes()?;
               let parsed = Header::sigma_parse_bytes(&bytes)?;
               
               prop_assert_eq!(header.id(), parsed.id());
           }
           
           #[test]
           fn transaction_roundtrip(tx in arb_transaction()) {
               let bytes = tx.sigma_serialize_bytes()?;
               let parsed = Transaction::sigma_parse_bytes(&bytes)?;
               
               prop_assert_eq!(tx.id(), parsed.id());
           }
       }
   }
   ```

### Phase 5: State Invariant Tests (Week 4)

6. **Add state property tests**:
   ```rust
   // crates/ergo-state/src/utxo.rs
   
   #[cfg(test)]
   mod property_tests {
       use super::*;
       use proptest::prelude::*;
       
       proptest! {
           #[test]
           fn utxo_add_remove_is_reversible(
               boxes in prop::collection::vec(arb_ergo_box(), 1..10),
           ) {
               let mut state = UtxoState::new();
               
               // Add all boxes
               for b in &boxes {
                   state.add_box(b.clone())?;
               }
               
               let root_after_add = state.root_hash();
               
               // Remove all boxes
               for b in boxes.iter().rev() {
                   state.remove_box(&b.box_id())?;
               }
               
               let root_after_remove = state.root_hash();
               
               // Should be back to empty state
               prop_assert_eq!(root_after_remove, UtxoState::empty_root());
           }
           
           #[test]
           fn box_lookup_returns_added_box(box_ in arb_ergo_box()) {
               let mut state = UtxoState::new();
               state.add_box(box_.clone())?;
               
               let retrieved = state.get_box(&box_.box_id())?;
               
               prop_assert!(retrieved.is_some());
               prop_assert_eq!(retrieved.unwrap().value(), box_.value());
           }
       }
   }
   ```

---

## Files to Create/Modify

### New Files
```
crates/ergo-consensus/src/test_generators.rs  # Core type generators
crates/ergo-state/src/test_generators.rs      # State-specific generators
crates/ergo-network/src/test_generators.rs    # Network message generators
crates/ergo-mempool/src/test_generators.rs    # Transaction generators
```

### Modified Files
```
crates/ergo-consensus/src/difficulty.rs       # Add property tests
crates/ergo-consensus/src/tx_validation.rs    # Add property tests
crates/ergo-consensus/src/block_validation.rs # Add property tests
crates/ergo-state/src/utxo.rs                 # Add property tests
crates/ergo-network/src/message.rs            # Add round-trip tests
crates/ergo-mempool/src/pool.rs               # Add ordering tests
```

### Cargo.toml Changes
```toml
# In workspace Cargo.toml or each crate
[dev-dependencies]
proptest = "1.4"
```

---

## Estimated Effort

| Task | Time |
|------|------|
| Add proptest dependency | 0.5 days |
| Core generators | 3 days |
| Consensus property tests | 3 days |
| Serialization tests | 2 days |
| State invariant tests | 2 days |
| Mempool property tests | 1 day |
| **Total** | **11.5 days** |

---

## Dependencies

- None - can be implemented independently
- Recommended before GAP_01 (Test Coverage) for efficient test generation

---

## Test Strategy

### Property Test Pattern
```rust
proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]
    
    #[test]
    fn property_name(input in generator()) {
        // Arrange
        let system = create_system();
        
        // Act
        let result = system.operation(input);
        
        // Assert property
        prop_assert!(property_holds(result));
    }
}
```

### Running Property Tests
```bash
# Run with default config (256 cases)
cargo test

# Run with more cases for thorough testing
PROPTEST_CASES=10000 cargo test

# Run with verbose output to see generated inputs
cargo test -- --nocapture

# Reproduce a failing case (use seed from failure output)
PROPTEST_SEED="0x1234..." cargo test
```

---

## Success Metrics

1. **Generator Coverage**: Generators for all core types
2. **Property Count**: At least 50 property tests
3. **Case Count**: Default 256 cases, CI runs 1000
4. **Shrinking**: Failing cases produce minimal counterexamples
5. **No Failures**: All property tests pass

---

## Scala Test References

| Generator | Scala Reference |
|-----------|-----------------|
| Transaction | `ErgoCoreTransactionGenerators.scala` |
| Block | `ValidBlocksGenerators.scala` |
| Header | `ErgoGenerators.scala` |
| Box | `ErgoCoreTransactionGenerators.scala` |
| Address | `ErgoAddressGenerators.scala` |

| Property Test | Scala Reference |
|---------------|-----------------|
| Mempool ordering | `ErgoMemPoolSpec.scala` |
| Difficulty | `DifficultySpec.scala` |
| Transaction validation | `ErgoTransactionSpec.scala` |
| State transitions | `UtxoStateSpecification.scala` |
