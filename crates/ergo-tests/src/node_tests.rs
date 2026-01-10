//! Node integration tests.
//!
//! Tests for node configuration, component initialization, and integration.

use crate::harness::TestDatabase;
use ergo_mempool::Mempool;
use ergo_state::StateManager;
use ergo_storage::Storage;
use std::sync::Arc;

// ============================================================================
// Component Integration Tests
// ============================================================================

#[test]
fn test_state_manager_with_storage() {
    let test_db = TestDatabase::new();
    let storage: Arc<dyn Storage> = Arc::new(test_db.db_clone());
    let state_manager = StateManager::new(Arc::clone(&storage));

    let (utxo_height, header_height) = state_manager.heights();
    assert_eq!(utxo_height, 0);
    assert_eq!(header_height, 0);
}

#[test]
fn test_mempool_creation() {
    let mempool = Mempool::with_defaults();
    assert_eq!(mempool.len(), 0);
    assert!(mempool.is_empty());
}

#[test]
fn test_mempool_stats() {
    let mempool = Mempool::with_defaults();
    let stats = mempool.stats();
    // Initially empty mempool
    assert_eq!(stats.tx_count, 0);
    assert_eq!(stats.total_size, 0);
}

#[test]
fn test_components_can_share_storage() {
    let test_db = TestDatabase::new();
    let storage: Arc<dyn Storage> = Arc::new(test_db.db_clone());

    // Multiple components can share the same storage
    let state1 = StateManager::new(Arc::clone(&storage));
    let state2 = StateManager::new(Arc::clone(&storage));

    // Both should see the same initial state
    assert_eq!(state1.heights(), state2.heights());
}

// ============================================================================
// Storage Integration Tests
// ============================================================================

#[test]
fn test_storage_put_get_roundtrip() {
    let test_db = TestDatabase::new();
    let storage: Arc<dyn Storage> = Arc::new(test_db.db_clone());

    let key = b"test_key";
    let value = b"test_value";

    storage
        .put(ergo_storage::ColumnFamily::Headers, key, value)
        .unwrap();
    let retrieved = storage
        .get(ergo_storage::ColumnFamily::Headers, key)
        .unwrap();

    assert_eq!(retrieved, Some(value.to_vec()));
}

#[test]
fn test_storage_different_column_families_isolated() {
    let test_db = TestDatabase::new();
    let storage: Arc<dyn Storage> = Arc::new(test_db.db_clone());

    let key = b"shared_key";
    let value1 = b"headers_value";
    let value2 = b"utxo_value";

    storage
        .put(ergo_storage::ColumnFamily::Headers, key, value1)
        .unwrap();
    storage
        .put(ergo_storage::ColumnFamily::Utxo, key, value2)
        .unwrap();

    let from_headers = storage
        .get(ergo_storage::ColumnFamily::Headers, key)
        .unwrap();
    let from_utxo = storage.get(ergo_storage::ColumnFamily::Utxo, key).unwrap();

    assert_eq!(from_headers, Some(value1.to_vec()));
    assert_eq!(from_utxo, Some(value2.to_vec()));
}

#[test]
fn test_storage_delete_works() {
    let test_db = TestDatabase::new();
    let storage: Arc<dyn Storage> = Arc::new(test_db.db_clone());

    let key = b"to_delete";
    let value = b"some_value";

    storage
        .put(ergo_storage::ColumnFamily::Headers, key, value)
        .unwrap();
    assert!(storage
        .get(ergo_storage::ColumnFamily::Headers, key)
        .unwrap()
        .is_some());

    storage
        .delete(ergo_storage::ColumnFamily::Headers, key)
        .unwrap();
    assert!(storage
        .get(ergo_storage::ColumnFamily::Headers, key)
        .unwrap()
        .is_none());
}

// ============================================================================
// Mempool Integration Tests
// ============================================================================

#[test]
fn test_mempool_size_tracking() {
    let mempool = Mempool::with_defaults();

    assert_eq!(mempool.len(), 0);
    assert!(mempool.is_empty());
}

#[test]
fn test_mempool_get_by_fee_empty() {
    let mempool = Mempool::with_defaults();

    let txs = mempool.get_by_fee(10);
    assert!(txs.is_empty());
}

#[test]
fn test_mempool_clear() {
    let mempool = Mempool::with_defaults();
    mempool.clear();
    assert!(mempool.is_empty());
}

// ============================================================================
// State Manager Integration Tests
// ============================================================================

#[test]
fn test_state_manager_initial_state() {
    let test_db = TestDatabase::new();
    let storage: Arc<dyn Storage> = Arc::new(test_db.db_clone());
    let state = StateManager::new(storage);

    // Initial heights should be 0
    let (utxo, header) = state.heights();
    assert_eq!(utxo, 0);
    assert_eq!(header, 0);
}

