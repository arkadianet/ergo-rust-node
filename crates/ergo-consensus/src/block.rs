//! Block types for the Ergo blockchain.
//!
//! This module re-exports and extends block types from sigma-rust
//! for use in the node implementation.

use ergo_lib::ergotree_ir::serialization::SigmaSerializable;
use serde::{Deserialize, Serialize};

// Re-export core types from sigma-rust
pub use ergo_chain_types::{
    ADDigest, BlockId, Digest32, EcPoint, ExtensionCandidate, Header, PreHeader, Votes,
};
pub use ergo_lib::chain::transaction::Transaction;
pub use ergo_lib::ergotree_ir::chain::ergo_box::{BoxId, ErgoBox, ErgoBoxCandidate};
pub use ergo_lib::ergotree_ir::chain::token::{Token, TokenAmount, TokenId};

use ergo_chain_types::AutolykosSolution;

/// Create a minimal "genesis parent" header for use when validating the genesis block (height 1).
/// This header has height 0 and all-zero IDs, serving as the virtual parent of the genesis block.
pub fn genesis_parent_header() -> Header {
    let zero_digest = Digest32::zero();
    let zero_block_id = BlockId(zero_digest);

    // Create minimal AutolykosSolution with default values
    // Use the identity (infinity) point as a placeholder miner_pk
    let autolykos_solution = AutolykosSolution {
        miner_pk: Box::new(ergo_chain_types::ec_point::identity()),
        pow_onetime_pk: None,
        nonce: vec![0u8; 8],
        pow_distance: None,
    };

    Header {
        version: 1,
        id: zero_block_id.clone(),
        parent_id: zero_block_id.clone(),
        ad_proofs_root: zero_digest,
        state_root: ADDigest::zero(),
        transaction_root: zero_digest,
        timestamp: 0,
        n_bits: 0,
        height: 0,
        extension_root: zero_digest,
        autolykos_solution,
        votes: Votes([0u8; 3]),
        unparsed_bytes: Box::new([]),
    }
}

/// Full block containing header, transactions, extension, and AD proofs.
#[derive(Debug, Clone)]
pub struct FullBlock {
    /// Block header.
    pub header: Header,
    /// Block transactions.
    pub transactions: BlockTransactions,
    /// Block extension (key-value pairs).
    pub extension: Extension,
    /// Authenticated data proofs (optional for UTXO state nodes).
    pub ad_proofs: Option<ADProofs>,
}

impl FullBlock {
    /// Create a new full block.
    pub fn new(
        header: Header,
        transactions: BlockTransactions,
        extension: Extension,
        ad_proofs: Option<ADProofs>,
    ) -> Self {
        Self {
            header,
            transactions,
            extension,
            ad_proofs,
        }
    }

    /// Get the block ID.
    pub fn id(&self) -> BlockId {
        self.header.id.clone()
    }

    /// Get the block height.
    pub fn height(&self) -> u32 {
        self.header.height
    }

    /// Get the parent block ID.
    pub fn parent_id(&self) -> BlockId {
        self.header.parent_id.clone()
    }

    /// Check if this block has AD proofs.
    pub fn has_ad_proofs(&self) -> bool {
        self.ad_proofs.is_some()
    }
}

/// Block transactions section.
#[derive(Debug, Clone)]
pub struct BlockTransactions {
    /// Header ID this section belongs to.
    pub header_id: BlockId,
    /// List of transactions in the block.
    pub txs: Vec<Transaction>,
    /// Block version (for serialization).
    pub block_version: u8,
}

/// Maximum number of transactions in a block.
const MAX_TRANSACTIONS_IN_BLOCK: u64 = 10_000_000;

impl BlockTransactions {
    /// Create a new block transactions section.
    pub fn new(header_id: BlockId, txs: Vec<Transaction>) -> Self {
        Self {
            header_id,
            txs,
            block_version: 2,
        }
    }

    /// Create with specific block version.
    pub fn with_version(header_id: BlockId, txs: Vec<Transaction>, block_version: u8) -> Self {
        Self {
            header_id,
            txs,
            block_version,
        }
    }

