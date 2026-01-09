//! Interlinks packing and unpacking for extension blocks.
//!
//! Interlinks are stored in block extensions as key-value pairs with prefix 0x01.

use crate::{ConsensusError, ConsensusResult};

/// Extension field prefix for interlinks vector.
pub const INTERLINKS_VECTOR_PREFIX: u8 = 0x01;

/// Pack interlinks into extension key-value pairs.
///
/// Format:
/// - Key: [0x01, index_byte]
/// - Value: [duplicates_count_byte, modifier_id_32_bytes]
///
/// Run-length encodes consecutive duplicate block IDs.
pub fn pack_interlinks(links: &[[u8; 32]]) -> Vec<(Vec<u8>, Vec<u8>)> {
    if links.is_empty() {
        return Vec::new();
    }

    let mut result = Vec::new();
    let mut idx: u8 = 0;

    let mut i = 0;
    while i < links.len() {
        let link = &links[i];

        // Count consecutive duplicates
        let count = links[i..]
            .iter()
            .take_while(|id| *id == link)
            .count()
            .min(255) as u8; // Max 255 duplicates per entry

        // Key: prefix + index
        let key = vec![INTERLINKS_VECTOR_PREFIX, idx];

        // Value: count + 32-byte block ID
        let mut value = vec![count];
        value.extend_from_slice(link);

        result.push((key, value));

        i += count as usize;
        idx = idx.wrapping_add(1);
    }

    result
}

/// Unpack interlinks from extension key-value pairs.
///
/// Reconstructs the full interlinks vector from run-length encoded format.
pub fn unpack_interlinks(fields: &[(Vec<u8>, Vec<u8>)]) -> ConsensusResult<Vec<[u8; 32]>> {
    // Filter and sort by index
    let mut interlink_fields: Vec<_> = fields
        .iter()
        .filter(|(key, value)| {
            key.len() == 2 && key[0] == INTERLINKS_VECTOR_PREFIX && value.len() == 33
        })
        .collect();

    // Sort by index (second byte of key)
    interlink_fields.sort_by_key(|(key, _)| key[1]);

    let mut result = Vec::new();

    for (key, value) in interlink_fields {
        let count = value[0] as usize;
        if count == 0 {
            return Err(ConsensusError::Validation(
                "Invalid interlinks: zero count".to_string(),
            ));
        }

        let mut block_id = [0u8; 32];
        block_id.copy_from_slice(&value[1..33]);

        // Add block ID 'count' times
        for _ in 0..count {
            result.push(block_id);
        }
    }

    Ok(result)
}

