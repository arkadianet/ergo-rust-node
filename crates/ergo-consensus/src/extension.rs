//! Extension validation and digest computation.
//!
//! Strict port of Scala's Extension validation rules:
//! - `ergo-core/src/main/scala/org/ergoplatform/modifiers/history/extension/Extension.scala`
//! - `ergo-core/src/main/scala/org/ergoplatform/modifiers/history/extension/ExtensionSerializer.scala`
//! - `ergo-core/src/main/scala/org/ergoplatform/nodeView/history/storage/modifierprocessors/ExtensionValidator.scala`
//!
//! ## Implemented Validation Rules
//!
//! | Rule | ID  | Description | Status |
//! |------|-----|-------------|--------|
//! | exKeyLength | 403 | Keys must be 2 bytes | Enforced by type |
//! | exValueLength | 404 | Values must be <= 64 bytes | Implemented |
//! | exDuplicateKeys | 405 | No duplicate keys | Implemented |
//! | exEmpty | 406 | Non-genesis must have fields | Implemented |
//!
//! ## Deferred Validation Rules (Interlinks)
//!
//! | Rule | ID  | Description | Status |
//! |------|-----|-------------|--------|
//! | exIlEncoding | 401 | Interlinks properly packed | **DEFERRED** |
//! | exIlStructure | 402 | Interlinks correct structure | **DEFERRED** |
//!
//! Interlinks validation (401/402) is consensus-critical but requires NiPoPoW
//! infrastructure. These rules will be implemented in Phase 3.5 or the NiPoPoW module.
//!
//! ## IMPORTANT: Merkle Tree Compatibility
//!
//! The `compute_digest()` function uses `ergo_merkle_tree::MerkleTree` which MUST
//! match Scala's `Algos.merkleTreeRoot` exactly (leaf hashing, internal nodes,
//! odd-leaf handling). **This has NOT been verified against real mainnet blocks yet.**
//!
//! Before trusting `compute_digest()` in production, add golden fixture tests
//! comparing against known (header.extensionRoot, extension_bytes) pairs from mainnet.

use crate::{ConsensusError, ConsensusResult};
use ergo_chain_types::{BlockId, Digest32};
use ergo_merkle_tree::{MerkleNode, MerkleTree};
use std::collections::HashSet;

// ============================================================================
// VLQ (Variable Length Quantity) encoding/decoding
// ============================================================================
//
// Scorex's putUShort/getUShort use VLQ encoding, not fixed-width integers.
// VLQ is a little-endian continuation-bit encoding:
// - Bit 7 (0x80) = continuation flag (1 = more bytes follow)
// - Bits 0-6 = 7 bits of value (little-endian order)

/// Encode a u64 value as VLQ bytes.
fn vlq_encode(mut value: u64) -> Vec<u8> {
    let mut result = Vec::new();
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80; // Set continuation bit
        }
        result.push(byte);
        if value == 0 {
            break;
        }
    }
    result
}

/// Decode a VLQ value from bytes. Returns (value, bytes_consumed).
fn vlq_decode(data: &[u8]) -> Result<(u64, usize), &'static str> {
    let mut result: u64 = 0;
    let mut shift = 0;
    let mut pos = 0;
    loop {
        if pos >= data.len() {
            return Err("Truncated VLQ");
        }
        let byte = data[pos];
        pos += 1;
        result |= ((byte & 0x7F) as u64) << shift;
        if (byte & 0x80) == 0 {
            break;
        }
        shift += 7;
        if shift > 63 {
            return Err("VLQ overflow");
        }
    }
    Ok((result, pos))
}

/// Calculate the number of bytes needed to encode a value as VLQ.
fn vlq_size(value: u64) -> usize {
    if value == 0 {
        return 1;
    }
    let bits = 64 - value.leading_zeros() as usize;
    (bits + 6) / 7 // Ceiling division by 7
}

/// Extension field key size (bytes).
pub const FIELD_KEY_SIZE: usize = 2;

/// Maximum extension field value size (bytes).
pub const FIELD_VALUE_MAX_SIZE: usize = 64;

/// Maximum total extension size (bytes).
/// Scala: Constants.MaxExtensionSizeMax = 32768
pub const MAX_EXTENSION_SIZE: usize = 32768;

/// Empty Merkle tree root (blake2b256 of empty byte array).
/// Scala: Algos.emptyMerkleTreeRoot
/// = 0e5751c026e543b2e8ab2eb06099daa1d1e5df47778f7787faab45cdf12fe3a8
pub const EMPTY_MERKLE_ROOT: [u8; 32] = [
    0x0e, 0x57, 0x51, 0xc0, 0x26, 0xe5, 0x43, 0xb2, 0xe8, 0xab, 0x2e, 0xb0, 0x60, 0x99, 0xda, 0xa1,
    0xd1, 0xe5, 0xdf, 0x47, 0x77, 0x8f, 0x77, 0x87, 0xfa, 0xab, 0x45, 0xcd, 0xf1, 0x2f, 0xe3, 0xa8,
];

/// Extension field (key-value pair).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionField {
    /// Field key (exactly 2 bytes).
    pub key: [u8; FIELD_KEY_SIZE],
    /// Field value (0-64 bytes).
    pub value: Vec<u8>,
}

impl ExtensionField {
    /// Create a new extension field.
    pub fn new(key: [u8; FIELD_KEY_SIZE], value: Vec<u8>) -> Self {
        Self { key, value }
    }

