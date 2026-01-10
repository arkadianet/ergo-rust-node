//! Test data generators for integration tests.
//!
//! Provides functions to generate test blocks, headers, transactions,
//! and other data structures needed for integration testing.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Global counter for unique ID generation
static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a unique test ID based on a seed.
pub fn test_id(seed: u8) -> Vec<u8> {
    let mut id = vec![0u8; 32];
    id[0] = seed;
    id[31] = seed.wrapping_mul(7);
    id
}

/// Generate a unique test box ID (without seed, uses atomic counter).
pub fn test_box_id() -> [u8; 32] {
    let counter = ID_COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut id = [0u8; 32];
    id[0] = 0xB0; // 'B' for box
    let counter_bytes = counter.to_be_bytes();
    id[1..9].copy_from_slice(&counter_bytes);
    id[31] = (counter % 256) as u8;
    id
}

/// Generate a test box ID with a seed (for deterministic tests).
pub fn test_box_id_seeded(seed: u8) -> Vec<u8> {
    let mut id = vec![0u8; 32];
    id[0] = 0xB0; // 'B' for box
    id[1] = seed;
    id[31] = seed.wrapping_add(1);
    id
}

/// Generate a unique test transaction ID (without seed, uses atomic counter).
pub fn test_tx_id() -> [u8; 32] {
    let counter = ID_COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut id = [0u8; 32];
    id[0] = 0xAA; // 'T' marker
    let counter_bytes = counter.to_be_bytes();
    id[1..9].copy_from_slice(&counter_bytes);
    id[31] = (counter % 256) as u8;
    id
}

/// Generate a test transaction ID with a seed (for deterministic tests).
pub fn test_tx_id_seeded(seed: u8) -> Vec<u8> {
    let mut id = vec![0u8; 32];
    id[0] = 0xAA; // 'T' marker
    id[1] = seed;
    id[31] = seed.wrapping_mul(3);
    id
}

/// Generate a unique test block ID (without seed, uses atomic counter).
pub fn test_block_id() -> [u8; 32] {
    let counter = ID_COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut id = [0u8; 32];
    id[0] = 0xBB; // Block marker
    let counter_bytes = counter.to_be_bytes();
    id[1..9].copy_from_slice(&counter_bytes);
    id[31] = (counter % 256) as u8;
    id
}

/// Generate a test block ID (header ID) for a specific height.
pub fn test_block_id_at_height(height: u32) -> Vec<u8> {
    let mut id = vec![0u8; 32];
    let height_bytes = height.to_be_bytes();
    id[0] = 0xBB; // Block marker
    id[1..5].copy_from_slice(&height_bytes);
    id[31] = (height % 256) as u8;
    id
}

/// Generate a parent block ID for a given height.
pub fn parent_block_id(height: u32) -> Vec<u8> {
    if height == 0 {
        vec![0u8; 32] // Genesis has zero parent
    } else {
        test_block_id_at_height(height - 1)
    }
}

/// Generate a test timestamp.
pub fn test_timestamp(offset_secs: u64) -> u64 {
    let base = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    base.saturating_sub(offset_secs * 1000)
}

/// Generate a monotonically increasing timestamp for a block at a given height.
pub fn block_timestamp(height: u32, interval_ms: u64) -> u64 {
    // Use a fixed base time for reproducibility
    let base_time: u64 = 1700000000000; // Some fixed timestamp in ms
    base_time + (height as u64 * interval_ms)
}

/// Simple test header data structure.
/// Uses fixed-size arrays for IDs to support property testing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestHeader {
    pub id: [u8; 32],
    pub parent_id: [u8; 32],
    pub height: u32,
    pub timestamp: u64,
    pub nbits: u32,
    pub version: u8,
}

impl TestHeader {
    /// Create a genesis header.
    pub fn genesis() -> Self {
        let mut id = [0u8; 32];
        id[0] = 0xBB;
        Self {
            id,
            parent_id: [0u8; 32],
            height: 0,
            timestamp: block_timestamp(0, 120_000),
            nbits: 0x1f00ffff, // Easy difficulty for tests
            version: 1,
        }
    }

