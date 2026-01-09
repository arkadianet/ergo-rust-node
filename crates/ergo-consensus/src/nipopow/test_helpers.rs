//! Shared test helpers for NiPoPoW tests.

use super::PoPowHeader;
use ergo_chain_types::{ADDigest, AutolykosSolution, BlockId, Digest32, Header, Votes};

/// Create a test header with given height and parent.
pub fn make_test_header(height: u32, parent_bytes: [u8; 32]) -> Header {
    let zero_digest = Digest32::zero();
    let autolykos_solution = AutolykosSolution {
        miner_pk: Box::new(ergo_chain_types::ec_point::identity()),
        pow_onetime_pk: None,
        nonce: vec![0u8; 8],
        pow_distance: None,
    };

    Header {
        version: 1,
        id: BlockId(Digest32::from([height as u8; 32])),
        parent_id: BlockId(Digest32::from(parent_bytes)),
        ad_proofs_root: zero_digest,
        state_root: ADDigest::zero(),
        transaction_root: zero_digest,
        timestamp: 0,
        n_bits: 0x1d00ffff,
        height,
        extension_root: zero_digest,
        autolykos_solution,
        votes: Votes([0u8; 3]),
        unparsed_bytes: Box::new([]),
    }
}

/// Create a test PoPowHeader with given height and parent.
pub fn make_popow_header(height: u32, parent_bytes: [u8; 32]) -> PoPowHeader {
    let header = make_test_header(height, parent_bytes);
    // Interlinks: genesis + previous
    let interlinks = if height <= 1 {
        vec![]
    } else {
        vec![[1u8; 32], parent_bytes]
    };
    PoPowHeader::new(header, interlinks, vec![])
}
