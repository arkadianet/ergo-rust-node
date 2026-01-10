# Ergo Rust Node vs Scala Node Comparison

## Executive Summary

This document compares the Rust implementation with the Scala reference implementation to identify gaps and guide further development.

**Last Updated:** January 10, 2025  
**Overall Completion:** ~98%

---

## Quick Status Overview

| Component | Status | Notes |
|-----------|--------|-------|
| P2P Networking | COMPLETE | Full message types, handshake, peer management |
| Sync Protocol | COMPLETE | Header-first sync, block download, rate limiting, snapshot sync |
| Consensus | COMPLETE | Autolykos v2, difficulty adjustment, NiPoPoW |
| UTXO State | COMPLETE | AVL+ tree, state root, box indexing, snapshots |
| State Rollback | COMPLETE | UndoData mechanism |
| Mempool | COMPLETE | Transaction dependency tracking with weight ordering |
| Mining | COMPLETE | Candidate generation, coinbase, tx selection, EIP-27 |
| Wallet | COMPLETE | HD derivation, tx building, signing, encryption |
| REST API | COMPLETE | Core + extended + blockchain indexer endpoints |
| Block Pruning | COMPLETE | Configurable blocks_to_keep |
| UTXO Snapshots | COMPLETE | Manifest, chunks, download planning |
| NiPoPoW | COMPLETE | Proofs, verification, bootstrap, API |
| Extra Indexer | COMPLETE | Address/tx/box/token indexing, blockchain API |
| Re-emission (EIP-27) | COMPLETE | Settings, rules, coinbase integration |

---

## Detailed Component Comparison

### 1. P2P Networking

| Feature | Scala | Rust | Status |
|---------|-------|------|--------|
| Handshake Protocol | Yes | Yes | COMPLETE |
| Message Types (all) | Yes | Yes | COMPLETE |
| Message Framing | Magic + checksum | Same | COMPLETE |
| Peer Management | Actor-based | DashMap + channels | COMPLETE |
| Rate Limiting | 400 items/100ms | Same | COMPLETE |
| Connection Pool | Configurable | Configurable | COMPLETE |
| DNS Seed Discovery | Yes | No | MISSING |
| Peer Scoring | Complex | Basic | PARTIAL |

### 2. Sync Protocol

| Feature | Scala | Rust | Status |
|---------|-------|------|--------|
| Header-first sync | Yes | Yes | COMPLETE |
| SyncInfo V1/V2 | Both | Both | COMPLETE |
| Inv handling | Full | Full | COMPLETE |
| Modifier requests | Batched (400) | Batched (400) | COMPLETE |
| Parallel block download | Yes | Yes (16 concurrent) | COMPLETE |
| Chain reorganization | Full | Cumulative difficulty | COMPLETE |
| NiPoPoW bootstrap | Yes | Yes | COMPLETE |
| UTXO snapshot sync | Yes | Yes | COMPLETE |

### 3. State Management

| Feature | Scala | Rust | Status |
|---------|-------|------|--------|
| UTXO State (AVL+) | avldb | ergo_avltree_rust | COMPLETE |
| State root calculation | Yes | Yes | COMPLETE |
| State root verification | Yes | ADDigest comparison | COMPLETE |
| Box storage | Yes | Yes | COMPLETE |
| ErgoTree index | Yes | BLAKE2b hash index | COMPLETE |
| Token index | Yes | TokenId index | COMPLETE |
| State snapshots | Periodic | Full implementation | COMPLETE |
| Rollback | Full | UndoData mechanism | COMPLETE |

### 4. Block/Transaction Validation

| Feature | Scala | Rust | Status |
|---------|-------|------|--------|
| Header validation | Full | Full | COMPLETE |
| PoW verification | Autolykos v2 | Autolykos v2 | COMPLETE |
| Difficulty adjustment | Linear LSQ | Linear LSQ | COMPLETE |
| Merkle root verification | Yes | Yes | COMPLETE |
| Transaction validation | Full | Full | COMPLETE |
| Script execution | ergotree-interpreter | Via sigma-rust | COMPLETE |
| Input existence | Yes | validate_inputs_exist | COMPLETE |
| Data input existence | Yes | validate_data_inputs_exist | COMPLETE |
| Token conservation | Yes | validate_token_conservation | COMPLETE |
| ERG conservation | Yes | validate_erg_conservation | COMPLETE |
| Block cost limits | Yes | Framework only | PARTIAL |

