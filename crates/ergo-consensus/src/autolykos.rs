//! Autolykos v2 Proof-of-Work implementation.
//!
//! Autolykos v2 is a memory-hard PoW algorithm designed for GPU mining.
//! The algorithm:
//!
//! 1. msg = H(header_without_pow) — nonce is NOT included in msg
//! 2. seed = complex multi-step hash using msg, nonce, height, and M constant
//! 3. Generates k indices using a sliding window over the extended seed
//! 4. For each index, computes H(idx || height || M)[1..] and sums the results
//! 5. hit = H(sum as 32-byte big-endian); valid if hit < target
//!
//! For verification, we compute elements on-demand rather than generating the full table.
//!
//! Key parameters:
//! - N = 2^n (default n=26) with 5% growth schedule after N_INCREASE_START (614,400)
//! - k = 32 (number of elements to sum, must be ≤32)
//! - M = 8KB constant (0..1024 as u64 big-endian)
//!
//! Reference: https://docs.ergoplatform.com/ErgoPow.pdf
//! Reference implementation: ergo-nipopow crate from sigma-rust

use crate::params::{
    AUTOLYKOS_K, AUTOLYKOS_V2_VERSION, N_INCREASE_PERIOD, N_INCREASE_START, N_MAX_HEIGHT,
};
use crate::{ConsensusError, ConsensusResult};
use blake2::{Blake2b, Digest};
pub use ergo_chain_types::AutolykosSolution;
use ergo_chain_types::{ec_point, Header};
use ergo_lib::ergotree_ir::serialization::SigmaSerializable;
use k256::{elliptic_curve::ops::Reduce, Scalar, U256};
use num_bigint::BigUint;
use std::sync::LazyLock;
use tracing::{debug, trace, warn};

/// Size of Blake2b256 output.
const HASH_SIZE: usize = 32;

/// M constant: 8KB of data (0..1024 as u64 big-endian).
/// Used in seed and element calculations to increase hash computation time.
/// This is computed once and cached statically.
pub static BIG_M: LazyLock<[u8; 8192]> = LazyLock::new(|| {
    let mut m = [0u8; 8192];
    for i in 0u64..1024 {
        let bytes = i.to_be_bytes();
        m[i as usize * 8..(i as usize + 1) * 8].copy_from_slice(&bytes);
    }
    m
});

/// Group order of secp256k1 curve (cached).
/// This is used as the numerator when converting n_bits difficulty to target.
/// In Ergo: target = GROUP_ORDER / decode_compact_bits(n_bits)
static GROUP_ORDER: LazyLock<BigUint> = LazyLock::new(|| {
    // secp256k1 group order: 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
    BigUint::parse_bytes(
        b"FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141",
        16,
    )
    .expect("hardcoded group order must parse")
});

/// Valid range for hashModQ rejection sampling.
/// This is (2^256 / q) * q - the largest number <= 2^256 divisible by q.
/// Used in Autolykos v1 to ensure uniform distribution mod q.
static VALID_RANGE: LazyLock<BigUint> = LazyLock::new(|| {
    let two_256 = BigUint::from(1u8) << 256;
    (&two_256 / &*GROUP_ORDER) * &*GROUP_ORDER
});

/// Compute Blake2b256 hash and reduce mod q (group order).
/// Uses rejection sampling to ensure uniform distribution.
/// Used in Autolykos v1 for element generation.
fn hash_mod_q(input: &[u8]) -> BigUint {
    let mut current_input = input.to_vec();
    loop {
        let mut hasher = Blake2b::<typenum::U32>::new();
        Digest::update(&mut hasher, &current_input);
        let hash: [u8; HASH_SIZE] = hasher.finalize().into();

        // Interpret hash as unsigned big integer
        let bi = BigUint::from_bytes_be(&hash);

        if bi < *VALID_RANGE {
            // In valid range, return result mod q
            return bi % &*GROUP_ORDER;
        }

        // Outside valid range, hash again using the hash as input
        // This is rejection sampling to ensure uniform distribution
        trace!("hashModQ: result outside valid range, rehashing");
        current_input = hash.to_vec();
    }
}

/// Convert BigUint to k256 Scalar, reducing mod group order.
fn biguint_to_scalar(value: &BigUint) -> Scalar {
    let mut arr = [0u8; 32];
    let bytes = value.to_bytes_be();

    if bytes.len() > 32 {
        // Value larger than 32 bytes - reduce first
        let reduced = value % &*GROUP_ORDER;
        let b = reduced.to_bytes_be();
        arr[32 - b.len()..].copy_from_slice(&b);
    } else {
        arr[32 - bytes.len()..].copy_from_slice(&bytes);
    }

    // Reduce mod group order
    Scalar::reduce(U256::from_be_slice(&arr))
}