    /// Get the number of transactions.
    pub fn len(&self) -> usize {
        self.txs.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.txs.is_empty()
    }

    /// Iterate over transactions.
    pub fn iter(&self) -> impl Iterator<Item = &Transaction> {
        self.txs.iter()
    }

    /// Get the coinbase transaction (first transaction).
    pub fn coinbase(&self) -> Option<&Transaction> {
        self.txs.first()
    }

    /// Parse BlockTransactions from binary data.
    /// Format:
    /// - 32 bytes: header ID
    /// - UInt: either tx_count (if < MAX) or MAX + block_version (if >= MAX)
    /// - If previous was >= MAX, another UInt for tx_count
    /// - Transactions serialized sequentially
    pub fn parse(data: &[u8]) -> Result<Self, BlockTransactionsParseError> {
        use sigma_ser::vlq_encode::ReadSigmaVlqExt;
        use std::io::Cursor;

        if data.len() < 32 {
            return Err(BlockTransactionsParseError::TooShort);
        }

        // Parse header ID (32 bytes)
        let mut header_bytes = [0u8; 32];
        header_bytes.copy_from_slice(&data[0..32]);
        let header_id = BlockId(Digest32::from(header_bytes));

        let mut cursor = Cursor::new(&data[32..]);

        // Read first UInt - either tx count or marker for new format
        let first_uint = cursor.get_u32().map_err(|_| {
            BlockTransactionsParseError::InvalidFormat("Failed to read first uint".into())
        })? as u64;

        let (block_version, tx_count) = if first_uint > MAX_TRANSACTIONS_IN_BLOCK {
            // New format: first_uint = MAX + block_version
            let version = (first_uint - MAX_TRANSACTIONS_IN_BLOCK) as u8;
            let count = cursor.get_u32().map_err(|_| {
                BlockTransactionsParseError::InvalidFormat("Failed to read tx count".into())
            })? as usize;
            (version, count)
        } else {
            // Old format: first_uint is tx count, version = 1
            (1u8, first_uint as usize)
        };

        // Parse transactions - we need to read them one at a time
        // Since transactions are variable length, we read remaining bytes and parse sequentially
        let mut txs = Vec::with_capacity(tx_count);
        let start_pos = 32 + cursor.position() as usize;
        let mut remaining = &data[start_pos..];

        for i in 0..tx_count {
            // sigma_parse_bytes consumes the exact bytes needed for one transaction
            // We need to track how many bytes each transaction consumed
            let tx = Transaction::sigma_parse_bytes(remaining).map_err(|e| {
                BlockTransactionsParseError::TransactionParse(format!("tx {}: {:?}", i, e))
            })?;

            // Get the serialized size to advance the slice
            let tx_bytes = tx.sigma_serialize_bytes().map_err(|e| {
                BlockTransactionsParseError::TransactionParse(format!("tx {} size: {:?}", i, e))
            })?;
            remaining = &remaining[tx_bytes.len()..];

            txs.push(tx);
        }

        Ok(Self {
            header_id,
            txs,
            block_version,
        })
    }

