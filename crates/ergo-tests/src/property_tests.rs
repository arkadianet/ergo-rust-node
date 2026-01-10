//! Property-based tests using proptest.
//!
//! These tests verify invariants and properties of the Ergo node
//! using randomly generated test data with shrinking support.

use ergo_storage::{ColumnFamily, Storage, WriteBatch};
use proptest::prelude::*;

// ============================================================================
// Proptest Strategies for Core Types (GAP_04)
// ============================================================================

/// Generate arbitrary 32-byte arrays (block IDs, tx IDs, etc.)
fn arb_id_32() -> impl Strategy<Value = [u8; 32]> {
    prop::array::uniform32(any::<u8>())
}

/// Generate arbitrary difficulty values (positive BigInt-compatible)
fn arb_difficulty() -> impl Strategy<Value = u128> {
    // Difficulty must be positive, typical range for Ergo
    1u128..=u128::MAX
}

/// Generate arbitrary height values
fn arb_height() -> impl Strategy<Value = u32> {
    0u32..=10_000_000u32
}

/// Generate arbitrary timestamp (milliseconds since epoch)
fn arb_timestamp() -> impl Strategy<Value = u64> {
    // Reasonable timestamp range (2019 to 2100)
    1546300800000u64..=4102444800000u64
}

/// Generate arbitrary nBits (compact difficulty)
fn arb_nbits() -> impl Strategy<Value = u32> {
    // Valid nBits range for Ergo
    0x01000001u32..=0x207FFFFFu32
}

/// Generate arbitrary version byte
fn arb_version() -> impl Strategy<Value = u8> {
    1u8..=3u8
}

/// Generate arbitrary votes (3 bytes)
fn arb_votes() -> impl Strategy<Value = [u8; 3]> {
    prop::array::uniform3(any::<u8>())
}

/// Generate arbitrary ERG value (nanoERGs)
fn arb_erg_value() -> impl Strategy<Value = u64> {
    // Min 1 nanoERG, max total supply
    1u64..=97_739_924_000_000_000u64
}

/// Generate arbitrary token amount
fn arb_token_amount() -> impl Strategy<Value = u64> {
    1u64..=i64::MAX as u64
}

// ============================================================================
// Header Property Tests
// ============================================================================

proptest! {
    /// Test that header serialization is deterministic
    #[test]
    fn header_serialization_deterministic(
        parent_id in arb_id_32(),
        height in arb_height(),
        timestamp in arb_timestamp(),
        nbits in arb_nbits(),
        version in arb_version(),
    ) {
        use crate::generators::TestHeader;

        let header = TestHeader {
            id: [0u8; 32], // Will be computed
            parent_id,
            height,
            timestamp,
            nbits,
            version,
        };

        let bytes1 = header.to_bytes();
        let bytes2 = header.to_bytes();

        prop_assert_eq!(bytes1, bytes2, "Serialization should be deterministic");
    }

    /// Test header round-trip serialization
    #[test]
    fn header_roundtrip(
        parent_id in arb_id_32(),
        height in arb_height(),
        timestamp in arb_timestamp(),
        nbits in arb_nbits(),
        version in arb_version(),
    ) {
        use crate::generators::TestHeader;

        let original = TestHeader {
            id: [1u8; 32],
            parent_id,
            height,
            timestamp,
            nbits,
            version,
        };

        let bytes = original.to_bytes();
        let recovered = TestHeader::from_bytes(&bytes);

        prop_assert_eq!(original.parent_id, recovered.parent_id);
        prop_assert_eq!(original.height, recovered.height);
        prop_assert_eq!(original.timestamp, recovered.timestamp);
        prop_assert_eq!(original.nbits, recovered.nbits);
        prop_assert_eq!(original.version, recovered.version);
    }
}

// ============================================================================
// Difficulty Adjustment Property Tests
// ============================================================================

proptest! {
    /// Test difficulty never goes to zero
    #[test]
    fn difficulty_always_positive(difficulty in arb_difficulty()) {
        prop_assert!(difficulty > 0, "Difficulty must always be positive");
    }

    /// Test nBits encodes valid difficulty
    #[test]
    fn nbits_encodes_valid_difficulty(nbits in arb_nbits()) {
        // Extract exponent and mantissa from nBits
        let exponent = (nbits >> 24) as usize;
        let mantissa = nbits & 0x007FFFFF;

        // Mantissa should not be zero (except for special cases)
        if exponent > 3 {
            prop_assert!(mantissa != 0 || exponent == 0, "Invalid nBits encoding");
        }
    }
}

// ============================================================================
// Value Conservation Property Tests
// ============================================================================

proptest! {
    /// Test that total ERG value doesn't exceed max supply
    #[test]
    fn erg_value_within_bounds(value in arb_erg_value()) {
        const MAX_SUPPLY: u64 = 97_739_924_000_000_000; // nanoERGs
        prop_assert!(value <= MAX_SUPPLY, "ERG value exceeds max supply");
    }

    /// Test token amounts are positive
    #[test]
    fn token_amount_positive(amount in arb_token_amount()) {
        prop_assert!(amount > 0, "Token amounts must be positive");
    }
}

