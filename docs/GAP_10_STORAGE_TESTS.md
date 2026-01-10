# GAP_10: Storage Layer Tests

## Priority: HIGH
## Effort: Medium
## Category: Testing

---

## Description

The Rust node's storage layer (`ergo-storage` crate) lacks comprehensive tests. While the indexer has some tests, the core database operations, column family management, and batch write functionality have minimal or no test coverage. This is concerning because storage bugs can cause data corruption and node failures.

---

## Current State (Rust)

### What Exists

In `crates/ergo-storage/src/`:
- `database.rs` - RocksDB wrapper (0 tests)
- `batch.rs` - Batch write operations (0 tests)
- `indexer/` - Extra indexer (has tests)
- `scan/` - Scan storage (minimal tests)

### Test Count

| Module | Tests | Status |
|--------|-------|--------|
| database.rs | 0 | Critical Gap |
| batch.rs | 0 | Critical Gap |
| indexer/ | 22 | Good |
| scan/ | 4 | Minimal |
| **Total** | 26 | Insufficient |

### What's Missing

1. **Database initialization tests**
2. **Column family operations** (get, put, delete)
3. **Batch write/commit tests**
4. **Atomic transaction tests**
5. **Error handling tests**
6. **Concurrent access tests**
7. **Data integrity tests**
8. **Recovery tests**

---

## Scala Reference

### Storage Tests

```scala
// File: src/test/scala/scorex/db/LDBKVStoreSpec.scala

class LDBKVStoreSpec extends AnyFlatSpec {
  
  "LevelDB store" should "persist and retrieve data" in {
    withStore { store =>
      val key = Array[Byte](1, 2, 3)
      val value = Array[Byte](4, 5, 6)
      
      store.insert(key, value)
      val retrieved = store.get(key)
      
      retrieved shouldBe Some(value)
    }
  }
  
  it should "delete data correctly" in {
    withStore { store =>
      val key = Array[Byte](1, 2, 3)
      val value = Array[Byte](4, 5, 6)
      
      store.insert(key, value)
      store.delete(key)
      val retrieved = store.get(key)
      
      retrieved shouldBe None
    }
  }
  
  it should "handle batch operations atomically" in {
    withStore { store =>
      val batch = store.newBatch()
      
      (0 until 100).foreach { i =>
        batch.insert(Array(i.toByte), Array((i * 2).toByte))
      }
      
      batch.commit()
      
      (0 until 100).foreach { i =>
        store.get(Array(i.toByte)) shouldBe Some(Array((i * 2).toByte))
      }
    }
  }
  
  it should "rollback uncommitted batch on failure" in {
    withStore { store =>
      val key = Array[Byte](1)
      store.insert(key, Array[Byte](0))
      
      val batch = store.newBatch()
      batch.insert(key, Array[Byte](1))
      // Don't commit - batch should be discarded
      
      store.get(key) shouldBe Some(Array[Byte](0))
    }
  }
}
```

### Versioned Store Tests

```scala
// File: src/test/scala/scorex/db/VersionedStoreSpec.scala

class VersionedStoreSpec extends AnyFlatSpec {
  
  "Versioned store" should "maintain history" in {
    withVersionedStore { store =>
      val key = Array[Byte](1)
      
      store.update(version1, Seq(key -> Array[Byte](1)))
      store.update(version2, Seq(key -> Array[Byte](2)))
      store.update(version3, Seq(key -> Array[Byte](3)))
      
      store.get(key) shouldBe Some(Array[Byte](3))
      store.getAtVersion(version2, key) shouldBe Some(Array[Byte](2))
    }
  }
  
  it should "rollback to previous version" in {
    withVersionedStore { store =>
      val key = Array[Byte](1)
      
      store.update(version1, Seq(key -> Array[Byte](1)))
      store.update(version2, Seq(key -> Array[Byte](2)))
      
      store.rollbackTo(version1)
      
      store.get(key) shouldBe Some(Array[Byte](1))
    }
  }
}
```

### History Storage Tests