    /// Serialized size of this field.
    /// Format: key (2) + value_len (1) + value (variable)
    pub fn serialized_size(&self) -> usize {
        FIELD_KEY_SIZE + 1 + self.value.len()
    }
}

/// Block extension containing key-value pairs.
#[derive(Debug, Clone)]
pub struct Extension {
    /// Header ID this extension belongs to.
    pub header_id: BlockId,
    /// Key-value fields in insertion order (order affects digest!).
    pub fields: Vec<ExtensionField>,
}

impl Extension {
    /// Create a new extension.
    pub fn new(header_id: BlockId, fields: Vec<ExtensionField>) -> Self {
        Self { header_id, fields }
    }

    /// Create an empty extension.
    pub fn empty(header_id: BlockId) -> Self {
        Self {
            header_id,
            fields: Vec::new(),
        }
    }

    /// Get a field by key.
    pub fn get(&self, key: &[u8; FIELD_KEY_SIZE]) -> Option<&[u8]> {
        self.fields
            .iter()
            .find(|f| &f.key == key)
            .map(|f| f.value.as_slice())
    }

    /// Get interlinks (for NiPoPoW).
    ///
    /// Interlinks are stored under reserved extension keys (see Scala interlinks key constants).
    /// Full parsing + validation is deferred until NiPoPoW infrastructure exists (rules 401/402).
    pub fn interlinks(&self) -> Option<Vec<BlockId>> {
        // TODO: implement interlinks parsing
        None
    }

    /// System parameters prefix in extension keys.
    /// Parameters are stored with key format: [0x00, param_id]
    pub const SYSTEM_PARAMS_PREFIX: u8 = 0x00;

    /// Parse system parameters from extension fields.
    ///
    /// Parameters in Ergo extensions are stored as:
    /// - Key: 2 bytes - [SYSTEM_PARAMS_PREFIX (0x00), param_id]
    /// - Value: 4 bytes - big-endian signed 32-bit integer
    ///
    /// Returns a map of parameter_id -> value, or an error if parsing fails.
    pub fn parse_parameters(
        &self,
    ) -> Result<std::collections::HashMap<u8, i32>, crate::ConsensusError> {
        use std::collections::HashMap;

        let mut params = HashMap::new();

        for field in &self.fields {
            // Check for system parameter prefix
            if field.key[0] == Self::SYSTEM_PARAMS_PREFIX {
                let param_id = field.key[1];

                // Skip special keys (soft-fork related, etc.)
                // Param IDs 1-8 are the core votable parameters
                // IDs >= 120 are soft-fork voting related
                if param_id >= 120 {
                    continue;
                }

                // Values must be exactly 4 bytes (big-endian i32)
                if field.value.len() != 4 {
                    return Err(crate::ConsensusError::InvalidExtension(format!(
                        "Parameter {} has invalid value length: {} (expected 4)",
                        param_id,
                        field.value.len()
                    )));
                }

                let value = i32::from_be_bytes([
                    field.value[0],
                    field.value[1],
                    field.value[2],
                    field.value[3],
                ]);

                params.insert(param_id, value);
            }
        }

        Ok(params)
    }

