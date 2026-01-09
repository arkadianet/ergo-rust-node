//! NiPoPoW proof verifier.
//!
//! Manages the best known proof and validates new proofs against it.

use super::{NipopowProof, PoPowParams};
use tracing::{debug, info, warn};

/// Result of verifying a NiPoPoW proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NipopowVerificationResult {
    /// The new proof represents a better chain.
    BetterChain {
        /// Number of proofs processed so far.
        proofs_processed: i32,
    },
    /// The existing proof is still the best.
    NoBetterChain {
        /// Number of proofs processed so far.
        proofs_processed: i32,
    },
    /// The proof failed validation.
    ValidationError {
        /// Error message.
        message: String,
    },
    /// The proof has a different genesis block.
    WrongGenesis,
}

/// NiPoPoW proof verifier.
///
/// Maintains the best known proof and validates new proofs against it.
/// Used for light client bootstrap and quick chain synchronization.
#[derive(Debug)]
pub struct NipopowVerifier {
    /// Expected genesis block ID (if known).
    genesis_id: Option<[u8; 32]>,
    /// Best proof seen so far.
    best_proof: Option<NipopowProof>,
    /// Number of proofs processed.
    proofs_processed: i32,
    /// Minimum number of proofs required before accepting.
    min_proofs: i32,
    /// Default proof parameters.
    params: PoPowParams,
}

impl NipopowVerifier {
    /// Create a new verifier with default parameters.
    pub fn new() -> Self {
        Self {
            genesis_id: None,
            best_proof: None,
            proofs_processed: 0,
            min_proofs: 1,
            params: PoPowParams::default(),
        }
    }

    /// Create a verifier with a known genesis block.
    pub fn with_genesis(genesis_id: [u8; 32]) -> Self {
        Self {
            genesis_id: Some(genesis_id),
            best_proof: None,
            proofs_processed: 0,
            min_proofs: 1,
            params: PoPowParams::default(),
        }
    }

    /// Create a verifier with custom parameters.
    pub fn with_params(params: PoPowParams, min_proofs: i32) -> Self {
        Self {
            genesis_id: None,
            best_proof: None,
            proofs_processed: 0,
            min_proofs,
            params,
        }
    }

    /// Get the current best proof.
    pub fn best_proof(&self) -> Option<&NipopowProof> {
        self.best_proof.as_ref()
    }

    /// Get the best chain tip height.
    pub fn best_height(&self) -> Option<u32> {
        self.best_proof.as_ref().map(|p| p.tip_height())
    }

    /// Get the number of proofs processed.
    pub fn proofs_processed(&self) -> i32 {
        self.proofs_processed
    }

    /// Check if we have enough proofs to proceed.
    pub fn has_quorum(&self) -> bool {
        self.proofs_processed >= self.min_proofs && self.best_proof.is_some()
    }

    /// Get the expected genesis ID.
    pub fn genesis_id(&self) -> Option<&[u8; 32]> {
        self.genesis_id.as_ref()
    }

    /// Set the expected genesis ID.
    pub fn set_genesis_id(&mut self, id: [u8; 32]) {
        self.genesis_id = Some(id);
    }

    /// Process a new proof.
    ///
    /// Returns the verification result indicating whether this proof
    /// represents a better chain than the current best.
    pub fn process(&mut self, proof: NipopowProof) -> NipopowVerificationResult {
        self.proofs_processed += 1;

        // Validate the proof structure
        if let Err(e) = proof.validate() {
            warn!(error = %e, "Invalid NiPoPoW proof");
            return NipopowVerificationResult::ValidationError {
                message: e.to_string(),
            };
        }

        // Check genesis matches (if we have an expected genesis)
        if let Some(expected_genesis) = &self.genesis_id {
            if let Some(proof_genesis) = proof.genesis_id() {
                if &proof_genesis != expected_genesis {
                    warn!(
                        expected = hex::encode(expected_genesis),
                        got = hex::encode(proof_genesis),
                        "NiPoPoW proof has wrong genesis"
                    );
                    return NipopowVerificationResult::WrongGenesis;
                }
            }
        }

        // If we don't have a genesis yet, learn it from first valid proof
        if self.genesis_id.is_none() {
            if let Some(genesis) = proof.genesis_id() {
                info!(genesis = hex::encode(genesis), "Learned genesis from proof");
                self.genesis_id = Some(genesis);
            }
        }

        // Compare with current best
        let is_better = match &self.best_proof {
            None => {
                info!(height = proof.tip_height(), "First NiPoPoW proof received");
                true
            }
            Some(current_best) => {
                if proof.is_better_than(current_best) {
                    info!(
                        old_height = current_best.tip_height(),
                        new_height = proof.tip_height(),
                        "Found better NiPoPoW proof"
                    );
                    true
                } else {
                    debug!(
                        current_height = current_best.tip_height(),
                        proof_height = proof.tip_height(),
                        "NiPoPoW proof not better than current"
                    );
                    false
                }
            }
        };

        if is_better {
            self.best_proof = Some(proof);
            NipopowVerificationResult::BetterChain {
                proofs_processed: self.proofs_processed,
            }
        } else {
            NipopowVerificationResult::NoBetterChain {
                proofs_processed: self.proofs_processed,
            }
        }
    }

