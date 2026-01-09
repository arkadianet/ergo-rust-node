//! Core NiPoPoW algorithms.
//!
//! Implements the key algorithms from the KMZ17 paper.

use num_bigint::BigUint;

/// NiPoPoW algorithms for proof generation and validation.
#[derive(Debug, Clone)]
pub struct NipopowAlgos {
    /// Security parameter (minimum superchain length).
    pub m: i32,
    /// Suffix length.
    pub k: i32,
}

impl Default for NipopowAlgos {
    fn default() -> Self {
        Self {
            m: super::DEFAULT_M,
            k: super::DEFAULT_K,
        }
    }
}

impl NipopowAlgos {
    /// Create new NiPoPoW algorithms with custom parameters.
    pub fn new(m: i32, k: i32) -> Self {
        Self { m, k }
    }

    /// Compute the best argument (proof score) for a chain.
    ///
    /// This is the key comparison metric from KMZ17.
    /// For each level μ where at least m superblocks exist,
    /// computes 2^μ × count and returns the maximum.
    pub fn best_arg(&self, levels: &[u32]) -> i64 {
        best_arg(levels, self.m)
    }
}

/// Compute the level (μ) of a block header.
///
/// Level is based on how much the PoW exceeded the target:
/// μ = floor(log2(target) - log2(pow_hit))
///
/// # Arguments
/// * `nbits` - Compressed difficulty target from header
/// * `pow_hit` - Actual PoW solution value (32 bytes, big-endian)
/// * `is_genesis` - Whether this is the genesis block
///
/// # Returns
/// The level μ ≥ 0. Genesis returns u32::MAX.
pub fn max_level_of(nbits: u32, pow_hit: &[u8; 32], is_genesis: bool) -> u32 {
    if is_genesis {
        return u32::MAX;
    }

    // Decode nBits to get target difficulty
    let target = decode_nbits(nbits);

    // Convert PoW hit to BigUint
    let hit = BigUint::from_bytes_be(pow_hit);

    // Check for zero or one to avoid division issues
    let one = BigUint::from(1u32);
    if hit == one || hit == BigUint::ZERO {
        return 0;
    }

    // Level = floor(log2(target / hit))
    // This is equivalent to floor(log2(target) - log2(hit))
    if target <= hit {
        return 0;
    }

    let ratio = &target / &hit;

    // Calculate floor(log2(ratio))
    // This is the number of bits minus 1
    let bits = ratio.bits();
    if bits == 0 {
        0
    } else {
        (bits - 1) as u32
    }
}

/// Compute the best argument (proof score) for a chain.
///
/// # Arguments
/// * `levels` - Levels of all headers in the chain segment
/// * `m` - Minimum superchain length
///
/// # Returns
/// Best score = max(2^μ × count) over all valid levels
pub fn best_arg(levels: &[u32], m: i32) -> i64 {
    if levels.is_empty() {
        return 0;
    }

    // Level 0: all blocks count
    let mut best_score = levels.len() as i64;

    // Check higher levels
    let mut level: u32 = 1;
    loop {
        let superblock_count = levels.iter().filter(|&&l| l >= level).count();

        if superblock_count >= m as usize {
            // Score = 2^level × count
            let score = (1i64 << level) * (superblock_count as i64);
            best_score = best_score.max(score);
            level += 1;
        } else {
            break;
        }

        // Safety limit to prevent infinite loop
        if level > 256 {
            break;
        }
    }

    best_score
}

/// Decode nBits compact difficulty format to full target.
///
/// nBits format: EEMMMMM where E is exponent and M is mantissa.
/// Target = M × 2^(8×(E-3))
fn decode_nbits(nbits: u32) -> BigUint {
    let exponent = (nbits >> 24) as usize;
    let mantissa = nbits & 0x007FFFFF;

    if exponent <= 3 {
        // Small exponent: shift mantissa right
        let shift = 8 * (3 - exponent);
        BigUint::from(mantissa >> shift)
    } else {
        // Large exponent: shift mantissa left
        let shift = 8 * (exponent - 3);
        BigUint::from(mantissa) << shift
    }
}

