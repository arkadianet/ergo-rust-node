//! Mining tests for ergo-mining crate.
//!
//! Tests for block candidate generation, coinbase creation, and mining operations.

use crate::harness::TestDatabase;
use ergo_mempool::Mempool;
use ergo_mining::{
    block_reward_at_height, calculate_total_reward, emission_at_height, reemission_at_height,
    CandidateGenerator, CoinbaseBuilder, Miner, MinerConfig, MiningError,
};
use ergo_state::StateManager;
use ergo_storage::Storage;
use std::sync::Arc;

// ============================================================================
// Block Reward Tests
// ============================================================================

#[test]
fn test_block_reward_at_genesis() {
    let reward = block_reward_at_height(1);
    // Initial reward is 75 ERG = 75_000_000_000 nanoERG
    assert_eq!(reward, 75_000_000_000);
}

#[test]
fn test_block_reward_early_blocks() {
    // First 2 years (fixed rate period) should be 75 ERG
    for height in [1, 100, 1000, 10000, 50000] {
        let reward = block_reward_at_height(height);
        assert_eq!(
            reward, 75_000_000_000,
            "Height {height} should have 75 ERG reward"
        );
    }
}

#[test]
fn test_block_reward_decreases_over_time() {
    let early_reward = block_reward_at_height(100);
    let late_reward = block_reward_at_height(2_000_000);

    assert!(
        late_reward < early_reward,
        "Reward should decrease over time"
    );
}

#[test]
fn test_block_reward_minimum() {
    // Very far in the future, reward should be at minimum (3 ERG)
    let reward = block_reward_at_height(10_000_000);
    assert_eq!(reward, 3_000_000_000, "Minimum reward should be 3 ERG");
}

#[test]
fn test_emission_at_height() {
    let emission = emission_at_height(1);
    assert!(emission > 0);
}

#[test]
fn test_emission_at_height_early_is_constant() {
    // During fixed rate period, emission is constant at 75 ERG
    let emission_100 = emission_at_height(100);
    let emission_1000 = emission_at_height(1000);

    assert_eq!(
        emission_100, emission_1000,
        "Emission should be constant during fixed rate period"
    );
}

#[test]
fn test_emission_decreases_after_fixed_period() {
    // After fixed rate period (~525600 blocks), emission starts decreasing
    let early_emission = emission_at_height(100);
    let late_emission = emission_at_height(2_000_000);

    assert!(
        late_emission < early_emission,
        "Emission should decrease over time after fixed rate period"
    );
}

#[test]
fn test_reemission_at_height_before_activation() {
    // Before EIP-27 activation, reemission should be 0
    let reemission = reemission_at_height(100);
    assert_eq!(reemission, 0);
}

#[test]
fn test_reemission_at_height_after_activation() {
    // After EIP-27 activation (height 777217), reemission should be non-zero
    let reemission = reemission_at_height(777_217);
    assert!(reemission > 0);
}

#[test]
fn test_calculate_total_reward_no_fees() {
    let height = 1000;
    let fees = 0;
    let total = calculate_total_reward(height, fees);

    assert_eq!(total, block_reward_at_height(height));
}

#[test]
fn test_calculate_total_reward_adds_correctly() {
    let height = 1000;
    let fees = 1_000_000_000; // 1 ERG in fees
    let total = calculate_total_reward(height, fees);

    assert_eq!(total, block_reward_at_height(height) + fees);
}

#[test]
fn test_calculate_total_reward_large_fees() {
    let height = 1000;
    let fees = 100_000_000_000; // 100 ERG in fees
    let total = calculate_total_reward(height, fees);

    assert_eq!(total, block_reward_at_height(height) + fees);
}

// ============================================================================
// Coinbase Builder Tests
// ============================================================================

#[test]
fn test_coinbase_builder_creation_mainnet() {
    use ergo_mining::NetworkPrefix;
    let builder = CoinbaseBuilder::new(NetworkPrefix::Mainnet);
    // Builder should be created successfully
    assert!(std::mem::size_of_val(&builder) > 0);
}

#[test]
fn test_coinbase_builder_creation_testnet() {
    use ergo_mining::NetworkPrefix;
    let builder = CoinbaseBuilder::new(NetworkPrefix::Testnet);
    assert!(std::mem::size_of_val(&builder) > 0);
}

#[test]
fn test_coinbase_builder_default() {
    let builder = CoinbaseBuilder::default();
    assert!(std::mem::size_of_val(&builder) > 0);
}