/// Autolykos v2 PoW verifier.
#[derive(Debug, Clone)]
pub struct AutolykosV2 {
    /// N base exponent (N = 2^n, default n=26 for mainnet).
    /// Must be < 32 to avoid overflow.
    n: u32,
    /// Number of elements to sum (k), must be ≤32.
    k: u32,
}

/// Default N exponent for mainnet (N = 2^26).
const DEFAULT_N_EXPONENT: u32 = 26;

impl Default for AutolykosV2 {
    fn default() -> Self {
        Self::new()
    }
}

impl AutolykosV2 {
    /// Create a new Autolykos v2 verifier with default parameters.
    pub fn new() -> Self {
        Self::with_params(DEFAULT_N_EXPONENT, AUTOLYKOS_K)
    }

    /// Create a verifier with custom parameters (for testing).
    /// `n` is the exponent such that base N = 2^n (must be < 32).
    /// `k` is the number of elements to sum (must be ≤32).
    ///
    /// # Panics
    /// Panics if `n >= 32` (would overflow u32).
    pub fn with_params(n: u32, k: u32) -> Self {
        assert!(n < 32, "n exponent must be < 32 to avoid overflow");
        assert!(k <= 32, "k must be <= 32");
        Self { n, k }
    }

    /// Calculate N (table size) for given version and height.
    /// For v1: returns base N (2^n).
    /// For v2: N grows 5% every N_INCREASE_PERIOD blocks after N_INCREASE_START,
    /// capped at N_MAX_HEIGHT.
    ///
    /// Note: Growth uses integer division `acc / 100 * 105` (NOT `acc * 105 / 100`)
    /// to match Scala reference rounding behavior.
    pub fn calc_big_n(&self, version: u8, height: u32) -> u32 {
        let n_base: u32 = 1u32 << self.n;
        if version < AUTOLYKOS_V2_VERSION {
            n_base
        } else {
            let height = height.min(N_MAX_HEIGHT);
            if height < N_INCREASE_START {
                n_base
            } else {
                let iters = (height - N_INCREASE_START) / N_INCREASE_PERIOD + 1;
                (0..iters).fold(n_base, |acc, _| acc / 100 * 105)
            }
        }
    }

    /// Verify the PoW solution in a block header.
    ///
    /// # Arguments
    /// * `header` - The block header to verify
    ///
    /// # Returns
    /// * `Ok(true)` if the solution is valid and meets the difficulty target
    /// * `Ok(false)` if the solution doesn't meet the target
    /// * `Err` if the solution is malformed
    pub fn verify_header(&self, header: &Header) -> ConsensusResult<bool> {
        // Get header bytes without PoW solution for hashing
        let header_bytes = header.serialize_without_pow().map_err(|e| {
            ConsensusError::InvalidPow(format!("Failed to serialize header: {}", e))
        })?;

        // Get the target from nBits (target = GROUP_ORDER / difficulty)
        let target = nbits_to_target(header.n_bits)?;

        // Verify the solution (pass version for v1/v2 algorithm selection)
        self.verify_solution(
            &header_bytes,
            &header.autolykos_solution,
            &target,
            header.version,
            header.height,
        )
    }

    /// Verify a PoW solution against header bytes and difficulty target.
    ///
    /// # Arguments
    /// * `header_bytes` - Serialized block header (without PoW solution)
    /// * `solution` - The PoW solution to verify
    /// * `target` - Difficulty target (solution must produce hit < target)
    /// * `version` - Block version (determines v1 vs v2 algorithm)
    /// * `height` - Block height (used to determine N parameter)
    ///
    /// # Returns
    /// * `Ok(true)` if the solution is valid
    /// * `Ok(false)` if the solution doesn't meet the target
    /// * `Err` if the solution is malformed
    pub fn verify_solution(
        &self,
        header_bytes: &[u8],
        solution: &AutolykosSolution,
        target: &BigUint,
        version: u8,
        height: u32,
    ) -> ConsensusResult<bool> {
        // Validate nonce length (must be exactly 8 bytes)
        if solution.nonce.len() != 8 {
            return Err(ConsensusError::InvalidPow(format!(
                "Invalid nonce length: {}, expected 8",
                solution.nonce.len()
            )));
        }

        // Calculate N for this version and height
        let big_n = self.calc_big_n(version, height);

        // Branch on version: v1 and v2 use different algorithms
        if version < AUTOLYKOS_V2_VERSION {
            // ===== Autolykos v1 =====
            // V1 uses elliptic curve verification: w^f == g^d * pk
            self.verify_solution_v1(header_bytes, solution, target, big_n)
        } else {
            // ===== Autolykos v2 =====
            // V2 uses hash comparison: hit < target
            self.verify_solution_v2(header_bytes, solution, target, big_n, height)
        }
    }

