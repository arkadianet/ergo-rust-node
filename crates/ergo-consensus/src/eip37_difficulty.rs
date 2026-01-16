//! EIP-37 Difficulty Adjustment Algorithm.
//!
//! Strict port of Scala's DifficultyAdjustment and DifficultySerializer.
//! Uses signed BigInt for all intermediate math to match Scala exactly.
//!
//! Reference: `ergo-core/src/main/scala/org/ergoplatform/mining/difficulty/DifficultyAdjustment.scala`
//! Reference: `ergo-core/src/main/scala/org/ergoplatform/mining/difficulty/DifficultySerializer.scala`

use crate::chain_params::ChainParams;
use crate::{ConsensusError, ConsensusResult};
use num_bigint::{BigInt, Sign};
use num_traits::{Signed, Zero};
use tracing::debug;

/// Header data needed for difficulty calculation.
#[derive(Debug, Clone)]
pub struct EpochHeader {
    /// Block height.
    pub height: u32,
    /// Block timestamp in milliseconds.
    pub timestamp: u64,
    /// Required difficulty as BigInt (always positive for valid headers).
    pub difficulty: BigInt,
}

/// EIP-37 difficulty calculator.
///
/// Uses ChainParams for network-specific values.
pub struct Eip37DifficultyCalculator<'a> {
    params: &'a ChainParams,
}

impl<'a> Eip37DifficultyCalculator<'a> {
    /// Create a new calculator with the given chain parameters.
    pub fn new(params: &'a ChainParams) -> Self {
        Self { params }
    }

    /// Calculate difficulty for a block at `next_height`.
    ///
    /// # Arguments
    /// * `headers` - Headers needed for calculation. At epoch boundaries, this should be
    ///   the result of `ChainParams::previous_heights_for_recalculation()` (up to 9 headers
    ///   for mainnet with use_last_epochs=8). At non-boundary heights, only the parent
    ///   header is needed.
    /// * `next_height` - Height of the block being validated.
    ///
    /// # Returns
    /// The required difficulty as BigInt (positive).
    pub fn calculate(&self, headers: &[EpochHeader], next_height: u32) -> ConsensusResult<BigInt> {
        // Genesis or very early chain - use initial difficulty
        if next_height == 0 || headers.is_empty() {
            return Ok(BigInt::from_biguint(
                Sign::Plus,
                self.params.initial_difficulty().clone(),
            ));
        }

        // Not at epoch boundary - use parent's difficulty
        if !self.params.is_epoch_boundary(next_height) {
            return Ok(headers
                .last()
                .map(|h| h.difficulty.clone())
                .unwrap_or_else(|| {
                    BigInt::from_biguint(Sign::Plus, self.params.initial_difficulty().clone())
                }));
        }

        // At epoch boundary - need at least 2 headers for calculation
        if headers.len() < 2 {
            return Ok(headers
                .last()
                .map(|h| h.difficulty.clone())
                .unwrap_or_else(|| {
                    BigInt::from_biguint(Sign::Plus, self.params.initial_difficulty().clone())
                }));
        }

        // EIP-37 or pre-EIP-37 algorithm
        if self.params.is_eip37_active(next_height) {
            self.eip37_calculate(headers, self.params.epoch_length(next_height))
        } else {
            self.pre_eip37_calculate(headers, self.params.epoch_length(next_height))
        }
    }

    /// EIP-37 difficulty calculation (Scala: eip37Calculate).
    ///
    /// Step order from Scala:
    /// 1. predictiveDiff = calculate(previousHeaders, epochLength)
    /// 2. limitedPredictiveDiff = clamp predictiveDiff to [1/2, 3/2] of lastDiff
    /// 3. classicDiff = bitcoinCalculate(first, last, epochLength)
    /// 4. avg = (classicDiff + limitedPredictiveDiff) / 2
    /// 5. uncompressedDiff = clamp avg to [1/2, 3/2] of lastDiff
    /// 6. return decodeCompactBits(encodeCompactBits(uncompressedDiff))
    fn eip37_calculate(
        &self,
        headers: &[EpochHeader],
        epoch_length: u32,
    ) -> ConsensusResult<BigInt> {
        let last_diff = &headers.last().unwrap().difficulty;

        // Step 1: Predictive difficulty via linear regression
        let predictive_diff = self.interpolate(headers, epoch_length);

        // Step 2: Clamp predictive to [1/2, 3/2] of last
        let upper_bound = last_diff * 3 / 2;
        let lower_bound = last_diff / 2;
        let limited_predictive: BigInt = if predictive_diff > *last_diff {
            std::cmp::min(&predictive_diff, &upper_bound).clone()
        } else {
            std::cmp::max(&predictive_diff, &lower_bound).clone()
        };

        // Step 3: Classic Bitcoin-style difficulty
        let classic_diff = self.bitcoin_calculate(
            headers.first().unwrap(),
            headers.last().unwrap(),
            epoch_length,
        );

        // Step 4: Average
        let avg: BigInt = (&classic_diff + &limited_predictive) / 2;

        // Step 5: Clamp average to [1/2, 3/2] of last (reuse bounds)
        let uncompressed: BigInt = if avg > *last_diff {
            std::cmp::min(&avg, &upper_bound).clone()
        } else {
            std::cmp::max(&avg, &lower_bound).clone()
        };

        // Step 6: Normalize via nBits round-trip
        let nbits = encode_compact_bits(&uncompressed);
        let normalized = decode_compact_bits(nbits)?;

        debug!(
            predictive = %predictive_diff,
            limited_predictive = %limited_predictive,
            classic = %classic_diff,
            avg = %avg,
            uncompressed = %uncompressed,
            nbits = format!("0x{:08x}", nbits),
            normalized = %normalized,
            "EIP-37 difficulty calculation"
        );

        Ok(normalized)
    }