#[test]
fn test_coinbase_builder_miner_reward() {
    let builder = CoinbaseBuilder::default();
    let reward = builder.miner_reward_at_height(100);
    // Early blocks should have 75 ERG reward
    assert_eq!(reward, 75_000_000_000);
}

#[test]
fn test_coinbase_builder_reemission_before_activation() {
    let builder = CoinbaseBuilder::default();
    let reemission = builder.reemission_at_height(100);
    // Before EIP-27 activation, should be 0
    assert_eq!(reemission, 0);
}

#[test]
fn test_coinbase_builder_reemission_after_activation() {
    let builder = CoinbaseBuilder::default();
    let reemission = builder.reemission_at_height(777_217);
    // After EIP-27 activation, should be non-zero
    assert!(reemission > 0);
}

#[test]
fn test_coinbase_builder_total_miner_income() {
    let builder = CoinbaseBuilder::default();
    let income = builder.total_miner_income_at_height(100);
    // Total income should equal direct reward before reemission period
    assert_eq!(income, builder.miner_reward_at_height(100));
}

// ============================================================================
// Candidate Generator Tests
// ============================================================================

fn create_candidate_generator() -> (Arc<CandidateGenerator>, TestDatabase) {
    let test_db = TestDatabase::new();
    let storage: Arc<dyn Storage> = Arc::new(test_db.db_clone());
    let state_manager = Arc::new(StateManager::new(Arc::clone(&storage)));
    let mempool = Arc::new(Mempool::with_defaults());

    let generator = Arc::new(CandidateGenerator::new(state_manager, mempool));
    (generator, test_db)
}

#[test]
fn test_candidate_generator_creation() {
    let (generator, _db) = create_candidate_generator();
    assert!(generator.reward_address().is_none());
}

#[test]
fn test_candidate_generator_set_reward_address() {
    let (generator, _db) = create_candidate_generator();

    generator.set_reward_address("9fRusAarL1KkrWQVsxSRVYnvWxaAT2A96cKtNn9tvPh5XUyo".to_string());

    assert!(generator.reward_address().is_some());
    assert_eq!(
        generator.reward_address().unwrap(),
        "9fRusAarL1KkrWQVsxSRVYnvWxaAT2A96cKtNn9tvPh5XUyo"
    );
}

#[test]
fn test_candidate_generator_change_reward_address() {
    let (generator, _db) = create_candidate_generator();

    generator.set_reward_address("address1".to_string());
    assert_eq!(generator.reward_address().unwrap(), "address1");

    generator.set_reward_address("address2".to_string());
    assert_eq!(generator.reward_address().unwrap(), "address2");
}

#[test]
fn test_candidate_generator_no_reward_address_error() {
    let (generator, _db) = create_candidate_generator();

    let result = generator.generate();

    assert!(result.is_err());
    match result.unwrap_err() {
        MiningError::NoRewardAddress => {}
        other => panic!("Expected NoRewardAddress error, got {:?}", other),
    }
}

#[test]
fn test_candidate_generator_with_reward_address() {
    let (generator, _db) = create_candidate_generator();

    generator.set_reward_address("9fRusAarL1KkrWQVsxSRVYnvWxaAT2A96cKtNn9tvPh5XUyo".to_string());

    let result = generator.generate();
    assert!(
        result.is_ok(),
        "Should generate candidate with reward address set"
    );

    let candidate = result.unwrap();
    assert!(candidate.height > 0);
    assert!(candidate.reward > 0);
}

#[test]
fn test_candidate_has_valid_height() {
    let (generator, _db) = create_candidate_generator();
    generator.set_reward_address("test_address".to_string());

    let candidate = generator.generate().unwrap();

    // Height should be current height + 1 (at least 1)
    assert!(candidate.height >= 1);
}

#[test]
fn test_candidate_has_valid_timestamp() {
    let (generator, _db) = create_candidate_generator();
    generator.set_reward_address("test_address".to_string());

    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    let candidate = generator.generate().unwrap();

    let after = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    assert!(candidate.timestamp >= before);
    assert!(candidate.timestamp <= after);
}

#[test]
fn test_candidate_has_valid_difficulty() {
    let (generator, _db) = create_candidate_generator();
    generator.set_reward_address("test_address".to_string());

    let candidate = generator.generate().unwrap();

    // n_bits should be non-zero
    assert!(candidate.n_bits > 0);
}

