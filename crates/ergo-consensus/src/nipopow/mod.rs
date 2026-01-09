//! NiPoPoW (Non-Interactive Proofs of Proof-of-Work) support.
//!
//! This module implements NiPoPoW as described in the KMZ17 paper (FC20 version).
//! It provides:
//! - Interlinks data structures and algorithms
//! - Proof generation and validation
//! - Light client bootstrap support
//!
//! # Overview
//!
//! NiPoPoW enables efficient proofs that a chain represents significant PoW.
//! It works by:
//! 1. Classifying blocks by their "level" (based on how much they exceeded target)
//! 2. Maintaining "interlinks" in each block to higher-level superblocks
//! 3. Generating proofs by selecting relevant superblocks at each level
//!
//! # Key Concepts
//!
//! - **Level (μ)**: A block's level is based on how much its PoW exceeded the target
//! - **Superblock**: A block at level μ ≥ 1 (exceeded target by 2^μ factor)
//! - **Interlinks**: A vector of block IDs creating a skip-list structure
//! - **Proof**: Selected headers + interlinks proofs demonstrating chain work

mod algos;
mod interlinks;
mod popow_header;
mod proof;
#[cfg(test)]
pub(crate) mod test_helpers;
mod verifier;

pub use algos::{best_arg, max_level_of, NipopowAlgos};
pub use interlinks::{pack_interlinks, unpack_interlinks, INTERLINKS_VECTOR_PREFIX};
pub use popow_header::PoPowHeader;
pub use proof::{NipopowProof, PoPowParams};
pub use verifier::{NipopowVerificationResult, NipopowVerifier};

/// Default security parameter (minimum superchain length).
pub const DEFAULT_M: i32 = 30;

/// Default suffix length.
pub const DEFAULT_K: i32 = 30;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_exports() {
        // Verify key types are exported
        let _params = PoPowParams::default();
        assert_eq!(DEFAULT_M, 30);
        assert_eq!(DEFAULT_K, 30);
    }
}
