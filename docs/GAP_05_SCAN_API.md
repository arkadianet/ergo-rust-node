# GAP_05: Scan API

## Priority: MEDIUM
## Effort: Medium
## Category: API

---

## Description

The Scan API allows external applications to register scanning predicates to track specific boxes (UTXOs) in the blockchain. This is essential for wallet integrations, dApps, and services that need to monitor specific addresses, tokens, or contract templates. While the Rust node has scan storage infrastructure (`ergo-storage/src/scan/`), the API endpoints are not implemented.

---

## Current State (Rust)

### What Exists

The scan storage layer is already implemented:

```
crates/ergo-storage/src/scan/
├── mod.rs           # Module exports
├── storage.rs       # ScanStorage implementation
├── predicate.rs     # ScanningPredicate types
└── types.rs         # ScanBox, ScanState types
```

Key types available:
- `ScanStorage` - Persists scans and tracked boxes
- `ScanningPredicate` - Defines what to track
- `ScanBox` - Box with scan metadata
- `ScanState` - Current state of a scan

### What's Missing

All `/scan/*` API endpoints:
- `POST /scan/register`
- `POST /scan/deregister`
- `GET /scan/listAll`
- `GET /scan/unspentBoxes/{scanId}`
- `GET /scan/spentBoxes/{scanId}`
- `POST /scan/stopTracking`
- `POST /scan/addBox`

---

## Scala Reference

### ScanApiRoute.scala

```scala
// File: src/main/scala/org/ergoplatform/http/api/ScanApiRoute.scala

val routes: Route = 
  pathPrefix("scan") {
    registerR ~
    deregisterR ~
    listAllR ~
    unspentBoxesR ~
    spentBoxesR ~
    stopTrackingR ~
    addBoxR
  }

// Register a new scan
private def registerR: Route = path("register") {
  post {
    entity(as[ScanRequest]) { request =>
      withAuth {
        val scanId = ergoWalletActor ? RegisterScan(request)
        complete(scanId.mapTo[ScanId])
      }
    }
  }
}

// Deregister an existing scan  
private def deregisterR: Route = path("deregister") {
  post {
    entity(as[ScanIdWrapper]) { wrapper =>
      withAuth {
        ergoWalletActor ? DeregisterScan(wrapper.scanId)
        complete(StatusCodes.OK)
      }
    }
  }
}

// List all registered scans
private def listAllR: Route = path("listAll") {
  get {
    withAuth {
      val scans = ergoWalletActor ? GetAllScans
      complete(scans.mapTo[Seq[Scan]])
    }
  }
}

// Get unspent boxes for a scan
private def unspentBoxesR: Route = path("unspentBoxes" / IntNumber) { scanId =>
  parameters('minConfirmations.as[Int] ? 0, 
             'minInclusionHeight.as[Int] ? 0,
             'maxInclusionHeight.as[Int] ? Int.MaxValue) { 
    (minConf, minHeight, maxHeight) =>
    get {
      val boxes = ergoWalletActor ? GetScanUnspentBoxes(
        ScanId @@ scanId, minConf, minHeight, maxHeight
      )
      complete(boxes.mapTo[Seq[WalletBox]])
    }
  }
}

// Get spent boxes for a scan
private def spentBoxesR: Route = path("spentBoxes" / IntNumber) { scanId =>
  get {
    val boxes = ergoWalletActor ? GetScanSpentBoxes(ScanId @@ scanId)
    complete(boxes.mapTo[Seq[WalletBox]])
  }
}

// Stop tracking a specific box
private def stopTrackingR: Route = path("stopTracking") {
  post {
    entity(as[StopTrackingRequest]) { request =>
      withAuth {
        ergoWalletActor ? StopTracking(request.scanId, request.boxId)
        complete(StatusCodes.OK)
      }
    }
  }
}

// Add a box to a scan manually
private def addBoxR: Route = path("addBox") {
  post {
    entity(as[AddBoxRequest]) { request =>
      withAuth {
        ergoWalletActor ? AddBox(request.scanId, request.box)
        complete(StatusCodes.OK)
      }
    }
  }
}
```

### ScanRequest Types