```scala
// File: src/test/scala/org/ergoplatform/nodeView/history/HistoryStorageSpec.scala

class HistoryStorageSpec extends AnyFlatSpec {
  
  "History storage" should "store and retrieve headers" in {
    withHistoryStorage { storage =>
      val header = generateHeader()
      
      storage.insert(header)
      val retrieved = storage.getHeader(header.id)
      
      retrieved shouldBe Some(header)
    }
  }
  
  it should "maintain best header" in {
    withHistoryStorage { storage =>
      val headers = (1 to 10).map(h => generateHeader(height = h))
      headers.foreach(storage.insert)
      
      storage.bestHeader shouldBe headers.last
    }
  }
  
  it should "handle fork correctly" in {
    withHistoryStorage { storage =>
      val mainChain = generateChain(10)
      val fork = generateFork(mainChain, forkPoint = 5, length = 7)
      
      mainChain.foreach(storage.insert)
      fork.foreach(storage.insert)
      
      // Fork should become best chain
      storage.bestHeader shouldBe fork.last
    }
  }
}
```

---

## Impact

### Why Storage Tests Matter

1. **Data Integrity**: Storage bugs can corrupt blockchain data
2. **Crash Recovery**: Must handle unexpected shutdowns
3. **Concurrency**: Multiple components access storage
4. **Performance**: Batch operations must be efficient
5. **Correctness**: State root depends on correct storage

### Risks Without Tests

- Silent data corruption
- Inconsistent state after crash
- Race conditions in concurrent access
- Memory leaks in long-running nodes
- Incorrect rollback behavior

---

## Implementation Plan

### Phase 1: Database Core Tests (2 days)

1. **Create database tests** in `crates/ergo-storage/src/database_tests.rs`:
   ```rust
   use crate::database::{Database, ColumnFamily};
   use tempfile::TempDir;
   
   fn create_test_db() -> (Database, TempDir) {
       let dir = TempDir::new().unwrap();
       let db = Database::open(dir.path()).unwrap();
       (db, dir)
   }
   
   #[cfg(test)]
   mod tests {
       use super::*;
       
       #[test]
       fn test_open_and_close() {
           let (db, _dir) = create_test_db();
           drop(db); // Should close cleanly
       }
       
       #[test]
       fn test_reopen_database() {
           let dir = TempDir::new().unwrap();
           
           // Open, write, close
           {
               let db = Database::open(dir.path()).unwrap();
               db.put(ColumnFamily::Metadata, b"key", b"value").unwrap();
           }
           
           // Reopen and verify
           {
               let db = Database::open(dir.path()).unwrap();
               let value = db.get(ColumnFamily::Metadata, b"key").unwrap();
               assert_eq!(value, Some(b"value".to_vec()));
           }
       }
       
       #[test]
       fn test_put_get_delete() {
           let (db, _dir) = create_test_db();
           
           // Put
           db.put(ColumnFamily::Utxo, b"box1", b"data1").unwrap();
           
           // Get
           let value = db.get(ColumnFamily::Utxo, b"box1").unwrap();
           assert_eq!(value, Some(b"data1".to_vec()));
           
           // Delete
           db.delete(ColumnFamily::Utxo, b"box1").unwrap();
           
           // Verify deleted
           let value = db.get(ColumnFamily::Utxo, b"box1").unwrap();
           assert_eq!(value, None);
       }
       
       #[test]
       fn test_column_family_isolation() {
           let (db, _dir) = create_test_db();
           
           db.put(ColumnFamily::Utxo, b"key", b"utxo_value").unwrap();
           db.put(ColumnFamily::Headers, b"key", b"header_value").unwrap();
           
           let utxo_val = db.get(ColumnFamily::Utxo, b"key").unwrap();
           let header_val = db.get(ColumnFamily::Headers, b"key").unwrap();
           
           assert_eq!(utxo_val, Some(b"utxo_value".to_vec()));
           assert_eq!(header_val, Some(b"header_value".to_vec()));
       }
       
       #[test]
       fn test_get_nonexistent_key() {
           let (db, _dir) = create_test_db();
           
           let value = db.get(ColumnFamily::Utxo, b"nonexistent").unwrap();
           assert_eq!(value, None);
       }
       
       #[test]
       fn test_overwrite_value() {
           let (db, _dir) = create_test_db();
           
           db.put(ColumnFamily::Utxo, b"key", b"value1").unwrap();
           db.put(ColumnFamily::Utxo, b"key", b"value2").unwrap();
           
           let value = db.get(ColumnFamily::Utxo, b"key").unwrap();
           assert_eq!(value, Some(b"value2".to_vec()));
       }
   }
   ```