/// Calculate the expected interlinks for the next block.
///
/// # Arguments
/// * `prev_header_id` - ID of the previous block
/// * `prev_level` - Level of the previous block (from max_level_of)
/// * `prev_interlinks` - Interlinks of the previous block
/// * `is_genesis` - Whether the previous block is genesis
///
/// # Returns
/// New interlinks vector for the next block
pub fn update_interlinks(
    prev_header_id: [u8; 32],
    prev_level: u32,
    prev_interlinks: &[[u8; 32]],
    is_genesis: bool,
) -> Vec<[u8; 32]> {
    if is_genesis {
        // First block after genesis: interlinks = [genesis_id]
        return vec![prev_header_id];
    }

    if prev_interlinks.is_empty() {
        // Should not happen in valid chain, but handle gracefully
        return vec![prev_header_id];
    }

    // Genesis is always first element
    let genesis = prev_interlinks[0];

    if prev_level == 0 {
        // Level 0 blocks don't modify interlinks structure
        prev_interlinks.to_vec()
    } else {
        // Higher level blocks replace entries at their level and below
        let tail = &prev_interlinks[1..];
        let level = prev_level as usize;

        let mut result = vec![genesis];

        // Keep entries from tail that are at higher levels
        if tail.len() > level {
            result.extend_from_slice(&tail[..tail.len() - level]);
        }

        // Add previous header ID 'level' times
        for _ in 0..level {
            result.push(prev_header_id);
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pack_unpack_empty() {
        let links: Vec<[u8; 32]> = Vec::new();
        let packed = pack_interlinks(&links);
        assert!(packed.is_empty());

        let unpacked = unpack_interlinks(&packed).unwrap();
        assert!(unpacked.is_empty());
    }

    #[test]
    fn test_pack_unpack_single() {
        let id = [1u8; 32];
        let links = vec![id];

        let packed = pack_interlinks(&links);
        assert_eq!(packed.len(), 1);
        assert_eq!(packed[0].0, vec![0x01, 0x00]); // prefix + index
        assert_eq!(packed[0].1[0], 1); // count
        assert_eq!(&packed[0].1[1..], &id); // block ID

        let unpacked = unpack_interlinks(&packed).unwrap();
        assert_eq!(unpacked, links);
    }

    #[test]
    fn test_pack_unpack_duplicates() {
        let id1 = [1u8; 32];
        let id2 = [2u8; 32];
        // Genesis + 3x id1 + 2x id2
        let links = vec![id1, id1, id1, id1, id2, id2];

        let packed = pack_interlinks(&links);
        assert_eq!(packed.len(), 2);

        // First entry: 4 copies of id1
        assert_eq!(packed[0].1[0], 4);
        assert_eq!(&packed[0].1[1..], &id1);

        // Second entry: 2 copies of id2
        assert_eq!(packed[1].1[0], 2);
        assert_eq!(&packed[1].1[1..], &id2);

        let unpacked = unpack_interlinks(&packed).unwrap();
        assert_eq!(unpacked, links);
    }

    #[test]
    fn test_pack_unpack_alternating() {
        let id1 = [1u8; 32];
        let id2 = [2u8; 32];
        let links = vec![id1, id2, id1, id2];

        let packed = pack_interlinks(&links);
        assert_eq!(packed.len(), 4); // No run-length compression

        let unpacked = unpack_interlinks(&packed).unwrap();
        assert_eq!(unpacked, links);
    }

    #[test]
    fn test_update_interlinks_genesis() {
        let genesis_id = [0u8; 32];

        let result = update_interlinks(genesis_id, 0, &[], true);

        assert_eq!(result, vec![genesis_id]);
    }

    #[test]
    fn test_update_interlinks_level_0() {
        let genesis_id = [0u8; 32];
        let prev_id = [1u8; 32];
        let interlinks = vec![genesis_id];

        // Level 0 block doesn't change interlinks
        let result = update_interlinks(prev_id, 0, &interlinks, false);
        assert_eq!(result, interlinks);
    }

    #[test]
    fn test_update_interlinks_level_1() {
        let genesis_id = [0u8; 32];
        let prev_id = [1u8; 32];
        let interlinks = vec![genesis_id];

        // Level 1 block adds itself once
        let result = update_interlinks(prev_id, 1, &interlinks, false);
        assert_eq!(result, vec![genesis_id, prev_id]);
    }

    #[test]
    fn test_update_interlinks_level_2() {
        let genesis_id = [0u8; 32];
        let block1 = [1u8; 32];
        let block2 = [2u8; 32];
        let interlinks = vec![genesis_id, block1, block1];

        // Level 2 block replaces last 2 entries with itself
        let result = update_interlinks(block2, 2, &interlinks, false);
        assert_eq!(result, vec![genesis_id, block2, block2]);
    }

    #[test]
    fn test_unpack_ignores_other_fields() {
        let id = [1u8; 32];
        let mut fields = pack_interlinks(&vec![id]);

        // Add unrelated extension fields
        fields.push((vec![0x02, 0x00], vec![0x00; 33])); // Different prefix
        fields.push((vec![0x01], vec![0x01; 33])); // Wrong key length

        let unpacked = unpack_interlinks(&fields).unwrap();
        assert_eq!(unpacked, vec![id]);
    }
}