    /// Verify Autolykos v1 solution using EC equation: w^f == g^d * pk
    ///
    /// Algorithm:
    /// 1. Check d < b (target)
    /// 2. Check pk and w are not infinity
    /// 3. msg = H(header_without_pow)
    /// 4. seed = msg || nonce
    /// 5. indices = genIndexes(seed, N)
    /// 6. For each idx: element = hashModQ(idx || M || pk || msg || w)
    /// 7. f = sum of elements mod q
    /// 8. Verify: w^f == g^d * pk
    fn verify_solution_v1(
        &self,
        header_bytes: &[u8],
        solution: &AutolykosSolution,
        target: &BigUint,
        big_n: u32,
    ) -> ConsensusResult<bool> {
        // Extract v1-specific fields (must be present for v1 blocks)
        let w = solution.pow_onetime_pk.as_ref().ok_or_else(|| {
            ConsensusError::InvalidPow("Autolykos v1 requires pow_onetime_pk (w)".to_string())
        })?;
        // pow_distance is Option<BigUint> in sigma-rust develop branch (already non-negative)
        let d_abs: &BigUint = solution.pow_distance.as_ref().ok_or_else(|| {
            ConsensusError::InvalidPow("Autolykos v1 requires pow_distance (d)".to_string())
        })?;

        // Check d < target (required for valid PoW)
        if d_abs >= target {
            debug!("PoW v1 failed: d ({}) >= target ({})", d_abs, target);
            return Ok(false);
        }

        // Check pk is not infinity
        if ec_point::is_identity(&solution.miner_pk) {
            warn!("PoW v1: miner_pk is infinity point");
            return Ok(false);
        }

        // Check w is not infinity
        if ec_point::is_identity(w) {
            warn!("PoW v1: pow_onetime_pk (w) is infinity point");
            return Ok(false);
        }

        // msg = H(header_without_pow) - nonce NOT included in hash
        let msg = Self::calculate_msg(header_bytes);
        trace!("PoW v1 msg hash: {}", hex::encode(&msg));

        // seed = msg || nonce (concatenation, not hashing)
        let mut seed = Vec::with_capacity(msg.len() + solution.nonce.len());
        seed.extend_from_slice(&msg);
        seed.extend_from_slice(&solution.nonce);

        // Generate indices using sliding window over H(seed)
        let indices = self.gen_indexes_v1(&seed, big_n);
        trace!("Generated {} v1 indices", indices.len());

        // Get serialized pk and w bytes
        let pk_bytes = solution.miner_pk.sigma_serialize_bytes().map_err(|e| {
            ConsensusError::InvalidPow(format!("Failed to serialize miner pk: {}", e))
        })?;
        let w_bytes = w
            .sigma_serialize_bytes()
            .map_err(|e| ConsensusError::InvalidPow(format!("Failed to serialize w: {}", e)))?;

        // Calculate f = sum of elements mod q
        // Each element = hashModQ(idx || M || pk || msg || w)
        let f = self.calculate_f_v1(&indices, &msg, &pk_bytes, &w_bytes);
        trace!("PoW v1 f = {}", f);

        // Convert f to scalar for EC operations
        let f_scalar = biguint_to_scalar(&f);

        // Convert d to scalar
        let d_scalar = biguint_to_scalar(&d_abs);

        // Compute left = w^f
        let left = ec_point::exponentiate(w, &f_scalar);

        // Compute right = g^d * pk
        let g = ec_point::generator();
        let g_d = ec_point::exponentiate(&g, &d_scalar);
        // Note: EcPoint::Mul is actually point addition (multiplicative group notation)
        let right = g_d * &*solution.miner_pk;

        // Verify: left == right
        let valid = left == right;

        debug!(
            "PoW v1 verification: w^f {} g^d * pk",
            if valid { "==" } else { "!=" }
        );

        Ok(valid)
    }

