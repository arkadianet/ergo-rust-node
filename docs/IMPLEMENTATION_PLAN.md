# Ergo Rust Node - Implementation Plan

## Overview

This document outlines the implementation plan for the Ergo Rust Node, a full blockchain node implementation with feature parity to the [Scala reference implementation](https://github.com/ergoplatform/ergo).

## Current Status

**Last Updated:** January 10, 2025

The project has completed all major implementation phases. The node is fully functional with header/block synchronization, mining, wallet, and advanced features including UTXO snapshot sync, NiPoPoW support, extra indexer, and EIP-27 re-emission.

| Crate | Status | Description |
|-------|--------|-------------|
| ergo-storage | COMPLETE | RocksDB storage layer with column families, extra indexer |
| ergo-consensus | COMPLETE | Autolykos v2 PoW, difficulty adjustment, validation, NiPoPoW, re-emission |
| ergo-state | COMPLETE | UTXO state with AVL tree, history management, box indexing, snapshots |
| ergo-mempool | COMPLETE | Transaction pool with fee ordering, dependency tracking |
| ergo-network | COMPLETE | P2P protocol, TCP handling, handshake, peer management |
| ergo-sync | COMPLETE | Sync state machine, header sync, block download, snapshot sync |
| ergo-api | COMPLETE | REST API endpoints (core + extended + blockchain indexer) |
| ergo-mining | COMPLETE | Block candidate generation, coinbase with EIP-27, tx selection |
| ergo-wallet | COMPLETE | HD wallet, transaction building, signing |
| ergo-node | COMPLETE | Main binary with all components wired up, block pruning |

**Overall Completion: ~98%**

---

## Completed Work

### Phase 1: Core Infrastructure

- [x] Workspace structure with 10 crates
- [x] Integration with sigma-rust types (ergo-lib, ergo-chain-types)
- [x] RocksDB storage with column families
- [x] Error types and result handling
- [x] Configuration system (TOML-based)
- [x] Logging with tracing

### Phase 2: State Management

- [x] UTXO State with AVL tree integration via ergo_avltree_rust
- [x] State root calculation and verification
- [x] Box indexing (ErgoTree hash, Token ID)
- [x] History management with header/block storage
- [x] Cumulative difficulty for fork selection
- [x] Fork detection via find_fork_height

### Phase 3: Consensus

- [x] Autolykos v2 PoW verification
- [x] Difficulty adjustment (linear least squares over 8 epochs)
- [x] Header and block validation
- [x] Transaction validation (inputs, scripts, token/ERG conservation)

### Phase 4: Networking

- [x] P2P message types (Handshake, GetPeers, Peers, SyncInfo, Inv, RequestModifier, Modifier)
- [x] Message codec with framing and checksums
- [x] Peer management with scoring
- [x] Connection pool management

### Phase 5: Synchronization

- [x] Header-first sync strategy
- [x] Block download scheduler with parallel downloads
- [x] Rate limiting (matches Scala: 400 items, 100ms interval)
- [x] Chain reorganization handling

### Phase 6: Node Binary

- [x] Component initialization and coordination
- [x] Event routing between network, sync, and state
- [x] Graceful shutdown
- [x] Peer reconnection with exponential backoff

### Phase 7: Mining

- [x] Block candidate generation with proper difficulty
- [x] Transaction selection by fee from mempool
- [x] Coinbase transaction creation with emission schedule
- [x] External miner support

### Phase 8: API (Core Endpoints)

- [x] /info - Node information
- [x] /blocks - Block queries
- [x] /transactions - Transaction submission
- [x] /utxo - UTXO queries
- [x] /peers - Peer management
- [x] /mining - Mining interface (candidate, solution, rewardAddress)
- [x] /wallet - Wallet operations (init, unlock, lock, status, balances, addresses, send)

### Phase 9: Wallet

- [x] HD key derivation (BIP32/BIP39, EIP-3 compliant)
- [x] Mnemonic support (12-24 words)
- [x] Transaction building with box selection
- [x] Transaction signing via ergo-lib
- [x] AES-256-GCM encryption with Argon2id key derivation

### Phase 10: Mempool

- [x] Fee-based transaction ordering
- [x] Double-spend detection
- [x] Size limits and expiry

### Phase 11: Transaction Dependency Tracking

- [x] Weight field in PooledTransaction
- [x] Parent-child relationship tracking via output_to_tx mapping
- [x] Recursive weight propagation (update_family)
- [x] Weight-based ordering (parents before children)
- [x] DoS protection (MAX_PARENT_SCAN_DEPTH, MAX_PARENT_SCAN_TIME_MS)

### Phase 12: Block Pruning

- [x] blocks_to_keep configuration (-1 = keep all)
- [x] PruningConfig in History
- [x] minimal_full_block_height tracking
- [x] Pruning logic respecting voting epoch boundaries (every 1024 blocks)
- [x] should_download_block_at_height() check for sync

### Phase 13: Extended API Endpoints

- [x] /utils/* (seed generation, blake2b hashing, address validation/conversion)
- [x] /emission/* (emission info at height, emission scripts with EIP-27)
- [x] /script/* (p2sAddress, p2shAddress, addressToTree, addressToBytes)

### Phase 14: UTXO Snapshot Sync

- [x] SnapshotsDb for manifest/chunk storage
- [x] Manifest and subtree serialization
- [x] Download plan management with chunk tracking
- [x] P2P messages (GetSnapshotsInfo, SnapshotsInfo, GetManifest, Manifest, GetUtxoSnapshotChunk, UtxoSnapshotChunk)
- [x] Sync module integration with SyncState::DownloadingSnapshot
- [x] Snapshot peer registration and tracking
- [x] Configurable snapshot sync enable/disable

### Phase 15: NiPoPoW Support

- [x] PoPowHeader with interlinks serialization
- [x] NipopowAlgos (max_level_of, interlinks packing, best_arg, lowest_common_ancestor)
- [x] NipopowProof struct with validation and comparison
- [x] NipopowVerifier state machine for bootstrap
- [x] P2P messages (GetNipopowProof, NipopowProof)
- [x] API endpoints (/nipopow/proof/{m}/{k}, /nipopow/popowHeaderById/{headerId})
- [x] Comprehensive test suite (38 tests)

### Phase 16: Extra Indexer

- [x] IndexedErgoBox with spending info, global index, tokens
- [x] IndexedErgoTransaction with input/output indexes
- [x] IndexedErgoAddress with balance tracking (nanoERG + tokens)
- [x] IndexedToken with creation info and box tracking
- [x] ExtraIndexer with index_output, spend_box, index_transaction, register_token
- [x] Atomic flush with WriteBatch
- [x] Column families (ExtraIndex, BoxIndex, TxNumericIndex)
- [x] /blockchain/* API endpoints (indexedHeight, transaction/byId, box/byId, box/byIndex, token/byId, balance/byAddress, box/byAddress, box/byTokenId)
- [x] Pagination support with offset/limit

### Phase 17: Re-emission (EIP-27)

- [x] ReemissionSettings with mainnet/testnet configurations
- [x] ReemissionRules with emission_at_height, reemission_for_height, miner_reward_at_height
- [x] Claimable re-emission after height 2,080,800 (3 ERG/block)
- [x] Total miner income calculation (direct + claimable)
- [x] O(1) issued_coins_after_height using closed-form arithmetic series
- [x] Updated emission API with re-emission amounts
- [x] CoinbaseBuilder with EIP-27 support
- [x] Lazy static rules for API performance

---

## Remaining Work

### Production Hardening

- [ ] Full integration testing with Scala node
- [ ] Stress testing and benchmarking
- [ ] Memory profiling and optimization
- [ ] Documentation improvements

### Optional Enhancements

- [ ] Scan API (/scan/register, /scan/deregister, /scan/listAll, /scan/unspentBoxes, /scan/spentBoxes)
- [ ] Script execution API (/script/executeWithContext) - endpoint exists, needs full implementation
- [ ] Prometheus metrics export
- [ ] Docker containerization

---

## Test Coverage

| Crate | Tests | Status |
|-------|-------|--------|
| ergo-consensus | 112 | PASSING |
| ergo-mempool | 22 | PASSING |
| ergo-network | 19 | PASSING |
| ergo-state | 37 | PASSING |
| ergo-sync | 15 | PASSING |
| ergo-mining | 6 | PASSING |
| ergo-wallet | 23 | PASSING |
| ergo-storage | 22 | PASSING |
| ergo-api | 13 | PASSING |
| ergo-node | 2 | PASSING |
| **Total** | **~280** | **PASSING** |

---

## Performance Targets

| Metric | Target | Current |
|--------|--------|---------|
| Block validation | < 100ms | TBD |
| Transaction validation | < 10ms | TBD |
| State lookup | < 1ms | TBD |
| API response | < 50ms | TBD |
| Header sync speed | > 100 headers/sec | ~17 headers/sec |
| Block sync speed | > 100 blocks/sec | TBD |
| Memory (full node) | < 4GB | TBD |

---

## Dependencies

### From sigma-rust
- `ergo-lib` - Core types, wallet utilities, HD derivation
- `ergotree-interpreter` - ErgoScript execution
- `ergo-chain-types` - Headers, BlockId, basic types
- `sigma-ser` - Binary serialization
- `ergo-nipopow` - NiPoPoW proofs
- `ergo_avltree_rust` - AVL+ tree implementation

### External
- `tokio` - Async runtime
- `rocksdb` - Storage backend
- `axum` - HTTP server
- `k256` - secp256k1 cryptography
- `blake2` - Hashing
- `tokio-util` - Codec support
- `once_cell` - Static lazy initialization

---

## Milestones

| Milestone | Status |
|-----------|--------|
| M1: Compile & Basic Tests | COMPLETE |
| M2: Core Components | COMPLETE |
| M3: Sync Headers | COMPLETE |
| M4: Sync Blocks | COMPLETE |
| M5: Mining Support | COMPLETE |
| M6: Wallet Complete | COMPLETE |
| M7: API Complete | COMPLETE |
| M8: Advanced Features | COMPLETE |
| M9: Production Ready | IN PROGRESS |

---

## Architecture

```
                    +-----------------------------------------------------+
                    |                    ergo-node                        |
                    |         (CLI, Configuration, Coordination)          |
                    +-----------------------------------------------------+
                                           |
           +-------------------------------+-------------------------------+
           |                               |                               |
           v                               v                               v
+---------------------+      +---------------------+      +---------------------+
|    ergo-network     |      |     ergo-sync       |      |     ergo-api        |
|  (P2P, Handshake,   |<---->|  (Sync Protocol,    |      |   (REST Server,     |
|   Peer Management)  |      |   Block Download,   |      |    Handlers,        |
|                     |      |   Snapshot Sync)    |      |    Blockchain API)  |
+---------------------+      +---------------------+      +---------------------+
                                       |
           +---------------------------+---------------------------+
           |                           |                           |
           v                           v                           v
+---------------------+      +---------------------+      +---------------------+
|   ergo-consensus    |      |    ergo-state       |      |    ergo-mempool     |
|  (PoW, Difficulty,  |<---->|  (UTXO, History,    |<---->|   (Tx Pool, Fee     |
|   Validation,       |      |   Rollback,         |      |    Ordering,        |
|   NiPoPoW,          |      |   Snapshots)        |      |    Dependencies)    |
|   Re-emission)      |      |                     |      |                     |
+---------------------+      +---------------------+      +---------------------+
                                       |
           +---------------------------+---------------------------+
           |                           |                           |
           v                           v                           v
+---------------------+      +---------------------+      +---------------------+
|    ergo-mining      |      |   ergo-storage      |      |    ergo-wallet      |
|  (Candidates,       |      |  (RocksDB, Column   |      |  (HD Keys, TX       |
|   Coinbase,         |      |   Families,         |      |   Building,         |
|   EIP-27)           |      |   Extra Indexer)    |      |   Signing)          |
+---------------------+      +---------------------+      +---------------------+
```