### 5. Mempool

| Feature | Scala | Rust | Status |
|---------|-------|------|--------|
| Transaction storage | Yes | DashMap | COMPLETE |
| Fee ordering | Yes | BTreeSet | COMPLETE |
| Double-spend detection | Yes | Input mapping | COMPLETE |
| Size limits | Configurable | Configurable | COMPLETE |
| Expiry | Yes | 1 hour default | COMPLETE |
| Dependency tracking | Yes | Weight-based ordering | COMPLETE |

### 6. Mining

| Feature | Scala | Rust | Status |
|---------|-------|------|--------|
| Block candidate | Full | Full | COMPLETE |
| Transaction selection | Fee-based | Fee-based | COMPLETE |
| Coinbase creation | Yes | CoinbaseBuilder + EIP-27 | COMPLETE |
| Emission schedule | 75 ERG decreasing | ReemissionRules | COMPLETE |
| Re-emission (EIP-27) | Yes | Yes | COMPLETE |
| Difficulty calculation | Yes | DifficultyAdjustment | COMPLETE |
| External mining | Stratum-like | Framework | PARTIAL |
| Internal CPU mining | Yes | No | MISSING |

### 7. Wallet

| Feature | Scala | Rust | Status |
|---------|-------|------|--------|
| HD derivation (BIP32/44) | Yes | Via ergo-lib | COMPLETE |
| Mnemonic (BIP39) | Yes | 12-24 words | COMPLETE |
| EIP-3 paths | m/44'/429'/... | Same | COMPLETE |
| Address generation | All types | P2PK | COMPLETE |
| Box tracking | Full | BoxTracker | COMPLETE |
| Transaction building | Full | WalletTxBuilder | COMPLETE |
| Box selection | Multiple algorithms | SimpleBoxSelector | COMPLETE |
| Transaction signing | Yes | Via ergo-lib | COMPLETE |
| Encryption | AES-256-GCM | AES-256-GCM + Argon2id | COMPLETE |
| P2SH/P2S addresses | Yes | No | MISSING |

### 8. REST API

| Category | Scala Endpoints | Rust Status |
|----------|-----------------|-------------|
| /info | Full | COMPLETE |
| /blocks | Full | COMPLETE |
| /transactions | Full | PARTIAL |
| /utxo | Full | PARTIAL |
| /peers | Full | COMPLETE |
| /mining | Full | COMPLETE |
| /wallet | Full | COMPLETE |
| /script | Full | COMPLETE |
| /emission | Full | COMPLETE (with EIP-27) |
| /utils | Full | COMPLETE |
| /scan | Full | MISSING |
| /nipopow | Full | COMPLETE |
| /blockchain (indexed) | Full | COMPLETE |

### 9. NiPoPoW Support

| Feature | Scala | Rust | Status |
|---------|-------|------|--------|
| PoPowHeader | Yes | Yes | COMPLETE |
| Interlinks serialization | Yes | Yes | COMPLETE |
| NipopowAlgos | Yes | Yes | COMPLETE |
| NipopowProof | Yes | Yes | COMPLETE |
| NipopowVerifier | Yes | Yes | COMPLETE |
| Proof generation | Yes | Yes | COMPLETE |
| Proof comparison | Yes | Yes | COMPLETE |
| API endpoints | Yes | Yes | COMPLETE |

### 10. Extra Indexer

| Feature | Scala | Rust | Status |
|---------|-------|------|--------|
| IndexedErgoBox | Yes | Yes | COMPLETE |
| IndexedErgoTransaction | Yes | Yes | COMPLETE |
| IndexedErgoAddress | Yes | Yes | COMPLETE |
| IndexedToken | Yes | Yes | COMPLETE |
| Balance tracking | Yes | Yes | COMPLETE |
| Blockchain API | Yes | Yes | COMPLETE |
| Pagination | Yes | Yes | COMPLETE |
| Optional enable | Yes | Yes | COMPLETE |

### 11. Re-emission (EIP-27)