### Phase 2: Batch Operation Tests (1.5 days)

2. **Create batch tests** in `crates/ergo-storage/src/batch_tests.rs`:
   ```rust
   #[cfg(test)]
   mod tests {
       use super::*;
       
       #[test]
       fn test_batch_write() {
           let (db, _dir) = create_test_db();
           let mut batch = db.write_batch();
           
           for i in 0..100u8 {
               batch.put(ColumnFamily::Utxo, &[i], &[i * 2]);
           }
           
           db.write(batch).unwrap();
           
           // Verify all written
           for i in 0..100u8 {
               let value = db.get(ColumnFamily::Utxo, &[i]).unwrap();
               assert_eq!(value, Some(vec![i * 2]));
           }
       }
       
       #[test]
       fn test_batch_atomicity() {
           let (db, _dir) = create_test_db();
           
           // Write initial value
           db.put(ColumnFamily::Utxo, b"key", b"original").unwrap();
           
           // Create batch but don't write
           let mut batch = db.write_batch();
           batch.put(ColumnFamily::Utxo, b"key", b"modified");
           
           // Drop batch without writing
           drop(batch);
           
           // Original value should remain
           let value = db.get(ColumnFamily::Utxo, b"key").unwrap();
           assert_eq!(value, Some(b"original".to_vec()));
       }
       
       #[test]
       fn test_batch_mixed_operations() {
           let (db, _dir) = create_test_db();
           
           // Initial data
           db.put(ColumnFamily::Utxo, b"delete_me", b"value").unwrap();
           
           // Batch with puts and deletes
           let mut batch = db.write_batch();
           batch.put(ColumnFamily::Utxo, b"new_key", b"new_value");
           batch.delete(ColumnFamily::Utxo, b"delete_me");
           batch.put(ColumnFamily::Headers, b"header", b"header_data");
           
           db.write(batch).unwrap();
           
           assert_eq!(
               db.get(ColumnFamily::Utxo, b"new_key").unwrap(),
               Some(b"new_value".to_vec())
           );
           assert_eq!(db.get(ColumnFamily::Utxo, b"delete_me").unwrap(), None);
           assert_eq!(
               db.get(ColumnFamily::Headers, b"header").unwrap(),
               Some(b"header_data".to_vec())
           );
       }
       
       #[test]
       fn test_large_batch() {
           let (db, _dir) = create_test_db();
           let mut batch = db.write_batch();
           
           // Write 10,000 entries
           for i in 0..10_000u32 {
               let key = i.to_be_bytes();
               let value = (i * 2).to_be_bytes();
               batch.put(ColumnFamily::Utxo, &key, &value);
           }
           
           let start = std::time::Instant::now();
           db.write(batch).unwrap();
           let duration = start.elapsed();
           
           // Should complete in reasonable time
           assert!(duration.as_secs() < 5);
           
           // Verify some entries
           for i in [0u32, 5000, 9999] {
               let key = i.to_be_bytes();
               let value = db.get(ColumnFamily::Utxo, &key).unwrap();
               assert_eq!(value, Some((i * 2).to_be_bytes().to_vec()));
           }
       }
   }
   ```

### Phase 3: Concurrent Access Tests (1.5 days)