#[test]
fn test_candidate_mining_message_not_empty() {
    let (generator, _db) = create_candidate_generator();
    generator.set_reward_address("test_address".to_string());

    let candidate = generator.generate().unwrap();

    assert!(!candidate.mining_message().is_empty());
}

#[test]
fn test_candidate_header_bytes_correct_length() {
    let (generator, _db) = create_candidate_generator();
    generator.set_reward_address("test_address".to_string());

    let candidate = generator.generate().unwrap();

    // Header should be at least 180 bytes (version + parent + roots + timestamp + nbits + height + extension + votes)
    assert!(
        candidate.header_bytes.len() >= 180,
        "Header should be at least 180 bytes, got {}",
        candidate.header_bytes.len()
    );
}

#[test]
fn test_candidate_invalidation() {
    let (generator, _db) = create_candidate_generator();
    generator.set_reward_address("test_address".to_string());

    // Generate first candidate
    let candidate1 = generator.generate().unwrap();

    // Invalidate cache
    generator.invalidate();

    // Generate second candidate - should have new timestamp
    std::thread::sleep(std::time::Duration::from_millis(10));
    let candidate2 = generator.generate().unwrap();

    // Timestamps should be different
    assert!(candidate2.timestamp >= candidate1.timestamp);
}

#[test]
fn test_get_or_generate_caches() {
    let (generator, _db) = create_candidate_generator();
    generator.set_reward_address("test_address".to_string());

    // First call generates
    let candidate1 = generator.get_or_generate().unwrap();

    // Second call should return cached (same timestamp)
    let candidate2 = generator.get_or_generate().unwrap();

    assert_eq!(candidate1.timestamp, candidate2.timestamp);
    assert_eq!(candidate1.height, candidate2.height);
}

// ============================================================================
// Miner Tests
// ============================================================================

fn create_test_miner() -> (Miner, TestDatabase) {
    let test_db = TestDatabase::new();
    let storage: Arc<dyn Storage> = Arc::new(test_db.db_clone());
    let state_manager = Arc::new(StateManager::new(Arc::clone(&storage)));
    let mempool = Arc::new(Mempool::with_defaults());
    let candidate_gen = Arc::new(CandidateGenerator::new(state_manager, mempool));

    let config = MinerConfig::default();

    let miner = Miner::new(config, candidate_gen);
    (miner, test_db)
}

fn create_test_miner_with_address() -> (Miner, TestDatabase) {
    let test_db = TestDatabase::new();
    let storage: Arc<dyn Storage> = Arc::new(test_db.db_clone());
    let state_manager = Arc::new(StateManager::new(Arc::clone(&storage)));
    let mempool = Arc::new(Mempool::with_defaults());
    let candidate_gen = Arc::new(CandidateGenerator::new(state_manager, mempool));

    let config = MinerConfig {
        reward_address: "test_miner_address".to_string(),
        ..Default::default()
    };

    let miner = Miner::new(config, candidate_gen);
    (miner, test_db)
}

#[test]
fn test_miner_creation() {
    let (miner, _db) = create_test_miner();
    assert!(!miner.is_enabled());
}

#[test]
fn test_miner_start() {
    let (miner, _db) = create_test_miner();

    miner.start();

    assert!(miner.is_enabled());
}

#[test]
fn test_miner_stop() {
    let (miner, _db) = create_test_miner();

    miner.start();
    assert!(miner.is_enabled());

    miner.stop();
    assert!(!miner.is_enabled());
}

#[test]
fn test_miner_start_stop_toggle() {
    let (miner, _db) = create_test_miner();

    for _ in 0..3 {
        miner.start();
        assert!(miner.is_enabled());
        miner.stop();
        assert!(!miner.is_enabled());
    }
}

#[test]
fn test_miner_set_reward_address() {
    let (miner, _db) = create_test_miner_with_address();

    // Reward address should be set
    assert!(miner.reward_address().is_some());
}

#[test]
fn test_miner_change_reward_address() {
    let (miner, _db) = create_test_miner();

    miner.set_reward_address("new_address".to_string());

    assert_eq!(miner.reward_address(), Some("new_address".to_string()));
}

#[test]
fn test_miner_get_candidate_when_disabled() {
    let (miner, _db) = create_test_miner_with_address();

    // Miner not started
    let result = miner.get_candidate();

    assert!(result.is_err());
}

#[test]
fn test_miner_get_candidate_when_enabled() {
    let (miner, _db) = create_test_miner_with_address();

    miner.start();
    let result = miner.get_candidate();

    assert!(result.is_ok());
}