    /// Get system parameters from extension (legacy method).
    #[deprecated(note = "Use parse_parameters() instead")]
    pub fn parameters(&self) -> Vec<(u8, u64)> {
        self.parse_parameters()
            .map(|params| {
                params
                    .into_iter()
                    .filter_map(|(k, v)| u64::try_from(v).ok().map(|u| (k, u)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Serialize extension to bytes (Scala-compatible format).
    ///
    /// Format:
    /// - header_id: 32 bytes
    /// - field_count: VLQ encoded
    /// - for each field:
    ///   - key: 2 bytes
    ///   - value_len: u8
    ///   - value: variable bytes
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.serialized_size());

        // Header ID (32 bytes)
        buf.extend_from_slice(self.header_id.0.as_ref());

        // Field count (VLQ encoded)
        buf.extend_from_slice(&vlq_encode(self.fields.len() as u64));

        // Fields
        for field in &self.fields {
            buf.extend_from_slice(&field.key);
            buf.push(field.value.len() as u8);
            buf.extend_from_slice(&field.value);
        }

        buf
    }

    /// Total serialized size.
    pub fn serialized_size(&self) -> usize {
        32 + vlq_size(self.fields.len() as u64)
            + self
                .fields
                .iter()
                .map(|f| f.serialized_size())
                .sum::<usize>()
    }

    /// Parse extension from bytes (Scala-compatible format).
    ///
    /// Rejects if total size >= MAX_EXTENSION_SIZE (matching Scala's require check).
    ///
    /// **Note:** This parser consumes exactly the bytes needed for the declared field count.
    /// Trailing bytes are ignored, which matches Scala's reader-based parsing in a
    /// length-delimited modifier context. Callers should pass the exact modifier slice.
    pub fn parse(data: &[u8]) -> Result<Self, ExtensionParseError> {
        // Minimum: 32 (header_id) + 1 (VLQ min) = 33 bytes
        if data.len() < 33 {
            return Err(ExtensionParseError::TooShort);
        }

        // Track bytes read for size limit check
        let mut pos = 0;

        // Header ID (32 bytes)
        let mut header_bytes = [0u8; 32];
        header_bytes.copy_from_slice(&data[pos..pos + 32]);
        let header_id = BlockId(Digest32::from(header_bytes));
        pos += 32;

        // Field count (VLQ encoded)
        let (field_count_u64, vlq_len) = vlq_decode(&data[pos..])
            .map_err(|e| ExtensionParseError::InvalidFormat(e.to_string()))?;
        pos += vlq_len;

        // Enforce UShort semantics (0..65535) even though VLQ can encode larger
        if field_count_u64 > 0xFFFF {
            return Err(ExtensionParseError::InvalidFormat(
                "field count exceeds UShort max (65535)".to_string(),
            ));
        }
        let field_count = field_count_u64 as usize;

        // Sanity check: each field is at least 3 bytes (2 key + 1 len + 0 value)
        let remaining = data.len().saturating_sub(pos);
        if field_count > 0 && field_count * 3 > remaining {
            return Err(ExtensionParseError::InvalidFormat(format!(
                "field count {} impossible with {} remaining bytes",
                field_count, remaining
            )));
        }

        // Parse fields
        let mut fields = Vec::with_capacity(field_count);
        for _ in 0..field_count {
            // Check size limit BEFORE reading field (Scala's takeWhile + require pattern)
            if pos >= MAX_EXTENSION_SIZE {
                return Err(ExtensionParseError::SizeLimitExceeded);
            }

            // Key (2 bytes)
            if pos + FIELD_KEY_SIZE > data.len() {
                return Err(ExtensionParseError::UnexpectedEnd);
            }
            let key: [u8; FIELD_KEY_SIZE] = data[pos..pos + FIELD_KEY_SIZE].try_into().unwrap();
            pos += FIELD_KEY_SIZE;

            // Value length (1 byte)
            if pos >= data.len() {
                return Err(ExtensionParseError::UnexpectedEnd);
            }
            let value_len = data[pos] as usize;
            pos += 1;

            // Value
            if pos + value_len > data.len() {
                return Err(ExtensionParseError::UnexpectedEnd);
            }
            let value = data[pos..pos + value_len].to_vec();
            pos += value_len;

            fields.push(ExtensionField { key, value });
        }

        // Final size check (Scala: require(r.position - startPosition < MaxExtensionSizeMax))
        if pos >= MAX_EXTENSION_SIZE {
            return Err(ExtensionParseError::SizeLimitExceeded);
        }

        Ok(Self { header_id, fields })
    }

    /// Compute extension digest (Merkle root).
    ///
    /// Scala: Extension.merkleTree / ExtensionCandidate.digest
    ///
    /// Leaf format (kvToLeaf): `[key_length: 1 byte][key: 2 bytes][value: variable]`
    /// Empty extension returns `blake2b256([])`.
    ///
    /// # WARNING
    ///
    /// This uses `ergo_merkle_tree::MerkleTree` which MUST match Scala's implementation.
    /// Verify against golden fixtures before trusting in production.
    pub fn compute_digest(&self) -> Digest32 {
        if self.fields.is_empty() {
            return Digest32::from(EMPTY_MERKLE_ROOT);
        }

        // Convert fields to Merkle leaves
        // Scala: kvToLeaf = Bytes.concat(Array(key.length.toByte), key, value)
        let leaves: Vec<MerkleNode> = self
            .fields
            .iter()
            .map(|field| {
                let mut leaf = Vec::with_capacity(1 + field.key.len() + field.value.len());
                leaf.push(field.key.len() as u8); // key_length (always 2)
                leaf.extend_from_slice(&field.key);
                leaf.extend_from_slice(&field.value);
                MerkleNode::from_bytes(leaf)
            })
            .collect();

        let tree = MerkleTree::new(leaves);
        tree.root_hash()
    }

    /// Validate extension structure (consensus rules 403-406).
    ///
    /// **Does NOT validate interlinks (rules 401/402).** Those require NiPoPoW
    /// infrastructure and will be implemented separately.
    ///
    /// Scala: ExtensionValidator
    /// - 403: exKeyLength - all keys must be 2 bytes (enforced by type)
    /// - 404: exValueLength - all values must be <= 64 bytes
    /// - 405: exDuplicateKeys - no duplicate keys
    /// - 406: exEmpty - non-genesis blocks must have fields
    pub fn validate(&self, is_genesis: bool) -> ConsensusResult<()> {
        // Rule 404: Value length check
        for (i, field) in self.fields.iter().enumerate() {
            if field.value.len() > FIELD_VALUE_MAX_SIZE {
                return Err(ConsensusError::InvalidExtension(format!(
                    "field {} value too long: {} bytes, max {}",
                    i,
                    field.value.len(),
                    FIELD_VALUE_MAX_SIZE
                )));
            }
        }

        // Rule 405: Duplicate keys check
        let mut seen_keys = HashSet::new();
        for field in &self.fields {
            if !seen_keys.insert(field.key) {
                return Err(ConsensusError::InvalidExtension(format!(
                    "duplicate key: {:02x}{:02x}",
                    field.key[0], field.key[1]
                )));
            }
        }

        // Rule 406: Non-genesis must have fields
        if !is_genesis && self.fields.is_empty() {
            return Err(ConsensusError::InvalidExtension(
                "non-genesis block extension cannot be empty".to_string(),
            ));
        }

        // Serialized size check
        if self.serialized_size() >= MAX_EXTENSION_SIZE {
            return Err(ConsensusError::InvalidExtension(format!(
                "extension too large: {} bytes, max {}",
                self.serialized_size(),
                MAX_EXTENSION_SIZE - 1
            )));
        }

        Ok(())
    }
}

/// Verify extension matches header (root + header_id + structure).
///
/// This is the main entry point for extension validation during block verification.
/// Validates:
/// - Header ID matches
/// - Structure rules (403-406)
/// - Computed digest matches expected root
///
/// # Interlinks (rules 401/402)
///
/// **FAIL-CLOSED**: For non-genesis blocks, this function currently returns an error
/// because interlinks validation (401/402) is not yet implemented. Interlinks are
/// consensus-critical and must be validated before accepting blocks.
///
/// Once `verify_interlinks()` is implemented, this function should call it or
/// the caller should invoke it separately.
pub fn verify_extension_root(
    extension: &Extension,
    expected_header_id: &BlockId,
    expected_root: &Digest32,
    is_genesis: bool,
) -> ConsensusResult<()> {
    // Check header ID matches (cheap, helpful error message)
    if extension.header_id.0.as_ref() != expected_header_id.0.as_ref() {
        return Err(ConsensusError::InvalidExtension(format!(
            "extension header_id mismatch: got {}, expected {}",
            hex::encode(extension.header_id.0.as_ref()),
            hex::encode(expected_header_id.0.as_ref())
        )));
    }

    // Validate structure (rules 403-406) - cheap checks first
    extension.validate(is_genesis)?;

    // FAIL-CLOSED: Interlinks validation (401/402) not yet implemented.
    // Non-genesis blocks require valid interlinks, which we cannot verify yet.
    // Check after structure validation so basic errors get useful messages.
    if !is_genesis {
        return Err(ConsensusError::InvalidExtension(
            "interlinks validation (rules 401/402) not yet implemented; cannot accept non-genesis blocks until NiPoPoW infrastructure is ready".to_string(),
        ));
    }

    // Compute and compare digest (expensive, only for genesis until interlinks ready)
    let computed = extension.compute_digest();
    if computed.as_ref() != expected_root.as_ref() {
        return Err(ConsensusError::InvalidExtension(format!(
            "extension root mismatch: computed {}, expected {}",
            hex::encode(computed.as_ref()),
            hex::encode(expected_root.as_ref())
        )));
    }

    Ok(())
}

/// Errors during extension parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionParseError {
    /// Data too short.
    TooShort,
    /// Unexpected end of data.
    UnexpectedEnd,
    /// Extension size limit exceeded.
    SizeLimitExceeded,
    /// Invalid field format.
    InvalidFormat(String),
}

impl std::fmt::Display for ExtensionParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort => write!(f, "extension data too short"),
            Self::UnexpectedEnd => write!(f, "unexpected end of extension data"),
            Self::SizeLimitExceeded => write!(f, "extension size limit exceeded"),
            Self::InvalidFormat(msg) => write!(f, "invalid extension format: {}", msg),
        }
    }
}

