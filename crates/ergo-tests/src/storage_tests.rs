//! Comprehensive storage layer tests.
//!
//! These tests cover database operations, batch processing,
//! concurrent access, and data integrity.

use crate::harness::*;
use ergo_storage::{ColumnFamily, Database, Storage, WriteBatch};
use std::sync::Arc;
use std::thread;

// ============================================================================
// Database Core Tests (GAP_10)
// ============================================================================

/// Test database open and close.
#[test]
fn test_database_open_close() {
    let test_db = TestDatabase::new();

    // Write some data
    test_db
        .put(ColumnFamily::Metadata, b"key", b"value")
        .unwrap();

    // Drop the database (implicit close)
    drop(test_db);

    // Database should be cleanly closed without panic
}

/// Test database reopen with data persistence.
#[test]
fn test_database_reopen() {
    use tempfile::TempDir;

    let temp_dir = TempDir::new().unwrap();

    // Open, write, close
    {
        let db = Database::open(temp_dir.path()).unwrap();
        db.put(ColumnFamily::Metadata, b"persist_key", b"persist_value")
            .unwrap();
    }

    // Reopen and verify
    {
        let db = Database::open(temp_dir.path()).unwrap();
        let value = db.get(ColumnFamily::Metadata, b"persist_key").unwrap();
        assert_eq!(value, Some(b"persist_value".to_vec()));
    }
}

/// Test all column families are accessible.
#[test]
fn test_all_column_families() {
    let test_db = TestDatabase::new();

    let column_families = [
        ColumnFamily::Headers,
        ColumnFamily::BlockTransactions,
        ColumnFamily::Extensions,
        ColumnFamily::AdProofs,
        ColumnFamily::Utxo,
        ColumnFamily::UtxoSnapshots,
        ColumnFamily::HeaderChain,
        ColumnFamily::TxIndex,
        ColumnFamily::UndoData,
        ColumnFamily::ErgoTreeIndex,
        ColumnFamily::TokenIndex,
        ColumnFamily::CumulativeDifficulty,
        ColumnFamily::Metadata,
        ColumnFamily::ExtraIndex,
        ColumnFamily::BoxIndex,
        ColumnFamily::TxNumericIndex,
        ColumnFamily::Scans,
        ColumnFamily::ScanBoxes,
        ColumnFamily::Default,
    ];

    // Test write and read for each column family
    for (i, cf) in column_families.iter().enumerate() {
        let key = format!("test_key_{}", i);
        let value = format!("test_value_{}", i);

        test_db.put(*cf, key.as_bytes(), value.as_bytes()).unwrap();
        let retrieved = test_db.get(*cf, key.as_bytes()).unwrap();

        assert_eq!(
            retrieved,
            Some(value.as_bytes().to_vec()),
            "Failed for column family {:?}",
            cf
        );
    }
}

/// Test column family isolation.
#[test]
fn test_column_family_isolation() {
    let test_db = TestDatabase::new();

    let key = b"shared_key";

    // Write different values to same key in different column families
    test_db
        .put(ColumnFamily::Headers, key, b"headers_value")
        .unwrap();
    test_db.put(ColumnFamily::Utxo, key, b"utxo_value").unwrap();
    test_db
        .put(ColumnFamily::Metadata, key, b"metadata_value")
        .unwrap();

    // Verify each column family has its own value
    assert_eq!(
        test_db.get(ColumnFamily::Headers, key).unwrap(),
        Some(b"headers_value".to_vec())
    );
    assert_eq!(
        test_db.get(ColumnFamily::Utxo, key).unwrap(),
        Some(b"utxo_value".to_vec())
    );
    assert_eq!(
        test_db.get(ColumnFamily::Metadata, key).unwrap(),
        Some(b"metadata_value".to_vec())
    );
}

/// Test get nonexistent key returns None.
#[test]
fn test_get_nonexistent() {
    let test_db = TestDatabase::new();

    let result = test_db
        .get(ColumnFamily::Metadata, b"nonexistent_key")
        .unwrap();
    assert_eq!(result, None);
}

/// Test delete nonexistent key doesn't error.
#[test]
fn test_delete_nonexistent() {
    let test_db = TestDatabase::new();

    // Should not error when deleting non-existent key
    let result = test_db.delete(ColumnFamily::Metadata, b"nonexistent_key");
    assert!(result.is_ok());
}