    /// Compute the merkle root of the transactions.
    /// This uses the same algorithm as the Scala node to produce the transactions root hash.
    ///
    /// For block version 1: Merkle tree over transaction IDs only
    /// For block version > 1: Merkle tree over (transaction IDs ++ witness IDs)
    ///
    /// Transaction ID = blake2b256(bytes_to_sign) - already computed by ergo-lib as tx.id()
    /// Witness ID = blake2b256(concatenated spending proofs).tail (31 bytes, first byte removed)
    pub fn compute_merkle_root(&self) -> Digest32 {
        use ergo_chain_types::blake2b256_hash;
        use ergo_merkle_tree::{MerkleNode, MerkleTree};

        if self.txs.is_empty() {
            // Special case for empty merkle tree - matches Scala's emptyMerkleTreeRoot
            return Digest32::from(blake2b256_hash(&[]));
        }

        // Collect transaction IDs (32 bytes each)
        let tx_ids: Vec<Vec<u8>> = self
            .txs
            .iter()
            .map(|tx| tx.id().0.as_ref().to_vec())
            .collect();

        // For block version 1, only use transaction IDs
        // For version > 1, also include witness IDs
        let leaves: Vec<MerkleNode> = if self.block_version == 1 {
            tx_ids.into_iter().map(MerkleNode::from_bytes).collect()
        } else {
            // Compute witness IDs for each transaction
            // witnessSerializedId = hash(concat of all spending proofs).tail (31 bytes)
            let witness_ids: Vec<Vec<u8>> = self
                .txs
                .iter()
                .map(|tx| {
                    // Concatenate all spending proofs
                    let mut all_proofs = Vec::new();
                    for input in tx.inputs.iter() {
                        all_proofs.extend_from_slice(input.spending_proof.proof.as_ref());
                    }
                    // Hash and take tail (remove first byte) to get 31-byte witness ID
                    let hash = blake2b256_hash(&all_proofs);
                    hash.as_ref()[1..].to_vec() // .tail removes first byte
                })
                .collect();

            // Combine tx_ids and witness_ids
            tx_ids
                .into_iter()
                .chain(witness_ids)
                .map(MerkleNode::from_bytes)
                .collect()
        };

        if leaves.is_empty() {
            return Digest32::from(blake2b256_hash(&[]));
        }

        let tree = MerkleTree::new(leaves);
        tree.root_hash()
    }

    /// Validate that this BlockTransactions matches the given header.
    /// Checks:
    /// 1. The header_id matches the header's ID
    /// 2. The computed merkle root matches the header's transactions_root
    pub fn validate_against_header(&self, header: &Header) -> Result<(), BlockValidationError> {
        // Check header ID matches
        if self.header_id != header.id {
            return Err(BlockValidationError::HeaderIdMismatch {
                expected: hex::encode(header.id.0.as_ref()),
                got: hex::encode(self.header_id.0.as_ref()),
            });
        }

        // Compute merkle root and compare
        let computed_root = self.compute_merkle_root();
        let header_root = &header.transaction_root;

        if computed_root.as_ref() != header_root.0.as_ref() {
            return Err(BlockValidationError::TransactionsRootMismatch {
                expected: hex::encode(header_root.0.as_ref()),
                computed: hex::encode(computed_root.as_ref()),
            });
        }

        Ok(())
    }

    /// Extract all input box IDs from the transactions (boxes being spent).
    pub fn get_input_box_ids(&self) -> Vec<BoxId> {
        self.txs
            .iter()
            .flat_map(|tx| tx.inputs.iter().map(|input| input.box_id.clone()))
            .collect()
    }

    /// Extract all output boxes from the transactions (boxes being created).
    /// Returns tuples of (ErgoBox, tx_id, output_index).
    pub fn get_outputs(&self) -> Vec<(ErgoBox, Vec<u8>, u16)> {
        self.txs
            .iter()
            .flat_map(|tx| {
                let tx_id: Vec<u8> = tx.id().as_ref().to_vec();
                tx.outputs
                    .iter()
                    .enumerate()
                    .map(move |(idx, output)| (output.clone(), tx_id.clone(), idx as u16))
            })
            .collect()
    }

    /// Get all transaction IDs in this block.
    pub fn get_tx_ids(&self) -> Vec<Vec<u8>> {
        self.txs
            .iter()
            .map(|tx| tx.id().as_ref().to_vec())
            .collect()
    }
}

/// Errors that can occur during block validation.
#[derive(Debug, Clone)]
pub enum BlockValidationError {
    /// Header ID doesn't match.
    HeaderIdMismatch { expected: String, got: String },
    /// Transactions merkle root doesn't match header.
    TransactionsRootMismatch { expected: String, computed: String },
    /// Block has no transactions.
    EmptyBlock,
    /// Invalid coinbase transaction.
    InvalidCoinbase(String),
}

