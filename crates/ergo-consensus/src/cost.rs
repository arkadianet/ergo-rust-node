//! Block and transaction cost tracking for validation.
//!
//! This module provides:
//! - Cost constants matching the Ergo protocol specification
//! - `CostAccumulator` for tracking costs during validation
//! - Transaction and block cost calculation utilities
//!
//! ## Cost Model
//!
//! Transaction cost consists of:
//! - Base cost per input (covers UTXO lookup)
//! - Base cost per output (covers output creation)
//! - Base cost per data input (covers read-only lookup)
//! - Size cost proportional to transaction bytes
//! - Script execution cost (from ErgoScript interpreter)
//!
//! ## Limits
//!
//! - MAX_TX_COST: Maximum cost for a single transaction (1,000,000)
//! - MAX_BLOCK_COST: Maximum total cost for a block (8,000,000)
//!
//! These limits prevent resource exhaustion attacks and ensure
//! predictable validation times.

use thiserror::Error;

/// Cost-related error types.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CostError {
    /// Cost limit exceeded.
    #[error("Cost limit exceeded: current {current}, limit {limit}")]
    LimitExceeded {
        /// Current accumulated cost.
        current: u64,
        /// Maximum allowed cost.
        limit: u64,
    },

    /// Transaction cost exceeded.
    #[error("Transaction cost {cost} exceeds limit {limit}")]
    TransactionCostExceeded {
        /// Transaction cost.
        cost: u64,
        /// Maximum allowed.
        limit: u64,
    },

    /// Block cost exceeded.
    #[error("Block cost {cost} exceeds limit {limit}")]
    BlockCostExceeded {
        /// Block cost.
        cost: u64,
        /// Maximum allowed.
        limit: u64,
    },
}

/// Cost constants for Ergo block/transaction validation.
///
/// These values are derived from the Ergo protocol specification
/// and match the Scala reference implementation.
pub struct CostConstants;

impl CostConstants {
    /// Maximum script execution cost per transaction.
    ///
    /// This limits the computational complexity of scripts within
    /// a single transaction. Transactions exceeding this limit
    /// are rejected.
    pub const MAX_TX_COST: u64 = 1_000_000;

    /// Maximum total cost per block.
    ///
    /// This limits the total computational cost of all transactions
    /// in a block. Miners must ensure blocks don't exceed this limit.
    pub const MAX_BLOCK_COST: u64 = 8_000_000;

    /// Base cost per input (covers UTXO lookup and proof verification setup).
    ///
    /// Each input requires looking up the corresponding UTXO and
    /// preparing context for script verification.
    pub const INPUT_BASE_COST: u64 = 2_000;

    /// Base cost per output (covers output serialization and storage prep).
    ///
    /// Each output must be serialized and prepared for UTXO storage.
    pub const OUTPUT_BASE_COST: u64 = 100;

    /// Base cost per data input (covers read-only UTXO lookup).
    ///
    /// Data inputs require UTXO lookup but no spending verification.
    pub const DATA_INPUT_COST: u64 = 100;

    /// Cost per byte of transaction size.
    ///
    /// Larger transactions consume more bandwidth and storage,
    /// so they incur proportional cost.
    pub const SIZE_COST_PER_BYTE: u64 = 2;

    /// Minimum transaction cost (prevents zero-cost transactions).
    ///
    /// Even the simplest transaction has some base cost.
    pub const MIN_TX_COST: u64 = 1_000;
}

/// Accumulates costs during validation with limit checking.
///
/// The `CostAccumulator` tracks the running total of costs during
/// transaction or block validation. It automatically checks against
/// a configured limit and returns an error if exceeded.
///
/// # Example
///
/// ```ignore
/// use ergo_consensus::cost::{CostAccumulator, CostConstants};
///
/// let mut acc = CostAccumulator::new(CostConstants::MAX_TX_COST);
///
/// // Add input costs
/// acc.add(CostConstants::INPUT_BASE_COST * 2)?;
///
/// // Add script execution cost
/// acc.add(script_cost)?;
///
/// println!("Total cost: {}", acc.total());
/// ```
#[derive(Debug, Clone)]
pub struct CostAccumulator {
    /// Current accumulated cost.
    current: u64,
    /// Maximum allowed cost (limit).
    limit: u64,
}

