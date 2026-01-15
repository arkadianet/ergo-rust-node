# Ergo Rust Node Architecture

This document provides a comprehensive overview of how the Ergo Rust node operates, syncs, and processes data. It should be updated whenever significant architectural changes are made.

## Table of Contents

1. [System Overview](#system-overview)
2. [Crate Structure](#crate-structure)
3. [Node Initialization](#node-initialization)
4. [Synchronization](#synchronization)
5. [State Management](#state-management)
6. [Consensus and Validation](#consensus-and-validation)
7. [Networking](#networking)
8. [Storage](#storage)
9. [Mining](#mining)
10. [Mempool](#mempool)
11. [API](#api)
12. [Wallet](#wallet)

---

## System Overview

The Ergo Rust node is a full node implementation that:
- Synchronizes with the Ergo blockchain network
- Validates blocks and transactions
- Maintains UTXO state
- Supports mining operations
- Provides REST API for external interaction
- Includes an integrated HD wallet

### High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         ergo-node                                │
│  (CLI, Configuration, Component Orchestration)                   │
└─────────────────────────────────────────────────────────────────┘
                              │
       ┌──────────────────────┼──────────────────────┐
       │                      │                      │
       ▼                      ▼                      ▼
┌─────────────┐      ┌─────────────┐      ┌─────────────┐
│  ergo-api   │      │ ergo-mining │      │ ergo-wallet │
│  (REST API) │      │  (Mining)   │      │  (HD Keys)  │
└─────────────┘      └─────────────┘      └─────────────┘
       │                      │                      │
       └──────────────────────┼──────────────────────┘
                              │
       ┌──────────────────────┼──────────────────────┐
       │                      │                      │
       ▼                      ▼                      ▼
┌─────────────┐      ┌─────────────┐      ┌─────────────┐
│ ergo-sync   │      │ergo-mempool │      │ergo-network │
│ (Sync Proto)│      │  (Tx Pool)  │      │   (P2P)     │
└─────────────┘      └─────────────┘      └─────────────┘
       │                      │                      │
       └──────────────────────┼──────────────────────┘
                              │
       ┌──────────────────────┼──────────────────────┐
       │                      │                      │
       ▼                      ▼                      ▼
┌─────────────┐      ┌─────────────┐      ┌─────────────┐
│ergo-consensu│      │ ergo-state  │      │ergo-storage │
│  (Rules)    │      │ (UTXO/AVL+) │      │ (RocksDB)   │
└─────────────┘      └─────────────┘      └─────────────┘
```

---

## Crate Structure

### ergo-node
**Purpose**: Main binary, CLI, configuration, component lifecycle

**Key Components**:
- `main.rs`: Entry point, signal handling
- `config.rs`: TOML configuration parsing, `NodeConfig`, `ValidationConfig`
- `node.rs`: `Node` struct orchestrating all components

**Why**: Separates application concerns from core logic. Handles graceful shutdown, configuration loading, and component coordination.

### ergo-consensus
**Purpose**: Consensus rules, validation logic, protocol parameters

**Key Components**:
- `block_validation.rs`: `FullBlockValidator` for block/transaction validation
- `header_validation.rs`: Header chain validation
- `pow.rs`: Autolykos v2 PoW verification
- `difficulty.rs`: Difficulty adjustment algorithm
- `chain_params.rs`: Dynamic protocol parameters from extensions
- `block.rs`: Block structures, extension parsing

**Why**: Consensus rules must be isolated and well-tested. Any deviation causes chain splits.

### ergo-state
**Purpose**: Blockchain state management, UTXO set, history

**Key Components**:
- `manager.rs`: `StateManager` coordinating UTXO and history
- `utxo.rs`: `UtxoState` with AVL+ tree for box storage
- `history.rs`: `History` for headers, blocks, extensions
- `rollback.rs`: `UndoData` for chain reorganizations

**Why**: State is the source of truth. Must support efficient queries and rollbacks.

### ergo-sync
**Purpose**: Block synchronization protocol

**Key Components**:
- `protocol.rs`: `SyncProtocol` state machine
- `headers.rs`: Header-first sync logic
- `blocks.rs`: Block download scheduling
- `tasks.rs`: Download task management

**Why**: Sync is complex with parallel downloads, timeouts, and reorganizations. Isolated for testability.

### ergo-network
**Purpose**: P2P networking, peer management

**Key Components**:
- `peer.rs`: `Peer` connection handling
- `protocol.rs`: Message serialization/deserialization
- `manager.rs`: `PeerManager` for connection pool
- `messages.rs`: P2P message types

**Why**: Network code is security-critical. Must match Scala node exactly for compatibility.

### ergo-storage
**Purpose**: Persistent storage abstraction

**Key Components**:
- `rocks.rs`: RocksDB wrapper
- `columns.rs`: Column family definitions
- `blocks.rs`: Block storage operations
- `headers.rs`: Header chain storage

**Why**: Storage abstraction allows testing with mock backends and potential future storage migrations.

### ergo-mempool
**Purpose**: Unconfirmed transaction pool

**Key Components**:
- `lib.rs`: `Mempool` with fee-ordered transactions
- `validation.rs`: Pre-acceptance validation

**Why**: Mempool prevents spam and optimizes block building by fee.

### ergo-mining
**Purpose**: Block production support

**Key Components**:
- `candidate.rs`: Block candidate generation
- `coinbase.rs`: Coinbase transaction creation
- `selection.rs`: Transaction selection by fee

**Why**: Mining requires careful transaction selection and proper coinbase construction.

### ergo-api
**Purpose**: REST API server

**Key Components**:
- `server.rs`: Axum HTTP server
- `routes/`: Endpoint handlers
- `types.rs`: JSON request/response types

**Why**: API compatibility with Scala node enables ecosystem tool reuse.

### ergo-wallet
**Purpose**: Key management, transaction building

**Key Components**:
- `wallet.rs`: HD wallet with BIP32/39
- `derivation.rs`: EIP-3 compliant paths
- `builder.rs`: Transaction construction
- `encryption.rs`: Key storage encryption

**Why**: Wallet enables self-custody and transaction signing without external tools.

---

## Node Initialization

### Startup Sequence

```
1. Parse CLI arguments (clap)
2. Load configuration (TOML)
3. Initialize storage (RocksDB)
4. Create StateManager
   ├── Load UTXO state
   ├── Load header chain
   └── Initialize chain parameters from stored extensions
5. Create Mempool
6. Start NetworkManager
   ├── Bind to port
   └── Connect to known peers
7. Start SyncProtocol
8. Start API server
9. (Optional) Start Mining
10. Enter main event loop
```

### Chain Parameter Initialization

On startup, the node must reconstruct the current protocol parameters by scanning epoch boundaries:

```rust
fn init_parameters_from_storage(&self) {
    let current_height = self.utxo.height();
    let epoch_length = 1024;  // VOTING_EPOCH_LENGTH
    
    // Find most recent epoch boundary
    let most_recent_epoch = (current_height / epoch_length) * epoch_length;
    
    // Build parameters by scanning epoch boundaries
    let mut params = ChainParameters::default();
    let mut epoch = epoch_length;
    
    while epoch <= most_recent_epoch {
        if let Some(block) = self.get_block_at_height(epoch) {
            if let Ok(parsed) = block.extension.parse_parameters() {
                params = ChainParameters::from_extension(epoch, &parsed, &params);
            }
        }
        epoch += epoch_length;
    }
    
    *self.current_parameters.write() = params;
}
```

**Why**: Parameters are voted by miners and stored in block extensions at epoch boundaries. The node must reconstruct state by replaying these votes.

---

## Synchronization

### Sync Modes

The node supports two state types:

1. **UTXO Mode** (default): Full state storage
   - Downloads: Headers + BlockTransactions + Extensions
   - Stores: Complete UTXO set
   - Validates: Full transaction execution

2. **Digest Mode**: Proof-based validation
   - Downloads: Headers + BlockTransactions + Extensions + ADProofs
   - Stores: State root hash only
   - Validates: Using authenticated data proofs

### Header-First Sync

```
Phase 1: Header Sync
┌─────────┐     SyncInfo(V2)      ┌─────────┐
│  Local  │ ──────────────────▶   │  Peer   │
│  Node   │                       │         │
│         │ ◀────────────────── │         │
└─────────┘     InvData(headers)  └─────────┘
     │
     │  RequestModifier(headers)
     ▼
┌─────────┐
│ Validate│ PoW, difficulty, chain
│ Headers │
└─────────┘
     │
     ▼
Phase 2: Block Sync
┌─────────┐  RequestModifier(txs,ext)  ┌─────────┐
│  Local  │ ──────────────────────────▶ │  Peer   │
│  Node   │                             │         │
│         │ ◀────────────────────────── │         │
└─────────┘     ModifiersData           └─────────┘
     │
     ▼
┌─────────┐
│ Validate│ Execute transactions
│ Blocks  │ Update UTXO state
└─────────┘
```

**Why Header-First**: 
- Headers are small (~200 bytes) and can be validated quickly (PoW check)
- Allows parallel block downloads once header chain is known
- Detects invalid chains early without downloading full blocks

### Block Download Strategy

```rust
// Request both BlockTransactions and Extension for each block
fn on_request_blocks(&mut self, header: &Header) {
    // Compute section IDs from header roots
    let tx_id = compute_block_section_id(
        ModifierType::BlockTransactions,
        &header.id,
        &header.transactions_root,
    );
    let ext_id = compute_block_section_id(
        ModifierType::Extension,
        &header.id,
        &header.extension_root,
    );
    
    // Queue both for download
    self.download_manager.queue(tx_id, ModifierType::BlockTransactions);
    self.download_manager.queue(ext_id, ModifierType::Extension);
}
```

**Why**: Extensions contain voted protocol parameters. Without them, the node cannot validate transactions correctly after parameter changes.

### Parallel Download

- Multiple blocks downloaded simultaneously (configurable parallelism)
- Distributed across multiple peers to avoid overloading single peer
- Timeout and retry logic for failed downloads
- Out-of-order arrival handled via pending buffer

### Chain Reorganization

When a competing chain with higher cumulative difficulty is found:

```
1. Identify fork point (common ancestor)
2. Generate UndoData for blocks being removed
3. Rollback UTXO state to fork point
4. Apply new chain blocks
5. Update best header pointer
```

**Why**: Nakamoto consensus requires following the chain with most cumulative work.

---

## State Management

### UTXO State

The UTXO (Unspent Transaction Output) state tracks all spendable boxes:

```rust
pub struct UtxoState {
    /// AVL+ tree storing box_id -> serialized_box
    tree: AvlTree,
    /// Current state root hash
    root_hash: Digest32,
    /// Height of last applied block
    height: u32,
}
```

**Operations**:
- `get_box(box_id)`: Retrieve box by ID
- `apply_block(block)`: Spend inputs, create outputs
- `rollback(undo_data)`: Reverse block application

### State Root Verification

The state root is an AVL+ tree root hash that commits to the entire UTXO set:

```
State Root = Hash(AVL+ Tree Root)

AVL+ Tree Structure:
       [root]
      /      \
   [node]   [node]
   /    \
[box1] [box2] ...
```

**Why**: State roots enable:
- Consensus on state (all nodes must agree)
- Light client proofs
- Snapshot verification

### Box Indexing

Additional indexes for efficient queries:

1. **ErgoTree Hash Index**: Find boxes by spending condition
2. **Token ID Index**: Find boxes containing specific tokens
3. **Address Index**: Find boxes by owner address

```
Column Families:
├── Boxes:        box_id -> serialized_box
├── ErgoTreeIdx:  ergotree_hash -> [box_id, ...]
├── TokenIdx:     token_id -> [box_id, ...]
└── AddressIdx:   address_hash -> [box_id, ...]
```

### Rollback Support

Each block application generates `UndoData`:

```rust
pub struct UndoData {
    /// Boxes that were spent (need to restore)
    spent_boxes: Vec<ErgoBox>,
    /// Boxes that were created (need to remove)
    created_box_ids: Vec<BoxId>,
}
```

**Why**: Chain reorganizations require reversing state changes. UndoData provides the information needed without re-executing transactions.

---

## Consensus and Validation

### Block Validation Pipeline

```
1. Header Validation
   ├── Version check
   ├── Parent exists and is valid
   ├── Height = parent.height + 1
   ├── Timestamp > parent.timestamp
   ├── nBits matches difficulty adjustment
   └── PoW solution valid

2. Extension Validation
   ├── Merkle root matches header.extension_root
   └── Key-value pairs well-formed

3. Transaction Validation (for each tx)
   ├── Inputs exist in UTXO or current block
   ├── Data inputs exist
   ├── Scripts execute successfully
   ├── ERG conservation: inputs >= outputs + fee
   ├── Token conservation: no token creation (except coinbase)
   └── Cost within limit (accumulated + tx_cost <= max_block_cost)

4. Block Validation
   ├── Transactions merkle root matches header
   ├── First tx is valid coinbase
   └── Total block cost within limit
```

### Transaction Cost Model

Each transaction has a cost based on:

```rust
cost = base_cost 
     + (num_inputs * input_cost)
     + (num_data_inputs * data_input_cost)
     + (num_outputs * output_cost)
     + (num_tokens * token_access_cost)
     + script_execution_cost
```

**Parameters** (from block extensions):
- `input_cost`: 2,000
- `data_input_cost`: 100
- `output_cost`: 100
- `token_access_cost`: 100

**Limits**:
- `max_block_cost`: Dynamic, voted by miners (started at 1,000,000, increased over time)
- `max_transaction_cost` (mempool): Configurable, default 4,900,000

### Protocol Parameters

Parameters are stored in block extensions and updated at epoch boundaries (every 1024 blocks):

```rust
pub struct ChainParameters {
    pub height: u32,
    pub max_block_cost: u64,        // ID: 4
    pub storage_fee_factor: u64,    // ID: 1
    pub min_value_per_byte: u64,    // ID: 2
    pub max_block_size: u64,        // ID: 3
    pub token_access_cost: u64,     // ID: 5
    pub input_cost: u64,            // ID: 6
    pub data_input_cost: u64,       // ID: 7
    pub output_cost: u64,           // ID: 8
}
```

**Extension Format**:
- Key: 2 bytes `[0x00, param_id]` (0x00 = SystemParametersPrefix)
- Value: 4 bytes big-endian i32

**Why Dynamic Parameters**: Allows protocol upgrades without hard forks. Miners vote on parameter changes through extension fields.

### Autolykos v2 PoW

Memory-hard proof-of-work algorithm:

```
1. Build ~2GB table from block header
2. Find nonce where:
   - Select k=32 elements from table using nonce
   - Sum of elements < target (from nBits)
3. Solution: nonce + 32 element indices
```

**Why Autolykos**: ASIC-resistant design promotes decentralization by allowing GPU mining.

### Difficulty Adjustment

Adjusted every epoch (1024 blocks) using linear least squares regression:

```
1. Collect timestamps and difficulties for last 8 epochs
2. Fit linear model: difficulty = a * time + b
3. Calculate target difficulty for 2-minute blocks
4. Clamp adjustment to 2x per epoch
```

**Why**: Smooth difficulty adjustment prevents timestamp manipulation and maintains stable block times.

---

## Networking

### P2P Protocol

Message-based protocol over TCP:

```
Message Frame:
┌──────────┬──────────┬──────────┬───────────┐
│ Magic(4) │ Code(1)  │ Length(4)│ Payload   │
├──────────┼──────────┼──────────┼───────────┤
│ Network  │ Message  │ Payload  │ [Checksum]│
│ ID       │ Type     │ Size     │ + Data    │
└──────────┴──────────┴──────────┴───────────┘

Checksum only present if length > 0
```

**Message Types**:
| Code | Name | Purpose |
|------|------|---------|
| 1 | GetPeers | Request peer addresses |
| 2 | Peers | Peer address list |
| 22 | RequestModifier | Request block sections |
| 33 | Modifier | Block section data |
| 55 | Inv | Inventory announcement |
| 65 | SyncInfo | Chain status for sync |
| 75 | Handshake | Connection establishment |

### Handshake Protocol

```
1. Connect TCP
2. Send Handshake (agent, version, features, timestamp)
3. Receive Handshake
4. Validate peer (network ID, protocol version)
5. Connection established
```

### Peer Management

```rust
pub struct PeerManager {
    /// Active connections
    peers: HashMap<PeerId, PeerHandle>,
    /// Known peer addresses
    known_peers: HashSet<SocketAddr>,
    /// Banned peers
    banned: HashMap<PeerId, BanReason>,
    /// Connection limits
    max_connections: usize,
}
```

**Peer Selection**:
- Prefer peers with higher reported height during sync
- Distribute requests across peers to avoid overloading
- Track peer quality (latency, success rate)

---

## Storage

### RocksDB Layout

```
Column Families:
├── Headers         # header_id -> serialized_header
├── HeadersByHeight # height -> header_id
├── Blocks          # block_id -> serialized_block_transactions
├── Extensions      # block_id -> serialized_extension
├── Boxes           # box_id -> serialized_box
├── BoxesByErgoTree # ergotree_hash -> box_ids
├── BoxesByToken    # token_id -> box_ids
├── UndoData        # block_id -> undo_data
├── StateRoot       # "root" -> state_root_hash
└── Meta            # various metadata
```

### Write-Ahead Logging

RocksDB WAL ensures durability:
- Writes logged before applied
- Recovery replays WAL on crash
- Configurable sync behavior

### Batch Operations

Block application uses atomic batches:

```rust
fn apply_block(&mut self, block: &FullBlock) -> Result<()> {
    let mut batch = WriteBatch::new();
    
    // Spend inputs
    for input in block.inputs() {
        batch.delete(CF::Boxes, input.box_id);
    }
    
    // Create outputs
    for output in block.outputs() {
        batch.put(CF::Boxes, output.box_id, serialize(output));
    }
    
    // Store undo data
    batch.put(CF::UndoData, block.id, serialize(undo_data));
    
    // Atomic commit
    self.db.write(batch)?;
    Ok(())
}
```

**Why Batches**: Ensures state consistency. Either all changes apply or none do.

---

## Mining

### Block Candidate Generation

```rust
pub fn generate_candidate(
    state: &StateManager,
    mempool: &Mempool,
    reward_address: &Address,
) -> BlockCandidate {
    // 1. Get current chain tip
    let parent = state.best_header();
    
    // 2. Calculate difficulty
    let n_bits = calculate_difficulty(parent, state);
    
    // 3. Select transactions by fee
    let txs = mempool.select_transactions(max_block_cost);
    
    // 4. Create coinbase
    let coinbase = create_coinbase(parent.height + 1, reward_address);
    
    // 5. Build candidate
    BlockCandidate {
        parent_id: parent.id,
        height: parent.height + 1,
        n_bits,
        transactions: [coinbase].chain(txs).collect(),
        extension: build_extension(),
    }
}
```

### Transaction Selection

Greedy selection by fee density (fee / cost):

```rust
fn select_transactions(&self, max_cost: u64) -> Vec<Transaction> {
    let mut selected = Vec::new();
    let mut total_cost = 0;
    
    // Transactions ordered by fee/cost ratio
    for tx in self.by_fee_ratio() {
        if total_cost + tx.cost <= max_cost {
            selected.push(tx);
            total_cost += tx.cost;
        }
    }
    
    selected
}
```

### Coinbase Transaction

Special first transaction in each block:

```rust
fn create_coinbase(height: u32, reward_address: &Address) -> Transaction {
    let reward = emission_at_height(height);
    
    Transaction {
        inputs: vec![],  // No inputs for coinbase
        outputs: vec![
            ErgoBox {
                value: reward,
                ergo_tree: reward_address.script(),
                creation_height: height,
                // ...
            }
        ],
    }
}
```

**Emission Schedule**:
- Blocks 1-525,600: 75 ERG
- After: Decreases 3 ERG per 64,800 blocks
- Minimum: 3 ERG

---

## Mempool

### Structure

```rust
pub struct Mempool {
    /// Transactions by ID
    transactions: HashMap<TxId, UnconfirmedTx>,
    /// Fee-ordered index
    by_fee: BTreeSet<(FeeRatio, TxId)>,
    /// Dependency tracking
    dependencies: HashMap<BoxId, Vec<TxId>>,
    /// Size limits
    max_size: usize,
    max_cost: u64,
}
```

### Transaction Acceptance

```
1. Basic validation
   ├── Size within limit
   ├── Cost within limit (configurable max_transaction_cost)
   └── Fee sufficient

2. Input validation
   ├── Inputs exist (UTXO or mempool)
   └── No double-spend

3. Script validation
   └── All input scripts pass

4. Add to pool
   ├── Index by fee
   └── Track dependencies
```

### Eviction Policy

When pool is full:
1. Remove lowest fee transactions first
2. Remove transactions with unconfirmed dependencies that were evicted
3. Maintain minimum fee threshold

---

## API

### Endpoint Categories

```
/info                 # Node information
/blocks               # Block queries
/transactions         # Transaction submission
/wallet               # Wallet operations
/mining               # Mining operations
/peers                # Peer management
/utils                # Utilities (address validation, etc.)
```

### Authentication

API key authentication for sensitive endpoints:

```rust
async fn check_api_key(
    headers: &HeaderMap,
    config: &ApiConfig,
) -> Result<(), ApiError> {
    let key = headers
        .get("api_key")
        .ok_or(ApiError::Unauthorized)?;
    
    if key != config.api_key {
        return Err(ApiError::Unauthorized);
    }
    
    Ok(())
}
```

### Rate Limiting

Configurable per-endpoint rate limits to prevent abuse.

---

## Wallet

### HD Derivation (EIP-3)

```
Master Key (from mnemonic)
    │
    └── m/44'/429'/0'/0/0  (first address)
    └── m/44'/429'/0'/0/1  (second address)
    └── m/44'/429'/0'/0/2  (third address)
    ...

Path components:
- 44': BIP44 purpose
- 429': Ergo coin type (registered)
- 0': Account (hardened)
- 0: External chain
- N: Address index
```

### Key Storage

Encrypted with AES-256-GCM:

```rust
pub struct EncryptedWallet {
    /// Argon2id-derived key encryption key
    kek: [u8; 32],
    /// AES-GCM encrypted mnemonic
    encrypted_mnemonic: Vec<u8>,
    /// Nonce for AES-GCM
    nonce: [u8; 12],
}
```

**Key Derivation Parameters** (OWASP recommendations):
- Memory: 64 MB
- Iterations: 3
- Parallelism: 4

### Transaction Building

```rust
pub fn build_transaction(
    wallet: &Wallet,
    outputs: Vec<ErgoBox>,
    fee: u64,
) -> Result<Transaction> {
    // 1. Calculate required ERG
    let required = outputs.iter().map(|o| o.value).sum::<u64>() + fee;
    
    // 2. Select input boxes
    let inputs = wallet.select_boxes(required)?;
    
    // 3. Calculate change
    let change = inputs.total_value() - required;
    
    // 4. Build outputs (including change)
    let mut all_outputs = outputs;
    if change > 0 {
        all_outputs.push(wallet.change_box(change)?);
    }
    
    // 5. Sign transaction
    let unsigned = UnsignedTransaction::new(inputs, all_outputs);
    wallet.sign(unsigned)
}
```

---

## Configuration Reference

```toml
[node]
# State type: "utxo" or "digest"
state_type = "utxo"
# Network: "mainnet" or "testnet"
network = "mainnet"
# Data directory
data_dir = ".ergo"

[validation]
# Maximum transaction cost for mempool (default: 4,900,000)
max_transaction_cost = 4900000
# Maximum transaction size in bytes (default: 98,304)
max_transaction_size = 98304

[network]
# Bind address for P2P
bind_address = "0.0.0.0:9030"
# Maximum peer connections
max_connections = 30
# Known peers for bootstrap
known_peers = ["213.239.193.208:9030", "159.65.11.55:9030"]

[api]
# API bind address
bind_address = "127.0.0.1:9053"
# API key for authenticated endpoints
api_key = "your-secret-key"

[mining]
# Enable mining
enabled = false
# Use external miner
use_external_miner = true
# Reward address
reward_address = "9f..."

[wallet]
# Wallet data directory
data_dir = ".ergo-wallet"
# Pre-generate addresses
pre_generate = 20
```

---

## Maintenance Notes

### When to Update This Document

Update this document when:
1. Adding new crates or major components
2. Changing sync strategy or validation rules
3. Modifying storage schema
4. Adding new protocol message types
5. Changing parameter handling

### Testing Checklist

Before releasing changes:
1. Unit tests pass: `cargo test --workspace`
2. Sync test: Full sync to mainnet tip
3. Compatibility: Connect to Scala nodes
4. State roots: Match reference implementation
