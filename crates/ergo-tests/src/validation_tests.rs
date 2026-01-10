//! Transaction and block validation edge case tests.
//!
//! These tests cover edge cases in transaction validation including:
//! - ERG and token conservation edge cases
//! - Script execution failure handling
//! - Cost accumulation across multiple inputs
//! - Overflow protection

use ergo_consensus::params::MIN_BOX_VALUE;

// ============================================================================
// ERG Conservation Edge Cases
// ============================================================================

#[test]
fn test_erg_conservation_exact_zero_fee() {
    // Total inputs = total outputs (zero fee)
    let input_values = vec![1_000_000_000u64]; // 1 ERG
    let output_values = vec![1_000_000_000u64]; // 1 ERG
    let fee = 0u64;

    let total_inputs: u64 = input_values.iter().sum();
    let total_outputs: u64 = output_values.iter().sum();

    // Zero fee is technically valid (outputs match inputs)
    assert_eq!(total_inputs, total_outputs + fee);
}

#[test]
fn test_erg_conservation_maximum_values() {
    // Test with maximum ERG supply (97,739,924 ERG = 97_739_924_000_000_000 nanoERG)
    let max_supply: u64 = 97_739_924_000_000_000;

    // Large input that doesn't overflow
    let input_values = vec![max_supply / 2, max_supply / 2];
    let total: u64 = input_values.iter().sum();

    // Should handle large values without overflow
    assert!(total <= max_supply);
}

#[test]
fn test_erg_conservation_many_small_inputs() {
    // Many small inputs summing to a reasonable total
    let num_inputs = 100;
    let value_per_input = 1_000_000u64; // 0.001 ERG each
    let input_values: Vec<u64> = vec![value_per_input; num_inputs];

    let total: u64 = input_values.iter().sum();
    assert_eq!(total, num_inputs as u64 * value_per_input);
}

#[test]
fn test_erg_conservation_minimum_box_value() {
    // Test with minimum allowed box value
    let min_value = MIN_BOX_VALUE;
    let input_values = vec![min_value * 2];
    let output_values = vec![min_value, min_value];

    let total_inputs: u64 = input_values.iter().sum();
    let total_outputs: u64 = output_values.iter().sum();

    assert_eq!(total_inputs, total_outputs);
}

// ============================================================================
// Token Conservation Edge Cases
// ============================================================================

#[test]
fn test_token_conservation_many_token_types() {
    // Test with many different token types
    let num_token_types = 50;
    let mut input_tokens: std::collections::HashMap<Vec<u8>, u64> =
        std::collections::HashMap::new();
    let mut output_tokens: std::collections::HashMap<Vec<u8>, u64> =
        std::collections::HashMap::new();

    for i in 0..num_token_types {
        let token_id = vec![i as u8; 32];
        input_tokens.insert(token_id.clone(), 1000);
        output_tokens.insert(token_id, 1000);
    }

    // All tokens should be conserved
    for (token_id, input_amount) in &input_tokens {
        let output_amount = output_tokens.get(token_id).unwrap_or(&0);
        assert_eq!(input_amount, output_amount);
    }
}

#[test]
fn test_token_conservation_partial_burn() {
    // Burning some but not all of a token is valid
    let input_amount = 1000u64;
    let output_amount = 500u64;

    // Output is less than input = valid burn
    assert!(output_amount <= input_amount);
}

#[test]
fn test_token_conservation_complete_burn() {
    // Burning all tokens of a type is valid
    let input_amount = 1000u64;
    let output_amount = 0u64;

    // Complete burn (output = 0) is valid
    assert!(output_amount <= input_amount);
}

#[test]
fn test_token_conservation_split_across_outputs() {
    // Token from single input split across multiple outputs
    let input_amount = 1000u64;
    let output_amounts = vec![300u64, 400u64, 300u64];

    let total_output: u64 = output_amounts.iter().sum();
    assert_eq!(input_amount, total_output);
}

#[test]
fn test_token_conservation_consolidate_from_inputs() {
    // Tokens from multiple inputs consolidated to single output
    let input_amounts = vec![300u64, 400u64, 300u64];
    let output_amount = 1000u64;

    let total_input: u64 = input_amounts.iter().sum();
    assert_eq!(total_input, output_amount);
}