impl CostAccumulator {
    /// Create a new cost accumulator with the given limit.
    ///
    /// # Arguments
    ///
    /// * `limit` - Maximum cost allowed before returning an error.
    pub fn new(limit: u64) -> Self {
        Self { current: 0, limit }
    }

    /// Create an accumulator for transaction validation.
    pub fn for_transaction() -> Self {
        Self::new(CostConstants::MAX_TX_COST)
    }

    /// Create an accumulator for block validation.
    pub fn for_block() -> Self {
        Self::new(CostConstants::MAX_BLOCK_COST)
    }

    /// Add cost to the accumulator.
    ///
    /// Returns an error if the limit would be exceeded.
    ///
    /// # Arguments
    ///
    /// * `cost` - Cost to add.
    ///
    /// # Returns
    ///
    /// Ok(()) if the cost was added successfully, or an error if
    /// the limit was exceeded.
    pub fn add(&mut self, cost: u64) -> Result<(), CostError> {
        self.current = self.current.saturating_add(cost);
        if self.current > self.limit {
            Err(CostError::LimitExceeded {
                current: self.current,
                limit: self.limit,
            })
        } else {
            Ok(())
        }
    }

    /// Add cost without limit checking.
    ///
    /// Use this when you want to accumulate costs but check
    /// the limit separately (e.g., at the end of validation).
    pub fn add_unchecked(&mut self, cost: u64) {
        self.current = self.current.saturating_add(cost);
    }

    /// Check if the current cost exceeds the limit.
    pub fn is_exceeded(&self) -> bool {
        self.current > self.limit
    }

    /// Get the current accumulated cost.
    pub fn total(&self) -> u64 {
        self.current
    }

    /// Get the remaining cost budget.
    pub fn remaining(&self) -> u64 {
        self.limit.saturating_sub(self.current)
    }

    /// Get the configured limit.
    pub fn limit(&self) -> u64 {
        self.limit
    }

    /// Reset the accumulator to zero.
    pub fn reset(&mut self) {
        self.current = 0;
    }

    /// Check if adding a cost would exceed the limit (without adding).
    pub fn would_exceed(&self, cost: u64) -> bool {
        self.current.saturating_add(cost) > self.limit
    }
}

/// Result of calculating transaction cost.
#[derive(Debug, Clone)]
pub struct TransactionCostResult {
    /// Total transaction cost.
    pub total_cost: u64,
    /// Base cost (inputs + outputs + data inputs).
    pub base_cost: u64,
    /// Size-based cost.
    pub size_cost: u64,
    /// Script execution cost (from interpreter).
    pub script_cost: u64,
    /// Per-input script costs (for debugging/analysis).
    pub per_input_costs: Vec<u64>,
}

impl TransactionCostResult {
    /// Create a new transaction cost result.
    pub fn new(
        base_cost: u64,
        size_cost: u64,
        script_cost: u64,
        per_input_costs: Vec<u64>,
    ) -> Self {
        Self {
            total_cost: base_cost + size_cost + script_cost,
            base_cost,
            size_cost,
            script_cost,
            per_input_costs,
        }
    }

    /// Check if the cost exceeds the transaction limit.
    pub fn exceeds_tx_limit(&self) -> bool {
        self.total_cost > CostConstants::MAX_TX_COST
    }
}

/// Calculate the base cost for a transaction (excluding script execution).
///
/// This calculates the fixed costs based on transaction structure:
/// - Cost per input
/// - Cost per output
/// - Cost per data input
/// - Cost per byte of transaction size
///
/// # Arguments
///
/// * `num_inputs` - Number of regular inputs.
/// * `num_outputs` - Number of outputs.
/// * `num_data_inputs` - Number of data inputs.
/// * `tx_size_bytes` - Transaction size in bytes.
///
/// # Returns
///
/// The base cost (before script execution).
pub fn calculate_base_cost(
    num_inputs: usize,
    num_outputs: usize,
    num_data_inputs: usize,
    tx_size_bytes: usize,
) -> u64 {
    let input_cost = num_inputs as u64 * CostConstants::INPUT_BASE_COST;
    let output_cost = num_outputs as u64 * CostConstants::OUTPUT_BASE_COST;
    let data_input_cost = num_data_inputs as u64 * CostConstants::DATA_INPUT_COST;
    let size_cost = tx_size_bytes as u64 * CostConstants::SIZE_COST_PER_BYTE;

    input_cost + output_cost + data_input_cost + size_cost
}

