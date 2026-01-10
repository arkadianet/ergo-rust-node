//! State management integration tests.
//!
//! These tests cover high-level state management scenarios.
//! Note: Detailed unit tests for UtxoState with BoxEntry and StateChange
//! are in ergo-state/src/utxo.rs. This module focuses on integration patterns.
//!
//! Tests covered in ergo-state unit tests include:
//! - Box application and spending
//! - Rollback with box restoration
//! - Index maintenance
//! - State root calculation

use ergo_state::UtxoState;
use ergo_storage::Database;
use std::sync::Arc;
use tempfile::TempDir;

// ============================================================================
// Test Helpers
// ============================================================================

fn create_test_state() -> (UtxoState, TempDir) {
    let tmp = TempDir::new().unwrap();
    let db = Database::open(tmp.path()).unwrap();
    let state = UtxoState::new(Arc::new(db));
    (state, tmp)
}

// ============================================================================
// Basic State Initialization Tests
// ============================================================================

#[test]
fn test_state_initialization() {
    let (state, _tmp) = create_test_state();

    // State should start at height 0
    assert_eq!(state.height(), 0);
    // State root should be 32 bytes (empty tree digest)
    assert_eq!(state.state_root().len(), 32);
}

#[test]
fn test_multiple_state_instances() {
    // Create multiple independent state instances
    let (state1, _tmp1) = create_test_state();
    let (state2, _tmp2) = create_test_state();

    // Both should start at height 0
    assert_eq!(state1.height(), 0);
    assert_eq!(state2.height(), 0);

    // State roots should be identical for empty states
    assert_eq!(state1.state_root(), state2.state_root());
}

// ============================================================================
// Sync Mode Tests
// ============================================================================

#[test]
fn test_sync_mode_toggle() {
    let (state, _tmp) = create_test_state();

    // Initially not in sync mode
    assert!(!state.is_sync_mode());

    // Enable sync mode
    state.set_sync_mode(true);
    assert!(state.is_sync_mode());

    // Disable sync mode
    state.set_sync_mode(false);
    assert!(!state.is_sync_mode());
}

#[test]
fn test_sync_mode_multiple_toggles() {
    let (state, _tmp) = create_test_state();

    for _ in 0..10 {
        state.set_sync_mode(true);
        assert!(state.is_sync_mode());
        state.set_sync_mode(false);
        assert!(!state.is_sync_mode());
    }
}

// ============================================================================
// Thread Safety Tests
// ============================================================================

#[test]
fn test_state_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<UtxoState>();
}

#[test]
fn test_state_arc_sharing() {
    let (state, _tmp) = create_test_state();
    let state = Arc::new(state);

    // Clone Arc for different "threads"
    let state1 = Arc::clone(&state);
    let state2 = Arc::clone(&state);

    // Both should see the same height
    assert_eq!(state1.height(), state2.height());
    assert_eq!(state1.state_root(), state2.state_root());
}

#[test]
fn test_concurrent_reads() {
    use std::thread;

    let (state, _tmp) = create_test_state();
    let state = Arc::new(state);

    let handles: Vec<_> = (0..4)
        .map(|_| {
            let state_clone = Arc::clone(&state);
            thread::spawn(move || {
                for _ in 0..100 {
                    let _ = state_clone.height();
                    let _ = state_clone.state_root();
                    let _ = state_clone.is_sync_mode();
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }
}

// ============================================================================
// State Root Consistency Tests
// ============================================================================

#[test]
fn test_empty_state_root_is_deterministic() {
    // Create multiple states and verify they have the same root
    let roots: Vec<Vec<u8>> = (0..5)
        .map(|_| {
            let (state, _tmp) = create_test_state();
            state.state_root()
        })
        .collect();

    // All roots should be identical
    for root in &roots[1..] {
        assert_eq!(&roots[0], root);
    }
}

#[test]
fn test_state_root_length() {
    let (state, _tmp) = create_test_state();

    // State root should always be 32 bytes (256-bit hash)
    let root = state.state_root();
    assert_eq!(root.len(), 32);
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_rollback_to_current_height() {
    let (state, _tmp) = create_test_state();

    // Rollback to current height should be a no-op or succeed
    let result = state.rollback(0);
    // Either succeeds or fails gracefully
    match result {
        Ok(()) => assert_eq!(state.height(), 0),
        Err(_) => {} // Also acceptable
    }
}

#[test]
fn test_rollback_to_future_height_fails() {
    let (state, _tmp) = create_test_state();

    // Cannot rollback to a future height
    let result = state.rollback(100);
    assert!(result.is_err(), "Should not rollback to future height");
}

#[test]
fn test_rollback_negative_delta() {
    let (state, _tmp) = create_test_state();

    // Height is 0, trying to rollback to 1 should fail
    let result = state.rollback(1);
    assert!(result.is_err());
}

// ============================================================================
// Database Persistence Tests
// ============================================================================

#[test]
fn test_state_uses_database() {
    let tmp = TempDir::new().unwrap();
    let db = Database::open(tmp.path()).unwrap();

    {
        let state = UtxoState::new(Arc::new(db.clone()));
        assert_eq!(state.height(), 0);
    }

    // Reopening should give consistent state
    {
        let state = UtxoState::new(Arc::new(db));
        assert_eq!(state.height(), 0);
    }
}

// ============================================================================
// Height Tracking Tests
// ============================================================================

#[test]
fn test_initial_height_is_zero() {
    let (state, _tmp) = create_test_state();
    assert_eq!(state.height(), 0);
}

#[test]
fn test_height_type_is_u32() {
    let (state, _tmp) = create_test_state();
    let height: u32 = state.height();
    assert_eq!(height, 0u32);
}

// ============================================================================
// Box Query Tests (Without Creating Boxes)
// ============================================================================

#[test]
fn test_contains_nonexistent_box() {
    let (state, _tmp) = create_test_state();

    // Query for a box that doesn't exist
    let fake_box_id = [0u8; 32];
    let result = state.contains_box_bytes(&fake_box_id);

    assert!(result.is_ok());
    assert!(!result.unwrap());
}

#[test]
fn test_get_nonexistent_box() {
    let (state, _tmp) = create_test_state();

    // Query for a box that doesn't exist
    let fake_box_id = [0u8; 32];
    let result = state.get_box_by_bytes(&fake_box_id);

    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

#[test]
fn test_query_many_nonexistent_boxes() {
    let (state, _tmp) = create_test_state();

    // Query for many boxes that don't exist
    for i in 0..100u8 {
        let mut box_id = [0u8; 32];
        box_id[0] = i;

        let result = state.contains_box_bytes(&box_id);
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }
}
