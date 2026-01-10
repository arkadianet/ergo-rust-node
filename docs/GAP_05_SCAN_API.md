# GAP_05: Scan API

## Status: IMPLEMENTED
## Priority: MEDIUM
## Effort: Medium
## Category: API

---

## Implementation Summary (January 2025)

The Scan API has been fully implemented following EIP-0001.

### Files Implemented

**Storage Layer** (`crates/ergo-storage/src/scan/`):
- `mod.rs` - Module exports
- `storage.rs` - `ScanStorage` with full CRUD operations
- `predicate.rs` - `ScanningPredicate` types (ContainsAsset, Equals, Contains, And, Or)
- `types.rs` - `Scan`, `ScanBox`, `ScanRequest`, `ScanWalletInteraction`

**API Layer** (`crates/ergo-api/src/handlers/scan.rs`):
- All 8 endpoints implemented with proper error handling
- Query parameter support for filtering boxes
- Hex encoding/decoding for box IDs and data

**Routes** (`crates/ergo-api/src/routes.rs`):
- All scan routes registered

### Endpoints Implemented

| Endpoint | Handler | Description |
|----------|---------|-------------|
| `POST /scan/register` | `register` | Register a new scan |
| `POST /scan/deregister` | `deregister` | Remove a scan |
| `GET /scan/listAll` | `list_all` | List all registered scans |
| `GET /scan/unspentBoxes/:scanId` | `unspent_boxes` | Get unspent boxes for scan |
| `GET /scan/spentBoxes/:scanId` | `spent_boxes` | Get spent boxes for scan |
| `POST /scan/stopTracking` | `stop_tracking` | Stop tracking a box |
| `POST /scan/addBox` | `add_box` | Manually add a box |
| `POST /scan/p2sRule` | `p2s_rule` | Helper for P2S address tracking |

### Query Parameters (for box endpoints)

- `minConfirmations` - Minimum confirmations (default: 0)
- `maxConfirmations` - Maximum confirmations (default: MAX)
- `minInclusionHeight` - Minimum inclusion height (default: 0)
- `maxInclusionHeight` - Maximum inclusion height (default: MAX)
- `limit` - Max boxes to return (default: 50)
- `offset` - Pagination offset (default: 0)

### Predicate Types Supported

```rust
pub enum ScanningPredicate {
    ContainsAsset { asset_id: Vec<u8> },
    Equals { reg_id: u8, value: Vec<u8> },
    Contains { reg_id: u8, value: Vec<u8> },
    And { args: Vec<ScanningPredicate> },
    Or { args: Vec<ScanningPredicate> },
}
```

### State Integration

- `AppState.scan_storage` - Optional `Arc<ScanStorage<Database>>`
- `with_scan_storage()` builder method for enabling scan support
- Graceful handling when scan storage not enabled

### Tests

Storage tests in `crates/ergo-storage/src/scan/storage.rs`:
- `test_register_scan`
- `test_deregister_scan`
- `test_add_and_get_box`
- `test_unspent_and_spent_boxes`
- `test_stop_tracking`

Predicate tests in `crates/ergo-storage/src/scan/predicate.rs`:
- `test_contains_asset`
- `test_equals`
- `test_contains`
- `test_and`
- `test_or`
- `test_json_serialization`

---

## Description

The Scan API allows external applications to register scanning predicates to track specific boxes (UTXOs) in the blockchain. This is essential for wallet integrations, dApps, and services that need to monitor specific addresses, tokens, or contract templates.

---

## Scala File References

| Feature | Scala File |
|---------|------------|
| API routes | `src/main/scala/org/ergoplatform/http/api/ScanApiRoute.scala` |
| Predicates | `ergo-wallet/src/main/scala/org/ergoplatform/wallet/scanning/ScanningPredicate.scala` |
| JSON codecs | `ergo-wallet/src/main/scala/org/ergoplatform/wallet/scanning/ScanningPredicateJsonCodecs.scala` |
| Scan storage | `ergo-wallet/src/main/scala/org/ergoplatform/wallet/scanning/ScanWallet.scala` |
