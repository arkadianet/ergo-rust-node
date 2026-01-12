//! Test vectors for Autolykos v2 PoW verification.
//!
//! Provides utilities for loading real mainnet header fixtures
//! and mutating them for negative test cases.

use ergo_chain_types::Header;
use sigma_ser::ScorexSerializable;
use std::path::Path;

/// Load a header fixture from the tests/fixtures directory.
///
/// # Panics
/// Panics if the fixture file cannot be read or parsed.
pub fn load_header_fixture(filename: &str) -> Header {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(filename);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("Failed to read fixture {}: {}", path.display(), e));
    Header::scorex_parse_bytes(&bytes)
        .unwrap_or_else(|e| panic!("Failed to parse fixture {}: {}", path.display(), e))
}

/// Mutate a header's nonce to invalidate PoW (struct-level, not bytes).
///
/// # Panics
/// Panics if nonce is empty (should never happen for valid mainnet headers).
pub fn mutate_header_nonce(header: &Header) -> Header {
    assert!(
        !header.autolykos_solution.nonce.is_empty(),
        "Cannot mutate empty nonce - fixture may be invalid"
    );
    let mut mutated = header.clone();
    mutated.autolykos_solution.nonce[0] ^= 0xFF;
    mutated
}
