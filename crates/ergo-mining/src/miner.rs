//! Miner implementation.

use crate::{BlockCandidate, CandidateGenerator, MiningError, MiningResult};
use ergo_consensus::{nbits_to_target, AutolykosSolution, AutolykosV2};
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{debug, info};

/// Miner configuration.
#[derive(Debug, Clone)]
pub struct MinerConfig {
    /// Enable internal CPU mining.
    pub internal_mining: bool,
    /// Use external miner (stratum protocol).
    pub external_mining: bool,
    /// Reward address.
    pub reward_address: String,
    /// Number of mining threads.
    pub threads: usize,
}

impl Default for MinerConfig {
    fn default() -> Self {
        Self {
            internal_mining: false,
            external_mining: true,
            reward_address: String::new(),
            threads: 1,
        }
    }
}

/// Mining statistics.
#[derive(Debug, Clone, Default)]
pub struct MiningStats {
    /// Candidates generated.
    pub candidates_generated: u64,
    /// Solutions received.
    pub solutions_received: u64,
    /// Valid solutions.
    pub valid_solutions: u64,
    /// Invalid solutions.
    pub invalid_solutions: u64,
    /// Blocks mined.
    pub blocks_mined: u64,
}

/// Block miner.
pub struct Miner {
    /// Configuration.
    config: MinerConfig,
    /// Candidate generator.
    candidate_gen: Arc<CandidateGenerator>,
    /// Autolykos verifier.
    autolykos: AutolykosV2,
    /// Mining enabled flag.
    enabled: AtomicBool,
    /// Current candidate.
    current_candidate: RwLock<Option<BlockCandidate>>,
    /// Mining statistics.
    stats: RwLock<MiningStats>,
}

impl Miner {
    /// Create a new miner.
    pub fn new(config: MinerConfig, candidate_gen: Arc<CandidateGenerator>) -> Self {
        if !config.reward_address.is_empty() {
            candidate_gen.set_reward_address(config.reward_address.clone());
        }

        Self {
            config,
            candidate_gen,
            autolykos: AutolykosV2::new(),
            enabled: AtomicBool::new(false),
            current_candidate: RwLock::new(None),
            stats: RwLock::new(MiningStats::default()),
        }
    }

    /// Enable mining.
    pub fn start(&self) {
        self.enabled.store(true, Ordering::SeqCst);
        info!("Mining enabled");
    }

    /// Disable mining.
    pub fn stop(&self) {
        self.enabled.store(false, Ordering::SeqCst);
        info!("Mining disabled");
    }

    /// Check if mining is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// Get current mining candidate.
    pub fn get_candidate(&self) -> MiningResult<BlockCandidate> {
        if !self.is_enabled() {
            return Err(MiningError::CandidateFailed(
                "Mining not enabled".to_string(),
            ));
        }

        let candidate = self.candidate_gen.get_or_generate()?;
        *self.current_candidate.write() = Some(candidate.clone());

        self.stats.write().candidates_generated += 1;

        Ok(candidate)
    }

    /// Submit a mining solution.
    pub fn submit_solution(&self, solution: AutolykosSolution) -> MiningResult<bool> {
        if !self.is_enabled() {
            return Err(MiningError::CandidateFailed(
                "Mining not enabled".to_string(),
            ));
        }

        self.stats.write().solutions_received += 1;

        let candidate = self
            .current_candidate
            .read()
            .clone()
            .ok_or_else(|| MiningError::InvalidSolution("No current candidate".to_string()))?;

        // Get difficulty target
        let target = nbits_to_target(candidate.n_bits);

        // Verify solution
        let valid = self.autolykos.verify_solution(
            &candidate.header_bytes,
            &solution,
            &target,
            candidate.height,
        )?;

        if valid {
            info!(height = candidate.height, "Valid solution found!");
            self.stats.write().valid_solutions += 1;

            // Would broadcast block here
            self.stats.write().blocks_mined += 1;

            // Invalidate candidate
            self.candidate_gen.invalidate();
            *self.current_candidate.write() = None;

            Ok(true)
        } else {
            debug!("Invalid solution submitted");
            self.stats.write().invalid_solutions += 1;
            Ok(false)
        }
    }

    /// Get mining statistics.
    pub fn stats(&self) -> MiningStats {
        self.stats.read().clone()
    }

    /// Get reward address.
    pub fn reward_address(&self) -> Option<String> {
        self.candidate_gen.reward_address()
    }

    /// Set reward address.
    pub fn set_reward_address(&self, address: String) {
        self.candidate_gen.set_reward_address(address);
    }

    /// Notify of new block (invalidates current work).
    pub fn on_new_block(&self, _height: u32) {
        self.candidate_gen.invalidate();
        *self.current_candidate.write() = None;
        debug!("Mining work invalidated due to new block");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ergo_mempool::Mempool;
    use ergo_state::StateManager;
    use ergo_storage::Database;
    use tempfile::TempDir;

    fn create_test_miner() -> (Miner, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db = Database::open(tmp.path()).unwrap();
        let state = Arc::new(StateManager::new(Arc::new(db)));
        let mempool = Arc::new(Mempool::with_defaults());
        let candidate_gen = Arc::new(CandidateGenerator::new(state, mempool));

        let config = MinerConfig {
            reward_address: "test_address".to_string(),
            ..Default::default()
        };

        let miner = Miner::new(config, candidate_gen);
        (miner, tmp)
    }

    #[test]
    fn test_miner_enable_disable() {
        let (miner, _tmp) = create_test_miner();

        assert!(!miner.is_enabled());

        miner.start();
        assert!(miner.is_enabled());

        miner.stop();
        assert!(!miner.is_enabled());
    }
}