    /// Pre-EIP-37 difficulty calculation.
    fn pre_eip37_calculate(
        &self,
        headers: &[EpochHeader],
        epoch_length: u32,
    ) -> ConsensusResult<BigInt> {
        let last_diff = &headers.last().unwrap().difficulty;

        // Linear regression
        let new_diff = self.interpolate(headers, epoch_length);

        // Pre-EIP-37 uses 2x bounds (not 3/2)
        let clamped = if new_diff > *last_diff {
            new_diff.min(last_diff * 2)
        } else {
            new_diff.max(last_diff / 2)
        };

        // Normalize via nBits round-trip
        let nbits = encode_compact_bits(&clamped);
        decode_compact_bits(nbits)
    }

    /// Bitcoin-style difficulty calculation (Scala: bitcoinCalculate).
    ///
    /// difficulty = end.difficulty * desiredInterval * epochLength / (end.timestamp - start.timestamp)
    fn bitcoin_calculate(
        &self,
        start: &EpochHeader,
        end: &EpochHeader,
        epoch_length: u32,
    ) -> BigInt {
        let desired_interval = BigInt::from(self.params.block_interval_ms);
        let epoch_len = BigInt::from(epoch_length);
        let time_diff = BigInt::from(end.timestamp) - BigInt::from(start.timestamp);

        // Scala does plain division, no special handling
        if time_diff.is_zero() {
            // Avoid division by zero - shouldn't happen with valid headers
            return end.difficulty.clone();
        }

        &end.difficulty * &desired_interval * &epoch_len / &time_diff
    }

    /// Linear regression interpolation (Scala: interpolate).
    ///
    /// Uses precision constant (10^9) for fixed-point arithmetic.
    /// Returns extrapolated difficulty at next epoch boundary.
    fn interpolate(&self, headers: &[EpochHeader], epoch_length: u32) -> BigInt {
        let size = headers.len();
        if size == 1 {
            return headers[0].difficulty.clone();
        }

        let precision = BigInt::from(self.params.precision_constant);
        let n = BigInt::from(size as u64);

        // Data points: (height, difficulty)
        let data: Vec<(BigInt, BigInt)> = headers
            .iter()
            .map(|h| (BigInt::from(h.height), h.difficulty.clone()))
            .collect();

        // Calculate sums (all signed BigInt)
        let xy_sum: BigInt = data.iter().map(|(x, y)| x * y).sum();
        let x_sum: BigInt = data.iter().map(|(x, _)| x.clone()).sum();
        let x2_sum: BigInt = data.iter().map(|(x, _)| x * x).sum();
        let y_sum: BigInt = data.iter().map(|(_, y)| y.clone()).sum();

        // b = (xySum * size - xSum * ySum) * PrecisionConstant / (x2Sum * size - xSum * xSum)
        let b_num = &xy_sum * &n - &x_sum * &y_sum;
        let b_denom = &x2_sum * &n - &x_sum * &x_sum;

        if b_denom.is_zero() {
            // Degenerate case
            return headers.last().unwrap().difficulty.clone();
        }

        let b = &b_num * &precision / &b_denom;

        // a = (ySum * PrecisionConstant - b * xSum) / size / PrecisionConstant
        let a = (&y_sum * &precision - &b * &x_sum) / &n / &precision;

        // Extrapolate: point = max(heights) + epochLength
        let max_height = headers.iter().map(|h| h.height).max().unwrap();
        let point = BigInt::from(max_height + epoch_length);

        // result = a + b * point / PrecisionConstant
        &a + &b * &point / &precision
    }
}