```scala
// Scan registration request
case class ScanRequest(
  scanName: String,
  trackingRule: ScanningPredicate
)

// Scanning predicate types
sealed trait ScanningPredicate

case class EqualsScanningPredicate(
  register: ErgoBox.RegisterId,
  value: SValue
) extends ScanningPredicate

case class ContainsScanningPredicate(
  register: ErgoBox.RegisterId,  
  value: SValue
) extends ScanningPredicate

case class ContainsAssetPredicate(
  assetId: ErgoBox.TokenId
) extends ScanningPredicate

// Combined predicates
case class AndScanningPredicate(
  predicates: Seq[ScanningPredicate]
) extends ScanningPredicate

case class OrScanningPredicate(
  predicates: Seq[ScanningPredicate]
) extends ScanningPredicate
```

---

## Impact

### User Impact

1. **dApp Integration**: dApps need to track contract boxes
2. **Token Monitoring**: Track specific token movements
3. **Multi-sig Wallets**: Track boxes for multi-sig addresses
4. **Payment Processors**: Monitor incoming payments

### Ecosystem Impact

1. **Off-chain bots**: Need efficient box tracking
2. **Bridges**: Monitor cross-chain contract boxes
3. **DEX integration**: Track liquidity pool boxes

---

## Implementation Plan

### Phase 1: API Types (1 day)

1. **Create request/response types** in `crates/ergo-api/src/types/scan.rs`:
   ```rust
   use serde::{Deserialize, Serialize};
   use ergo_storage::scan::{ScanningPredicate, ScanId};
   
   #[derive(Deserialize)]
   pub struct ScanRequest {
       pub scan_name: String,
       pub tracking_rule: ScanningPredicateDto,
   }
   
   #[derive(Deserialize)]
   #[serde(tag = "predicate")]
   pub enum ScanningPredicateDto {
       #[serde(rename = "equals")]
       Equals { register: String, value: serde_json::Value },
       
       #[serde(rename = "contains")]
       Contains { register: String, value: serde_json::Value },
       
       #[serde(rename = "containsAsset")]
       ContainsAsset { asset_id: String },
       
       #[serde(rename = "and")]
       And { args: Vec<ScanningPredicateDto> },
       
       #[serde(rename = "or")]
       Or { args: Vec<ScanningPredicateDto> },
   }
   
   #[derive(Serialize)]
   pub struct ScanResponse {
       pub scan_id: i32,
   }
   
   #[derive(Serialize)]
   pub struct ScanInfo {
       pub scan_id: i32,
       pub scan_name: String,
       pub tracking_rule: ScanningPredicateDto,
   }
   
   #[derive(Deserialize)]
   pub struct UnspentBoxesQuery {
       pub min_confirmations: Option<u32>,
       pub min_inclusion_height: Option<u32>,
       pub max_inclusion_height: Option<u32>,
   }
   
   #[derive(Deserialize)]
   pub struct StopTrackingRequest {
       pub scan_id: i32,
       pub box_id: String,
   }
   
   #[derive(Deserialize)]
   pub struct AddBoxRequest {
       pub scan_id: i32,
       pub box: ErgoBoxDto,
   }
   ```

### Phase 2: API Handlers (2 days)