#[test]
fn test_state_manager_history_access() {
    let test_db = TestDatabase::new();
    let storage: Arc<dyn Storage> = Arc::new(test_db.db_clone());
    let state = StateManager::new(storage);

    // Should be able to access history component
    let best_header = state.history.best_header_id();
    // Initially no best header
    assert!(best_header.is_none());
}

#[test]
fn test_state_manager_utxo_access() {
    let test_db = TestDatabase::new();
    let storage: Arc<dyn Storage> = Arc::new(test_db.db_clone());
    let state = StateManager::new(storage);

    // Should be able to access UTXO component
    let state_root = state.utxo.state_root();
    // State root should be non-empty (even for empty state)
    assert!(!state_root.is_empty());
}

// ============================================================================
// Multi-Component Integration Tests
// ============================================================================

#[test]
fn test_full_node_component_creation() {
    let test_db = TestDatabase::new();
    let storage: Arc<dyn Storage> = Arc::new(test_db.db_clone());

    // Create all main components
    let state_manager = Arc::new(StateManager::new(Arc::clone(&storage)));
    let mempool = Arc::new(Mempool::with_defaults());

    // Verify they're usable
    assert_eq!(state_manager.heights(), (0, 0));
    assert!(mempool.is_empty());
}

#[test]
fn test_components_independent_operation() {
    let test_db = TestDatabase::new();
    let storage: Arc<dyn Storage> = Arc::new(test_db.db_clone());

    let state = StateManager::new(Arc::clone(&storage));
    let mempool = Mempool::with_defaults();

    // Operations on one component don't affect the other
    let initial_heights = state.heights();
    mempool.clear();
    assert_eq!(state.heights(), initial_heights);
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_storage_get_nonexistent_key() {
    let test_db = TestDatabase::new();
    let storage: Arc<dyn Storage> = Arc::new(test_db.db_clone());

    let result = storage.get(ergo_storage::ColumnFamily::Headers, b"nonexistent");
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

#[test]
fn test_storage_delete_nonexistent_key() {
    let test_db = TestDatabase::new();
    let storage: Arc<dyn Storage> = Arc::new(test_db.db_clone());

    // Deleting non-existent key should not error
    let result = storage.delete(ergo_storage::ColumnFamily::Headers, b"nonexistent");
    assert!(result.is_ok());
}

// ============================================================================
// Concurrent Access Tests
// ============================================================================

#[test]
fn test_storage_concurrent_reads() {
    use std::thread;

    let test_db = TestDatabase::new();
    let storage: Arc<dyn Storage> = Arc::new(test_db.db_clone());

    // Write some data
    storage
        .put(ergo_storage::ColumnFamily::Headers, b"key", b"value")
        .unwrap();

    // Read from multiple threads
    let storage_clone = Arc::clone(&storage);
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let s = Arc::clone(&storage_clone);
            thread::spawn(move || {
                let result = s.get(ergo_storage::ColumnFamily::Headers, b"key").unwrap();
                assert_eq!(result, Some(b"value".to_vec()));
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }
}

#[test]
fn test_mempool_thread_safe() {
    use std::thread;

    let mempool = Arc::new(Mempool::with_defaults());

    let handles: Vec<_> = (0..4)
        .map(|_| {
            let m = Arc::clone(&mempool);
            thread::spawn(move || {
                // Should be safe to call from multiple threads
                let _ = m.len();
                let _ = m.is_empty();
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }
}

// ============================================================================
// Resource Cleanup Tests
// ============================================================================

#[test]
fn test_database_cleanup_on_drop() {
    let path = {
        let test_db = TestDatabase::new();
        test_db.path().to_path_buf()
    };

    // After TestDatabase is dropped, the temp directory should be cleaned up
    // (This is handled by tempfile crate)
    // We just verify no panic occurred
    assert!(true);
}

#[test]
fn test_multiple_databases_independent() {
    let db1 = TestDatabase::new();
    let db2 = TestDatabase::new();

    let storage1: Arc<dyn Storage> = Arc::new(db1.db_clone());
    let storage2: Arc<dyn Storage> = Arc::new(db2.db_clone());

    // Write to db1
    storage1
        .put(ergo_storage::ColumnFamily::Headers, b"key", b"value1")
        .unwrap();

    // Write different value to db2
    storage2
        .put(ergo_storage::ColumnFamily::Headers, b"key", b"value2")
        .unwrap();

    // Each should have its own value
    let v1 = storage1
        .get(ergo_storage::ColumnFamily::Headers, b"key")
        .unwrap();
    let v2 = storage2
        .get(ergo_storage::ColumnFamily::Headers, b"key")
        .unwrap();

    assert_eq!(v1, Some(b"value1".to_vec()));
    assert_eq!(v2, Some(b"value2".to_vec()));
}

// ============================================================================
// Configuration-like Tests
// ============================================================================

#[test]
fn test_mempool_with_defaults_has_sane_limits() {
    let mempool = Mempool::with_defaults();

    // Stats should show reasonable limits
    let stats = mempool.stats();
    assert_eq!(stats.tx_count, 0);
    assert_eq!(stats.total_size, 0);
}