/// Matches Java BigInteger.toByteArray() semantics (two's complement, minimal).
fn to_java_byte_array(value: &BigInt) -> Vec<u8> {
    let bytes = value.to_signed_bytes_be();
    if bytes.is_empty() {
        vec![0]
    } else {
        bytes
    }
}

/// Matches Java BigInteger.longValue(): low 64 bits of two's complement.
fn bigint_long_value(value: &BigInt) -> i64 {
    let bytes = value.to_signed_bytes_be();
    let mut arr = if value.is_negative() {
        [0xFFu8; 8]
    } else {
        [0u8; 8]
    };
    let take = bytes.len().min(8);
    arr[8 - take..].copy_from_slice(&bytes[bytes.len() - take..]);
    i64::from_be_bytes(arr)
}

/// Encode difficulty as compact nBits (Scala: DifficultySerializer.encodeCompactBits).
///
/// Line-by-line port of Scala:
/// ```scala
/// def encodeCompactBits(requiredDifficulty: BigInt): Long = {
///   val value = requiredDifficulty.bigInteger
///   var result: Long = 0L
///   var size: Int = value.toByteArray.length
///   if (size <= 3) {
///     result = value.longValue << 8 * (3 - size)
///   } else {
///     result = value.shiftRight(8 * (size - 3)).longValue
///   }
///   if ((result & 0x00800000L) != 0) {
///     result >>= 8
///     size += 1
///   }
///   result |= size << 24
///   val a: Int = if (value.signum == -1) 0x00800000 else 0
///   result |= a
///   result
/// }
/// ```
pub fn encode_compact_bits(difficulty: &BigInt) -> u32 {
    if difficulty.is_zero() {
        return 0;
    }

    let bytes = to_java_byte_array(difficulty);
    let mut size = bytes.len();

    let mut result: i64;
    if size <= 3 {
        result = bigint_long_value(difficulty) << (8 * (3 - size));
    } else {
        let shift = 8 * (size - 3);
        result = bigint_long_value(&(difficulty >> shift));
    }

    if (result & 0x0080_0000) != 0 {
        result >>= 8;
        size += 1;
    }

    result |= (size as i64) << 24;

    if difficulty.is_negative() {
        result |= 0x0080_0000;
    }

    result as u32
}

/// Decode compact nBits to difficulty BigInt (Scala: DifficultySerializer.decodeCompactBits).
///
/// Line-by-line port of Scala:
/// ```scala
/// def decodeCompactBits(compact: Long): BigInt = {
///   val size: Int = (compact >> 24).toInt & 0xFF
///   val bytes: Array[Byte] = new Array[Byte](4 + size)
///   bytes(3) = size.toByte
///   if (size >= 1) bytes(4) = ((compact >> 16) & 0xFF).toByte
///   if (size >= 2) bytes(5) = ((compact >> 8) & 0xFF).toByte
///   if (size >= 3) bytes(6) = (compact & 0xFF).toByte
///   decodeMPI(bytes)
/// }
/// ```
pub fn decode_compact_bits(compact: u32) -> ConsensusResult<BigInt> {
    let compact = compact as i64; // Match Scala Long
    let size = ((compact >> 24) & 0xFF) as usize;

    // Build MPI-format byte array
    let mut bytes = vec![0u8; 4 + size];
    bytes[3] = size as u8;
    if size >= 1 {
        bytes[4] = ((compact >> 16) & 0xFF) as u8;
    }
    if size >= 2 {
        bytes[5] = ((compact >> 8) & 0xFF) as u8;
    }
    if size >= 3 {
        bytes[6] = (compact & 0xFF) as u8;
    }

    let result = decode_mpi(&bytes);

    if result.is_zero() {
        return Err(ConsensusError::InvalidDifficulty {
            got: format!("zero difficulty from 0x{:08x}", compact),
            expected: "positive difficulty".to_string(),
        });
    }

    if result.is_negative() {
        return Err(ConsensusError::InvalidDifficulty {
            got: format!("negative difficulty from 0x{:08x}", compact),
            expected: "positive difficulty".to_string(),
        });
    }

    Ok(result)
}