3. **Create concurrency tests**:
   ```rust
   #[cfg(test)]
   mod concurrent_tests {
       use super::*;
       use std::sync::Arc;
       use std::thread;
       
       #[test]
       fn test_concurrent_reads() {
           let (db, _dir) = create_test_db();
           let db = Arc::new(db);
           
           // Write initial data
           for i in 0..100u8 {
               db.put(ColumnFamily::Utxo, &[i], &[i]).unwrap();
           }
           
           // Concurrent reads
           let handles: Vec<_> = (0..10)
               .map(|_| {
                   let db = db.clone();
                   thread::spawn(move || {
                       for i in 0..100u8 {
                           let value = db.get(ColumnFamily::Utxo, &[i]).unwrap();
                           assert_eq!(value, Some(vec![i]));
                       }
                   })
               })
               .collect();
           
           for handle in handles {
               handle.join().unwrap();
           }
       }
       
       #[test]
       fn test_concurrent_writes() {
           let (db, _dir) = create_test_db();
           let db = Arc::new(db);
           
           let handles: Vec<_> = (0..10u8)
               .map(|thread_id| {
                   let db = db.clone();
                   thread::spawn(move || {
                       for i in 0..100u8 {
                           let key = [thread_id, i];
                           let value = [thread_id, i, 1];
                           db.put(ColumnFamily::Utxo, &key, &value).unwrap();
                       }
                   })
               })
               .collect();
           
           for handle in handles {
               handle.join().unwrap();
           }
           
           // Verify all writes
           for thread_id in 0..10u8 {
               for i in 0..100u8 {
                   let key = [thread_id, i];
                   let value = db.get(ColumnFamily::Utxo, &key).unwrap();
                   assert_eq!(value, Some(vec![thread_id, i, 1]));
               }
           }
       }
       
       #[tokio::test]
       async fn test_async_access() {
           let (db, _dir) = create_test_db();
           let db = Arc::new(db);
           
           let mut handles = Vec::new();
           
           for i in 0..10u8 {
               let db = db.clone();
               handles.push(tokio::spawn(async move {
                   db.put(ColumnFamily::Utxo, &[i], &[i * 2]).unwrap();
                   let value = db.get(ColumnFamily::Utxo, &[i]).unwrap();
                   assert_eq!(value, Some(vec![i * 2]));
               }));
           }
           
           for handle in handles {
               handle.await.unwrap();
           }
       }
   }
   ```

### Phase 4: Data Integrity Tests (1 day)

4. **Create integrity tests**:
   ```rust
   #[cfg(test)]
   mod integrity_tests {
       use super::*;
       
       #[test]
       fn test_empty_key() {
           let (db, _dir) = create_test_db();
           
           db.put(ColumnFamily::Utxo, b"", b"value").unwrap();
           let value = db.get(ColumnFamily::Utxo, b"").unwrap();
           
           assert_eq!(value, Some(b"value".to_vec()));
       }
       
       #[test]
       fn test_empty_value() {
           let (db, _dir) = create_test_db();
           
           db.put(ColumnFamily::Utxo, b"key", b"").unwrap();
           let value = db.get(ColumnFamily::Utxo, b"key").unwrap();
           
           assert_eq!(value, Some(vec![]));
       }
       
       #[test]
       fn test_large_value() {
           let (db, _dir) = create_test_db();
           
           let large_value = vec![0u8; 1_000_000]; // 1MB
           db.put(ColumnFamily::Utxo, b"large", &large_value).unwrap();
           
           let retrieved = db.get(ColumnFamily::Utxo, b"large").unwrap();
           assert_eq!(retrieved, Some(large_value));
       }
       
       #[test]
       fn test_binary_data() {
           let (db, _dir) = create_test_db();
           
           let key: Vec<u8> = (0..=255u8).collect();
           let value: Vec<u8> = (0..=255u8).rev().collect();
           
           db.put(ColumnFamily::Utxo, &key, &value).unwrap();
           let retrieved = db.get(ColumnFamily::Utxo, &key).unwrap();
           
           assert_eq!(retrieved, Some(value));
       }
       
       #[test]
       fn test_persistence_after_crash_simulation() {
           let dir = TempDir::new().unwrap();
           
           // Write data
           {
               let db = Database::open(dir.path()).unwrap();
               db.put(ColumnFamily::Utxo, b"critical", b"data").unwrap();
               // Simulate crash by not calling flush
           }
           
           // Reopen and verify
           {
               let db = Database::open(dir.path()).unwrap();
               let value = db.get(ColumnFamily::Utxo, b"critical").unwrap();
               assert_eq!(value, Some(b"data".to_vec()));
           }
       }
   }
   ```

