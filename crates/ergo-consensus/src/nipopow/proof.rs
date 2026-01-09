//! NiPoPoW proof structure and validation.

use super::{algos::best_arg, PoPowHeader};
use crate::{ConsensusError, ConsensusResult};
use ergo_chain_types::Header;
use std::collections::HashSet;

/// NiPoPoW proof parameters.
#[derive(Debug, Clone, Copy)]
pub struct PoPowParams {
    /// Security parameter - minimum superchain length at each level.
    pub m: i32,
    /// Suffix length - number of headers at the end.
    pub k: i32,
    /// Whether this is a continuous proof (includes difficulty headers).
    pub continuous: bool,
}

impl Default for PoPowParams {
    fn default() -> Self {
        Self {
            m: super::DEFAULT_M,
            k: super::DEFAULT_K,
            continuous: false,
        }
    }
}

impl PoPowParams {
    /// Create new parameters.
    pub fn new(m: i32, k: i32, continuous: bool) -> Self {
        Self { m, k, continuous }
    }

    /// Create parameters for P2P proof requests.
    pub fn for_p2p() -> Self {
        Self {
            m: super::DEFAULT_M,
            k: super::DEFAULT_K,
            continuous: true,
        }
    }
}

/// A NiPoPoW proof representing a chain of work.
///
/// The proof consists of:
/// - A prefix: selected superblocks from the chain
/// - A suffix: the last k headers in full form
#[derive(Debug, Clone)]
pub struct NipopowProof {
    /// Security parameter used to generate this proof.
    pub m: i32,
    /// Suffix length.
    pub k: i32,
    /// Prefix headers (selected superblocks).
    pub prefix: Vec<PoPowHeader>,
    /// First header of the suffix (with interlinks).
    pub suffix_head: PoPowHeader,
    /// Remaining suffix headers (k-1 headers, no interlinks needed).
    pub suffix_tail: Vec<Header>,
    /// Whether this is a continuous proof.
    pub continuous: bool,
}

impl NipopowProof {
    /// Create a new NiPoPoW proof.
    pub fn new(
        m: i32,
        k: i32,
        prefix: Vec<PoPowHeader>,
        suffix_head: PoPowHeader,
        suffix_tail: Vec<Header>,
        continuous: bool,
    ) -> Self {
        Self {
            m,
            k,
            prefix,
            suffix_head,
            suffix_tail,
            continuous,
        }
    }

    /// Get the genesis block ID from this proof.
    pub fn genesis_id(&self) -> Option<[u8; 32]> {
        self.prefix.first().map(|h| h.id())
    }

    /// Get all headers in the proof chain (prefix + suffix).
    pub fn headers_chain(&self) -> Vec<&Header> {
        let mut chain: Vec<&Header> = self.prefix.iter().map(|ph| &ph.header).collect();
        chain.push(&self.suffix_head.header);
        chain.extend(self.suffix_tail.iter());
        chain
    }

    /// Get the height of the proof tip (last header).
    pub fn tip_height(&self) -> u32 {
        self.suffix_tail
            .last()
            .map(|h| h.height)
            .unwrap_or_else(|| self.suffix_head.height())
    }

    /// Get the ID of the proof tip.
    pub fn tip_id(&self) -> [u8; 32] {
        if let Some(last) = self.suffix_tail.last() {
            let id_bytes: &[u8] = last.id.0.as_ref();
            let mut result = [0u8; 32];
            result.copy_from_slice(id_bytes);
            result
        } else {
            self.suffix_head.id()
        }
    }

    /// Check if the proof has valid heights (strictly increasing).
    pub fn has_valid_heights(&self) -> bool {
        let heights: Vec<u32> = self.headers_chain().iter().map(|h| h.height).collect();

        for window in heights.windows(2) {
            if window[0] >= window[1] {
                return false;
            }
        }

        true
    }

    /// Check if headers are properly connected via parent or interlinks.
    pub fn has_valid_connections(&self) -> bool {
        // Check prefix connections
        for window in self.prefix.windows(2) {
            let prev = &window[0];
            let curr = &window[1];

            if !curr.connects_to(&prev.id()) {
                return false;
            }
        }

        // Check suffix_head connects to last prefix header
        if let Some(last_prefix) = self.prefix.last() {
            if !self.suffix_head.connects_to(&last_prefix.id()) {
                return false;
            }
        }

        // Check suffix_tail connections (simple parent chain)
        let mut prev_id = self.suffix_head.id();
        for header in &self.suffix_tail {
            let parent_id: &[u8] = header.parent_id.0.as_ref();
            if parent_id != prev_id {
                return false;
            }
            let id_bytes: &[u8] = header.id.0.as_ref();
            prev_id.copy_from_slice(id_bytes);
        }

        true
    }