2. **Create scan handlers** in `crates/ergo-api/src/handlers/scan.rs`:
   ```rust
   use axum::{extract::{Path, Query, State}, Json};
   use crate::types::scan::*;
   use crate::state::ApiState;
   use crate::error::ApiResult;
   
   /// POST /scan/register
   pub async fn register_scan(
       State(state): State<ApiState>,
       Json(request): Json<ScanRequest>,
   ) -> ApiResult<Json<ScanResponse>> {
       let predicate = convert_predicate(request.tracking_rule)?;
       let scan_id = state.scan_storage.register_scan(
           &request.scan_name,
           predicate,
       ).await?;
       
       Ok(Json(ScanResponse { scan_id }))
   }
   
   /// POST /scan/deregister
   pub async fn deregister_scan(
       State(state): State<ApiState>,
       Json(request): Json<ScanIdWrapper>,
   ) -> ApiResult<()> {
       state.scan_storage.deregister_scan(request.scan_id).await?;
       Ok(())
   }
   
   /// GET /scan/listAll
   pub async fn list_all_scans(
       State(state): State<ApiState>,
   ) -> ApiResult<Json<Vec<ScanInfo>>> {
       let scans = state.scan_storage.list_all_scans().await?;
       let scan_infos = scans.into_iter()
           .map(|s| ScanInfo {
               scan_id: s.id,
               scan_name: s.name,
               tracking_rule: convert_predicate_to_dto(s.predicate),
           })
           .collect();
       
       Ok(Json(scan_infos))
   }
   
   /// GET /scan/unspentBoxes/{scanId}
   pub async fn get_unspent_boxes(
       State(state): State<ApiState>,
       Path(scan_id): Path<i32>,
       Query(params): Query<UnspentBoxesQuery>,
   ) -> ApiResult<Json<Vec<WalletBoxDto>>> {
       let boxes = state.scan_storage.get_unspent_boxes(
           scan_id,
           params.min_confirmations.unwrap_or(0),
           params.min_inclusion_height.unwrap_or(0),
           params.max_inclusion_height.unwrap_or(u32::MAX),
       ).await?;
       
       Ok(Json(boxes.into_iter().map(Into::into).collect()))
   }
   
   /// GET /scan/spentBoxes/{scanId}
   pub async fn get_spent_boxes(
       State(state): State<ApiState>,
       Path(scan_id): Path<i32>,
   ) -> ApiResult<Json<Vec<WalletBoxDto>>> {
       let boxes = state.scan_storage.get_spent_boxes(scan_id).await?;
       Ok(Json(boxes.into_iter().map(Into::into).collect()))
   }
   
   /// POST /scan/stopTracking
   pub async fn stop_tracking(
       State(state): State<ApiState>,
       Json(request): Json<StopTrackingRequest>,
   ) -> ApiResult<()> {
       let box_id = BoxId::from_str(&request.box_id)?;
       state.scan_storage.stop_tracking(request.scan_id, &box_id).await?;
       Ok(())
   }
   
   /// POST /scan/addBox
   pub async fn add_box(
       State(state): State<ApiState>,
       Json(request): Json<AddBoxRequest>,
   ) -> ApiResult<()> {
       let ergo_box = convert_box_dto(request.box)?;
       state.scan_storage.add_box(request.scan_id, ergo_box).await?;
       Ok(())
   }
   ```

### Phase 3: Route Registration (0.5 days)

3. **Add routes** in `crates/ergo-api/src/routes.rs`:
   ```rust
   use crate::handlers::scan;
   
   pub fn scan_routes() -> Router<ApiState> {
       Router::new()
           .route("/scan/register", post(scan::register_scan))
           .route("/scan/deregister", post(scan::deregister_scan))
           .route("/scan/listAll", get(scan::list_all_scans))
           .route("/scan/unspentBoxes/:scanId", get(scan::get_unspent_boxes))
           .route("/scan/spentBoxes/:scanId", get(scan::get_spent_boxes))
           .route("/scan/stopTracking", post(scan::stop_tracking))
           .route("/scan/addBox", post(scan::add_box))
   }
   ```

### Phase 4: Storage Integration (1 day)

4. **Extend ScanStorage** if needed in `crates/ergo-storage/src/scan/storage.rs`:
   ```rust
   impl ScanStorage {
       /// Register a new scan and return its ID
       pub async fn register_scan(
           &self,
           name: &str,
           predicate: ScanningPredicate,
       ) -> Result<i32> { ... }
       
       /// Deregister a scan by ID
       pub async fn deregister_scan(&self, scan_id: i32) -> Result<()> { ... }
       
       /// Get all registered scans
       pub async fn list_all_scans(&self) -> Result<Vec<Scan>> { ... }
       
       /// Get unspent boxes matching a scan
       pub async fn get_unspent_boxes(
           &self,
           scan_id: i32,
           min_confirmations: u32,
           min_height: u32,
           max_height: u32,
       ) -> Result<Vec<ScanBox>> { ... }
       
       /// Get spent boxes for a scan
       pub async fn get_spent_boxes(&self, scan_id: i32) -> Result<Vec<ScanBox>> { ... }
   }
   ```