### Phase 5: Iterator and Range Tests (1 day)

5. **Create iterator tests**:
   ```rust
   #[cfg(test)]
   mod iterator_tests {
       use super::*;
       
       #[test]
       fn test_iterate_all() {
           let (db, _dir) = create_test_db();
           
           for i in 0..10u8 {
               db.put(ColumnFamily::Utxo, &[i], &[i * 2]).unwrap();
           }
           
           let entries: Vec<_> = db.iter(ColumnFamily::Utxo).collect();
           
           assert_eq!(entries.len(), 10);
           for (i, (key, value)) in entries.iter().enumerate() {
               assert_eq!(key, &vec![i as u8]);
               assert_eq!(value, &vec![i as u8 * 2]);
           }
       }
       
       #[test]
       fn test_iterate_prefix() {
           let (db, _dir) = create_test_db();
           
           // Insert with different prefixes
           db.put(ColumnFamily::Utxo, b"a:1", b"v1").unwrap();
           db.put(ColumnFamily::Utxo, b"a:2", b"v2").unwrap();
           db.put(ColumnFamily::Utxo, b"b:1", b"v3").unwrap();
           
           let entries: Vec<_> = db.iter_prefix(ColumnFamily::Utxo, b"a:").collect();
           
           assert_eq!(entries.len(), 2);
       }
       
       #[test]
       fn test_iterate_range() {
           let (db, _dir) = create_test_db();
           
           for i in 0..100u8 {
               db.put(ColumnFamily::Utxo, &[i], &[i]).unwrap();
           }
           
           let entries: Vec<_> = db
               .iter_range(ColumnFamily::Utxo, &[10], &[20])
               .collect();
           
           assert_eq!(entries.len(), 10);
           assert_eq!(entries[0].0, vec![10]);
           assert_eq!(entries[9].0, vec![19]);
       }
   }
   ```

---

## Files to Create

```
crates/ergo-storage/src/tests/
├── mod.rs              # Test module exports
├── database_tests.rs   # Core database tests
├── batch_tests.rs      # Batch operation tests
├── concurrent_tests.rs # Concurrency tests
├── integrity_tests.rs  # Data integrity tests
└── iterator_tests.rs   # Iterator and range tests
```

### Cargo.toml Additions

```toml
[dev-dependencies]
tempfile = "3.8"
```

---

## Estimated Effort

| Task | Time |
|------|------|
| Database core tests | 2 days |
| Batch operation tests | 1.5 days |
| Concurrent access tests | 1.5 days |
| Data integrity tests | 1 day |
| Iterator tests | 1 day |
| **Total** | **7 days** |

---

## Dependencies

- `tempfile` crate for test directories
- Existing database implementation

---

## Test Strategy

### Running Tests

```bash
# Run all storage tests
cargo test -p ergo-storage

# Run specific test module
cargo test -p ergo-storage database_tests

# Run with verbose output
cargo test -p ergo-storage -- --nocapture

# Run tests in parallel
cargo test -p ergo-storage -- --test-threads=4
```

### Test Coverage

```bash
# Generate coverage report
cargo tarpaulin -p ergo-storage --out html
```

---

## Success Metrics

1. **Test Count**: At least 50 storage tests
2. **Coverage**: 80%+ line coverage for database.rs and batch.rs
3. **Concurrency**: All concurrent tests pass consistently
4. **Persistence**: Data survives simulated crashes
5. **Performance**: Batch writes complete within time limits

---

## Scala File References

| Feature | Scala File |
|---------|------------|
| KV store tests | `scorex-core/src/test/scala/scorex/db/LDBKVStoreSpec.scala` |
| Versioned store | `scorex-core/src/test/scala/scorex/db/VersionedStoreSpec.scala` |
| History storage | `src/test/scala/org/ergoplatform/nodeView/history/HistoryStorageSpec.scala` |