    /// Create a header at a specific height.
    pub fn at_height(height: u32) -> Self {
        let id_vec = test_block_id_at_height(height);
        let parent_vec = parent_block_id(height);

        let mut id = [0u8; 32];
        let mut parent_id = [0u8; 32];
        id.copy_from_slice(&id_vec);
        parent_id.copy_from_slice(&parent_vec);

        Self {
            id,
            parent_id,
            height,
            timestamp: block_timestamp(height, 120_000),
            nbits: 0x1f00ffff,
            version: 1,
        }
    }

    /// Create a chain of headers from genesis to the given length.
    pub fn chain(length: usize) -> Vec<Self> {
        let mut chain: Vec<TestHeader> = Vec::with_capacity(length);
        for i in 0..length {
            let height = i as u32;
            let mut header = Self::at_height(height);
            if i > 0 {
                header.parent_id = chain[i - 1].id;
            }
            chain.push(header);
        }
        chain
    }

    /// Serialize header to bytes (simple format for testing).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(78);
        bytes.extend_from_slice(&self.id);
        bytes.extend_from_slice(&self.parent_id);
        bytes.extend_from_slice(&self.height.to_be_bytes());
        bytes.extend_from_slice(&self.timestamp.to_be_bytes());
        bytes.extend_from_slice(&self.nbits.to_be_bytes());
        bytes.push(self.version);
        bytes
    }

    /// Deserialize header from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        assert!(bytes.len() >= 77, "Header bytes too short");

        let mut id = [0u8; 32];
        let mut parent_id = [0u8; 32];
        id.copy_from_slice(&bytes[0..32]);
        parent_id.copy_from_slice(&bytes[32..64]);

        Self {
            id,
            parent_id,
            height: u32::from_be_bytes(bytes[64..68].try_into().unwrap()),
            timestamp: u64::from_be_bytes(bytes[68..76].try_into().unwrap()),
            nbits: u32::from_be_bytes(bytes[76..80].try_into().unwrap()),
            version: bytes[80],
        }
    }
}

/// Simple test transaction data structure.
#[derive(Debug, Clone)]
pub struct TestTransaction {
    pub id: Vec<u8>,
    pub inputs: Vec<Vec<u8>>,
    pub outputs: Vec<TestOutput>,
    pub fee: u64,
}

/// Simple test output (box) data structure.
#[derive(Debug, Clone)]
pub struct TestOutput {
    pub id: Vec<u8>,
    pub value: u64,
    pub ergo_tree: Vec<u8>,
    pub creation_height: u32,
}

impl TestTransaction {
    /// Create a simple transaction.
    pub fn new(seed: u8, num_inputs: usize, num_outputs: usize, fee: u64) -> Self {
        let id = test_tx_id_seeded(seed);

        let inputs: Vec<Vec<u8>> = (0..num_inputs)
            .map(|i| test_box_id_seeded(seed.wrapping_add(i as u8).wrapping_mul(2)))
            .collect();

        let outputs: Vec<TestOutput> = (0..num_outputs)
            .map(|i| TestOutput {
                id: test_box_id_seeded(seed.wrapping_add(100).wrapping_add(i as u8)),
                value: 1_000_000_000,              // 1 ERG
                ergo_tree: vec![0x00, 0x08, 0xcd], // Simple P2PK prefix
                creation_height: 0,
            })
            .collect();

        Self {
            id,
            inputs,
            outputs,
            fee,
        }
    }

    /// Create a coinbase transaction for a given height.
    pub fn coinbase(height: u32) -> Self {
        let id = test_tx_id_seeded(height as u8);

        Self {
            id: id.clone(),
            inputs: vec![], // Coinbase has no inputs
            outputs: vec![TestOutput {
                id: test_box_id_seeded(height as u8),
                value: 67_500_000_000, // Block reward
                ergo_tree: vec![0x00, 0x08, 0xcd],
                creation_height: height,
            }],
            fee: 0,
        }
    }

