# GAP_02: Missing/Incomplete API Endpoints

## Status: IMPLEMENTED
## Priority: HIGH
## Effort: Medium
## Category: API

---

## Implementation Summary (January 2025)

All originally identified missing endpoints have been implemented or were found to already exist under different names.

### Endpoints That Already Existed
| GAP Claim | Actual Endpoint | Status |
|-----------|-----------------|--------|
| `/transactions/feeHistogram` | `/transactions/poolHistogram` | Already existed |
| `/transactions/recommendedFee` | `/transactions/getFee` | Already existed |
| `/transactions/expectedWaitTime/{fee}` | `/transactions/waitTime` | Already existed |
| `/utxo/withPool/byIds` | POST `/utxo/withPool/byIds` | Already existed |
| `/scan/*` (7 endpoints) | All 8 scan endpoints | Already existed |

### Newly Implemented Endpoints
| Endpoint | Handler | File |
|----------|---------|------|
| GET `/transactions/unconfirmed/byOutputId/:boxId` | `get_unconfirmed_by_output_id` | transactions.rs |
| GET `/transactions/unconfirmed/byTokenId/:tokenId` | `get_unconfirmed_by_token_id` | transactions.rs |
| GET `/transactions/unconfirmed/byErgoTree/:tree` | `get_unconfirmed_by_ergo_tree` | transactions.rs |
| POST `/utxo/getBoxesBinaryProof` | `get_boxes_binary_proof` | utxo.rs |
| POST `/node/shutdown` | `shutdown` | node.rs |

### New Mempool Query Methods
Added to `crates/ergo-mempool/src/pool.rs`:
- `get_by_output_id(&self, box_id: &[u8]) -> Option<PooledTransaction>`
- `get_by_token_id(&self, token_id: &[u8]) -> Vec<PooledTransaction>`
- `get_by_ergo_tree(&self, tree_hash: &[u8]) -> Vec<PooledTransaction>`

### Files Modified
- `crates/ergo-mempool/src/pool.rs` - Added 3 query methods
- `crates/ergo-mempool/Cargo.toml` - Added blake2 dependency
- `crates/ergo-api/src/handlers/transactions.rs` - Added 3 handlers
- `crates/ergo-api/src/handlers/utxo.rs` - Added binary proof handler
- `crates/ergo-api/src/handlers/node.rs` - Created (shutdown handler)
- `crates/ergo-api/src/handlers/mod.rs` - Export node module
- `crates/ergo-api/src/state.rs` - Added shutdown_signal field
- `crates/ergo-api/src/routes.rs` - Registered 5 new routes

---

## Description

Several REST API endpoints available in the Scala node are missing or incomplete in the Rust implementation. While the core API is functional, some advanced query endpoints for transactions, UTXOs, and node management are not yet implemented.

---

## Current State (Rust)

### Implemented Endpoints