#[test]
fn test_miner_stats_initial() {
    let (miner, _db) = create_test_miner();

    let stats = miner.stats();

    assert_eq!(stats.candidates_generated, 0);
    assert_eq!(stats.solutions_received, 0);
    assert_eq!(stats.valid_solutions, 0);
    assert_eq!(stats.invalid_solutions, 0);
    assert_eq!(stats.blocks_mined, 0);
}

#[test]
fn test_miner_stats_after_get_candidate() {
    let (miner, _db) = create_test_miner_with_address();

    miner.start();
    let _ = miner.get_candidate();

    let stats = miner.stats();
    assert_eq!(stats.candidates_generated, 1);
}

#[test]
fn test_miner_on_new_block_invalidates() {
    let (miner, _db) = create_test_miner_with_address();

    miner.start();
    let _ = miner.get_candidate();

    // Notify new block
    miner.on_new_block(100);

    // Getting candidate again should work
    let result = miner.get_candidate();
    assert!(result.is_ok());
}

// ============================================================================
// Block Candidate Structure Tests
// ============================================================================

#[test]
fn test_block_candidate_clone() {
    let (generator, _db) = create_candidate_generator();
    generator.set_reward_address("test_address".to_string());

    let candidate = generator.generate().unwrap();
    let cloned = candidate.clone();

    assert_eq!(candidate.height, cloned.height);
    assert_eq!(candidate.timestamp, cloned.timestamp);
    assert_eq!(candidate.n_bits, cloned.n_bits);
    assert_eq!(candidate.reward, cloned.reward);
}

#[test]
fn test_block_candidate_debug() {
    let (generator, _db) = create_candidate_generator();
    generator.set_reward_address("test_address".to_string());

    let candidate = generator.generate().unwrap();
    let debug_str = format!("{:?}", candidate);

    assert!(debug_str.contains("BlockCandidate"));
    assert!(debug_str.contains("height"));
}

#[test]
fn test_candidate_transaction_ids_initially_empty() {
    let (generator, _db) = create_candidate_generator();
    generator.set_reward_address("test_address".to_string());

    let candidate = generator.generate().unwrap();

    // With empty mempool, no transactions
    assert!(candidate.transaction_ids.is_empty());
}

#[test]
fn test_candidate_state_root_not_empty() {
    let (generator, _db) = create_candidate_generator();
    generator.set_reward_address("test_address".to_string());

    let candidate = generator.generate().unwrap();

    assert!(!candidate.state_root.is_empty());
}

#[test]
fn test_candidate_extension_hash_32_bytes() {
    let (generator, _db) = create_candidate_generator();
    generator.set_reward_address("test_address".to_string());

    let candidate = generator.generate().unwrap();

    assert_eq!(candidate.extension_hash.len(), 32);
}

#[test]
fn test_candidate_transactions_root_32_bytes() {
    let (generator, _db) = create_candidate_generator();
    generator.set_reward_address("test_address".to_string());

    let candidate = generator.generate().unwrap();

    assert_eq!(candidate.transactions_root.len(), 32);
}

// ============================================================================
// Mining Config Tests
// ============================================================================

#[test]
fn test_miner_config_default() {
    let config = MinerConfig::default();

    assert!(!config.internal_mining);
    assert!(config.external_mining);
    assert!(config.reward_address.is_empty());
    assert_eq!(config.threads, 1);
}

#[test]
fn test_miner_config_custom() {
    let config = MinerConfig {
        internal_mining: true,
        external_mining: false,
        reward_address: "test_address".to_string(),
        threads: 4,
    };

    assert!(config.internal_mining);
    assert!(!config.external_mining);
    assert_eq!(config.reward_address, "test_address");
    assert_eq!(config.threads, 4);
}

#[test]
fn test_miner_config_clone() {
    let config = MinerConfig {
        internal_mining: true,
        external_mining: true,
        reward_address: "cloned_address".to_string(),
        threads: 8,
    };

    let cloned = config.clone();

    assert_eq!(config.internal_mining, cloned.internal_mining);
    assert_eq!(config.external_mining, cloned.external_mining);
    assert_eq!(config.reward_address, cloned.reward_address);
    assert_eq!(config.threads, cloned.threads);
}

#[test]
fn test_miner_config_debug() {
    let config = MinerConfig::default();
    let debug_str = format!("{:?}", config);

    assert!(debug_str.contains("MinerConfig"));
}
