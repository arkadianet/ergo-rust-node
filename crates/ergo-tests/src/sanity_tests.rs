//! Sanity tests for basic node operations.
//!
//! These tests verify fundamental functionality like:
//! - Database operations
//! - Header storage and retrieval
//! - Basic state operations
//! - Chain progression

use crate::generators::*;
use crate::harness::*;
use ergo_storage::{ColumnFamily, Storage, WriteBatch};

/// Test that headers can be stored and retrieved.
#[test]
fn test_header_storage_and_retrieval() {
    let ctx = TestContext::new();

    let header = TestHeader::at_height(10);
    let header_bytes = header.to_bytes();

    // Store header
    ctx.db
        .put(ColumnFamily::Headers, &header.id, &header_bytes)
        .unwrap();

    // Retrieve and verify
    let retrieved = ctx.db.get(ColumnFamily::Headers, &header.id).unwrap();
    assert!(retrieved.is_some());

    let restored = TestHeader::from_bytes(&retrieved.unwrap());
    assert_eq!(restored.height, 10);
    assert_eq!(restored.id, header.id);
}

/// Test that a chain of headers can be stored.
#[test]
fn test_header_chain_storage() {
    let ctx = TestContext::new();

    let chain = TestHeader::chain(100);

    // Store all headers
    let mut batch = WriteBatch::new();
    for header in &chain {
        batch.put(ColumnFamily::Headers, header.id.to_vec(), header.to_bytes());
    }
    ctx.db.write_batch(batch).unwrap();

    // Verify all headers can be retrieved
    for header in &chain {
        let retrieved = ctx.db.get(ColumnFamily::Headers, &header.id).unwrap();
        assert!(
            retrieved.is_some(),
            "Header at height {} not found",
            header.height
        );

        let restored = TestHeader::from_bytes(&retrieved.unwrap());
        assert_eq!(restored.height, header.height);
    }
}

/// Test header chain height tracking.
#[test]
fn test_header_chain_height_tracking() {
    let ctx = TestContext::new();

    let chain = TestHeader::chain(50);

    // Store headers and track best height
    for header in &chain {
        ctx.db
            .put(ColumnFamily::Headers, &header.id, &header.to_bytes())
            .unwrap();

        // Store height -> header_id mapping
        let height_key = header.height.to_be_bytes();
        ctx.db
            .put(ColumnFamily::HeaderChain, &height_key, &header.id)
            .unwrap();
    }

    // Store best height in metadata
    let best_height = chain.last().unwrap().height;
    ctx.db
        .put(
            ColumnFamily::Metadata,
            b"best_header_height",
            &best_height.to_be_bytes(),
        )
        .unwrap();

    // Verify best height
    let stored_height = ctx
        .db
        .get(ColumnFamily::Metadata, b"best_header_height")
        .unwrap()
        .unwrap();
    let height = u32::from_be_bytes(stored_height.try_into().unwrap());
    assert_eq!(height, 49);

    // Verify we can look up any header by height
    for h in 0..50 {
        let height_key = (h as u32).to_be_bytes();
        let header_id = ctx
            .db
            .get(ColumnFamily::HeaderChain, &height_key)
            .unwrap()
            .unwrap();
        assert_eq!(header_id, chain[h].id);
    }
}

/// Test UTXO storage and retrieval.
#[test]
fn test_utxo_storage() {
    let ctx = TestContext::new();

    // Create some test outputs
    let outputs: Vec<_> = (0..10)
        .map(|i| TestOutput {
            id: test_box_id_seeded(i),
            value: (i as u64 + 1) * 1_000_000_000,
            ergo_tree: vec![0x00, 0x08, 0xcd, i],
            creation_height: i as u32,
        })
        .collect();

    // Store UTXOs
    for output in &outputs {
        let value_bytes = output.value.to_be_bytes();
        ctx.db
            .put(ColumnFamily::Utxo, &output.id, &value_bytes)
            .unwrap();
    }

    // Verify all UTXOs can be retrieved
    for output in &outputs {
        let retrieved = ctx.db.get(ColumnFamily::Utxo, &output.id).unwrap();
        assert!(retrieved.is_some());

        let value = u64::from_be_bytes(retrieved.unwrap().try_into().unwrap());
        assert_eq!(value, output.value);
    }

    // Test UTXO deletion (spending)
    ctx.db.delete(ColumnFamily::Utxo, &outputs[0].id).unwrap();
    let deleted = ctx.db.get(ColumnFamily::Utxo, &outputs[0].id).unwrap();
    assert!(deleted.is_none());
}