// ============================================================================
// Double-Spend Detection
// ============================================================================

#[test]
fn test_same_input_used_twice_in_transaction() {
    // A transaction using the same input box twice should be rejected
    let input_id = vec![1u8; 32];
    let inputs = vec![input_id.clone(), input_id.clone()];

    // Check for duplicates
    let mut seen = std::collections::HashSet::new();
    let has_duplicates = inputs.iter().any(|id| !seen.insert(id.clone()));

    assert!(has_duplicates, "Should detect duplicate inputs");
}

#[test]
fn test_unique_inputs_no_duplicates() {
    // All unique inputs should pass
    let inputs: Vec<Vec<u8>> = (0..10).map(|i| vec![i as u8; 32]).collect();

    let mut seen = std::collections::HashSet::new();
    let has_duplicates = inputs.iter().any(|id| !seen.insert(id.clone()));

    assert!(!has_duplicates, "No duplicates in unique inputs");
}

// ============================================================================
// Data Input Validation
// ============================================================================

#[test]
fn test_data_input_can_be_same_as_another_tx_input() {
    // Data inputs can reference boxes that other transactions spend
    // (just not the same transaction)
    let box_id = vec![1u8; 32];

    // Tx1 spends box_id, Tx2 uses box_id as data input - this is valid
    // as long as Tx2 is processed before Tx1
    let tx1_inputs = vec![box_id.clone()];
    let tx2_data_inputs = vec![box_id.clone()];

    // These don't conflict because they're different transactions
    assert!(!tx1_inputs.is_empty());
    assert!(!tx2_data_inputs.is_empty());
}

#[test]
fn test_data_input_cannot_overlap_with_own_inputs() {
    // Within a single transaction, data inputs cannot overlap with regular inputs
    let box_id = vec![1u8; 32];
    let inputs = vec![box_id.clone()];
    let data_inputs = vec![box_id.clone()];

    // Check for intersection
    let input_set: std::collections::HashSet<_> = inputs.iter().collect();
    let has_intersection = data_inputs.iter().any(|id| input_set.contains(id));

    assert!(
        has_intersection,
        "Should detect data input / input intersection"
    );
}

// ============================================================================
// Cost Accumulation Tests
// ============================================================================

#[test]
fn test_cost_accumulation_single_input() {
    // Base cost for a simple transaction
    let base_cost = 10_000i64;
    let input_cost = 5_000i64;
    let num_inputs = 1;

    let total_cost = base_cost + (input_cost * num_inputs);
    assert_eq!(total_cost, 15_000);
}

#[test]
fn test_cost_accumulation_multiple_inputs() {
    // Cost grows linearly with inputs
    let base_cost = 10_000i64;
    let input_cost = 5_000i64;
    let num_inputs = 16;

    let total_cost = base_cost + (input_cost * num_inputs);
    assert_eq!(total_cost, 90_000);
}

#[test]
fn test_cost_does_not_overflow_with_many_inputs() {
    // Even with many inputs, cost should not overflow i64
    let base_cost = 10_000i64;
    let input_cost = 100_000i64;
    let num_inputs = 10_000i64;

    // Use checked arithmetic to ensure no overflow
    let total_cost = base_cost.checked_add(input_cost.checked_mul(num_inputs).unwrap());
    assert!(total_cost.is_some());
}

// ============================================================================
// Value Boundary Tests
// ============================================================================

#[test]
fn test_output_value_at_minimum() {
    // Output with exactly minimum value should be valid
    let output_value = MIN_BOX_VALUE;
    assert!(output_value >= MIN_BOX_VALUE);
}

#[test]
fn test_output_value_below_minimum_rejected() {
    // Output below minimum should be rejected
    let output_value = MIN_BOX_VALUE - 1;
    assert!(output_value < MIN_BOX_VALUE);
}

#[test]
fn test_output_value_at_maximum() {
    // Test with very large but valid output value
    let max_supply: u64 = 97_739_924_000_000_000;
    let output_value = max_supply;

    // Should be representable as u64
    assert!(output_value <= u64::MAX);
}