impl std::fmt::Display for BlockValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HeaderIdMismatch { expected, got } => {
                write!(f, "Header ID mismatch: expected {}, got {}", expected, got)
            }
            Self::TransactionsRootMismatch { expected, computed } => {
                write!(
                    f,
                    "Transactions root mismatch: header has {}, computed {}",
                    expected, computed
                )
            }
            Self::EmptyBlock => write!(f, "Block has no transactions"),
            Self::InvalidCoinbase(msg) => write!(f, "Invalid coinbase: {}", msg),
        }
    }
}

impl std::error::Error for BlockValidationError {}

/// Errors that can occur when parsing BlockTransactions.
#[derive(Debug, Clone)]
pub enum BlockTransactionsParseError {
    /// Data too short.
    TooShort,
    /// Invalid format.
    InvalidFormat(String),
    /// Transaction parse error.
    TransactionParse(String),
}

/// Block extension containing key-value pairs.
#[derive(Debug, Clone)]
pub struct Extension {
    /// Header ID this extension belongs to.
    pub header_id: BlockId,
    /// Key-value fields (key is 2 bytes, value is up to 64 bytes).
    pub fields: Vec<ExtensionField>,
}

/// A single extension field.
#[derive(Debug, Clone)]
pub struct ExtensionField {
    /// Field key (2 bytes).
    pub key: [u8; 2],
    /// Field value (up to 64 bytes).
    pub value: Vec<u8>,
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
    pub fn get(&self, key: &[u8; 2]) -> Option<&[u8]> {
        self.fields
            .iter()
            .find(|f| &f.key == key)
            .map(|f| f.value.as_slice())
    }

    /// Get interlinks (for NiPoPoW).
    pub fn interlinks(&self) -> Option<Vec<BlockId>> {
        // Interlinks are stored with key prefix 0x00
        // This is a simplified implementation
        None // TODO: Implement interlinks parsing
    }

    /// Get system parameters from extension.
    pub fn parameters(&self) -> Vec<(u8, u64)> {
        // Parameters are stored with specific key prefixes
        // This is a simplified implementation
        Vec::new() // TODO: Implement parameter parsing
    }
}

/// Authenticated data structure proofs.
#[derive(Debug, Clone)]
pub struct ADProofs {
    /// Header ID these proofs belong to.
    pub header_id: BlockId,
    /// Serialized proofs bytes.
    pub proof_bytes: Vec<u8>,
}

impl ADProofs {
    /// Create new AD proofs.
    pub fn new(header_id: BlockId, proof_bytes: Vec<u8>) -> Self {
        Self {
            header_id,
            proof_bytes,
        }
    }

    /// Get the size of the proofs.
    pub fn size(&self) -> usize {
        self.proof_bytes.len()
    }
}

/// Modifier types in Ergo protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum ModifierType {
    /// Unconfirmed transaction (mempool).
    Transaction = 2,
    /// Block header.
    Header = 101,
    /// Block transactions.
    BlockTransactions = 102,
    /// Block extension.
    Extension = 108,
    /// AD proofs.
    ADProofs = 104,
    /// Full block (virtual type for sync).
    FullBlock = 100,
}

impl ModifierType {
    /// Create from byte value.
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            2 => Some(Self::Transaction),
            101 => Some(Self::Header),
            102 => Some(Self::BlockTransactions),
            108 => Some(Self::Extension),
            104 => Some(Self::ADProofs),
            100 => Some(Self::FullBlock),
            _ => None,
        }
    }

    /// Convert to byte value.
    pub fn to_byte(self) -> u8 {
        self as u8
    }
}

/// Block section identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModifierId {
    /// Modifier type.
    pub modifier_type: ModifierType,
    /// Modifier ID (32 bytes).
    pub id: Digest32,
}

impl ModifierId {
    /// Create a new modifier ID.
    pub fn new(modifier_type: ModifierType, id: Digest32) -> Self {
        Self { modifier_type, id }
    }

    /// Create a header modifier ID.
    pub fn header(id: BlockId) -> Self {
        Self {
            modifier_type: ModifierType::Header,
            id: id.0,
        }
    }