    /// Verify Autolykos v2 solution using hash comparison: hit < target
    fn verify_solution_v2(
        &self,
        header_bytes: &[u8],
        solution: &AutolykosSolution,
        target: &BigUint,
        big_n: u32,
        height: u32,
    ) -> ConsensusResult<bool> {
        // msg = H(header_without_pow) — nonce NOT included
        let msg = Self::calculate_msg(header_bytes);
        trace!("PoW v2 msg hash: {}", hex::encode(&msg));

        // seed = calc_seed_v2(big_n, msg, nonce, height)
        let seed = Self::calc_seed_v2(big_n, &msg, &solution.nonce, height);
        trace!("PoW v2 seed: {}", hex::encode(&seed));

        // Generate k indices using sliding window
        let indices = self.gen_indexes(&seed, big_n);
        trace!("Generated {} v2 indices", indices.len());

        // sum = sum of H(idx || height || M)[1..] for each index
        let sum = Self::calc_elements_sum(&indices, height);

        // hit = H(sum normalized to 32 bytes)
        let hit = Self::calc_hit(&sum);
        let hit_value = BigUint::from_bytes_be(&hit);

        debug!(
            "PoW v2 verification: hit={}, target={}",
            hex::encode(&hit),
            target
        );

        // Check hit < target (strict inequality per spec)
        Ok(hit_value < *target)
    }

    /// Calculate message hash: H(header_without_pow).
    /// Used by both v1 and v2 - nonce is NOT included in msg.
    /// In v1, seed = msg || nonce (concatenation).
    /// In v2, nonce is used separately in seed calculation.
    fn calculate_msg(header_bytes: &[u8]) -> [u8; HASH_SIZE] {
        let mut hasher = Blake2b::<typenum::U32>::new();
        Digest::update(&mut hasher, header_bytes);
        hasher.finalize().into()
    }

    /// Calculate seed for Autolykos v2 (algorithm 1, line 4 in ErgoPow paper).
    ///
    /// Steps:
    /// 1. hash1 = H(msg || nonce)
    /// 2. pre_i = last 8 bytes of hash1 as u64 big-endian
    /// 3. i = (pre_i % big_n) as 4-byte big-endian
    /// 4. f = H(i || height_bytes || M)
    /// 5. seed = H(f[1..] || msg || nonce)
    fn calc_seed_v2(
        big_n: u32,
        msg: &[u8; HASH_SIZE],
        nonce: &[u8],
        height: u32,
    ) -> [u8; HASH_SIZE] {
        // Step 1: hash1 = H(msg || nonce)
        let mut hasher = Blake2b::<typenum::U32>::new();
        Digest::update(&mut hasher, msg);
        Digest::update(&mut hasher, nonce);
        let hash1: [u8; HASH_SIZE] = hasher.finalize().into();

        // Step 2: pre_i = last 8 bytes as u64
        let pre_i = u64::from_be_bytes(hash1[24..32].try_into().unwrap());

        // Step 3: i = pre_i % big_n as 4-byte big-endian
        let i = (pre_i % big_n as u64) as u32;
        let i_bytes = i.to_be_bytes();

        // Step 4: f = H(i || height_bytes || M)
        let height_bytes = height.to_be_bytes();
        let mut hasher = Blake2b::<typenum::U32>::new();
        Digest::update(&mut hasher, &i_bytes);
        Digest::update(&mut hasher, &height_bytes);
        Digest::update(&mut hasher, &*BIG_M);
        let f: [u8; HASH_SIZE] = hasher.finalize().into();

        // Step 5: seed = H(f[1..] || msg || nonce)
        let mut hasher = Blake2b::<typenum::U32>::new();
        Digest::update(&mut hasher, &f[1..]); // skip first byte
        Digest::update(&mut hasher, msg);
        Digest::update(&mut hasher, nonce);
        hasher.finalize().into()
    }

    /// Generate k indices from the seed using sliding window (v2 algorithm).
    ///
    /// Algorithm:
    /// 1. Extend seed (32 bytes) with its first 3 bytes to get 35 bytes
    /// 2. For each i in 0..k, take bytes [i..i+4] as u32 big-endian
    /// 3. Index = value % big_n
    ///
    /// Note: k must be <= 32 (validated in constructor).
    fn gen_indexes(&self, seed: &[u8; HASH_SIZE], big_n: u32) -> Vec<u32> {
        debug_assert!(self.k <= 32, "k must be <= 32 for sliding window");

        // Extend seed: 32 bytes + first 3 bytes = 35 bytes
        // This allows sliding window of 4 bytes for up to k=32 positions
        let mut extended = [0u8; 35];
        extended[..32].copy_from_slice(seed);
        extended[32..35].copy_from_slice(&seed[..3]);

        let mut indices = Vec::with_capacity(self.k as usize);
        for i in 0..self.k as usize {
            let val = u32::from_be_bytes(extended[i..i + 4].try_into().unwrap());
            indices.push(val % big_n);
        }
        indices
    }