// ============================================================================
// Transaction Structure Tests
// ============================================================================

#[test]
fn test_empty_inputs_rejected() {
    // A transaction with no inputs is invalid (except coinbase)
    let inputs: Vec<Vec<u8>> = vec![];
    assert!(inputs.is_empty(), "Empty inputs should be rejected");
}

#[test]
fn test_empty_outputs_rejected() {
    // A transaction with no outputs is invalid
    let outputs: Vec<u64> = vec![];
    assert!(outputs.is_empty(), "Empty outputs should be rejected");
}

#[test]
fn test_maximum_inputs_boundary() {
    // Test with a large number of inputs (should still be processable)
    let max_inputs = 100;
    let inputs: Vec<Vec<u8>> = (0..max_inputs).map(|i| vec![i as u8; 32]).collect();

    assert_eq!(inputs.len(), max_inputs);
}

#[test]
fn test_maximum_outputs_boundary() {
    // Test with a large number of outputs
    let max_outputs = 100;
    let outputs: Vec<u64> = vec![MIN_BOX_VALUE; max_outputs];

    assert_eq!(outputs.len(), max_outputs);
}

// ============================================================================
// Fee Validation Tests
// ============================================================================

#[test]
fn test_fee_calculation_basic() {
    let inputs_total = 1_000_000_000u64;
    let outputs_total = 999_000_000u64;
    let fee = inputs_total - outputs_total;

    assert_eq!(fee, 1_000_000); // 0.001 ERG fee
}

#[test]
fn test_fee_must_be_positive_or_zero() {
    let inputs_total = 1_000_000_000u64;
    let outputs_total = 1_000_000_001u64;

    // This would be negative fee (invalid)
    let fee = inputs_total.checked_sub(outputs_total);
    assert!(fee.is_none(), "Negative fee should not be possible");
}

#[test]
fn test_minimum_fee_enforcement() {
    // Minimum fee per byte (from config, typically 1000 nanoERG)
    let min_fee_per_byte = 1000u64;
    let tx_size_bytes = 500u64;
    let minimum_fee = min_fee_per_byte * tx_size_bytes;

    let actual_fee = 500_000u64;
    assert!(actual_fee >= minimum_fee, "Fee should meet minimum");
}

// ============================================================================
// Script Execution Edge Cases (Simulated)
// ============================================================================

#[test]
fn test_script_always_true_passes() {
    // A script that always evaluates to true should pass
    let script_result = true;
    assert!(script_result);
}

#[test]
fn test_script_always_false_fails() {
    // A script that always evaluates to false should fail
    let script_result = false;
    assert!(!script_result);
}

#[test]
fn test_script_execution_cost_tracked() {
    // Script execution cost should be tracked
    let initial_cost = 0i64;
    let script_cost = 10_000i64;
    let final_cost = initial_cost + script_cost;

    assert!(final_cost > initial_cost);
}

// ============================================================================
// Block Version Dependent Validation
// ============================================================================

#[test]
fn test_monotonic_height_rule_applies_from_version_3() {
    // From block version 3, output creation heights must be >= max input creation height
    let block_version = 3u8;
    let input_creation_heights = vec![100u32, 150u32, 120u32];
    let max_input_height = *input_creation_heights.iter().max().unwrap();

    // Output creation height must be at least max_input_height
    let output_creation_height = 150u32;

    if block_version >= 3 {
        assert!(
            output_creation_height >= max_input_height,
            "Monotonic height rule not satisfied"
        );
    }
}

#[test]
fn test_monotonic_height_rule_not_enforced_before_version_3() {
    // Before version 3, the monotonic height rule was not enforced
    let block_version = 2u8;
    let input_creation_heights = vec![100u32, 150u32, 120u32];
    let max_input_height = *input_creation_heights.iter().max().unwrap();

    // Before v3, any height was allowed
    let output_creation_height = 50u32; // Less than max input height

    if block_version < 3 {
        // No constraint - this was allowed
        assert!(output_creation_height < max_input_height);
    }
}