    /// Create a block transactions modifier ID.
    pub fn block_transactions(id: BlockId) -> Self {
        Self {
            modifier_type: ModifierType::BlockTransactions,
            id: id.0,
        }
    }
}

/// Block processing status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockStatus {
    /// Block is unknown.
    Unknown,
    /// Block header is valid and stored.
    HeaderValid,
    /// Block header is invalid.
    HeaderInvalid,
    /// Block body (transactions) are being validated.
    BodyValidating,
    /// Block is fully valid.
    Valid,
    /// Block is invalid.
    Invalid,
}

/// Information about a block in the chain.
#[derive(Debug, Clone)]
pub struct BlockInfo {
    /// Block ID.
    pub id: BlockId,
    /// Block height.
    pub height: u32,
    /// Block timestamp.
    pub timestamp: u64,
    /// Block difficulty.
    pub difficulty: u128,
    /// Block status.
    pub status: BlockStatus,
    /// Whether the block is on the main chain.
    pub is_main_chain: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_modifier_type_conversion() {
        assert_eq!(ModifierType::from_byte(101), Some(ModifierType::Header));
        assert_eq!(
            ModifierType::from_byte(102),
            Some(ModifierType::BlockTransactions)
        );
        assert_eq!(ModifierType::from_byte(2), Some(ModifierType::Transaction));
        assert_eq!(ModifierType::from_byte(255), None);

        assert_eq!(ModifierType::Header.to_byte(), 101);
        assert_eq!(ModifierType::Transaction.to_byte(), 2);
    }

    /// Test vectors from Scala node's ErgoTransactionSpec.scala
    /// These ensure byte-for-byte serialization compatibility with the Scala node.
    ///
    /// Source: ergo/ergo-core/src/test/scala/org/ergoplatform/modifiers/mempool/ErgoTransactionSpec.scala
    mod scala_serialization_compat {
        use ergo_lib::chain::transaction::Transaction;
        use ergo_lib::ergotree_ir::serialization::SigmaSerializable;