/// Count superblocks at each level in a chain segment.
///
/// # Arguments
/// * `headers_with_levels` - Iterator of (header_id, level) pairs
///
/// # Returns
/// Vec where index is level and value is count of superblocks at that level
pub fn count_superblocks_by_level(levels: &[u32]) -> Vec<usize> {
    if levels.is_empty() {
        return Vec::new();
    }

    let max_level = *levels.iter().max().unwrap_or(&0);
    let mut counts = vec![0usize; (max_level + 1) as usize];

    for &level in levels {
        // A block at level L is also a superblock at all levels < L
        for l in 0..=level {
            counts[l as usize] += 1;
        }
    }

    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_max_level_genesis() {
        let pow_hit = [0u8; 32];
        assert_eq!(max_level_of(0, &pow_hit, true), u32::MAX);
    }

    #[test]
    fn test_max_level_zero() {
        // PoW hit equals target: level 0
        let nbits = 0x1d00ffff_u32; // Bitcoin-style difficulty 1
        let pow_hit = {
            let target = decode_nbits(nbits);
            let bytes = target.to_bytes_be();
            let mut result = [0u8; 32];
            let start = 32 - bytes.len().min(32);
            result[start..].copy_from_slice(&bytes[..bytes.len().min(32)]);
            result
        };

        let level = max_level_of(nbits, &pow_hit, false);
        assert_eq!(level, 0);
    }

    #[test]
    fn test_max_level_higher() {
        // PoW hit is 1/4 of target: level 2
        let nbits = 0x1d00ffff_u32;
        let target = decode_nbits(nbits);
        let hit = &target / 4u32;
        let bytes = hit.to_bytes_be();
        let mut pow_hit = [0u8; 32];
        let start = 32 - bytes.len().min(32);
        pow_hit[start..].copy_from_slice(&bytes[..bytes.len().min(32)]);

        let level = max_level_of(nbits, &pow_hit, false);
        assert_eq!(level, 2);
    }

    #[test]
    fn test_decode_nbits() {
        // Standard Bitcoin difficulty 1
        let target = decode_nbits(0x1d00ffff);
        // Should be 0x00000000FFFF0000...0000 (26 bytes of zeros at end)
        assert!(target > BigUint::ZERO);
    }

    #[test]
    fn test_best_arg_empty() {
        assert_eq!(best_arg(&[], 30), 0);
    }

    #[test]
    fn test_best_arg_all_level_zero() {
        // 100 blocks all at level 0
        let levels = vec![0u32; 100];
        // Score = 100 (all blocks, 2^0 = 1)
        assert_eq!(best_arg(&levels, 30), 100);
    }

    #[test]
    fn test_best_arg_mixed_levels() {
        // 100 blocks: 50 at level 0, 30 at level 1, 20 at level 2
        let mut levels = vec![0u32; 50];
        levels.extend(vec![1u32; 30]);
        levels.extend(vec![2u32; 20]);

        let m = 10;
        // Level 0: 100 blocks × 2^0 = 100
        // Level 1: 50 blocks × 2^1 = 100
        // Level 2: 20 blocks × 2^2 = 80
        // Best = 100
        assert_eq!(best_arg(&levels, m), 100);
    }

    #[test]
    fn test_best_arg_high_level_wins() {
        // 100 blocks all at level 5
        let levels = vec![5u32; 100];

        let m = 10;
        // Level 5: 100 blocks × 2^5 = 3200
        assert_eq!(best_arg(&levels, m), 3200);
    }

    #[test]
    fn test_count_superblocks_by_level() {
        let levels = vec![0, 1, 2, 1, 0, 3];
        let counts = count_superblocks_by_level(&levels);

        // Level 0: all 6 blocks
        // Level 1: blocks with level >= 1 = 4 (indices 1,2,3,5)
        // Level 2: blocks with level >= 2 = 2 (indices 2,5)
        // Level 3: blocks with level >= 3 = 1 (index 5)
        assert_eq!(counts, vec![6, 4, 2, 1]);
    }

    #[test]
    fn test_nipopow_algos() {
        let algos = NipopowAlgos::new(10, 10);
        let levels = vec![0u32; 50];
        assert_eq!(algos.best_arg(&levels), 50);
    }
}