### Phase 5: Box Scanning Integration (1 day)

5. **Integrate scanning into block processing**:
   ```rust
   // In state manager or block processor
   impl StateManager {
       async fn process_block(&mut self, block: &Block) -> Result<()> {
           // ... existing block processing ...
           
           // Scan new outputs for registered scans
           for tx in block.transactions() {
               for output in tx.outputs() {
                   self.scan_storage.check_and_track(output).await?;
               }
               
               for input in tx.inputs() {
                   self.scan_storage.mark_spent(&input.box_id).await?;
               }
           }
           
           Ok(())
       }
   }
   ```

---

## Files to Create/Modify

### New Files
```
crates/ergo-api/src/types/scan.rs    # Request/response types
crates/ergo-api/src/handlers/scan.rs # API handlers
```

### Modified Files
```
crates/ergo-api/src/routes.rs        # Add scan routes
crates/ergo-api/src/types/mod.rs     # Export scan types
crates/ergo-api/src/handlers/mod.rs  # Export scan handlers
crates/ergo-storage/src/scan/storage.rs  # Extend storage methods
crates/ergo-state/src/manager.rs     # Integrate scanning
```

---

## Estimated Effort

| Task | Time |
|------|------|
| API types | 1 day |
| API handlers | 2 days |
| Route registration | 0.5 days |
| Storage extensions | 1 day |
| Block processing integration | 1 day |
| Testing | 1.5 days |
| **Total** | **7 days** |

---

## Dependencies

- Existing `ScanStorage` implementation
- State manager for block processing integration
- API authentication (if required)

---

## Test Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn test_register_and_list_scan() {
        let storage = create_test_storage();
        
        let scan_id = storage.register_scan("test", predicate).await.unwrap();
        let scans = storage.list_all_scans().await.unwrap();
        
        assert_eq!(scans.len(), 1);
        assert_eq!(scans[0].id, scan_id);
    }
    
    #[tokio::test]
    async fn test_track_matching_box() {
        let storage = create_test_storage();
        let predicate = ContainsAsset { asset_id: token_id.clone() };
        
        let scan_id = storage.register_scan("token_tracker", predicate).await.unwrap();
        
        let box_with_token = create_box_with_token(&token_id);
        storage.check_and_track(&box_with_token).await.unwrap();
        
        let unspent = storage.get_unspent_boxes(scan_id, 0, 0, u32::MAX).await.unwrap();
        assert_eq!(unspent.len(), 1);
    }
}
```

### API Tests

```rust
#[tokio::test]
async fn test_scan_api_workflow() {
    let app = create_test_app();
    
    // Register scan
    let response = app.post("/scan/register")
        .json(&ScanRequest {
            scan_name: "test".into(),
            tracking_rule: ScanningPredicateDto::ContainsAsset {
                asset_id: "abc123".into(),
            },
        })
        .send().await;
    
    assert_eq!(response.status(), 200);
    let scan: ScanResponse = response.json().await;
    
    // List scans
    let response = app.get("/scan/listAll").send().await;
    let scans: Vec<ScanInfo> = response.json().await;
    assert!(scans.iter().any(|s| s.scan_id == scan.scan_id));
}
```

---

## Success Metrics

1. All 7 endpoints implemented and functional
2. Scans persist across node restarts
3. Boxes are tracked correctly when matching predicate
4. Spent boxes are correctly identified
5. API tests pass

---

## Scala File References

| Feature | Scala File |
|---------|------------|
| API routes | `src/main/scala/org/ergoplatform/http/api/ScanApiRoute.scala` |
| Predicates | `ergo-wallet/src/main/scala/org/ergoplatform/wallet/scanning/ScanningPredicate.scala` |
| JSON codecs | `ergo-wallet/src/main/scala/org/ergoplatform/wallet/scanning/ScanningPredicateJsonCodecs.scala` |
| Scan storage | `ergo-wallet/src/main/scala/org/ergoplatform/wallet/scanning/ScanWallet.scala` |