/// Decode MPI (Multiple Precision Integer) format (Scala: decodeMPI).
///
/// MPI format: first 4 bytes are length (big-endian), followed by magnitude bytes.
/// If MSB of first magnitude byte is set, number is negative.
fn decode_mpi(bytes: &[u8]) -> BigInt {
    if bytes.len() < 4 {
        return BigInt::zero();
    }

    // First 4 bytes are length (only byte 3 is used for size)
    let size = bytes[3] as usize;
    if size == 0 || bytes.len() < 4 + size {
        return BigInt::zero();
    }

    let magnitude = &bytes[4..4 + size];
    if magnitude.is_empty() {
        return BigInt::zero();
    }

    // Check sign bit
    let is_negative = (magnitude[0] & 0x80) != 0;

    if is_negative {
        // Remove sign bit and negate
        let mut mag = magnitude.to_vec();
        mag[0] &= 0x7F;
        -BigInt::from_bytes_be(Sign::Plus, &mag)
    } else {
        BigInt::from_bytes_be(Sign::Plus, magnitude)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_bigint::BigUint;

    #[test]
    fn test_mainnet_initial_difficulty_nbits() {
        // Golden test: mainnet initial difficulty must roundtrip correctly
        let d = BigInt::from_biguint(
            Sign::Plus,
            BigUint::parse_bytes(b"011765000000", 16).unwrap(),
        );

        assert_eq!(encode_compact_bits(&d), 0x06011765);
        assert_eq!(decode_compact_bits(0x06011765).unwrap(), d);
    }

    #[test]
    fn test_encode_decode_roundtrip_for_clean_values() {
        // Values that roundtrip cleanly (no precision loss):
        // - Small values (size <= 3, no sign byte issue)
        // - Values with MSB clear in first byte
        // - Values already normalized (low byte 0x00 if size > 3)
        let test_values = vec![
            BigInt::from(1),
            BigInt::from(0x7FFFFF), // MSB clear, fits in 3 bytes
            BigInt::from(0x123400), // Already has 0x00 low byte
            BigInt::from_biguint(
                Sign::Plus,
                BigUint::parse_bytes(b"011765000000", 16).unwrap(),
            ),
        ];

        for value in test_values {
            let encoded = encode_compact_bits(&value);
            let decoded = decode_compact_bits(encoded).unwrap();
            assert_eq!(decoded, value, "Roundtrip failed for {}", value);
        }
    }

    #[test]
    fn test_compact_bits_is_lossy() {
        // Compact bits encode/decode via MPI format with limited precision.
        // The mantissa portion only carries ~23 bits of significance, so
        // large values lose their low-order bits through the roundtrip.
        let value = BigInt::from(0x12345678u32);
        let encoded = encode_compact_bits(&value);
        let decoded = decode_compact_bits(encoded).unwrap();

        // Should NOT equal original (lossy)
        assert_ne!(
            decoded, value,
            "Compact bits should be lossy for values exceeding mantissa precision"
        );

        // Should equal truncated value
        assert_eq!(decoded, BigInt::from(0x12345600u32));
    }

    #[test]
    fn test_compact_bits_sign_byte_edge_case() {
        // When the first byte has MSB set (0x80), Java BigInteger.toByteArray()
        // prepends a 0x00 sign byte. This affects the size field in compact encoding
        // and triggers a shift-right, causing additional precision loss.
        let value = BigInt::from(0x801234u32);
        let encoded = encode_compact_bits(&value);
        let decoded = decode_compact_bits(encoded).unwrap();

        // Lossy: low byte dropped due to sign-byte normalization
        assert_ne!(decoded, value);
        assert_eq!(decoded, BigInt::from(0x801200u32));

        // Size should be 4 (3 bytes + sign byte adjustment)
        assert_eq!((encoded >> 24) & 0xFF, 4);
    }

    #[test]
    fn test_java_byte_array_positive_no_sign_byte() {
        // Value where MSB is NOT set - no leading zero needed
        let value = BigInt::from(0x7F);
        let bytes = to_java_byte_array(&value);
        assert_eq!(bytes, vec![0x7F]);
    }

    #[test]
    fn test_java_byte_array_positive_with_sign_byte() {
        // Value where MSB IS set - needs leading zero
        let value = BigInt::from(0x80);
        let bytes = to_java_byte_array(&value);
        assert_eq!(bytes, vec![0x00, 0x80]);
    }

    #[test]
    fn test_bigint_long_value() {
        // Positive values
        assert_eq!(bigint_long_value(&BigInt::from(0)), 0);
        assert_eq!(bigint_long_value(&BigInt::from(1)), 1);
        assert_eq!(bigint_long_value(&BigInt::from(i64::MAX)), i64::MAX);

        // Negative values
        assert_eq!(bigint_long_value(&BigInt::from(-1)), -1);
        assert_eq!(bigint_long_value(&BigInt::from(-128)), -128);

        // Large positive (truncates to low 64 bits)
        let large = BigInt::from(1u128 << 64) + BigInt::from(42);
        assert_eq!(bigint_long_value(&large), 42);
    }

    #[test]
    fn test_decode_mpi_positive() {
        // MPI: [0, 0, 0, 2, 0x01, 0x00] = 256
        let bytes = vec![0, 0, 0, 2, 0x01, 0x00];
        let result = decode_mpi(&bytes);
        assert_eq!(result, BigInt::from(256));
    }

    #[test]
    fn test_decode_mpi_negative() {
        // MPI: [0, 0, 0, 1, 0x81] = -1 (0x80 is sign bit, 0x01 is magnitude)
        let bytes = vec![0, 0, 0, 1, 0x81];
        let result = decode_mpi(&bytes);
        assert_eq!(result, BigInt::from(-1));
    }

    #[test]
    fn test_clamp_eip37_upper() {
        let last_diff = BigInt::from(1000);
        let too_high = BigInt::from(2000);

        // Should clamp to 3/2 of last = 1500
        let clamped = if too_high > last_diff {
            too_high.min(&last_diff * 3 / 2)
        } else {
            too_high
        };
        assert_eq!(clamped, BigInt::from(1500));
    }

    #[test]
    fn test_clamp_eip37_lower() {
        let last_diff = BigInt::from(1000);
        let too_low = BigInt::from(100);

        // Should clamp to 1/2 of last = 500
        let clamped = if too_low > last_diff {
            too_low
        } else {
            too_low.max(&last_diff / 2)
        };
        assert_eq!(clamped, BigInt::from(500));
    }

    #[test]
    fn test_bitcoin_calculate_on_target() {
        let params = ChainParams::mainnet();
        let calc = Eip37DifficultyCalculator::new(&params);

        let start = EpochHeader {
            height: 0,
            timestamp: 0,
            difficulty: BigInt::from(1000),
        };
        let end = EpochHeader {
            height: 128,
            timestamp: 128 * 120_000, // Exactly on target
            difficulty: BigInt::from(1000),
        };

        let result = calc.bitcoin_calculate(&start, &end, 128);
        assert_eq!(result, BigInt::from(1000));
    }

    #[test]
    fn test_bitcoin_calculate_too_fast() {
        let params = ChainParams::mainnet();
        let calc = Eip37DifficultyCalculator::new(&params);

        let start = EpochHeader {
            height: 0,
            timestamp: 0,
            difficulty: BigInt::from(1000),
        };
        let end = EpochHeader {
            height: 128,
            timestamp: 128 * 60_000, // Half the target time
            difficulty: BigInt::from(1000),
        };

        let result = calc.bitcoin_calculate(&start, &end, 128);
        // Difficulty should double
        assert_eq!(result, BigInt::from(2000));
    }

    #[test]
    fn test_interpolate_single_header() {
        let params = ChainParams::mainnet();
        let calc = Eip37DifficultyCalculator::new(&params);

        let headers = vec![EpochHeader {
            height: 0,
            timestamp: 0,
            difficulty: BigInt::from(1000),
        }];

        let result = calc.interpolate(&headers, 128);
        assert_eq!(result, BigInt::from(1000));
    }

    #[test]
    fn test_interpolate_increasing() {
        let params = ChainParams::mainnet();
        let calc = Eip37DifficultyCalculator::new(&params);

        let headers = vec![
            EpochHeader {
                height: 0,
                timestamp: 0,
                difficulty: BigInt::from(100),
            },
            EpochHeader {
                height: 128,
                timestamp: 128 * 120_000,
                difficulty: BigInt::from(200),
            },
            EpochHeader {
                height: 256,
                timestamp: 256 * 120_000,
                difficulty: BigInt::from(300),
            },
        ];

        let result = calc.interpolate(&headers, 128);
        // Upward trend - should predict above 300
        assert!(result > BigInt::from(300));
    }

    #[test]
    fn test_interpolate_decreasing() {
        let params = ChainParams::mainnet();
        let calc = Eip37DifficultyCalculator::new(&params);

        let headers = vec![
            EpochHeader {
                height: 0,
                timestamp: 0,
                difficulty: BigInt::from(300),
            },
            EpochHeader {
                height: 128,
                timestamp: 128 * 120_000,
                difficulty: BigInt::from(200),
            },
            EpochHeader {
                height: 256,
                timestamp: 256 * 120_000,
                difficulty: BigInt::from(100),
            },
        ];

        let result = calc.interpolate(&headers, 128);
        // Downward trend - should predict below 100
        // (BigInt handles negative intermediate values correctly)
        assert!(result < BigInt::from(100));
    }
}
