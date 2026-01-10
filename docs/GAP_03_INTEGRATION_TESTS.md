# GAP_03: Integration Tests

## Priority: CRITICAL
## Effort: Large
## Category: Testing

---

## Description

The Rust Ergo node has an empty integration test suite. The `tests/integration/` directory exists but contains no tests. Integration tests are essential for verifying that components work correctly together and that the node behaves correctly as a complete system.

---

## Current State (Rust)

### Directory Structure
```
tests/
└── integration/  (EMPTY - no files)
```

### What Exists
- Unit tests embedded in source files
- No multi-component tests
- No end-to-end tests
- No network simulation tests
- No API integration tests

### What's Missing
- Sanity test suite (like Scala's ErgoSanity)
- Sync protocol tests (header sync, block sync)
- State transition tests
- Multi-node simulation
- API integration tests
- Fork resolution tests

---

## Scala Reference

### Sanity Test Suite

```scala
// File: src/test/scala/org/ergoplatform/sanity/ErgoSanity.scala

class ErgoSanityUTXO extends ErgoSanity[UtxoState] {
  override val historyHeight = 10
  
  "Full block application" should "correctly update state" in {
    val state = createState()
    val blocks = generateBlocks(historyHeight)
    
    blocks.foreach { block =>
      val newState = state.applyModifier(block)
      newState.rootHash shouldEqual expectedRoot(block)
    }
  }
  
  "Chain reorganization" should "restore correct state" in {
    // Apply main chain
    val mainChain = generateChain(10)
    mainChain.foreach(state.applyModifier)
    
    // Create and apply fork
    val fork = generateFork(mainChain, forkPoint = 5, length = 7)
    
    // Verify reorg happens and state is correct
    fork.foreach(state.applyModifier)
    state.rootHash shouldEqual expectedRoot(fork.last)
  }
}
```

### Sync Tests

```scala
// File: src/test/scala/org/ergoplatform/nodeView/ErgoNodeViewSynchronizerSpecification.scala

class ErgoNodeViewSynchronizerSpecification extends AkkaSpec {
  
  "Synchronizer" should "request headers from peers" in {
    val (node, peer) = createNodeAndPeer()
    
    // Trigger sync
    node ! SyncInfo(peer.headers)
    
    // Verify header request sent
    expectMsgType[GetHeaders](timeout)
  }
  
  "Synchronizer" should "handle block download" in {
    val node = createNode()
    val blocks = generateBlocks(100)
    
    // Send headers first
    blocks.map(_.header).foreach { h =>
      node ! ModifiersFromRemote(h)
    }
    
    // Then send blocks
    blocks.foreach { b =>
      node ! ModifiersFromRemote(b)
    }
    
    // Verify state updated
    eventually {
      node.history.bestHeaderHeight shouldEqual 100
    }
  }
}
```

### Multi-Node Tests

```scala
// File: src/test/scala/org/ergoplatform/nodeView/UtxoStateNodesSyncSpec.scala

class UtxoStateNodesSyncSpec extends NetworkSpec {
  
  "Two nodes" should "sync to same state" in {
    val node1 = createNode()
    val node2 = createNode()
    
    // Node1 mines blocks
    val blocks = node1.mineBlocks(10)
    
    // Connect nodes
    connectNodes(node1, node2)
    
    // Wait for sync
    eventually {
      node2.history.bestHeaderHeight shouldEqual 10
      node2.state.rootHash shouldEqual node1.state.rootHash
    }
  }
}
```

### Fork Resolution Tests

```scala
// File: src/test/scala/org/ergoplatform/nodeView/ForkResolutionSpec.scala

class ForkResolutionSpec extends NetworkSpec {
  
  "Node" should "switch to longer chain" in {
    val node = createNode()
    
    // Apply short chain
    val shortChain = generateChain(5)
    shortChain.foreach(node.applyBlock)
    
    // Apply longer chain from fork point
    val longChain = generateChain(8, forkFrom = shortChain(2))
    longChain.foreach(node.applyBlock)
    
    // Verify switch to longer chain
    node.history.bestBlockId shouldEqual longChain.last.id
  }
}
```

---

## Impact

### Why Integration Tests Matter

1. **Component Interaction**: Unit tests don't catch integration bugs
2. **State Consistency**: Full node must maintain consistent state across components
3. **Protocol Compliance**: Must behave correctly with real network peers
4. **Regression Prevention**: Catch bugs that span multiple crates

### Risks Without Integration Tests

- State corruption during sync
- Failed chain reorganizations
- Incompatible block validation
- Mempool/state desynchronization
- API returning stale data

---

## Implementation Plan

### Phase 1: Test Infrastructure (Week 1)

1. **Create test harness** in `tests/integration/`:
   ```rust
   // tests/integration/mod.rs
   mod harness;
   mod sanity;
   mod sync;
   mod api;
   mod fork;
   
   // tests/integration/harness.rs
   pub struct TestNode {
       pub state: StateManager,
       pub history: History,
       pub mempool: Mempool,
       pub network: MockNetwork,
   }
   
   impl TestNode {
       pub async fn new() -> Self { ... }
       pub async fn apply_block(&mut self, block: &Block) -> Result<()> { ... }
       pub async fn mine_block(&mut self) -> Block { ... }
       pub fn state_root(&self) -> Digest32 { ... }
   }
   ```

2. **Create block generators**:
   ```rust
   // tests/integration/generators.rs
   pub fn generate_chain(length: usize) -> Vec<Block> { ... }
   pub fn generate_fork(base: &[Block], fork_point: usize, length: usize) -> Vec<Block> { ... }
   pub fn generate_transactions(count: usize) -> Vec<Transaction> { ... }
   ```

### Phase 2: Sanity Tests (Week 2)

3. **Basic sanity tests**:
   ```rust
   // tests/integration/sanity.rs
   
   #[tokio::test]
   async fn test_block_application_updates_state() {
       let mut node = TestNode::new().await;
       let blocks = generate_chain(10);
       
       for block in &blocks {
           node.apply_block(block).await.unwrap();
           assert_eq!(node.state_root(), expected_root(block));
       }
   }
   
   #[tokio::test]
   async fn test_transaction_validation() {
       let mut node = TestNode::new().await;
       let valid_tx = generate_valid_transaction(&node);
       let invalid_tx = generate_invalid_transaction();
       
       assert!(node.mempool.add(valid_tx).is_ok());
       assert!(node.mempool.add(invalid_tx).is_err());
   }
   ```

### Phase 3: Sync Tests (Week 3)

4. **Header sync tests**:
   ```rust
   // tests/integration/sync.rs
   
   #[tokio::test]
   async fn test_header_sync() {
       let node1 = TestNode::new().await;
       let node2 = TestNode::new().await;
       
       // Node1 has headers
       let headers = generate_headers(100);
       for h in &headers {
           node1.history.add_header(h).await;
       }
       
       // Simulate sync
       let sync_info = node1.get_sync_info();
       node2.handle_sync_info(sync_info).await;
       
       // Verify headers requested and applied
       assert_eq!(node2.history.best_header_height(), 100);
   }
   
   #[tokio::test]
   async fn test_block_download() {
       let mut node = TestNode::new().await;
       let blocks = generate_chain(50);
       
       // Add headers first
       for block in &blocks {
           node.history.add_header(&block.header).await;
       }
       
       // Then download blocks
       for block in &blocks {
           node.apply_block(block).await.unwrap();
       }
       
       assert_eq!(node.state.height(), 50);
   }
   ```

### Phase 4: Fork Resolution Tests (Week 4)

5. **Chain reorganization tests**:
   ```rust
   // tests/integration/fork.rs
   
   #[tokio::test]
   async fn test_switch_to_longer_chain() {
       let mut node = TestNode::new().await;
       
       // Apply short chain
       let short_chain = generate_chain(5);
       for block in &short_chain {
           node.apply_block(block).await.unwrap();
       }
       
       // Create longer fork
       let long_chain = generate_fork(&short_chain, 2, 6);
       
       // Apply fork blocks
       for block in &long_chain {
           node.apply_block(block).await.unwrap();
       }
       
       // Verify switched to longer chain
       assert_eq!(node.history.best_block_id(), long_chain.last().unwrap().id());
   }
   
   #[tokio::test]
   async fn test_state_rollback_on_reorg() {
       let mut node = TestNode::new().await;
       
       // Apply chain with transactions
       let chain = generate_chain_with_txs(10);
       for block in &chain {
           node.apply_block(block).await.unwrap();
       }
       
       let state_before_reorg = node.state_root();
       
       // Create fork that spends same inputs differently
       let fork = generate_conflicting_fork(&chain, 5);
       for block in &fork {
           node.apply_block(block).await.unwrap();
       }
       
       // Verify state is different and consistent
       assert_ne!(node.state_root(), state_before_reorg);
       assert!(node.state.verify_consistency());
   }
   ```

### Phase 5: API Integration Tests (Week 5)

6. **API tests with real node**:
   ```rust
   // tests/integration/api.rs
   
   #[tokio::test]
   async fn test_api_reflects_state() {
       let node = TestNode::new_with_api().await;
       let blocks = generate_chain(5);
       
       for block in &blocks {
           node.apply_block(block).await.unwrap();
       }
       
       // Query API
       let response = reqwest::get(&format!("{}/info", node.api_url()))
           .await.unwrap()
           .json::<NodeInfo>().await.unwrap();
       
       assert_eq!(response.full_height, 5);
   }
   
   #[tokio::test]
   async fn test_mempool_api() {
       let node = TestNode::new_with_api().await;
       let tx = generate_valid_transaction(&node);
       
       // Submit via API
       let response = reqwest::Client::new()
           .post(&format!("{}/transactions", node.api_url()))
           .json(&tx)
           .send().await.unwrap();
       
       assert_eq!(response.status(), 200);
       
       // Verify in mempool
       let unconfirmed = reqwest::get(&format!("{}/transactions/unconfirmed", node.api_url()))
           .await.unwrap()
           .json::<Vec<Transaction>>().await.unwrap();
       
       assert!(unconfirmed.iter().any(|t| t.id() == tx.id()));
   }
   ```

---

## Files to Create

```
tests/
└── integration/
    ├── mod.rs              # Test module declarations
    ├── harness.rs          # TestNode and utilities
    ├── generators.rs       # Block/tx generators
    ├── sanity.rs           # Basic sanity tests
    ├── sync.rs             # Sync protocol tests
    ├── fork.rs             # Fork resolution tests
    ├── api.rs              # API integration tests
    └── helpers.rs          # Shared test helpers
```

### Cargo.toml Addition

```toml
[[test]]
name = "integration"
path = "tests/integration/mod.rs"

[dev-dependencies]
tokio-test = "0.4"
tempfile = "3.8"
reqwest = { version = "0.11", features = ["json"] }
```

---

## Estimated Effort

| Task | Time |
|------|------|
| Test infrastructure | 3 days |
| Sanity tests | 3 days |
| Sync tests | 4 days |
| Fork resolution tests | 3 days |
| API integration tests | 2 days |
| Documentation | 1 day |
| **Total** | **16 days** |

---

## Dependencies

- Test generators need working block/transaction creation
- API tests need the API server runnable in tests
- Fork tests need working rollback mechanism

---

## Test Strategy

### Test Categories

1. **Sanity Tests**: Verify basic operations work
2. **Sync Tests**: Verify synchronization protocol
3. **Fork Tests**: Verify chain reorganization
4. **API Tests**: Verify API reflects node state
5. **Stress Tests**: Verify behavior under load

### Running Tests

```bash
# Run all integration tests
cargo test --test integration

# Run specific test module
cargo test --test integration sanity

# Run with logging
RUST_LOG=debug cargo test --test integration -- --nocapture
```

---

## Success Metrics

1. **Test Count**: At least 50 integration tests
2. **Coverage**: All major user flows tested
3. **Stability**: Tests pass consistently (no flakiness)
4. **Speed**: Full suite runs in under 5 minutes
5. **CI Integration**: Tests run on every PR

---

## Scala Test References

| Test Category | Scala Reference |
|---------------|-----------------|
| Sanity | `org.ergoplatform.sanity.ErgoSanity` |
| Sync | `org.ergoplatform.nodeView.ErgoNodeViewSynchronizerSpecification` |
| Fork | `org.ergoplatform.nodeView.ForkResolutionSpec` |
| Multi-node | `org.ergoplatform.nodeView.UtxoStateNodesSyncSpec` |
| State | `org.ergoplatform.nodeView.state.UtxoStateSpecification` |