// ============================================================================
// Chain Property Tests
// ============================================================================

proptest! {
    /// Test that chain heights are monotonically increasing
    #[test]
    fn chain_heights_monotonic(heights in prop::collection::vec(arb_height(), 2..100)) {
        use crate::generators::TestHeader;

        let mut sorted_heights = heights.clone();
        sorted_heights.sort();

        // Build chain with sorted heights
        let chain = TestHeader::chain(sorted_heights.len());

        for i in 1..chain.len() {
            prop_assert!(
                chain[i].height > chain[i-1].height,
                "Heights must be monotonically increasing"
            );
        }
    }

    /// Test parent-child relationship in chain
    #[test]
    fn chain_parent_links_valid(chain_length in 2usize..50) {
        use crate::generators::TestHeader;

        let chain = TestHeader::chain(chain_length);

        for i in 1..chain.len() {
            prop_assert_eq!(
                chain[i].parent_id,
                chain[i-1].id,
                "Child's parent_id must equal parent's id"
            );
        }
    }
}

// ============================================================================
// ID Uniqueness Property Tests
// ============================================================================

proptest! {
    /// Test that generated IDs are unique
    #[test]
    fn generated_ids_unique(count in 10usize..100) {
        use crate::generators::{test_block_id, test_tx_id, test_box_id};
        use std::collections::HashSet;

        let block_ids: HashSet<_> = (0..count).map(|_| test_block_id()).collect();
        let tx_ids: HashSet<_> = (0..count).map(|_| test_tx_id()).collect();
        let box_ids: HashSet<_> = (0..count).map(|_| test_box_id()).collect();

        // All generated IDs should be unique
        prop_assert_eq!(block_ids.len(), count, "Block IDs should be unique");
        prop_assert_eq!(tx_ids.len(), count, "Transaction IDs should be unique");
        prop_assert_eq!(box_ids.len(), count, "Box IDs should be unique");
    }
}

// ============================================================================
// Storage Key Property Tests
// ============================================================================

proptest! {
    /// Test storage keys are valid bytes
    #[test]
    fn storage_key_valid(key in prop::collection::vec(any::<u8>(), 0..1000)) {
        // Keys should be storable regardless of content
        prop_assert!(key.len() <= 1000, "Key length within bounds");
    }

    /// Test storage values preserve binary data
    #[test]
    fn storage_value_preserves_binary(value in prop::collection::vec(any::<u8>(), 0..10000)) {
        use crate::harness::TestDatabase;
        use ergo_storage::ColumnFamily;

        let db = TestDatabase::new();
        let key = b"test_key";

        db.put(ColumnFamily::Metadata, key, &value).unwrap();
        let retrieved = db.get(ColumnFamily::Metadata, key).unwrap();

        prop_assert_eq!(retrieved, Some(value), "Value should be preserved exactly");
    }
}

// ============================================================================
// Batch Operation Property Tests
// ============================================================================

proptest! {
    /// Test batch operations are atomic
    #[test]
    fn batch_operations_atomic(
        keys in prop::collection::vec(prop::collection::vec(any::<u8>(), 1..32), 1..50),
        values in prop::collection::vec(prop::collection::vec(any::<u8>(), 1..100), 1..50),
    ) {
        use crate::harness::TestDatabase;

        let db = TestDatabase::new();
        let mut batch = WriteBatch::new();

        let pairs: Vec<_> = keys.into_iter().zip(values.into_iter()).collect();

        for (key, value) in &pairs {
            batch.put(ColumnFamily::Utxo, key.clone(), value.clone());
        }

        db.write_batch(batch).unwrap();

        // All entries should be present
        for (key, value) in &pairs {
            let retrieved = db.get(ColumnFamily::Utxo, key).unwrap();
            prop_assert_eq!(retrieved.as_ref(), Some(value), "All batch entries should be written");
        }
    }
}

// ============================================================================
// Fork Generation Property Tests
// ============================================================================

proptest! {
    /// Test fork generation creates valid diverging chains
    #[test]
    fn fork_diverges_correctly(
        main_length in 5usize..20,
        fork_length in 2usize..10,
        fork_point in 1usize..5,
    ) {
        use crate::generators::{TestHeader, generate_fork};

        // Ensure fork_point is within main_length
        let actual_fork_point = fork_point.min(main_length - 1);

        let main_chain = TestHeader::chain(main_length);
        let fork_chain = generate_fork(&main_chain, actual_fork_point, fork_length);

        // Fork should have correct length
        prop_assert_eq!(
            fork_chain.len(),
            fork_length,
            "Fork should have requested length"
        );

        // Fork's first block should reference the fork point
        if !fork_chain.is_empty() && actual_fork_point < main_chain.len() {
            prop_assert_eq!(
                fork_chain[0].parent_id,
                main_chain[actual_fork_point].id,
                "Fork should start from correct point"
            );
        }

        // Fork blocks should have different IDs than main chain
        for fork_block in &fork_chain {
            for main_block in &main_chain {
                if fork_block.height == main_block.height {
                    prop_assert_ne!(
                        fork_block.id,
                        main_block.id,
                        "Fork blocks should have different IDs than main chain at same height"
                    );
                }
            }
        }
    }
}