/// Test overwrite existing value.
#[test]
fn test_overwrite_value() {
    let test_db = TestDatabase::new();

    let key = b"overwrite_key";

    test_db.put(ColumnFamily::Metadata, key, b"value1").unwrap();
    assert_eq!(
        test_db.get(ColumnFamily::Metadata, key).unwrap(),
        Some(b"value1".to_vec())
    );

    test_db.put(ColumnFamily::Metadata, key, b"value2").unwrap();
    assert_eq!(
        test_db.get(ColumnFamily::Metadata, key).unwrap(),
        Some(b"value2".to_vec())
    );

    test_db.put(ColumnFamily::Metadata, key, b"value3").unwrap();
    assert_eq!(
        test_db.get(ColumnFamily::Metadata, key).unwrap(),
        Some(b"value3".to_vec())
    );
}

// ============================================================================
// Batch Operation Tests (GAP_10)
// ============================================================================

/// Test basic batch write.
#[test]
fn test_batch_write_basic() {
    let test_db = TestDatabase::new();

    let mut batch = WriteBatch::new();

    for i in 0..100u8 {
        batch.put(ColumnFamily::Utxo, vec![i], vec![i * 2]);
    }

    test_db.write_batch(batch).unwrap();

    // Verify all written
    for i in 0..100u8 {
        let value = test_db.get(ColumnFamily::Utxo, &[i]).unwrap();
        assert_eq!(value, Some(vec![i * 2]));
    }
}

/// Test batch with mixed operations.
#[test]
fn test_batch_mixed_operations() {
    let test_db = TestDatabase::new();

    // Initial data
    test_db
        .put(ColumnFamily::Utxo, b"delete_me", b"old_value")
        .unwrap();
    test_db
        .put(ColumnFamily::Utxo, b"update_me", b"original")
        .unwrap();

    // Batch with puts and deletes
    let mut batch = WriteBatch::new();
    batch.put(
        ColumnFamily::Utxo,
        b"new_key".to_vec(),
        b"new_value".to_vec(),
    );
    batch.delete(ColumnFamily::Utxo, b"delete_me".to_vec());
    batch.put(
        ColumnFamily::Utxo,
        b"update_me".to_vec(),
        b"updated".to_vec(),
    );
    batch.put(
        ColumnFamily::Headers,
        b"header_key".to_vec(),
        b"header_data".to_vec(),
    );

    test_db.write_batch(batch).unwrap();

    // Verify results
    assert_eq!(
        test_db.get(ColumnFamily::Utxo, b"new_key").unwrap(),
        Some(b"new_value".to_vec())
    );
    assert_eq!(test_db.get(ColumnFamily::Utxo, b"delete_me").unwrap(), None);
    assert_eq!(
        test_db.get(ColumnFamily::Utxo, b"update_me").unwrap(),
        Some(b"updated".to_vec())
    );
    assert_eq!(
        test_db.get(ColumnFamily::Headers, b"header_key").unwrap(),
        Some(b"header_data".to_vec())
    );
}

/// Test batch atomicity - uncommitted batch doesn't affect database.
#[test]
fn test_batch_atomicity_uncommitted() {
    let test_db = TestDatabase::new();

    // Write initial value
    test_db
        .put(ColumnFamily::Utxo, b"key", b"original")
        .unwrap();

    // Create batch but don't write it
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Utxo, b"key".to_vec(), b"modified".to_vec());
    batch.put(
        ColumnFamily::Utxo,
        b"new_key".to_vec(),
        b"new_value".to_vec(),
    );

    // Drop batch without writing
    drop(batch);

    // Original value should remain
    assert_eq!(
        test_db.get(ColumnFamily::Utxo, b"key").unwrap(),
        Some(b"original".to_vec())
    );
    assert_eq!(test_db.get(ColumnFamily::Utxo, b"new_key").unwrap(), None);
}

/// Test batch across multiple column families.
#[test]
fn test_batch_multiple_column_families() {
    let test_db = TestDatabase::new();

    let mut batch = WriteBatch::new();

    batch.put(ColumnFamily::Headers, b"h1".to_vec(), b"header1".to_vec());
    batch.put(ColumnFamily::Utxo, b"u1".to_vec(), b"utxo1".to_vec());
    batch.put(ColumnFamily::TxIndex, b"t1".to_vec(), b"tx1".to_vec());
    batch.put(ColumnFamily::Metadata, b"m1".to_vec(), b"meta1".to_vec());

    test_db.write_batch(batch).unwrap();

    // Verify each column family
    assert_eq!(
        test_db.get(ColumnFamily::Headers, b"h1").unwrap(),
        Some(b"header1".to_vec())
    );
    assert_eq!(
        test_db.get(ColumnFamily::Utxo, b"u1").unwrap(),
        Some(b"utxo1".to_vec())
    );
    assert_eq!(
        test_db.get(ColumnFamily::TxIndex, b"t1").unwrap(),
        Some(b"tx1".to_vec())
    );
    assert_eq!(
        test_db.get(ColumnFamily::Metadata, b"m1").unwrap(),
        Some(b"meta1".to_vec())
    );
}