impl std::error::Error for ExtensionParseError {}

#[cfg(test)]
mod tests {
    use super::*;
    use ergo_chain_types::blake2b256_hash;

    #[test]
    fn test_empty_merkle_root_constant() {
        // Verify our constant matches blake2b256([])
        let computed = blake2b256_hash(&[]);
        assert_eq!(
            computed.as_ref(),
            &EMPTY_MERKLE_ROOT,
            "EMPTY_MERKLE_ROOT constant doesn't match blake2b256([])"
        );
    }

    #[test]
    fn test_empty_extension_digest() {
        let ext = Extension::empty(BlockId(Digest32::zero()));
        let digest = ext.compute_digest();
        assert_eq!(
            digest.as_ref(),
            &EMPTY_MERKLE_ROOT,
            "Empty extension should have empty merkle root"
        );
    }

    #[test]
    fn test_serialize_roundtrip() {
        let header_id = BlockId(Digest32::from([0xAB; 32]));
        let fields = vec![
            ExtensionField::new([0x00, 0x01], vec![0x11, 0x22, 0x33]),
            ExtensionField::new([0x00, 0x02], vec![0x44, 0x55]),
        ];
        let ext = Extension::new(header_id.clone(), fields);

        let serialized = ext.serialize();
        let parsed = Extension::parse(&serialized).expect("parse failed");

        assert_eq!(parsed.header_id, header_id);
        assert_eq!(parsed.fields.len(), 2);
        assert_eq!(parsed.fields[0].key, [0x00, 0x01]);
        assert_eq!(parsed.fields[0].value, vec![0x11, 0x22, 0x33]);
        assert_eq!(parsed.fields[1].key, [0x00, 0x02]);
        assert_eq!(parsed.fields[1].value, vec![0x44, 0x55]);
    }

    #[test]
    fn test_validate_value_too_long() {
        let ext = Extension::new(
            BlockId(Digest32::zero()),
            vec![ExtensionField::new([0x00, 0x01], vec![0u8; 65])], // 65 > 64
        );

        let result = ext.validate(false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("value too long"));
    }

    #[test]
    fn test_validate_duplicate_keys() {
        let ext = Extension::new(
            BlockId(Digest32::zero()),
            vec![
                ExtensionField::new([0x00, 0x01], vec![0x11]),
                ExtensionField::new([0x00, 0x01], vec![0x22]), // duplicate
            ],
        );

        let result = ext.validate(false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("duplicate key"));
    }