    // =====================================================================
    // Autolykos v1 helpers (used for blocks before height 417,792)
    // =====================================================================

    /// Generate k indices for v1 using sliding window over H(seed).
    ///
    /// Algorithm (same as Scala genIndexes):
    /// 1. hash = H(seed)
    /// 2. extendedHash = hash + hash[0..3] (35 bytes)
    /// 3. For each i in 0..k: index = BigInt(extendedHash[i..i+4]) mod N
    fn gen_indexes_v1(&self, seed: &[u8], big_n: u32) -> Vec<u32> {
        debug_assert!(self.k <= 32, "k must be <= 32 for sliding window");

        // hash = H(seed)
        let mut hasher = Blake2b::<typenum::U32>::new();
        Digest::update(&mut hasher, seed);
        let hash: [u8; HASH_SIZE] = hasher.finalize().into();

        // extendedHash = hash + hash[0..3] (35 bytes)
        let mut extended = [0u8; 35];
        extended[..32].copy_from_slice(&hash);
        extended[32..35].copy_from_slice(&hash[..3]);

        // Generate k indices using sliding window
        let mut indices = Vec::with_capacity(self.k as usize);
        for i in 0..self.k as usize {
            // BigInt(1, extendedHash.slice(i, i + 4)) - unsigned big-endian
            let val = u32::from_be_bytes(extended[i..i + 4].try_into().unwrap());
            indices.push(val % big_n);
        }
        indices
    }

    /// Calculate f for v1: sum of hashModQ(idx || M || pk || msg || w) mod q.
    ///
    /// Element generation: H(j|M|pk|m|w) mod q
    /// where M is the 8KB constant, pk is miner public key, m is msg hash, w is one-time pk.
    fn calculate_f_v1(
        &self,
        indices: &[u32],
        msg: &[u8],
        pk_bytes: &[u8],
        w_bytes: &[u8],
    ) -> BigUint {
        let mut sum = BigUint::from(0u32);

        for &idx in indices {
            // Build input: idx || M || pk || msg || w
            let idx_bytes = idx.to_be_bytes();
            let mut input =
                Vec::with_capacity(4 + BIG_M.len() + pk_bytes.len() + msg.len() + w_bytes.len());
            input.extend_from_slice(&idx_bytes);
            input.extend_from_slice(&*BIG_M);
            input.extend_from_slice(pk_bytes);
            input.extend_from_slice(msg);
            input.extend_from_slice(w_bytes);

            // element = hashModQ(input)
            let element = hash_mod_q(&input);
            sum += element;
        }

        // Return sum mod q
        sum % &*GROUP_ORDER
    }

    /// Calculate sum of elements for v2: sum of H(idx || height || M)[1..] for each index.
    ///
    /// Each element is computed as:
    /// - hash = H(idx as u32 BE || height as u32 BE || M)
    /// - element = hash[1..] (skip first byte, 31 bytes)
    /// - interpret as unsigned big-endian integer and accumulate sum
    ///
    /// Returns the sum as BigUint (may exceed 32 bytes).
    fn calc_elements_sum(indices: &[u32], height: u32) -> BigUint {
        let height_bytes = height.to_be_bytes();
        let mut sum = BigUint::from(0u32);

        for &idx in indices {
            let mut hasher = Blake2b::<typenum::U32>::new();
            Digest::update(&mut hasher, &idx.to_be_bytes());
            Digest::update(&mut hasher, &height_bytes);
            Digest::update(&mut hasher, &*BIG_M);
            let hash: [u8; HASH_SIZE] = hasher.finalize().into();

            // Skip first byte (31 bytes), interpret as big-endian unsigned
            let element_val = BigUint::from_bytes_be(&hash[1..]);
            sum += element_val;
        }

        sum
    }