/// Test large batch performance.
#[test]
fn test_batch_large_write() {
    let test_db = TestDatabase::new();

    let num_entries = 10_000;
    let mut batch = WriteBatch::with_capacity(num_entries);

    for i in 0..num_entries as u32 {
        let key = i.to_be_bytes();
        let value = vec![0u8; 256]; // 256 bytes per value
        batch.put(ColumnFamily::Utxo, key.to_vec(), value);
    }

    let start = std::time::Instant::now();
    test_db.write_batch(batch).unwrap();
    let duration = start.elapsed();

    // Should complete in reasonable time
    assert!(
        duration.as_secs() < 10,
        "Large batch took too long: {:?}",
        duration
    );

    // Verify random samples
    for i in [0u32, 1000, 5000, 9999] {
        let key = i.to_be_bytes();
        let value = test_db.get(ColumnFamily::Utxo, &key).unwrap();
        assert!(value.is_some(), "Key {} not found", i);
        assert_eq!(value.unwrap().len(), 256);
    }
}

/// Test batch with capacity.
#[test]
fn test_batch_with_capacity() {
    let batch = WriteBatch::with_capacity(100);
    assert!(batch.is_empty());
    assert_eq!(batch.len(), 0);
}

/// Test batch clear.
#[test]
fn test_batch_clear() {
    let mut batch = WriteBatch::new();

    batch.put(ColumnFamily::Utxo, b"key1".to_vec(), b"value1".to_vec());
    batch.put(ColumnFamily::Utxo, b"key2".to_vec(), b"value2".to_vec());

    assert_eq!(batch.len(), 2);

    batch.clear();

    assert!(batch.is_empty());
    assert_eq!(batch.len(), 0);
}

/// Test batch merge.
#[test]
fn test_batch_merge() {
    let test_db = TestDatabase::new();

    let mut batch1 = WriteBatch::new();
    batch1.put(ColumnFamily::Utxo, b"key1".to_vec(), b"value1".to_vec());
    batch1.put(ColumnFamily::Utxo, b"key2".to_vec(), b"value2".to_vec());

    let mut batch2 = WriteBatch::new();
    batch2.put(ColumnFamily::Utxo, b"key3".to_vec(), b"value3".to_vec());
    batch2.put(ColumnFamily::Headers, b"h1".to_vec(), b"header1".to_vec());

    // Merge batch2 into batch1
    batch1.merge(batch2);

    assert_eq!(batch1.len(), 4);

    // Write merged batch
    test_db.write_batch(batch1).unwrap();

    // Verify all entries
    assert!(test_db.get(ColumnFamily::Utxo, b"key1").unwrap().is_some());
    assert!(test_db.get(ColumnFamily::Utxo, b"key2").unwrap().is_some());
    assert!(test_db.get(ColumnFamily::Utxo, b"key3").unwrap().is_some());
    assert!(test_db.get(ColumnFamily::Headers, b"h1").unwrap().is_some());
}

// ============================================================================
// Concurrent Access Tests (GAP_10)
// ============================================================================

/// Test concurrent reads.
#[test]
fn test_concurrent_reads() {
    let test_db = Arc::new(TestDatabase::new());

    // Write initial data
    for i in 0..100u8 {
        test_db.put(ColumnFamily::Utxo, &[i], &[i]).unwrap();
    }

    // Spawn multiple reader threads
    let mut handles = vec![];

    for _ in 0..10 {
        let db = Arc::clone(&test_db);
        handles.push(thread::spawn(move || {
            for i in 0..100u8 {
                let value = db.get(ColumnFamily::Utxo, &[i]).unwrap();
                assert_eq!(value, Some(vec![i]));
            }
        }));
    }

    // Wait for all readers
    for handle in handles {
        handle.join().unwrap();
    }
}