/// Test batch write atomicity.
#[test]
fn test_batch_atomicity() {
    let ctx = TestContext::new();

    // Write initial state
    ctx.db
        .put(ColumnFamily::Metadata, b"counter", &0u64.to_be_bytes())
        .unwrap();

    // Create a batch with multiple operations
    let mut batch = WriteBatch::new();
    batch.put(
        ColumnFamily::Metadata,
        b"counter".to_vec(),
        100u64.to_be_bytes().to_vec(),
    );
    batch.put(ColumnFamily::Metadata, b"flag".to_vec(), b"set".to_vec());
    batch.put(
        ColumnFamily::Headers,
        b"header1".to_vec(),
        b"data1".to_vec(),
    );
    batch.delete(ColumnFamily::Metadata, b"old_key".to_vec()); // Deleting non-existent is ok

    // Execute batch
    ctx.db.write_batch(batch).unwrap();

    // Verify all operations succeeded
    let counter = ctx
        .db
        .get(ColumnFamily::Metadata, b"counter")
        .unwrap()
        .unwrap();
    assert_eq!(u64::from_be_bytes(counter.try_into().unwrap()), 100);

    let flag = ctx
        .db
        .get(ColumnFamily::Metadata, b"flag")
        .unwrap()
        .unwrap();
    assert_eq!(flag, b"set");

    let header = ctx
        .db
        .get(ColumnFamily::Headers, b"header1")
        .unwrap()
        .unwrap();
    assert_eq!(header, b"data1");
}

/// Test transaction index storage.
#[test]
fn test_transaction_index() {
    let ctx = TestContext::new();

    let block = TestBlock::at_height(5);

    // Store transaction index entries
    for (pos, tx) in block.transactions.iter().enumerate() {
        // tx_id -> (block_id, position)
        let mut index_data = Vec::new();
        index_data.extend_from_slice(&block.header.id);
        index_data.extend_from_slice(&(pos as u32).to_be_bytes());

        ctx.db
            .put(ColumnFamily::TxIndex, &tx.id, &index_data)
            .unwrap();
    }

    // Verify transaction can be located
    let tx = &block.transactions[0];
    let index_data = ctx.db.get(ColumnFamily::TxIndex, &tx.id).unwrap().unwrap();

    let block_id = &index_data[0..32];
    let position = u32::from_be_bytes(index_data[32..36].try_into().unwrap());

    assert_eq!(block_id, block.header.id.as_slice());
    assert_eq!(position, 0);
}

/// Test storing and retrieving cumulative difficulty.
#[test]
fn test_cumulative_difficulty_storage() {
    let ctx = TestContext::new();

    let chain = TestHeader::chain(10);

    // Simulate cumulative difficulty tracking
    let mut cumulative_diff: u128 = 0;

    for header in &chain {
        // Add block's difficulty to cumulative
        let block_diff = 1000u128; // Simplified
        cumulative_diff += block_diff;

        // Store as big-endian bytes
        let diff_bytes = cumulative_diff.to_be_bytes();
        ctx.db
            .put(ColumnFamily::CumulativeDifficulty, &header.id, &diff_bytes)
            .unwrap();
    }

    // Verify cumulative difficulty for each block
    for (i, header) in chain.iter().enumerate() {
        let stored = ctx
            .db
            .get(ColumnFamily::CumulativeDifficulty, &header.id)
            .unwrap()
            .unwrap();
        let diff = u128::from_be_bytes(stored.try_into().unwrap());

        let expected = 1000u128 * (i as u128 + 1);
        assert_eq!(diff, expected);
    }
}