    /// Validate the proof structure.
    pub fn validate(&self) -> ConsensusResult<()> {
        // Check minimum requirements
        if self.prefix.is_empty() {
            return Err(ConsensusError::Validation(
                "NiPoPoW proof prefix cannot be empty".to_string(),
            ));
        }

        if self.k < 1 {
            return Err(ConsensusError::Validation(
                "NiPoPoW k parameter must be >= 1".to_string(),
            ));
        }

        // Suffix should have exactly k headers (suffix_head + k-1 tail)
        let suffix_len = 1 + self.suffix_tail.len();
        if suffix_len != self.k as usize {
            return Err(ConsensusError::Validation(format!(
                "NiPoPoW suffix length mismatch: expected {}, got {}",
                self.k, suffix_len
            )));
        }

        // Check heights are valid
        if !self.has_valid_heights() {
            return Err(ConsensusError::Validation(
                "NiPoPoW proof has invalid heights".to_string(),
            ));
        }

        // Check connections are valid
        if !self.has_valid_connections() {
            return Err(ConsensusError::Validation(
                "NiPoPoW proof has invalid connections".to_string(),
            ));
        }

        Ok(())
    }

    /// Check if this proof is valid.
    pub fn is_valid(&self) -> bool {
        self.validate().is_ok()
    }

    /// Compare this proof against another, returning true if this is better.
    ///
    /// Comparison is based on the best_arg score of the diverging chain portions.
    pub fn is_better_than(&self, other: &NipopowProof) -> bool {
        if !self.is_valid() {
            return false;
        }
        if !other.is_valid() {
            return true;
        }

        // Find lowest common ancestor (branching point)
        let this_ids: HashSet<[u8; 32]> = self
            .headers_chain()
            .iter()
            .map(|h| {
                let id_bytes: &[u8] = h.id.0.as_ref();
                let mut result = [0u8; 32];
                result.copy_from_slice(id_bytes);
                result
            })
            .collect();

        // Find the first header in other that's also in this
        let common_height = other
            .headers_chain()
            .iter()
            .find(|h| {
                let id_bytes: &[u8] = h.id.0.as_ref();
                let mut id = [0u8; 32];
                id.copy_from_slice(id_bytes);
                this_ids.contains(&id)
            })
            .map(|h| h.height);

        let branch_height = match common_height {
            Some(h) => h,
            None => {
                // No common ancestor - compare total chain lengths
                return self.tip_height() > other.tip_height();
            }
        };

        // Get diverging portions
        let this_diverging: Vec<u32> = self
            .headers_chain()
            .iter()
            .filter(|h| h.height > branch_height)
            .map(|h| {
                // Extract PoW hit from autolykos solution for level calculation
                // For now, use a placeholder level of 0
                // TODO: Implement proper PoW hit extraction
                0u32
            })
            .collect();

        let other_diverging: Vec<u32> = other
            .headers_chain()
            .iter()
            .filter(|h| h.height > branch_height)
            .map(|_| 0u32)
            .collect();

        let this_score = best_arg(&this_diverging, self.m);
        let other_score = best_arg(&other_diverging, other.m);

        this_score > other_score
    }

    /// Serialize the proof to bytes.
    pub fn serialize(&self) -> Vec<u8> {
        // TODO: Implement proper serialization matching Scala format
        // For now, return empty - this needs to match Scala's NipopowProofSerializer
        Vec::new()
    }

    /// Parse a proof from bytes.
    pub fn parse(_data: &[u8]) -> ConsensusResult<Self> {
        // TODO: Implement proper deserialization matching Scala format
        Err(ConsensusError::Validation(
            "NiPoPoW proof parsing not yet implemented".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nipopow::test_helpers::{make_popow_header, make_test_header};

    #[test]
    fn test_popow_params_default() {
        let params = PoPowParams::default();
        assert_eq!(params.m, 30);
        assert_eq!(params.k, 30);
        assert!(!params.continuous);
    }

    #[test]
    fn test_proof_structure() {
        // Create a simple proof
        let genesis = make_popow_header(1, [0u8; 32]);
        let block2 = make_popow_header(2, [1u8; 32]);
        let suffix_head = make_popow_header(3, [2u8; 32]);
        let suffix_tail = vec![make_test_header(4, [3u8; 32])];

        let proof = NipopowProof::new(
            30,
            2, // k=2 means suffix_head + 1 tail
            vec![genesis, block2],
            suffix_head,
            suffix_tail,
            false,
        );

        assert_eq!(proof.tip_height(), 4);
        assert_eq!(proof.headers_chain().len(), 4);
    }

    #[test]
    fn test_proof_valid_heights() {
        let genesis = make_popow_header(1, [0u8; 32]);
        let block2 = make_popow_header(2, [1u8; 32]);
        let suffix_head = make_popow_header(3, [2u8; 32]);

        let proof = NipopowProof::new(30, 1, vec![genesis, block2], suffix_head, vec![], false);

        assert!(proof.has_valid_heights());
    }

    #[test]
    fn test_proof_invalid_heights() {
        let block1 = make_popow_header(5, [0u8; 32]);
        let block2 = make_popow_header(3, [5u8; 32]); // Height goes backwards!
        let suffix_head = make_popow_header(4, [3u8; 32]);

        let proof = NipopowProof::new(30, 1, vec![block1, block2], suffix_head, vec![], false);

        assert!(!proof.has_valid_heights());
    }

    #[test]
    fn test_proof_genesis_id() {
        let genesis = make_popow_header(1, [0u8; 32]);
        let suffix_head = make_popow_header(2, [1u8; 32]);

        let proof = NipopowProof::new(30, 1, vec![genesis.clone()], suffix_head, vec![], false);

        assert_eq!(proof.genesis_id(), Some(genesis.id()));
    }
}