    /// Calculate hit from the sum: normalize to 32 bytes, then hash.
    ///
    /// Steps:
    /// 1. Convert sum to 32-byte big-endian
    ///    - If sum < 32 bytes: zero-pad on the left
    ///    - If sum > 32 bytes: defensive fallback takes least-significant 32 bytes
    ///      (should be unreachable for k <= 32 and 31-byte elements)
    /// 2. hit = H(normalized_sum)
    fn calc_hit(sum: &BigUint) -> [u8; HASH_SIZE] {
        let sum_bytes = sum.to_bytes_be();
        debug_assert!(
            sum_bytes.len() <= HASH_SIZE,
            "sum should fit in 32 bytes for k<=32; got {} bytes",
            sum_bytes.len()
        );

        // Normalize sum to exactly 32 bytes:
        // - If < 32 bytes: zero-pad on left
        // - If > 32 bytes: take least-significant 32 bytes (defensive, shouldn't happen)
        let mut normalized = [0u8; HASH_SIZE];
        let start = HASH_SIZE.saturating_sub(sum_bytes.len());
        let copy_len = sum_bytes.len().min(HASH_SIZE);
        normalized[start..].copy_from_slice(&sum_bytes[sum_bytes.len() - copy_len..]);

        let mut hasher = Blake2b::<typenum::U32>::new();
        Digest::update(&mut hasher, &normalized);
        hasher.finalize().into()
    }

    /// Get the current N parameter.
    pub fn n(&self) -> u32 {
        self.n
    }

    /// Get the k parameter.
    pub fn k(&self) -> u32 {
        self.k
    }
}

/// Decode Bitcoin-style compact difficulty encoding.
///
/// nBits format: 0x[size][mantissa]
/// - size: 1 byte indicating the byte length of the value
/// - mantissa: 3 bytes (23-bit, high bit is sign in original Bitcoin)
///
/// Returns the decoded difficulty value.
fn decode_compact_bits(n_bits: u32) -> ConsensusResult<BigUint> {
    let size = (n_bits >> 24) as u32;

    // Sign bit (0x00800000) should never be set for difficulty
    if (n_bits & 0x0080_0000) != 0 {
        return Err(ConsensusError::InvalidPow(format!(
            "Invalid n_bits (negative compact): 0x{:08x}",
            n_bits
        )));
    }

    // 23-bit mantissa
    let mut mantissa = n_bits & 0x007f_ffff;

    let value = if size <= 3 {
        let shift = 8 * (3 - size);
        mantissa >>= shift;
        BigUint::from(mantissa)
    } else {
        let shift = 8 * (size - 3);
        BigUint::from(mantissa) << shift
    };

    if value == BigUint::from(0u8) {
        return Err(ConsensusError::InvalidPow(format!(
            "Invalid n_bits (zero difficulty): 0x{:08x}",
            n_bits
        )));
    }

    Ok(value)
}

/// Convert nBits to PoW target.
///
/// In Ergo, n_bits encodes the **difficulty**, not the target directly.
/// The target is computed as: target = GROUP_ORDER / decode_compact_bits(n_bits)
pub fn nbits_to_target(n_bits: u32) -> ConsensusResult<BigUint> {
    let diff = decode_compact_bits(n_bits)?;
    Ok((&*GROUP_ORDER) / diff)
}

/// Calculate difficulty from nBits (just decodes the compact format).
pub fn nbits_to_difficulty(n_bits: u32) -> ConsensusResult<BigUint> {
    decode_compact_bits(n_bits)
}

/// Convert difficulty BigUint to nBits compact representation.
pub fn difficulty_to_nbits(difficulty: &BigUint) -> u32 {
    if *difficulty == BigUint::from(0u32) {
        return 0;
    }

    let bytes = difficulty.to_bytes_be();
    let len = bytes.len();

    if len == 0 {
        return 0;
    }

    let (size, word) = if len <= 3 {
        let mut word = 0u32;
        for (i, &b) in bytes.iter().enumerate() {
            word |= (b as u32) << (8 * (2 - i));
        }
        (3u32, word)
    } else {
        let word = ((bytes[0] as u32) << 16) | ((bytes[1] as u32) << 8) | (bytes[2] as u32);
        (len as u32, word)
    };

    // Handle case where MSB is set (would be interpreted as negative)
    if word & 0x00800000 != 0 {
        ((size + 1) << 24) | (word >> 8)
    } else {
        (size << 24) | word
    }
}