/// Test undo data storage for state rollback.
#[test]
fn test_undo_data_storage() {
    let ctx = TestContext::new();

    // Simulate undo data: spent boxes and created boxes at each height
    for height in 1..=10u32 {
        let mut undo_data = Vec::new();

        // Number of spent boxes
        undo_data.extend_from_slice(&2u32.to_be_bytes());

        // Spent box IDs
        undo_data.extend_from_slice(&test_box_id_seeded(height as u8 * 2));
        undo_data.extend_from_slice(&test_box_id_seeded(height as u8 * 2 + 1));

        // Number of created boxes
        undo_data.extend_from_slice(&1u32.to_be_bytes());

        // Created box ID
        undo_data.extend_from_slice(&test_box_id_seeded(height as u8 + 100));

        let height_key = height.to_be_bytes();
        ctx.db
            .put(ColumnFamily::UndoData, &height_key, &undo_data)
            .unwrap();
    }

    // Verify undo data can be retrieved for rollback
    let height_key = 5u32.to_be_bytes();
    let undo_data = ctx
        .db
        .get(ColumnFamily::UndoData, &height_key)
        .unwrap()
        .unwrap();

    // Parse undo data
    let num_spent = u32::from_be_bytes(undo_data[0..4].try_into().unwrap());
    assert_eq!(num_spent, 2);

    let spent_box_1 = &undo_data[4..36];
    assert_eq!(spent_box_1, test_box_id_seeded(10).as_slice());
}

/// Test iterator over column family.
#[test]
fn test_column_family_iteration() {
    let ctx = TestContext::new();

    // Insert multiple entries
    for i in 0..20u8 {
        let key = vec![i];
        let value = vec![i * 2];
        ctx.db.put(ColumnFamily::Metadata, &key, &value).unwrap();
    }

    // Iterate and count
    let iter = ctx.db.iter(ColumnFamily::Metadata).unwrap();
    let entries: Vec<_> = iter.collect();

    assert_eq!(entries.len(), 20);

    // Verify entries are present (order may vary)
    for (key, value) in &entries {
        assert_eq!(key.len(), 1);
        assert_eq!(value.len(), 1);
        assert_eq!(value[0], key[0] * 2);
    }
}

/// Test database contains check.
#[test]
fn test_contains_check() {
    let ctx = TestContext::new();

    let key = b"existing_key";
    let value = b"some_value";

    // Should not contain initially
    assert!(!ctx.db.contains(ColumnFamily::Metadata, key).unwrap());

    // Add the key
    ctx.db.put(ColumnFamily::Metadata, key, value).unwrap();

    // Should contain now
    assert!(ctx.db.contains(ColumnFamily::Metadata, key).unwrap());

    // Delete the key
    ctx.db.delete(ColumnFamily::Metadata, key).unwrap();

    // Should not contain after delete
    assert!(!ctx.db.contains(ColumnFamily::Metadata, key).unwrap());
}

/// Test multi-get operation.
#[test]
fn test_multi_get() {
    let ctx = TestContext::new();

    // Insert some entries
    for i in 0..10u8 {
        ctx.db.put(ColumnFamily::Utxo, &[i], &[i * 3]).unwrap();
    }

    // Multi-get some keys (including non-existent ones)
    let keys: Vec<&[u8]> = vec![&[0], &[5], &[9], &[99]]; // 99 doesn't exist
    let results = ctx.db.multi_get(ColumnFamily::Utxo, &keys).unwrap();

    assert_eq!(results.len(), 4);
    assert_eq!(results[0], Some(vec![0]));
    assert_eq!(results[1], Some(vec![15]));
    assert_eq!(results[2], Some(vec![27]));
    assert_eq!(results[3], None); // Non-existent key
}

/// Test large batch write performance.
#[test]
fn test_large_batch_write() {
    let ctx = TestContext::new();

    let num_entries = 10_000;
    let mut batch = WriteBatch::with_capacity(num_entries);

    for i in 0..num_entries {
        let key = (i as u32).to_be_bytes();
        let value = vec![0u8; 100]; // 100 bytes per value
        batch.put(ColumnFamily::Utxo, key.to_vec(), value);
    }

    let start = std::time::Instant::now();
    ctx.db.write_batch(batch).unwrap();
    let duration = start.elapsed();

    // Should complete in reasonable time (< 5 seconds)
    assert!(
        duration.as_secs() < 5,
        "Batch write took too long: {:?}",
        duration
    );

    // Verify some entries
    for i in [0, 5000, 9999] {
        let key = (i as u32).to_be_bytes();
        let value = ctx.db.get(ColumnFamily::Utxo, &key).unwrap();
        assert!(value.is_some());
    }
}