    /// Serialize transaction to bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&self.id);
        bytes.extend_from_slice(&(self.inputs.len() as u32).to_be_bytes());
        for input in &self.inputs {
            bytes.extend_from_slice(input);
        }
        bytes.extend_from_slice(&(self.outputs.len() as u32).to_be_bytes());
        for output in &self.outputs {
            bytes.extend_from_slice(&output.id);
            bytes.extend_from_slice(&output.value.to_be_bytes());
        }
        bytes.extend_from_slice(&self.fee.to_be_bytes());
        bytes
    }
}

/// Simple test block data structure.
#[derive(Debug, Clone)]
pub struct TestBlock {
    pub header: TestHeader,
    pub transactions: Vec<TestTransaction>,
}

impl TestBlock {
    /// Create a block at a given height with a coinbase transaction.
    pub fn at_height(height: u32) -> Self {
        Self {
            header: TestHeader::at_height(height),
            transactions: vec![TestTransaction::coinbase(height)],
        }
    }

    /// Create a chain of blocks from genesis.
    pub fn chain(length: u32) -> Vec<Self> {
        (0..length).map(Self::at_height).collect()
    }

    /// Add a transaction to the block.
    pub fn with_transaction(mut self, tx: TestTransaction) -> Self {
        self.transactions.push(tx);
        self
    }
}

/// Generate a fork of a chain at a given height.
pub fn generate_fork(
    base_chain: &[TestHeader],
    fork_point: usize,
    fork_length: usize,
) -> Vec<TestHeader> {
    let mut fork: Vec<TestHeader> = Vec::new();

    for i in 0..fork_length {
        let height = (fork_point + i + 1) as u32;
        let parent_id = if i == 0 {
            base_chain[fork_point].id
        } else {
            fork[i - 1].id
        };

        // Generate different IDs for fork blocks
        let id_vec = test_block_id_at_height(height);
        let mut id = [0u8; 32];
        id.copy_from_slice(&id_vec);
        id[2] = 0xFF; // Mark as fork block

        fork.push(TestHeader {
            id,
            parent_id,
            height,
            timestamp: block_timestamp(height, 120_000) + 1000, // Slightly different timestamp
            nbits: 0x1f00ffff,
            version: 1,
        });
    }

    fork
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_chain_generation() {
        let chain = TestHeader::chain(10);

        assert_eq!(chain.len(), 10);
        assert_eq!(chain[0].height, 0);
        assert_eq!(chain[9].height, 9);

        // Verify parent links
        for i in 1..chain.len() {
            assert_eq!(chain[i].parent_id, chain[i - 1].id);
        }
    }

    #[test]
    fn test_header_serialization() {
        let header = TestHeader::at_height(42);
        let bytes = header.to_bytes();
        let restored = TestHeader::from_bytes(&bytes);

        assert_eq!(header.id, restored.id);
        assert_eq!(header.height, restored.height);
        assert_eq!(header.parent_id, restored.parent_id);
    }

    #[test]
    fn test_fork_generation() {
        let main_chain = TestHeader::chain(10);
        let fork = generate_fork(&main_chain, 5, 4);

        assert_eq!(fork.len(), 4);
        assert_eq!(fork[0].height, 6);
        assert_eq!(fork[3].height, 9);

        // Fork should branch from main chain at height 5
        assert_eq!(fork[0].parent_id, main_chain[5].id);

        // Fork blocks should have different IDs
        assert_ne!(fork[0].id, main_chain[6].id);
    }

    #[test]
    fn test_transaction_creation() {
        let tx = TestTransaction::new(1, 2, 3, 1_000_000);

        assert_eq!(tx.inputs.len(), 2);
        assert_eq!(tx.outputs.len(), 3);
        assert_eq!(tx.fee, 1_000_000);
    }

    #[test]
    fn test_block_creation() {
        let block = TestBlock::at_height(5);

        assert_eq!(block.header.height, 5);
        assert_eq!(block.transactions.len(), 1); // Just coinbase
    }

    #[test]
    fn test_unique_id_generation() {
        let id1 = test_block_id();
        let id2 = test_block_id();
        let id3 = test_tx_id();
        let id4 = test_box_id();

        // All IDs should be unique
        assert_ne!(id1, id2);
        assert_ne!(id3, id1);
        assert_ne!(id4, id1);
    }
}