    #[test]
    fn test_validate_empty_non_genesis() {
        let ext = Extension::empty(BlockId(Digest32::zero()));

        // Empty allowed for genesis
        assert!(ext.validate(true).is_ok());

        // Empty NOT allowed for non-genesis
        let result = ext.validate(false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot be empty"));
    }

    #[test]
    fn test_digest_deterministic() {
        let ext = Extension::new(
            BlockId(Digest32::zero()),
            vec![
                ExtensionField::new([0x00, 0x01], vec![0x11, 0x22]),
                ExtensionField::new([0x00, 0x02], vec![0x33]),
            ],
        );

        let digest1 = ext.compute_digest();
        let digest2 = ext.compute_digest();
        assert_eq!(digest1, digest2, "Digest should be deterministic");
    }

    #[test]
    fn test_digest_order_matters() {
        // Same fields, different order = different digest
        let ext1 = Extension::new(
            BlockId(Digest32::zero()),
            vec![
                ExtensionField::new([0x00, 0x01], vec![0x11]),
                ExtensionField::new([0x00, 0x02], vec![0x22]),
            ],
        );
        let ext2 = Extension::new(
            BlockId(Digest32::zero()),
            vec![
                ExtensionField::new([0x00, 0x02], vec![0x22]),
                ExtensionField::new([0x00, 0x01], vec![0x11]),
            ],
        );

        let digest1 = ext1.compute_digest();
        let digest2 = ext2.compute_digest();
        assert_ne!(digest1, digest2, "Field order should affect digest");
    }

    #[test]
    fn test_verify_extension_root_mismatch() {
        let header_id = BlockId(Digest32::zero());
        let ext = Extension::new(
            header_id.clone(),
            vec![ExtensionField::new([0x00, 0x01], vec![0x11])],
        );

        let wrong_root = Digest32::from([0xFF; 32]);
        // Use is_genesis=true to bypass interlinks check and test root comparison
        let result = verify_extension_root(&ext, &header_id, &wrong_root, true);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("root mismatch"));
    }

    #[test]
    fn test_verify_extension_root_match() {
        let header_id = BlockId(Digest32::zero());
        let ext = Extension::new(
            header_id.clone(),
            vec![ExtensionField::new([0x00, 0x01], vec![0x11])],
        );

        let correct_root = ext.compute_digest();
        // Use is_genesis=true to bypass interlinks check
        let result = verify_extension_root(&ext, &header_id, &correct_root, true);
        assert!(result.is_ok());
    }