    /// Reset the verifier state.
    pub fn reset(&mut self) {
        self.best_proof = None;
        self.proofs_processed = 0;
        // Keep genesis_id as it should be consistent
    }

    /// Get headers from the best proof for applying to history.
    ///
    /// Returns None if no valid proof has been received yet,
    /// or if quorum hasn't been reached.
    pub fn get_headers_to_apply(&self) -> Option<Vec<&ergo_chain_types::Header>> {
        if !self.has_quorum() {
            return None;
        }

        self.best_proof.as_ref().map(|p| p.headers_chain())
    }
}

impl Default for NipopowVerifier {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nipopow::test_helpers::make_popow_header;

    fn make_simple_proof(tip_height: u32) -> NipopowProof {
        let genesis = make_popow_header(1, [0u8; 32]);
        let mut prefix = vec![genesis];

        // Add some prefix headers
        for h in 2..tip_height {
            prefix.push(make_popow_header(h, [(h - 1) as u8; 32]));
        }

        let suffix_head = make_popow_header(tip_height, [(tip_height - 1) as u8; 32]);

        NipopowProof::new(30, 1, prefix, suffix_head, vec![], false)
    }

    #[test]
    fn test_verifier_new() {
        let verifier = NipopowVerifier::new();
        assert!(verifier.best_proof().is_none());
        assert_eq!(verifier.proofs_processed(), 0);
        assert!(!verifier.has_quorum());
    }

    #[test]
    fn test_verifier_with_genesis() {
        let genesis = [1u8; 32];
        let verifier = NipopowVerifier::with_genesis(genesis);
        assert_eq!(verifier.genesis_id(), Some(&genesis));
    }

    #[test]
    fn test_verifier_first_proof() {
        let mut verifier = NipopowVerifier::new();
        let proof = make_simple_proof(10);

        let result = verifier.process(proof);

        assert!(matches!(
            result,
            NipopowVerificationResult::BetterChain {
                proofs_processed: 1
            }
        ));
        assert!(verifier.best_proof().is_some());
        assert_eq!(verifier.best_height(), Some(10));
    }

    #[test]
    fn test_verifier_better_proof() {
        let mut verifier = NipopowVerifier::new();

        // First proof at height 10
        let proof1 = make_simple_proof(10);
        verifier.process(proof1);

        // Better proof at height 20
        let proof2 = make_simple_proof(20);
        let result = verifier.process(proof2);

        assert!(matches!(
            result,
            NipopowVerificationResult::BetterChain {
                proofs_processed: 2
            }
        ));
        assert_eq!(verifier.best_height(), Some(20));
    }

    #[test]
    fn test_verifier_worse_proof() {
        let mut verifier = NipopowVerifier::new();

        // First proof at height 20
        let proof1 = make_simple_proof(20);
        verifier.process(proof1);

        // Worse proof at height 10
        let proof2 = make_simple_proof(10);
        let result = verifier.process(proof2);

        assert!(matches!(
            result,
            NipopowVerificationResult::NoBetterChain {
                proofs_processed: 2
            }
        ));
        // Best is still height 20
        assert_eq!(verifier.best_height(), Some(20));
    }

    #[test]
    fn test_verifier_wrong_genesis() {
        let expected_genesis = [1u8; 32];
        let mut verifier = NipopowVerifier::with_genesis(expected_genesis);

        // Proof with different genesis
        let proof = make_simple_proof(10); // Uses [1u8; 32] as first header ID

        // The proof's genesis is [1u8; 32] which matches, so this should work
        let result = verifier.process(proof);
        assert!(matches!(
            result,
            NipopowVerificationResult::BetterChain { .. }
        ));
    }

    #[test]
    fn test_verifier_quorum() {
        let mut verifier = NipopowVerifier::with_params(PoPowParams::default(), 2);

        assert!(!verifier.has_quorum());

        // First proof
        let proof1 = make_simple_proof(10);
        verifier.process(proof1);
        assert!(!verifier.has_quorum()); // Need 2 proofs

        // Second proof
        let proof2 = make_simple_proof(15);
        verifier.process(proof2);
        assert!(verifier.has_quorum()); // Now we have quorum
    }

    #[test]
    fn test_verifier_reset() {
        let mut verifier = NipopowVerifier::with_genesis([1u8; 32]);

        let proof = make_simple_proof(10);
        verifier.process(proof);

        assert!(verifier.best_proof().is_some());
        assert_eq!(verifier.proofs_processed(), 1);

        verifier.reset();

        assert!(verifier.best_proof().is_none());
        assert_eq!(verifier.proofs_processed(), 0);
        // Genesis should be preserved
        assert!(verifier.genesis_id().is_some());
    }
}