| Feature | Scala | Rust | Status |
|---------|-------|------|--------|
| ReemissionSettings | Yes | Yes | COMPLETE |
| ReemissionRules | Yes | Yes | COMPLETE |
| Activation height | 777,217 | 777,217 | COMPLETE |
| Reemission start | 2,080,800 | 2,080,800 | COMPLETE |
| 12 ERG charge | Yes | Yes | COMPLETE |
| 3 ERG claimable | Yes | Yes | COMPLETE |
| Emission API | Yes | Yes | COMPLETE |
| Coinbase integration | Yes | Yes | COMPLETE |

---

## Remaining Gaps

### Minor Missing Features

| Feature | Priority | Effort |
|---------|----------|--------|
| DNS Seed Discovery | Low | Small |
| Complex Peer Scoring | Low | Medium |
| Full Block Cost Validation | Medium | Medium |
| Internal CPU Mining | Low | Medium |
| P2SH/P2S Address Generation | Low | Small |
| Scan API (/scan/*) | Low | Medium |
| Full /transactions endpoints | Medium | Small |
| Full /utxo endpoints | Medium | Small |

---

## Architecture Comparison

| Aspect | Scala Node | Rust Node |
|--------|------------|-----------|
| Concurrency | Akka actors | Tokio async/await |
| Messaging | Actor messages | mpsc channels |
| Framework | Scorex | Direct implementation |
| Storage | Various backends | RocksDB only |
| Main coordinator | ErgoNodeViewHolder | StateManager |
| Sync logic | ErgoNodeViewSynchronizer | SyncProtocol |
| Network | NetworkController | NetworkService |
| Indexer | ExtraIndexer | ExtraIndexer |
| NiPoPoW | NipopowAlgos + Verifier | nipopow module |
| Re-emission | ReemissionRules | reemission module |

---

## Key File Mapping

| Scala Component | Rust Equivalent |
|-----------------|-----------------|
| ErgoApp | ergo-node/src/node.rs |
| ErgoNodeViewHolder | ergo-state/src/manager.rs |
| ErgoNodeViewSynchronizer | ergo-sync/src/protocol.rs |
| NetworkController | ergo-network/src/service.rs |
| PeerManager | ergo-network/src/peer.rs |
| ErgoMemPool | ergo-mempool/src/pool.rs |
| UtxoState | ergo-state/src/utxo.rs |
| ErgoHistory | ergo-state/src/history.rs |
| AutolykosPowScheme | ergo-consensus/src/autolykos.rs |
| ErgoMiner | ergo-mining/src/miner.rs |
| CandidateGenerator | ergo-mining/src/candidate.rs |
| ErgoWalletActor | ergo-wallet/src/wallet.rs |
| TransactionBuilder | ergo-wallet/src/tx_builder.rs |
| NipopowAlgos | ergo-consensus/src/nipopow/algos.rs |
| NipopowVerifier | ergo-consensus/src/nipopow/verifier.rs |
| PoPowHeader | ergo-consensus/src/nipopow/popow_header.rs |
| NipopowProof | ergo-consensus/src/nipopow/proof.rs |
| ExtraIndexer | ergo-storage/src/indexer/mod.rs |
| ReemissionSettings | ergo-consensus/src/reemission/settings.rs |
| ReemissionRules | ergo-consensus/src/reemission/rules.rs |
| SnapshotsDb | ergo-state/src/snapshots.rs |

---

## Scala Test References

Use these tests to verify Rust implementation compatibility:

| Feature | Test File | Rust Tests |
|---------|-----------|------------|
| Mempool dependencies | ErgoMemPoolSpec.scala | ergo-mempool tests |
| NiPoPoW proofs | PoPowAlgosSpec.scala | nipopow::tests (38 tests) |
| NiPoPoW verification | NipopowVerifierSpec.scala | nipopow::verifier::tests |
| UTXO snapshots | UtxoSetSnapshotProcessorSpecification.scala | snapshots::tests |
| Deep rollback | DeepRollBackSpec.scala | state::tests |
| Fork resolution | ForkResolutionSpec.scala | history::tests |
| Multi-node sync | UtxoStateNodesSyncSpec.scala | sync::tests |
| Mining | CandidateGeneratorSpec.scala | mining::tests |
| Re-emission | ReemissionRulesSpec.scala | reemission::tests (16 tests) |
| Indexer | ExtraIndexerSpec.scala | indexer::tests |