    #[test]
    fn test_verify_extension_header_id_mismatch() {
        let ext = Extension::new(
            BlockId(Digest32::zero()),
            vec![ExtensionField::new([0x00, 0x01], vec![0x11])],
        );

        let wrong_header_id = BlockId(Digest32::from([0xFF; 32]));
        let root = ext.compute_digest();
        // Use is_genesis=true to bypass interlinks check and test header_id comparison
        let result = verify_extension_root(&ext, &wrong_header_id, &root, true);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("header_id mismatch"));
    }

    #[test]
    fn test_verify_extension_non_genesis_fails_closed() {
        let header_id = BlockId(Digest32::zero());
        let ext = Extension::new(
            header_id.clone(),
            vec![ExtensionField::new([0x00, 0x01], vec![0x11])],
        );

        let correct_root = ext.compute_digest();
        // Non-genesis should fail with interlinks error
        let result = verify_extension_root(&ext, &header_id, &correct_root, false);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("interlinks validation"));
    }

    #[test]
    fn test_parse_size_limit() {
        // Create extension that exceeds size limit
        let header_id = BlockId(Digest32::zero());
        let mut fields = Vec::new();
        // Each field: 2 (key) + 1 (len) + 64 (value) = 67 bytes
        // Need ~490 fields to exceed 32768 bytes
        for i in 0..500u16 {
            let key = i.to_be_bytes();
            fields.push(ExtensionField::new(key, vec![0u8; 64]));
        }
        let ext = Extension::new(header_id, fields);
        let serialized = ext.serialize();

        assert!(serialized.len() >= MAX_EXTENSION_SIZE);

        let result = Extension::parse(&serialized);
        assert!(matches!(
            result,
            Err(ExtensionParseError::SizeLimitExceeded)
        ));
    }

    // ========================================================================
    // GOLDEN FIXTURE TESTS
    // ========================================================================
    //
    // These tests verify Merkle tree compatibility with Scala by comparing
    // against known (extension_hash, fields) pairs from real mainnet blocks.
    //
    // Data extracted from mainnet via: curl http://localhost:9053/blocks/{id}

    /// Helper to create Extension and check digest against expected hash.
    /// Note: This validates Merkle tree computation, not binary parse/serialize compatibility.
    fn check_extension_digest(
        header_id_hex: &str,
        expected_hash_hex: &str,
        fields: Vec<(&str, &str)>,
    ) {
        let header_id = BlockId(Digest32::from(
            <[u8; 32]>::try_from(hex::decode(header_id_hex).unwrap()).unwrap(),
        ));
        let expected_root =
            Digest32::from(<[u8; 32]>::try_from(hex::decode(expected_hash_hex).unwrap()).unwrap());

        let ext_fields: Vec<ExtensionField> = fields
            .into_iter()
            .map(|(key_hex, value_hex)| {
                let key_bytes = hex::decode(key_hex).unwrap();
                assert_eq!(key_bytes.len(), 2, "expected 2-byte key, got {}", key_hex);
                ExtensionField::new(
                    [key_bytes[0], key_bytes[1]],
                    hex::decode(value_hex).unwrap(),
                )
            })
            .collect();

        let ext = Extension::new(header_id, ext_fields);
        let computed = ext.compute_digest();

        assert_eq!(computed, expected_root, "Merkle root mismatch");
    }

    /// Height 1 (genesis): empty extension
    #[test]
    fn test_golden_mainnet_height_1() {
        let header_id = BlockId(Digest32::from(
            <[u8; 32]>::try_from(
                hex::decode("b0244dfc267baca974a4caee06120321562784303a8a688976ae56170e4d175b")
                    .unwrap(),
            )
            .unwrap(),
        ));

        let ext = Extension::empty(header_id);
        let computed = ext.compute_digest();

        assert_eq!(
            computed.as_ref(),
            &EMPTY_MERKLE_ROOT,
            "Empty extension should match EMPTY_MERKLE_ROOT"
        );
    }

    /// Height 100: 4 interlink fields
    #[test]
    fn test_golden_mainnet_height_100() {
        check_extension_digest(
            "6ba802b17c9598a15c8da1736e975e34143e93d799f5d2a9bc408bd2b3f19a1f",
            "e50f879c7bfb17d9049b61f784473b44b39c1c2dbc97a1b5ebe9956275050582",
            vec![
                (
                    "0100",
                    "01b0244dfc267baca974a4caee06120321562784303a8a688976ae56170e4d175b",
                ),
                (
                    "0101",
                    "01855fc5c9eed868b43ea2c3df99ec17dd9d903187d891e2365a89b98125c994b2",
                ),
                (
                    "0102",
                    "0135bda2041b8ec2b429b8d1f86e5ec9ebdd03aa982635effd1693637a89d6fd8b",
                ),
                (
                    "0103",
                    "0399cca27ca09df4d90ca22b14277b94983d0f7da221e2cef55a6ad19e70c185f0",
                ),
            ],
        );
    }

    /// Height 1000: 5 interlink fields
    #[test]
    fn test_golden_mainnet_height_1000() {
        check_extension_digest(
            "80e4ce0c3337b62e7e1d98e0ee0131bbfb1b6472b543b1604caf9af331aa4e8d",
            "face3cc2af16d2353a943fea15a2560f7c9fc712b1ffcaac66bceb9796badd84",
            vec![
                (
                    "0100",
                    "01b0244dfc267baca974a4caee06120321562784303a8a688976ae56170e4d175b",
                ),
                (
                    "0101",
                    "028915208f1ea6fc521dc6c41160b73feba6b3e4e05850efe122032b0e3e220bd4",
                ),
                (
                    "0103",
                    "01db39f7e5927319f0c7e3fc82de2fd0d6254204d355aab2d0142ad28a5f6b4a49",
                ),
                (
                    "0104",
                    "01afbe8123831290472a3c9e298ac1c5ae991d52edb13ca07921a3d72749213b6e",
                ),
                (
                    "0105",
                    "064077fcf3359c15c3ad3797a78fff342166f09a7f1b22891a18030dcd8604b087",
                ),
            ],
        );
    }

    /// Height 10000: 10 interlink fields
    #[test]
    fn test_golden_mainnet_height_10000() {
        check_extension_digest(
            "d89ccbe0a67a779953eb9d4e55f2734d1953d870ed0ce923ea6cd9f27fb2cf05",
            "fc4db80fe3385601d1930e97af43778b1bc801d5279e7f31f6258a47b8816abb",
            vec![
                (
                    "0100",
                    "01b0244dfc267baca974a4caee06120321562784303a8a688976ae56170e4d175b",
                ),
                (
                    "0101",
                    "01ac8640777e90a8d7730582ee45f843826d17a50d1c50def8a859c0c8b2c30261",
                ),
                (
                    "0102",
                    "01aa31186639077b22185125cf2af382e4f45cf0ff8567728a9754361ae065da8c",
                ),
                (
                    "0103",
                    "02b352668721c07cef942e7cce58982fd154c313f8a0e8a0a873e7c3d4640bc5ff",
                ),
                (
                    "0105",
                    "0260ca4c8c381dd75c877dc19e4bba94faa538379473d555eaeef4979509860baf",
                ),
                (
                    "0107",
                    "0134f94672ae102071ca1b0a5a280b12716b940a147267ee68844ef2d47e4aed1e",
                ),
                (
                    "0108",
                    "03c29bf56a0073108fb567d34993444b6eaf4c97b2d205f501107690ae4b31adc8",
                ),
                (
                    "010b",
                    "01762ed118ba19ed1a0e7947982caf19db039dac4e88375ba13b908efe8d68ec68",
                ),
                (
                    "010c",
                    "017c81e55fd842c65304dd98e37f1bc29456e5f889829d985d4a89bd51b549fd3a",
                ),
                (
                    "010d",
                    "01d95cd7bdd56bf1e9008ffa2e991cd03c615af27968e871195e6d5978262c4a41",
                ),
            ],
        );
    }

    /// Height 100000: 7 interlink fields
    #[test]
    fn test_golden_mainnet_height_100000() {
        check_extension_digest(
            "9b61e8564436969ff3beeb7afbfcfea9794116cffc28f0cfd04dbaccae2df0d7",
            "700fe9fc286d070e3023a7436dc77e9562d88fe95eb5a9854530968d75a43ced",
            vec![
                (
                    "0100",
                    "01b0244dfc267baca974a4caee06120321562784303a8a688976ae56170e4d175b",
                ),
                (
                    "0101",
                    "05557fd0590616b4f6e51eaf54436d61e5585eebfc5a9e860861fc0876064bd3d9",
                ),
                (
                    "0106",
                    "08bfbba05dcf8c38a63e5a6ccc0783f23919d6aa9a987c39f9c8be9f5bb6eeb86b",
                ),
                (
                    "010e",
                    "01358ffda65c6b8a3adc2dff66e0e31039e4440f340f05f16f9d07c92b94e33ed4",
                ),
                (
                    "010f",
                    "02adc5f636843a6bd529d20e4668067465b8a66a2c14fa218e9c03b4ea2444c297",
                ),
                (
                    "0111",
                    "02d25eec3d4548ca4fdcdffc0ef23677d2d8d3fd4f0bf89570884cf02002d16301",
                ),
                (
                    "0113",
                    "0132f4a68f2eb9a4ff2b2c2626c1b4bd40670c607ef6be8cf4c26a85614e25184b",
                ),
            ],
        );
    }

    /// Height 500000: 9 interlink fields
    #[test]
    fn test_golden_mainnet_height_500000() {
        check_extension_digest(
            "0261b8bbe791aa26379c679e22359d21a92bda09abd369b938946d0128eed660",
            "e707ca7f23c5aae204a6efee62a80da649f5bb65ebfdc4bbd8c448108286a595",
            vec![
                (
                    "0100",
                    "01b0244dfc267baca974a4caee06120321562784303a8a688976ae56170e4d175b",
                ),
                (
                    "0101",
                    "01557fd0590616b4f6e51eaf54436d61e5585eebfc5a9e860861fc0876064bd3d9",
                ),
                (
                    "0102",
                    "04296e2707cf72b6a2c71e4966028d8786c7f5425850e9609757ce8b3713f548fe",
                ),
                (
                    "0106",
                    "01bca7a5ea56428e96962c8ec4149ff3f84486b7fde8f5f910644f35f12336d968",
                ),
                (
                    "0107",
                    "0144a3a530e8d34c88b53e82717c6f8f59b3c235718959f47c4cd520722281918c",
                ),
                (
                    "0108",
                    "04bdc4cf819e5a39a09f92c33d3aafde9d14594368e17bbb1499ebce9ec1f26434",
                ),
                (
                    "010c",
                    "04dd1e32fbae42094450b1be567bac803111d47f4a1fcf695569615b8a73c00560",
                ),
                (
                    "0110",
                    "02229a0653a86e9397c2cd7a5d87e742710eb3fcbe10e32b2ec56145b0514fa919",
                ),
                (
                    "0112",
                    "028345f2a105762203d6f8fc547861dc9aefd6477b333f7baa1aace61ddc3ceafb",
                ),
            ],
        );
    }

    /// Height 1000000: 12 interlink fields
    #[test]
    fn test_golden_mainnet_height_1000000() {
        check_extension_digest(
            "f04085962f306d8e4ee9f1a415abb82c32ac6a183d4d286e995569c978a7c5cb",
            "367ea050a780669223476b7c43a1a5d07b8b0e7dfb508ac10ac93814d4bcd78f",
            vec![
                (
                    "0100",
                    "01b0244dfc267baca974a4caee06120321562784303a8a688976ae56170e4d175b",
                ),
                (
                    "0101",
                    "031155d54de65f0130fae142aa4cf5a7728b7c30f5939d33fddf077e2008040a15",
                ),
                (
                    "0104",
                    "01b845aea6b6c84a44d5b744f92cdf85ae9a56567180c6ae723381d84ea8519055",
                ),
                (
                    "0105",
                    "01bc3b3aa2bd908c858f304a7b2e60f4d63a46914b202c945fe8a561097bf42f3e",
                ),
                (
                    "0106",
                    "024d84dcce00d11890ea160665dd318bce2123e775d1467f22287734606345a3cb",
                ),
                (
                    "0108",
                    "027d90b26fb36632a88168029f9e05f6e45359eb8beea967c2a0bbbd158f6f8bf5",
                ),
                (
                    "010a",
                    "01d009c93a6a8a9c5426ae6c7e5fdd9df20fb7a7e705f7a355f5ce9cf7d8f5fd08",
                ),
                (
                    "010b",
                    "023505532f829490cbf058aeb5b72e3e615e942e87cad64713a3538f4c11c7085e",
                ),
                (
                    "010d",
                    "016155b15e75b047b88e56519778156195b4b5c4db7de0bc2da37f4227e462105c",
                ),
                (
                    "010e",
                    "0120c05b3c20ea1df2a3e6bc89398a760901732a76f3349c48abed8b346f2a24ea",
                ),
                (
                    "010f",
                    "0287987ca7bc4c44e0b8416e9171b7ac1dbdb4294b1565a132cccf15d04690b405",
                ),
                (
                    "0111",
                    "04e0fec8999767561145b62d55dbc4f36cefa4cabda986769dbc41746580d58bca",
                ),
            ],
        );
    }

    /// Height 1500000: 10 interlink fields
    #[test]
    fn test_golden_mainnet_height_1500000() {
        check_extension_digest(
            "f5f148ba4e16fb0ede5d026536f2a29c62b695055762659d6d5ab18126aae6bc",
            "1c0daccd1759bfd56f899e4b6c624acd00a9ce7e2c8957a5ff9deb619485ee05",
            vec![
                (
                    "0100",
                    "01b0244dfc267baca974a4caee06120321562784303a8a688976ae56170e4d175b",
                ),
                (
                    "0101",
                    "031155d54de65f0130fae142aa4cf5a7728b7c30f5939d33fddf077e2008040a15",
                ),
                (
                    "0104",
                    "022e9e18180392c91ace964f24aff201511e891eddeed7c8049cfbe6dc0db6e9e1",
                ),
                (
                    "0106",
                    "025aa5ffce026d31f1ce6c410e5fdc2642c6467b94e9c3c4893056741cf81b1d96",
                ),
                (
                    "0108",
                    "01505afd8aae3aed665b41a96a1b204daefb9a45a7c37d77c722ec038c6c036db3",
                ),
                (
                    "0109",
                    "031d9d08fca843e5ba2eba0b5694ddc889576b3d845b2965573fb0b8b495299739",
                ),
                (
                    "010c",
                    "02df5de713b228b496be234356fa32e7f9d4f2e3b07bebbb21e4f052f2f41382f6",
                ),
                (
                    "010e",
                    "031773ce43367a6730c65411dd31f0bdfa046c3bf8dac329d50bfb07dafa5b22b2",
                ),
                (
                    "0111",
                    "0166824450c0052b7d01d13d744299df72aea39ad299cd477166763097aa4abeed",
                ),
                (
                    "0112",
                    "03e8ff7ce576a6172a0c957a14d7a1e850228a7c44a42befc1f7a229507c0fa0f2",
                ),
            ],
        );
    }

    // ========================================================================
    // MODIFIER ID VERIFICATION
    // ========================================================================
    //
    // extensionId = blake2b256(modifier_type_id || header_id || extensionHash)
    // where modifier_type_id = 108 (0x6c) for Extension.
    //
    // This verifies the modifier ID formula is correct, independent of serialization.

    /// Extension modifier type ID (from Ergo protocol).
    const EXTENSION_MODIFIER_TYPE: u8 = 108;

    /// Verify extensionId = blake2b256(0x6c || headerId || extensionHash)
    #[test]
    fn test_extension_modifier_id_formula() {
        // Height 100 fixture
        let header_id_hex = "6ba802b17c9598a15c8da1736e975e34143e93d799f5d2a9bc408bd2b3f19a1f";
        let extension_hash_hex = "e50f879c7bfb17d9049b61f784473b44b39c1c2dbc97a1b5ebe9956275050582";
        let expected_extension_id_hex =
            "40a304a3ecc0f64004bcb5f4f4ae288ecd603e83b7625ed860d429bbaa1dae84";

        let mut preimage = Vec::with_capacity(1 + 32 + 32);
        preimage.push(EXTENSION_MODIFIER_TYPE);
        preimage.extend_from_slice(&hex::decode(header_id_hex).unwrap());
        preimage.extend_from_slice(&hex::decode(extension_hash_hex).unwrap());

        let computed_id = blake2b256_hash(&preimage);
        assert_eq!(
            hex::encode(computed_id.as_ref()),
            expected_extension_id_hex,
            "extensionId formula mismatch"
        );
    }

    // ========================================================================
    // SERIALIZATION ROUNDTRIP TEST
    // ========================================================================
    //
    // Verifies parse() and serialize() are inverses. Combined with the golden
    // Merkle root tests, this gives confidence in the serialization format.
    //
    // For byte-identical serialization proof, raw modifier bytes from Scala
    // node storage would be needed (not available via REST API).

    /// Parse/serialize roundtrip - verify parser and serializer are inverses.
    #[test]
    fn test_parse_serialize_roundtrip() {
        let header_id_hex = "80e4ce0c3337b62e7e1d98e0ee0131bbfb1b6472b543b1604caf9af331aa4e8d";
        let fields_data = vec![
            (
                "0100",
                "01b0244dfc267baca974a4caee06120321562784303a8a688976ae56170e4d175b",
            ),
            (
                "0101",
                "028915208f1ea6fc521dc6c41160b73feba6b3e4e05850efe122032b0e3e220bd4",
            ),
            (
                "0103",
                "01db39f7e5927319f0c7e3fc82de2fd0d6254204d355aab2d0142ad28a5f6b4a49",
            ),
        ];

        // Build Extension and serialize
        let header_id = BlockId(Digest32::from(
            <[u8; 32]>::try_from(hex::decode(header_id_hex).unwrap()).unwrap(),
        ));
        let fields: Vec<ExtensionField> = fields_data
            .iter()
            .map(|(key_hex, value_hex)| {
                let key_bytes = hex::decode(key_hex).unwrap();
                ExtensionField::new(
                    [key_bytes[0], key_bytes[1]],
                    hex::decode(value_hex).unwrap(),
                )
            })
            .collect();
        let ext = Extension::new(header_id, fields);
        let bytes = ext.serialize();

        // Parse back
        let parsed = Extension::parse(&bytes).expect("parse failed");
        assert_eq!(hex::encode(parsed.header_id.0.as_ref()), header_id_hex);
        assert_eq!(parsed.fields.len(), fields_data.len());

        // Re-serialize and verify identical
        let reserialized = parsed.serialize();
        assert_eq!(
            hex::encode(&reserialized),
            hex::encode(&bytes),
            "roundtrip produced different bytes"
        );
    }
}