        fn hex_to_bytes(hex: &str) -> Vec<u8> {
            (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect()
        }

        /// Simple transfer transaction with 2 inputs and 2 outputs.
        /// From Scala test: "serialization vector" property test, first transaction.
        #[test]
        fn test_simple_transfer_tx() {
            // Transaction bytes from Scala ErgoTransactionSpec
            let bytes_str = "02c95c2ccf55e03cac6659f71ca4df832d28e2375569cec178dcb17f3e2e5f774238b4a04b4201da0578be3dac11067b567a73831f35b024a2e623c1f8da230407f63bab62c62ed9b93808b106b5a7e8b1751fa656f4c5de467400ca796a4fc9c0d746a69702a77bd78b1a80a5ef5bf5713bbd95d93a4f23b27ead385aea4d78a234c35accacdf8996b0af5b51e26fee29ea5c05468f23707d31c0df39400127391cd57a70eb856710db48bb9833606e0bf90340000000028094ebdc030008cd0326df75ea615c18acc6bb4b517ac82795872f388d5d180aac90eaa84de750b942e8070000c0843d1005040004000e36100204cf0f08cd0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798ea02d192a39a8cc7a701730073011001020402d19683030193a38cc7b2a57300000193c2b2a57301007473027303830108cdeeac93b1a57304e8070000";
            let expected_tx_id = "b59ca51f7470f291acc32e84870d00c4fda8b773f38f757f3d65d45265c13da5";

            let bytes = hex_to_bytes(bytes_str);
            let tx = Transaction::sigma_parse_bytes(&bytes).expect("Failed to parse transaction");

            // Verify transaction ID matches Scala
            let tx_id_hex = hex::encode(tx.id().0);
            assert_eq!(
                tx_id_hex, expected_tx_id,
                "Transaction ID mismatch - serialization incompatible with Scala node"
            );

            // Verify we have expected structure
            assert_eq!(tx.inputs.len(), 2, "Expected 2 inputs");
            assert_eq!(tx.outputs.len(), 2, "Expected 2 outputs");
            assert!(tx.data_inputs.is_none(), "Expected 0 data inputs");

            // Verify first input box ID
            let first_input_id = hex::encode(tx.inputs.first().box_id.as_ref());
            assert_eq!(
                first_input_id,
                "c95c2ccf55e03cac6659f71ca4df832d28e2375569cec178dcb17f3e2e5f7742"
            );

            // Verify re-serialization produces identical bytes
            let reserialized = tx.sigma_serialize_bytes().expect("Failed to serialize");
            assert_eq!(
                reserialized, bytes,
                "Re-serialization produced different bytes - incompatible with Scala node"
            );
        }

        /// Transaction with registers in outputs (R4-R8).
        /// From Scala test: tx2 in "serialization vector" property.
        #[test]
        fn test_tx_with_registers() {
            let bytes_str = "02c95c2ccf55e03cac6659f71ca4df832d28e2375569cec178dcb17f3e2e5f774238b4a04b4201da0578be3dac11067b567a73831f35b024a2e623c1f8da230407f63bab62c62ed9b93808b106b5a7e8b1751fa656f4c5de467400ca796a4fc9c0d746a69702a77bd78b1a80a5ef5bf5713bbd95d93a4f23b27ead385aea4d78a234c35accacdf8996b0af5b51e26fee29ea5c05468f23707d31c0df39400127391cd57a70eb856710db48bb9833606e0bf90340000000028094ebdc030008cd0326df75ea615c18acc6bb4b517ac82795872f388d5d180aac90eaa84de750b942e8070005020108cd0326df75ea615c18acc6bb4b517ac82795872f388d5d180aac90eaa84de750b94204141103020496d396010e211234561234561234561234561234561234561234561234561234561234561234568094ebdc030008cd0326df75ea615c18acc6bb4b517ac82795872f388d5d180aac90eaa84de750b942e8070000";
            let expected_tx_id = "bd04a93f67fda77d89afc38cd8237f142ad5a349405929fd1f7b7f24c4ea2e80";

            let bytes = hex_to_bytes(bytes_str);
            let tx = Transaction::sigma_parse_bytes(&bytes).expect("Failed to parse transaction");

            let tx_id_hex = hex::encode(tx.id().0);
            assert_eq!(
                tx_id_hex, expected_tx_id,
                "Transaction ID mismatch for tx with registers"
            );

            // Verify structure
            assert_eq!(tx.inputs.len(), 2);
            assert_eq!(tx.outputs.len(), 2);

            // Verify re-serialization
            let reserialized = tx.sigma_serialize_bytes().expect("Failed to serialize");
            assert_eq!(
                reserialized, bytes,
                "Re-serialization mismatch for tx with registers"
            );
        }

        /// Transaction with data inputs and tokens.
        /// From Scala test: third check() call in "serialization vector" property.
        #[test]
        fn test_tx_with_data_inputs_and_tokens() {
            let bytes_str = "02e76bf387ab2e63ba8f4e23267bc88265b5fee4950030199e2e2c214334251c6400002e9798d7eb0cd867f6dc29872f80de64c04cef10a99a58d007ef7855f0acbdb9000001f97d1dc4626de22db836270fe1aa004b99970791e4557de8f486f6d433b81195026df03fffc9042bf0edb0d0d36d7a675239b83a9080d39716b9aa0a64cccb9963e76bf387ab2e63ba8f4e23267bc88265b5fee4950030199e2e2c214334251c6403da92a8b8e3ad770008cd02db0ce4d301d6dc0b7a5fbe749588ef4ef68f2c94435020a3c31764ffd36a2176000200daa4eb6b01aec8d1ff0100da92a8b8e3ad770008cd02db0ce4d301d6dc0b7a5fbe749588ef4ef68f2c94435020a3c31764ffd36a2176000200daa4eb6b01aec8d1ff0100fa979af8988ce7010008cd02db0ce4d301d6dc0b7a5fbe749588ef4ef68f2c94435020a3c31764ffd36a2176000000";
            let expected_tx_id = "663ae91ab7145a4f42b5509e1a2fb0469b7cb46ea87fdfd90e0b4c8ef29c2493";

            let bytes = hex_to_bytes(bytes_str);
            let tx = Transaction::sigma_parse_bytes(&bytes).expect("Failed to parse transaction");

            let tx_id_hex = hex::encode(tx.id().0);
            assert_eq!(
                tx_id_hex, expected_tx_id,
                "Transaction ID mismatch for tx with data inputs and tokens"
            );

            // Verify structure
            assert_eq!(tx.inputs.len(), 2, "Expected 2 inputs");
            assert_eq!(tx.outputs.len(), 3, "Expected 3 outputs");
            let data_inputs = tx.data_inputs.as_ref().expect("Expected data inputs");
            assert_eq!(data_inputs.len(), 1, "Expected 1 data input");

            // Verify data input box ID
            let data_input_id = hex::encode(data_inputs.first().box_id.as_ref());
            assert_eq!(
                data_input_id,
                "f97d1dc4626de22db836270fe1aa004b99970791e4557de8f486f6d433b81195"
            );

            // Verify first output has tokens
            let first_output_tokens = tx.outputs.first().tokens.as_ref();
            assert_eq!(
                first_output_tokens.map(|t| t.len()).unwrap_or(0),
                2,
                "First output should have 2 tokens"
            );

            // Verify re-serialization
            let reserialized = tx.sigma_serialize_bytes().expect("Failed to serialize");
            assert_eq!(
                reserialized, bytes,
                "Re-serialization mismatch for tx with data inputs and tokens"
            );
        }

        /// Test that messageToSign (bytes_to_sign) matches Scala.
        /// This is critical for transaction signing compatibility.
        #[test]
        fn test_message_to_sign() {
            // Simple transfer tx - bytesToSign from Scala test
            let bytes_to_sign_str = "02c95c2ccf55e03cac6659f71ca4df832d28e2375569cec178dcb17f3e2e5f77420000ca796a4fc9c0d746a69702a77bd78b1a80a5ef5bf5713bbd95d93a4f23b27ead00000000028094ebdc030008cd0326df75ea615c18acc6bb4b517ac82795872f388d5d180aac90eaa84de750b942e8070000c0843d1005040004000e36100204cf0f08cd0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798ea02d192a39a8cc7a701730073011001020402d19683030193a38cc7b2a57300000193c2b2a57301007473027303830108cdeeac93b1a57304e8070000";

            // Full tx bytes (with proofs)
            let full_tx_bytes_str = "02c95c2ccf55e03cac6659f71ca4df832d28e2375569cec178dcb17f3e2e5f774238b4a04b4201da0578be3dac11067b567a73831f35b024a2e623c1f8da230407f63bab62c62ed9b93808b106b5a7e8b1751fa656f4c5de467400ca796a4fc9c0d746a69702a77bd78b1a80a5ef5bf5713bbd95d93a4f23b27ead385aea4d78a234c35accacdf8996b0af5b51e26fee29ea5c05468f23707d31c0df39400127391cd57a70eb856710db48bb9833606e0bf90340000000028094ebdc030008cd0326df75ea615c18acc6bb4b517ac82795872f388d5d180aac90eaa84de750b942e8070000c0843d1005040004000e36100204cf0f08cd0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798ea02d192a39a8cc7a701730073011001020402d19683030193a38cc7b2a57300000193c2b2a57301007473027303830108cdeeac93b1a57304e8070000";

            let full_bytes = hex_to_bytes(full_tx_bytes_str);
            let tx = Transaction::sigma_parse_bytes(&full_bytes).expect("Failed to parse tx");

            // Get bytes to sign from ergo-lib
            let bytes_to_sign = tx.bytes_to_sign().expect("Failed to get bytes to sign");
            let actual_bytes_to_sign_hex = hex::encode(&bytes_to_sign);

            assert_eq!(
                actual_bytes_to_sign_hex, bytes_to_sign_str,
                "bytes_to_sign mismatch - transaction signing would be incompatible"
            );
        }
    }
}