/// Test concurrent writes from different threads.
#[test]
fn test_concurrent_writes() {
    let test_db = Arc::new(TestDatabase::new());

    let mut handles = vec![];

    for thread_id in 0..10u8 {
        let db = Arc::clone(&test_db);
        handles.push(thread::spawn(move || {
            for i in 0..100u8 {
                let key = [thread_id, i];
                let value = [thread_id, i, 1];
                db.put(ColumnFamily::Utxo, &key, &value).unwrap();
            }
        }));
    }

    // Wait for all writers
    for handle in handles {
        handle.join().unwrap();
    }

    // Verify all writes
    for thread_id in 0..10u8 {
        for i in 0..100u8 {
            let key = [thread_id, i];
            let value = test_db.get(ColumnFamily::Utxo, &key).unwrap();
            assert_eq!(value, Some(vec![thread_id, i, 1]));
        }
    }
}

/// Test concurrent reads and writes.
#[test]
fn test_concurrent_read_write() {
    let test_db = Arc::new(TestDatabase::new());

    // Pre-populate some data
    for i in 0..50u8 {
        test_db.put(ColumnFamily::Utxo, &[i], &[i]).unwrap();
    }

    let mut handles = vec![];

    // Reader threads
    for _ in 0..5 {
        let db = Arc::clone(&test_db);
        handles.push(thread::spawn(move || {
            for _ in 0..100 {
                for i in 0..50u8 {
                    let _ = db.get(ColumnFamily::Utxo, &[i]);
                }
            }
        }));
    }

    // Writer threads (writing to different keys)
    for thread_id in 0..5u8 {
        let db = Arc::clone(&test_db);
        handles.push(thread::spawn(move || {
            for i in 0..100u8 {
                let key = [100 + thread_id, i]; // Different range from readers
                db.put(ColumnFamily::Utxo, &key, &[i]).unwrap();
            }
        }));
    }

    // Wait for all threads
    for handle in handles {
        handle.join().unwrap();
    }
}

// ============================================================================
// Data Integrity Tests
// ============================================================================

/// Test empty key handling.
#[test]
fn test_empty_key() {
    let test_db = TestDatabase::new();

    test_db
        .put(ColumnFamily::Metadata, b"", b"empty_key_value")
        .unwrap();
    let value = test_db.get(ColumnFamily::Metadata, b"").unwrap();

    assert_eq!(value, Some(b"empty_key_value".to_vec()));
}

/// Test empty value handling.
#[test]
fn test_empty_value() {
    let test_db = TestDatabase::new();

    test_db
        .put(ColumnFamily::Metadata, b"key_with_empty_value", b"")
        .unwrap();
    let value = test_db
        .get(ColumnFamily::Metadata, b"key_with_empty_value")
        .unwrap();

    assert_eq!(value, Some(vec![]));
}

/// Test large value handling.
#[test]
fn test_large_value() {
    let test_db = TestDatabase::new();

    let large_value = vec![0xABu8; 1_000_000]; // 1MB value

    test_db
        .put(ColumnFamily::Utxo, b"large_key", &large_value)
        .unwrap();
    let retrieved = test_db.get(ColumnFamily::Utxo, b"large_key").unwrap();

    assert_eq!(retrieved, Some(large_value));
}

/// Test binary data with all byte values.
#[test]
fn test_binary_data() {
    let test_db = TestDatabase::new();

    let key: Vec<u8> = (0..=255u8).collect();
    let value: Vec<u8> = (0..=255u8).rev().collect();

    test_db.put(ColumnFamily::Utxo, &key, &value).unwrap();
    let retrieved = test_db.get(ColumnFamily::Utxo, &key).unwrap();

    assert_eq!(retrieved, Some(value));
}

/// Test iterator returns all entries.
#[test]
fn test_iterator_completeness() {
    let test_db = TestDatabase::new();

    let num_entries = 100;

    for i in 0..num_entries {
        let key = (i as u32).to_be_bytes();
        let value = vec![i as u8];
        test_db.put(ColumnFamily::Metadata, &key, &value).unwrap();
    }

    let iter = test_db.iter(ColumnFamily::Metadata).unwrap();
    let entries: Vec<_> = iter.collect();

    assert_eq!(entries.len(), num_entries);
}

/// Test flush operation.
#[test]
fn test_flush() {
    let test_db = TestDatabase::new();

    // Write some data
    for i in 0..100u8 {
        test_db.put(ColumnFamily::Utxo, &[i], &[i * 2]).unwrap();
    }

    // Flush should not error
    test_db.flush().unwrap();

    // Data should still be accessible
    for i in 0..100u8 {
        let value = test_db.get(ColumnFamily::Utxo, &[i]).unwrap();
        assert_eq!(value, Some(vec![i * 2]));
    }
}