| Category | Endpoints | Status |
|----------|-----------|--------|
| `/info` | GET /info | Complete |
| `/blocks` | GET/POST /blocks/* | Complete |
| `/transactions` | POST /send, GET /unconfirmed | Partial |
| `/utxo` | GET /byId, /genesis | Partial |
| `/peers` | GET all, connected, blacklisted | Complete |
| `/mining` | GET candidate, POST solution | Complete |
| `/wallet` | Full wallet API | Complete |
| `/script` | p2sAddress, addressToTree, etc. | Complete |
| `/emission` | at/{height}, scripts | Complete |
| `/utils` | seed, hash, address utils | Complete |
| `/nipopow` | proof, popowHeader | Complete |
| `/blockchain` | Full indexed queries | Complete |

### Missing Endpoints

| Endpoint | Description | Scala Reference |
|----------|-------------|-----------------|
| `/transactions/unconfirmed/byOutputId/{boxId}` | Get unconfirmed tx by output box | TransactionApiRoute.scala:120 |
| `/transactions/unconfirmed/byTokenId/{tokenId}` | Get unconfirmed tx by token | TransactionApiRoute.scala:135 |
| `/transactions/unconfirmed/byErgoTree/{tree}` | Get unconfirmed tx by script | TransactionApiRoute.scala:150 |
| `/transactions/feeHistogram` | Mempool fee distribution | TransactionApiRoute.scala:165 |
| `/transactions/recommendedFee` | Suggested transaction fee | TransactionApiRoute.scala:180 |
| `/transactions/expectedWaitTime/{fee}` | Wait time for fee level | TransactionApiRoute.scala:195 |
| `/utxo/withPool/byIds` | Batch UTXO lookup with mempool | UtxoApiRoute.scala:85 |
| `/utxo/getBoxesBinaryProof` | Binary proof for boxes | UtxoApiRoute.scala:100 |
| `/node/shutdown` | Remote node shutdown | NodeApiRoute.scala |
| `/scan/*` | All scan endpoints | See GAP_05 |

---

## Scala Reference

### TransactionApiRoute.scala

```scala
// File: src/main/scala/org/ergoplatform/http/api/TransactionApiRoute.scala

// Unconfirmed by output box
path("transactions" / "unconfirmed" / "byOutputId" / Segment) { boxId =>
  get {
    ApiResponse(readersHolder.mempoolReaderOpt.map { mr =>
      mr.getAllUnconfirmedTxByOutputBox(ModifierId @@ boxId)
    })
  }
}

// Unconfirmed by token
path("transactions" / "unconfirmed" / "byTokenId" / Segment) { tokenId =>
  get {
    ApiResponse(readersHolder.mempoolReaderOpt.map { mr =>
      mr.getAllUnconfirmedTxByTokenId(TokenId @@ tokenId)
    })
  }
}

// Fee histogram
path("transactions" / "feeHistogram") {
  get {
    ApiResponse(readersHolder.mempoolReaderOpt.map { mr =>
      mr.getFeeHistogram
    })
  }
}

// Recommended fee
path("transactions" / "recommendedFee") {
  parameters('waitTime.as[Int].?, 'txSize.as[Int].?) { (waitTime, txSize) =>
    get {
      ApiResponse(readersHolder.mempoolReaderOpt.map { mr =>
        mr.getRecommendedFee(waitTime.getOrElse(1), txSize.getOrElse(100))
      })
    }
  }
}
```

### UtxoApiRoute.scala

```scala
// File: src/main/scala/org/ergoplatform/http/api/UtxoApiRoute.scala

// Batch UTXO lookup including mempool
path("utxo" / "withPool" / "byIds") {
  post {
    entity(as[Seq[String]]) { boxIds =>
      ApiResponse(
        boxIds.map { id =>
          readersHolder.stateReaderOpt.flatMap(_.boxById(ModifierId @@ id))
            .orElse(readersHolder.mempoolReaderOpt.flatMap(_.getOutputBox(id)))
        }
      )
    }
  }
}

// Binary proof for boxes
path("utxo" / "getBoxesBinaryProof") {
  post {
    entity(as[Seq[String]]) { boxIds =>
      ApiResponse(
        readersHolder.stateReaderOpt.map { sr =>
          sr.getBoxesBinaryProof(boxIds.map(ModifierId @@ _))
        }
      )
    }
  }
}
```

### ErgoMemPoolReader - Fee Statistics

```scala
// File: src/main/scala/org/ergoplatform/nodeView/mempool/ErgoMemPoolReader.scala

def getFeeHistogram: Seq[FeeHistogramBin] = {
  val bins = Array.fill(10)(0L)
  val fees = orderedTransactions.values.map(_.weightedFeePerByte)
  // Distribute into histogram bins
  fees.foreach { fee =>
    val binIndex = math.min((fee / FEE_BIN_SIZE).toInt, 9)
    bins(binIndex) += 1
  }
  bins.zipWithIndex.map { case (count, idx) =>
    FeeHistogramBin(idx * FEE_BIN_SIZE, (idx + 1) * FEE_BIN_SIZE, count)
  }
}

def getRecommendedFee(waitTimeMinutes: Int, txSizeBytes: Int): Long = {
  val targetPosition = orderedTransactions.size * waitTimeMinutes / 10
  val sortedFees = orderedTransactions.values.map(_.weightedFeePerByte).toSeq.sorted.reverse
  if (sortedFees.isEmpty) MIN_FEE_PER_BYTE * txSizeBytes
  else sortedFees.lift(targetPosition).getOrElse(sortedFees.last) * txSizeBytes
}
```

---

## Impact

### User Impact

1. **Wallet Integration**: Missing fee estimation makes transaction building harder
2. **Mempool Analysis**: Cannot analyze pending transaction pool
3. **Box Tracking**: Cannot track specific outputs in mempool
4. **Token Monitoring**: Cannot find pending token transfers

### Ecosystem Impact

1. **Explorer Integration**: Explorers need these endpoints for mempool views
2. **dApp Development**: Apps need fee recommendations
3. **Trading Bots**: Need to monitor pending transactions

---

## Implementation Plan

### Phase 1: Mempool Query Extensions (2 days)

1. **Add mempool query methods** to `ergo-mempool/src/pool.rs`:
   ```rust
   impl Mempool {
       pub fn get_by_output_box(&self, box_id: &BoxId) -> Vec<&Transaction> { ... }
       pub fn get_by_token_id(&self, token_id: &TokenId) -> Vec<&Transaction> { ... }
       pub fn get_by_ergo_tree(&self, tree_hash: &[u8; 32]) -> Vec<&Transaction> { ... }
   }
   ```

2. **Add fee statistics** to mempool:
   ```rust
   impl Mempool {
       pub fn fee_histogram(&self, bins: usize) -> Vec<FeeHistogramBin> { ... }
       pub fn recommended_fee(&self, wait_time: u32, tx_size: usize) -> u64 { ... }
       pub fn expected_wait_time(&self, fee_per_byte: u64) -> u32 { ... }
   }
   ```

### Phase 2: API Handlers (2 days)

3. **Add transaction handlers** in `ergo-api/src/handlers/transactions.rs`:
   ```rust
   pub async fn get_unconfirmed_by_output(
       State(state): State<ApiState>,
       Path(box_id): Path<String>,
   ) -> ApiResult<Json<Vec<Transaction>>> { ... }
   
   pub async fn get_fee_histogram(
       State(state): State<ApiState>,
   ) -> ApiResult<Json<Vec<FeeHistogramBin>>> { ... }
   
   pub async fn get_recommended_fee(
       State(state): State<ApiState>,
       Query(params): Query<FeeParams>,
   ) -> ApiResult<Json<RecommendedFee>> { ... }
   ```

4. **Add UTXO handlers** in `ergo-api/src/handlers/utxo.rs`:
   ```rust
   pub async fn get_boxes_with_pool_batch(
       State(state): State<ApiState>,
       Json(box_ids): Json<Vec<String>>,
   ) -> ApiResult<Json<Vec<Option<ErgoBox>>>> { ... }
   
   pub async fn get_boxes_binary_proof(
       State(state): State<ApiState>,
       Json(box_ids): Json<Vec<String>>,
   ) -> ApiResult<Json<BinaryProof>> { ... }
   ```

### Phase 3: Route Registration (1 day)

5. **Update routes** in `ergo-api/src/routes.rs`:
   ```rust
   .route("/transactions/unconfirmed/byOutputId/:boxId", get(get_unconfirmed_by_output))
   .route("/transactions/unconfirmed/byTokenId/:tokenId", get(get_unconfirmed_by_token))
   .route("/transactions/unconfirmed/byErgoTree/:tree", get(get_unconfirmed_by_tree))
   .route("/transactions/feeHistogram", get(get_fee_histogram))
   .route("/transactions/recommendedFee", get(get_recommended_fee))
   .route("/transactions/expectedWaitTime/:fee", get(get_expected_wait_time))
   .route("/utxo/withPool/byIds", post(get_boxes_with_pool_batch))
   .route("/utxo/getBoxesBinaryProof", post(get_boxes_binary_proof))
   ```

### Phase 4: Node Shutdown (0.5 days)

6. **Add node shutdown handler**:
   ```rust
   pub async fn shutdown_node(
       State(state): State<ApiState>,
   ) -> ApiResult<Json<()>> {
       state.shutdown_signal.send(()).await?;
       Ok(Json(()))
   }
   ```

---

## Files to Modify/Create

### Mempool Extensions
- `crates/ergo-mempool/src/pool.rs` - Add query methods
- `crates/ergo-mempool/src/statistics.rs` (new) - Fee statistics

### API Handlers
- `crates/ergo-api/src/handlers/transactions.rs` - Add new handlers
- `crates/ergo-api/src/handlers/utxo.rs` - Add batch/proof handlers
- `crates/ergo-api/src/handlers/node.rs` (new) - Node management

### Routes
- `crates/ergo-api/src/routes.rs` - Register new routes

### Types
- `crates/ergo-api/src/types.rs` - Add FeeHistogramBin, RecommendedFee

---

## Estimated Effort

| Task | Time |
|------|------|
| Mempool query methods | 1 day |
| Fee statistics | 1 day |
| API handlers | 2 days |
| Routes and types | 0.5 days |
| Testing | 1 day |
| **Total** | **5.5 days** |

---

## Dependencies

- Mempool must expose internal data structures for queries
- State manager must support binary proof generation

---

## Test Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_get_by_output_box() {
        let mempool = create_test_mempool();
        let tx = create_test_tx_with_output(box_id);
        mempool.add(tx.clone());
        
        let result = mempool.get_by_output_box(&box_id);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id(), tx.id());
    }
    
    #[test]
    fn test_fee_histogram() {
        let mempool = create_mempool_with_various_fees();
        let histogram = mempool.fee_histogram(10);
        
        assert_eq!(histogram.len(), 10);
        assert!(histogram.iter().map(|b| b.count).sum::<u64>() > 0);
    }
}
```

### API Tests

```rust
#[tokio::test]
async fn test_fee_histogram_endpoint() {
    let app = create_test_app();
    
    let response = app
        .oneshot(Request::get("/transactions/feeHistogram").body(Body::empty()).unwrap())
        .await
        .unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    
    let body: Vec<FeeHistogramBin> = parse_response(response).await;
    assert!(!body.is_empty());
}
```

---

## Success Metrics

1. All listed endpoints respond with correct data
2. Fee histogram matches mempool state
3. Recommended fee is within reasonable bounds
4. API tests pass for all new endpoints
5. OpenAPI spec updated to include new endpoints

---

## Scala File References

| Feature | Scala File |
|---------|------------|
| Transaction queries | `src/main/scala/org/ergoplatform/http/api/TransactionApiRoute.scala` |
| UTXO queries | `src/main/scala/org/ergoplatform/http/api/UtxoApiRoute.scala` |
| Mempool reader | `src/main/scala/org/ergoplatform/nodeView/mempool/ErgoMemPoolReader.scala` |
| Fee histogram | `src/main/scala/org/ergoplatform/nodeView/mempool/HistogramStats.scala` |
| Node API | `src/main/scala/org/ergoplatform/http/api/NodeApiRoute.scala` |
