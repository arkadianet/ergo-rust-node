//! PoPowHeader - Block header with interlinks and proof.

use ergo_chain_types::Header;

/// Proof-of-Proof-of-Work header.
///
/// Contains a block header along with its interlinks vector
/// and a Merkle proof that the interlinks are valid.
#[derive(Debug, Clone)]
pub struct PoPowHeader {
    /// The block header.
    pub header: Header,
    /// Unpacked interlinks vector.
    pub interlinks: Vec<[u8; 32]>,
    /// Serialized batch Merkle proof for interlinks.
    /// Proves that interlinks are included in the extension's Merkle tree.
    pub interlinks_proof: Vec<u8>,
}

impl PoPowHeader {
    /// Create a new PoPowHeader.
    pub fn new(header: Header, interlinks: Vec<[u8; 32]>, interlinks_proof: Vec<u8>) -> Self {
        Self {
            header,
            interlinks,
            interlinks_proof,
        }
    }

    /// Get the block ID.
    pub fn id(&self) -> [u8; 32] {
        let id_bytes: &[u8] = self.header.id.0.as_ref();
        let mut result = [0u8; 32];
        result.copy_from_slice(id_bytes);
        result
    }

    /// Get the block height.
    pub fn height(&self) -> u32 {
        self.header.height
    }

    /// Get the parent block ID.
    pub fn parent_id(&self) -> [u8; 32] {
        let id_bytes: &[u8] = self.header.parent_id.0.as_ref();
        let mut result = [0u8; 32];
        result.copy_from_slice(id_bytes);
        result
    }

    /// Check if this is the genesis block.
    pub fn is_genesis(&self) -> bool {
        self.header.height == 1
    }

    /// Get the nBits (compressed difficulty target).
    pub fn nbits(&self) -> u32 {
        self.header.n_bits
    }

    /// Check if interlinks are empty.
    pub fn has_interlinks(&self) -> bool {
        !self.interlinks.is_empty()
    }

    /// Get interlink at a specific level.
    ///
    /// Returns None if level is out of bounds.
    pub fn interlink_at(&self, level: usize) -> Option<&[u8; 32]> {
        self.interlinks.get(level)
    }

    /// Get the genesis block ID from interlinks.
    ///
    /// Genesis is always the first element of interlinks.
    pub fn genesis_id(&self) -> Option<&[u8; 32]> {
        self.interlinks.first()
    }

    /// Check if this header connects to another via interlinks or parent.
    ///
    /// A header connects to another if:
    /// - Its parent_id matches the other header's ID, OR
    /// - Any of its interlinks matches the other header's ID
    pub fn connects_to(&self, other_id: &[u8; 32]) -> bool {
        if &self.parent_id() == other_id {
            return true;
        }

        self.interlinks.iter().any(|link| link == other_id)
    }
}

impl PartialEq for PoPowHeader {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl Eq for PoPowHeader {}

impl std::hash::Hash for PoPowHeader {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id().hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nipopow::test_helpers::make_test_header;

    #[test]
    fn test_popow_header_basic() {
        let header = make_test_header(100, [1u8; 32]);
        let interlinks = vec![[0u8; 32], [1u8; 32], [2u8; 32]];

        let popow = PoPowHeader::new(header, interlinks.clone(), vec![]);

        assert_eq!(popow.height(), 100);
        assert!(!popow.is_genesis());
        assert!(popow.has_interlinks());
        assert_eq!(popow.genesis_id(), Some(&[0u8; 32]));
        assert_eq!(popow.interlink_at(1), Some(&[1u8; 32]));
        assert_eq!(popow.interlink_at(10), None);
    }

    #[test]
    fn test_popow_header_genesis() {
        let header = make_test_header(1, [0u8; 32]);
        let popow = PoPowHeader::new(header, vec![], vec![]);

        assert!(popow.is_genesis());
        assert!(!popow.has_interlinks());
    }

    #[test]
    fn test_popow_header_connects_via_parent() {
        let parent_id = [1u8; 32];
        let header = make_test_header(100, parent_id);
        let popow = PoPowHeader::new(header, vec![], vec![]);

        assert!(popow.connects_to(&parent_id));
        assert!(!popow.connects_to(&[2u8; 32]));
    }

    #[test]
    fn test_popow_header_connects_via_interlinks() {
        let header = make_test_header(100, [0u8; 32]);
        let interlinks = vec![[1u8; 32], [2u8; 32], [3u8; 32]];
        let popow = PoPowHeader::new(header, interlinks, vec![]);

        assert!(popow.connects_to(&[2u8; 32]));
        assert!(!popow.connects_to(&[4u8; 32]));
    }

    #[test]
    fn test_popow_header_equality() {
        let header1 = make_test_header(100, [1u8; 32]);
        let header2 = make_test_header(100, [1u8; 32]);

        let popow1 = PoPowHeader::new(header1.clone(), vec![], vec![]);
        let popow2 = PoPowHeader::new(header2, vec![[1u8; 32]], vec![1, 2, 3]);

        // Same header ID means equal, regardless of other fields
        assert_eq!(popow1, popow2);
    }
}