/// Estimate transaction cost without script execution.
///
/// This provides a lower bound estimate useful for mempool ordering
/// and quick rejection of obviously expensive transactions.
///
/// # Arguments
///
/// * `num_inputs` - Number of regular inputs.
/// * `num_outputs` - Number of outputs.
/// * `num_data_inputs` - Number of data inputs.
/// * `tx_size_bytes` - Transaction size in bytes.
///
/// # Returns
///
/// Estimated minimum transaction cost.
pub fn estimate_tx_cost(
    num_inputs: usize,
    num_outputs: usize,
    num_data_inputs: usize,
    tx_size_bytes: usize,
) -> u64 {
    let base = calculate_base_cost(num_inputs, num_outputs, num_data_inputs, tx_size_bytes);

    // Estimate script cost as minimum per input
    // (actual cost depends on script complexity)
    let estimated_script_cost = num_inputs as u64 * 1000; // Conservative estimate

    base.saturating_add(estimated_script_cost)
        .max(CostConstants::MIN_TX_COST)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============ CostAccumulator Tests ============

    #[test]
    fn test_accumulator_basic() {
        let mut acc = CostAccumulator::new(1000);

        assert_eq!(acc.total(), 0);
        assert_eq!(acc.remaining(), 1000);
        assert!(!acc.is_exceeded());

        acc.add(500).unwrap();
        assert_eq!(acc.total(), 500);
        assert_eq!(acc.remaining(), 500);
    }

    #[test]
    fn test_accumulator_limit_exceeded() {
        let mut acc = CostAccumulator::new(1000);

        acc.add(500).unwrap();
        acc.add(400).unwrap();

        // This should fail
        let result = acc.add(200);
        assert!(matches!(result, Err(CostError::LimitExceeded { .. })));

        // Cost should still be accumulated (for reporting)
        assert_eq!(acc.total(), 1100);
        assert!(acc.is_exceeded());
    }

    #[test]
    fn test_accumulator_exact_limit() {
        let mut acc = CostAccumulator::new(1000);

        acc.add(500).unwrap();
        acc.add(500).unwrap(); // Exactly at limit

        assert_eq!(acc.total(), 1000);
        assert!(!acc.is_exceeded());
        assert_eq!(acc.remaining(), 0);

        // One more should fail
        assert!(acc.add(1).is_err());
    }

    #[test]
    fn test_accumulator_would_exceed() {
        let acc = CostAccumulator::new(1000);

        assert!(!acc.would_exceed(500));
        assert!(!acc.would_exceed(1000));
        assert!(acc.would_exceed(1001));
    }

    #[test]
    fn test_accumulator_reset() {
        let mut acc = CostAccumulator::new(1000);

        acc.add(800).unwrap();
        assert_eq!(acc.total(), 800);

        acc.reset();
        assert_eq!(acc.total(), 0);
        assert_eq!(acc.remaining(), 1000);
    }

    #[test]
    fn test_accumulator_for_transaction() {
        let acc = CostAccumulator::for_transaction();
        assert_eq!(acc.limit(), CostConstants::MAX_TX_COST);
    }

    #[test]
    fn test_accumulator_for_block() {
        let acc = CostAccumulator::for_block();
        assert_eq!(acc.limit(), CostConstants::MAX_BLOCK_COST);
    }

    #[test]
    fn test_accumulator_add_unchecked() {
        let mut acc = CostAccumulator::new(1000);

        acc.add_unchecked(500);
        acc.add_unchecked(600); // Exceeds limit but doesn't error

        assert_eq!(acc.total(), 1100);
        assert!(acc.is_exceeded());
    }

    #[test]
    fn test_accumulator_saturating_add() {
        let mut acc = CostAccumulator::new(u64::MAX);

        acc.add_unchecked(u64::MAX);
        acc.add_unchecked(1000); // Should saturate, not overflow

        assert_eq!(acc.total(), u64::MAX);
    }

    // ============ Cost Calculation Tests ============

    #[test]
    fn test_calculate_base_cost() {
        // 2 inputs, 3 outputs, 1 data input, 100 bytes
        let cost = calculate_base_cost(2, 3, 1, 100);

        let expected = 2 * CostConstants::INPUT_BASE_COST
            + 3 * CostConstants::OUTPUT_BASE_COST
            + 1 * CostConstants::DATA_INPUT_COST
            + 100 * CostConstants::SIZE_COST_PER_BYTE;

        assert_eq!(cost, expected);
        assert_eq!(cost, 2 * 2000 + 3 * 100 + 1 * 100 + 100 * 2);
        assert_eq!(cost, 4000 + 300 + 100 + 200);
        assert_eq!(cost, 4600);
    }

    #[test]
    fn test_calculate_base_cost_empty() {
        let cost = calculate_base_cost(0, 0, 0, 0);
        assert_eq!(cost, 0);
    }

    #[test]
    fn test_estimate_tx_cost() {
        let estimate = estimate_tx_cost(2, 3, 0, 200);

        // Should include base costs plus estimated script cost
        let base = calculate_base_cost(2, 3, 0, 200);
        let script_estimate = 2 * 1000; // 2 inputs * 1000

        assert_eq!(estimate, base + script_estimate);
    }

    #[test]
    fn test_estimate_tx_cost_minimum() {
        // Even empty transaction should have minimum cost
        let estimate = estimate_tx_cost(0, 0, 0, 0);
        assert_eq!(estimate, CostConstants::MIN_TX_COST);
    }

    // ============ TransactionCostResult Tests ============

    #[test]
    fn test_transaction_cost_result() {
        let result = TransactionCostResult::new(4000, 200, 50000, vec![25000, 25000]);

        assert_eq!(result.total_cost, 4000 + 200 + 50000);
        assert_eq!(result.base_cost, 4000);
        assert_eq!(result.size_cost, 200);
        assert_eq!(result.script_cost, 50000);
        assert!(!result.exceeds_tx_limit());
    }

    #[test]
    fn test_transaction_cost_exceeds_limit() {
        let result = TransactionCostResult::new(0, 0, CostConstants::MAX_TX_COST + 1, vec![]);
        assert!(result.exceeds_tx_limit());
    }

    // ============ Constants Tests ============

    #[test]
    fn test_cost_constants_values() {
        // Verify constants match expected values from Ergo spec
        assert_eq!(CostConstants::MAX_TX_COST, 1_000_000);
        assert_eq!(CostConstants::MAX_BLOCK_COST, 8_000_000);
        assert_eq!(CostConstants::INPUT_BASE_COST, 2_000);
        assert_eq!(CostConstants::OUTPUT_BASE_COST, 100);
        assert_eq!(CostConstants::DATA_INPUT_COST, 100);
        assert_eq!(CostConstants::SIZE_COST_PER_BYTE, 2);
        assert_eq!(CostConstants::MIN_TX_COST, 1_000);
    }

    #[test]
    fn test_block_can_fit_multiple_max_txs() {
        // Block should fit at least 8 max-cost transactions
        let num_max_txs = CostConstants::MAX_BLOCK_COST / CostConstants::MAX_TX_COST;
        assert!(num_max_txs >= 8);
    }

    // ============ Edge Case Tests ============

    #[test]
    fn test_large_transaction() {
        // Simulate a large transaction
        let cost = calculate_base_cost(
            100,   // Many inputs
            200,   // Many outputs
            50,    // Many data inputs
            50000, // Large size
        );

        // Should be significant but calculable
        assert!(cost > 0);
        assert!(cost < u64::MAX);
    }

    #[test]
    fn test_cost_accumulator_multiple_transactions() {
        let mut block_acc = CostAccumulator::for_block();

        // Simulate adding multiple transaction costs
        for _ in 0..5 {
            let tx_cost = 500_000; // Half of max tx cost
            block_acc.add(tx_cost).unwrap();
        }

        // Should have used 2.5M of 8M budget
        assert_eq!(block_acc.total(), 2_500_000);
        assert_eq!(block_acc.remaining(), 5_500_000);
    }
}