/// Validate that the header's PoW solution is valid.
/// This is a convenience function that creates a verifier and checks the header.
pub fn validate_pow(header: &Header) -> ConsensusResult<bool> {
    let verifier = AutolykosV2::new();
    verifier.verify_header(header)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_compact_bits() {
        // Test known difficulty values (n_bits encodes difficulty, not target)
        let test_cases = vec![
            (0x1d00ffffu32, true), // Valid difficulty
            (0x1b0404cbu32, true), // Valid difficulty
            (0x17034d4bu32, true), // Valid difficulty
            (0x070baaaau32, true), // Real mainnet difficulty ~height 1M
        ];

        for (nbits, should_succeed) in test_cases {
            let result = decode_compact_bits(nbits);
            assert_eq!(
                result.is_ok(),
                should_succeed,
                "decode_compact_bits({:#x}) should {} but got {:?}",
                nbits,
                if should_succeed { "succeed" } else { "fail" },
                result
            );
            if should_succeed {
                assert!(result.unwrap() > BigUint::from(0u32));
            }
        }
    }

    #[test]
    fn test_nbits_to_target() {
        // Valid n_bits should produce a target (GROUP_ORDER / difficulty)
        let nbits = 0x1d00ffff_u32;
        let target = nbits_to_target(nbits).unwrap();
        assert!(target > BigUint::from(0u32));

        // Smaller size = smaller decoded difficulty = larger target (easier)
        let nbits_easier = 0x1c00ffff_u32;
        let target_easier = nbits_to_target(nbits_easier).unwrap();
        assert!(target_easier > target);

        // Zero n_bits should error (zero difficulty)
        assert!(nbits_to_target(0).is_err());

        // Negative compact (sign bit set) should error
        assert!(nbits_to_target(0x04800000).is_err());
    }

    #[test]
    fn test_autolykos_params() {
        let verifier = AutolykosV2::new();
        // n is the exponent (26), k is number of elements (32)
        assert_eq!(verifier.n(), DEFAULT_N_EXPONENT);
        assert_eq!(verifier.k(), AUTOLYKOS_K);
    }

    #[test]
    fn test_calc_big_n() {
        let verifier = AutolykosV2::new();
        let n_base = 1u32 << DEFAULT_N_EXPONENT; // 2^26

        // v1 always returns base N
        assert_eq!(verifier.calc_big_n(1, 700000), n_base);
        assert_eq!(verifier.calc_big_n(1, 100000), n_base);
        assert_eq!(verifier.calc_big_n(1, 70000000), n_base);

        // v2 before growth start
        assert_eq!(verifier.calc_big_n(2, 500000), n_base);
        assert_eq!(verifier.calc_big_n(2, 600000), n_base);

        // v2 after growth start (614,400) - matches sigma-rust test vectors
        assert_eq!(verifier.calc_big_n(2, 600 * 1024), 70464240);
        assert_eq!(verifier.calc_big_n(2, 650 * 1024), 73987410);
        assert_eq!(verifier.calc_big_n(2, 700000), 73987410);
        assert_eq!(verifier.calc_big_n(2, 788400), 81571035); // 3 years
        assert_eq!(verifier.calc_big_n(2, 1051200), 104107290); // 4 years
        assert_eq!(verifier.calc_big_n(2, 4198400), 2143944600); // max height
        assert_eq!(verifier.calc_big_n(2, 41984000), 2143944600); // beyond max
    }

    #[test]
    fn test_gen_indexes() {
        let verifier = AutolykosV2::new();
        // Use a non-trivial seed to exercise sliding window
        let seed: [u8; 32] = [
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54,
            0x32, 0x10, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb,
            0xcc, 0xdd, 0xee, 0xff,
        ];
        let big_n = 1u32 << DEFAULT_N_EXPONENT;
        let indices = verifier.gen_indexes(&seed, big_n);

        assert_eq!(indices.len(), AUTOLYKOS_K as usize);
        for &idx in &indices {
            assert!(idx < big_n);
        }

        // Verify sliding window produces different indices (not all same)
        let unique: std::collections::HashSet<_> = indices.iter().collect();
        assert!(
            unique.len() > 1,
            "sliding window should produce varied indices"
        );
    }

    #[test]
    fn test_difficulty_target_relationship() {
        // In Ergo: target = GROUP_ORDER / difficulty
        let nbits_mantissa_smaller = 0x1d00fffe_u32;
        let nbits_mantissa_larger = 0x1d00ffff_u32;

        let diff_smaller = nbits_to_difficulty(nbits_mantissa_smaller).unwrap();
        let diff_larger = nbits_to_difficulty(nbits_mantissa_larger).unwrap();
        let target_smaller = nbits_to_target(nbits_mantissa_smaller).unwrap();
        let target_larger = nbits_to_target(nbits_mantissa_larger).unwrap();

        assert!(diff_larger > diff_smaller);
        assert!(target_larger < target_smaller);
    }

    // ==================== Integration Tests with Real Headers ====================
    //
    // These tests require binary header fixtures in tests/fixtures/.
    // Fixtures must be captured from a trusted source (local Scala node).
    // See pow_test_vectors.rs for fixture loading utilities.

    #[test]
    fn test_valid_mainnet_header_post_growth_start() {
        use crate::pow_test_vectors::load_header_fixture;

        let header = load_header_fixture("header_height_1000000.bin");
        assert!(
            header.height >= N_INCREASE_START,
            "Fixture should be post-N-growth-start, got height {}",
            header.height
        );

        let result = validate_pow(&header);
        assert!(
            matches!(result, Ok(true)),
            "Valid header should pass PoW: {:?}",
            result
        );
    }

    #[test]
    fn test_valid_mainnet_header_pre_growth_start() {
        use crate::params::AUTOLYKOS_V2_ACTIVATION_HEIGHT;
        use crate::pow_test_vectors::load_header_fixture;

        let header = load_header_fixture("header_height_500000.bin");
        // Height 500,000 is in range [v2 activation, N growth start)
        // v2 algorithm with base N (no growth yet)
        assert!(
            header.height >= AUTOLYKOS_V2_ACTIVATION_HEIGHT,
            "Fixture should be post-v2-activation, got height {}",
            header.height
        );
        assert!(
            header.height < N_INCREASE_START,
            "Fixture should be pre-N-growth-start, got height {}",
            header.height
        );

        let result = validate_pow(&header);
        assert!(
            matches!(result, Ok(true)),
            "Valid header should pass PoW: {:?}",
            result
        );
    }

    #[test]
    fn test_mutated_nonce_fails_pow() {
        use crate::pow_test_vectors::{load_header_fixture, mutate_header_nonce};

        let original = load_header_fixture("header_height_1000000.bin");
        let mutated = mutate_header_nonce(&original);

        let result = validate_pow(&mutated);
        assert!(
            matches!(result, Ok(false)),
            "Mutated nonce should fail PoW target check: {:?}",
            result
        );
    }

    #[test]
    fn test_malformed_header_no_panic() {
        use ergo_chain_types::Header;
        use sigma_ser::ScorexSerializable;

        // Empty bytes
        assert!(Header::scorex_parse_bytes(&[]).is_err());
        // Random garbage
        assert!(Header::scorex_parse_bytes(&[0xFF; 50]).is_err());
        // Note: 300 zeros may successfully parse as a header with default values
        // depending on serialization format - not testing that case
    }

    /// Oracle test: compare our hit calculation with ergo-nipopow's reference
    #[test]
    fn test_pow_hit_matches_sigma_rust() {
        use crate::pow_test_vectors::load_header_fixture;
        use ergo_nipopow::NipopowAlgos;

        let header = load_header_fixture("header_height_614400.bin");

        // Get reference hit from sigma-rust via NipopowAlgos
        let nipopow = NipopowAlgos::default();
        let reference_hit = nipopow
            .pow_scheme
            .pow_hit(&header)
            .expect("reference pow_hit failed");

        // Get our computed values for debugging
        let verifier = AutolykosV2::new();
        let header_bytes = header.serialize_without_pow().expect("serialize failed");

        let big_n = verifier.calc_big_n(header.version, header.height);
        let msg = AutolykosV2::calculate_msg(&header_bytes);
        let seed =
            AutolykosV2::calc_seed_v2(big_n, &msg, &header.autolykos_solution.nonce, header.height);
        let indices = verifier.gen_indexes(&seed, big_n);
        let sum = AutolykosV2::calc_elements_sum(&indices, header.height);
        let hit = AutolykosV2::calc_hit(&sum);
        let our_hit = BigUint::from_bytes_be(&hit);

        // Convert reference hit to bytes for comparison
        let ref_bytes = reference_hit.to_bytes_be();
        let ref_hit = BigUint::from_bytes_be(&ref_bytes);

        // Debug output
        eprintln!("Header height: {}", header.height);
        eprintln!("Header version: {}", header.version);
        eprintln!("big_n: {}", big_n);
        eprintln!("msg: {}", hex::encode(&msg));
        eprintln!("seed: {}", hex::encode(&seed));
        eprintln!("first 5 indices: {:?}", &indices[..5]);
        eprintln!("our hit: {}", our_hit);
        eprintln!("ref hit: {}", ref_hit);

        assert_eq!(
            our_hit, ref_hit,
            "hit values must match sigma-rust reference"
        );
    }
}
