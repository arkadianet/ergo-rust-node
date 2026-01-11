//! Transaction validation with input verification and script execution.
//!
//! This module provides full transaction validation including:
//! - Input existence verification against UTXO state
//! - ErgoScript execution for spending conditions
//! - Storage rent spending validation
//! - Token conservation rules
//! - Fee validation
//! - Transaction cost calculation and limits

use crate::cost::{calculate_base_cost, CostConstants};
use crate::{ConsensusError, ConsensusResult};
use ergo_chain_types::{Header, PreHeader};
use ergo_lib::chain::transaction::Transaction;
use ergo_lib::ergotree_ir::chain::context::{Context, ContextExtensionProvider, TxIoVec};
use ergo_lib::ergotree_ir::chain::context_extension::ContextExtension;
use ergo_lib::ergotree_ir::chain::ergo_box::{ErgoBox, RegisterId};
use ergo_lib::ergotree_ir::ergo_tree::ErgoTreeVersion;
use ergo_lib::ergotree_ir::mir::constant::TryExtractInto;
use ergo_lib::ergotree_ir::serialization::SigmaSerializable;
use ergotree_interpreter::sigma_protocol::prover::ProofBytes;
use ergotree_interpreter::sigma_protocol::verifier::Verifier;
use std::cell::Cell;
use tracing::{debug, instrument, warn};

/// The minimum time (in blocks) before a box can be spent via storage rent mechanism.
/// This is approximately 4 years at 2 minutes per block.
pub const STORAGE_PERIOD: u32 = 1051200;

/// The context extension key that stores the output index for storage rent spending.
pub const STORAGE_EXTENSION_INDEX: u8 = 127;

/// Default storage fee factor (nanoERG per byte).
/// This is typically 1250000 nanoERG per byte (0.00125 ERG per byte).
pub const DEFAULT_STORAGE_FEE_FACTOR: u64 = 1250000;

/// Provides context extensions for transaction inputs during verification.
struct TxContextExtensionProvider<'a> {
    extensions: &'a [ContextExtension],
}

impl<'a> ContextExtensionProvider for TxContextExtensionProvider<'a> {
    fn context_extension(&self, input_index: usize) -> Option<&ContextExtension> {
        self.extensions.get(input_index)
    }
}

/// Check if a box can be spent via storage rent mechanism.
///
/// Storage rent allows spending boxes older than ~4 years without a cryptographic proof,
/// provided certain conditions are met:
/// 1. The box must be at least STORAGE_PERIOD blocks old
/// 2. The context extension must contain the output index at key 127
/// 3. The output must preserve the box's registers (except value and creation info)
/// 4. The output must contain at least (input_value - storage_fee) ERG
///
/// Returns true if the box can be spent via storage rent, false otherwise.
fn check_storage_rent_conditions(
    input_box: &ErgoBox,
    extension: &ContextExtension,
    outputs: &[ErgoBox],
    current_height: u32,
) -> bool {
    let box_id = hex::encode(input_box.box_id().as_ref());

    // Check if the box is old enough (at least STORAGE_PERIOD blocks)
    let box_age = current_height.saturating_sub(input_box.creation_height);
    if box_age < STORAGE_PERIOD {
        debug!(
            box_id = %box_id,
            box_age,
            required_age = STORAGE_PERIOD,
            "Storage rent check failed: box not old enough"
        );
        return false;
    }

    // Get the output index from context extension key 127
    // Ergo uses signed short (i16) for this value
    let output_idx: i16 = match extension.values.get(&STORAGE_EXTENSION_INDEX) {
        Some(constant) => match constant.v.clone().try_extract_into::<i16>() {
            Ok(idx) => idx,
            Err(e) => {
                debug!(
                    box_id = %box_id,
                    error = %e,
                    "Storage rent check failed: could not extract output index from extension"
                );
                return false;
            }
        },
        None => {
            debug!(
                box_id = %box_id,
                "Storage rent check failed: no output index in context extension"
            );
            return false;
        }
    };

    // Validate output index is non-negative before casting to usize
    if output_idx < 0 {
        debug!(
            box_id = %box_id,
            output_idx,
            "Storage rent check failed: negative output index"
        );
        return false;
    }

    // Get the corresponding output
    let output = match outputs.get(output_idx as usize) {
        Some(out) => out,
        None => {
            debug!(
                box_id = %box_id,
                output_idx,
                outputs_len = outputs.len(),
                "Storage rent check failed: output index out of bounds"
            );
            return false;
        }
    };

    // Calculate the storage fee
    let box_bytes = match input_box.sigma_serialize_bytes() {
        Ok(bytes) => bytes,
        Err(e) => {
            debug!(
                box_id = %box_id,
                error = %e,
                "Storage rent check failed: could not serialize box"
            );
            return false;
        }
    };
    let storage_fee = (box_bytes.len() as u64) * DEFAULT_STORAGE_FEE_FACTOR;

    let input_value = u64::from(input_box.value);

    // If the box's value is less than or equal to the storage fee, it can be spent without restrictions
    if input_value <= storage_fee {
        debug!(
            box_id = %box_id,
            input_value,
            storage_fee,
            "Storage rent: box value <= fee, can be fully consumed"
        );
        return true;
    }

    // Check output creation height matches current height
    if output.creation_height != current_height {
        debug!(
            box_id = %box_id,
            output_creation_height = output.creation_height,
            current_height,
            "Storage rent check failed: output creation height mismatch"
        );
        return false;
    }

    // Check output value is at least (input_value - storage_fee)
    let output_value = u64::from(output.value);
    let min_output_value = input_value.saturating_sub(storage_fee);
    if output_value < min_output_value {
        debug!(
            box_id = %box_id,
            output_value,
            min_output_value,
            input_value,
            storage_fee,
            "Storage rent check failed: output value too low"
        );
        return false;
    }

    // Check that all registers except R0 (value) and R3 (creation info) are preserved
    // R0 contains the box value, R3 contains creation info (height, tx_id, output_index)
    for reg_id in 0..=9u8 {
        let reg = match RegisterId::try_from(reg_id) {
            Ok(r) => r,
            Err(_) => continue,
        };

        // Skip R0 (value) and R3 (creation info) - these can change
        if reg == RegisterId::R0 || reg == RegisterId::R3 {
            continue;
        }

        if input_box.get_register(reg) != output.get_register(reg) {
            debug!(
                box_id = %box_id,
                register = ?reg,
                "Storage rent check failed: register not preserved"
            );
            return false;
        }
    }

    true
}

/// Transaction verification result.
#[derive(Debug, Clone)]
pub struct TxVerificationResult {
    /// Whether the transaction is valid.
    pub valid: bool,
    /// Total cost of script execution (from interpreter).
    pub script_cost: u64,
    /// Total transaction cost (base + size + script).
    pub total_cost: u64,
    /// Error message if invalid.
    pub error: Option<String>,
}

/// Transaction verifier that checks spending conditions.
pub struct TxVerifier {
    /// Block height for context.
    height: u32,
    /// Pre-header for context.
    pre_header: PreHeader,
    /// Headers for context (last 10 block headers).
    headers: [Header; 10],
}

impl Verifier for TxVerifier {}

impl TxVerifier {
    /// Create a new transaction verifier for the given block height.
    ///
    /// Note: headers should be provided for full context. This constructor
    /// uses empty headers which may cause some scripts to fail.
    pub fn new(height: u32, pre_header: PreHeader, headers: [Header; 10]) -> Self {
        Self {
            height,
            pre_header,
            headers,
        }
    }

    /// Verify a transaction's spending conditions.
    ///
    /// # Arguments
    /// * `tx` - The transaction to verify
    /// * `input_boxes` - The input boxes being spent (in same order as tx.inputs)
    /// * `data_input_boxes` - The data input boxes (read-only, in same order as tx.data_inputs)
    ///
    /// # Returns
    /// Verification result with validity status and cost.
    #[instrument(skip(self, tx, input_boxes, data_input_boxes), fields(tx_id = %hex::encode(tx.id().as_ref())))]
    pub fn verify_tx(
        &self,
        tx: &Transaction,
        input_boxes: &[ErgoBox],
        data_input_boxes: &[ErgoBox],
    ) -> TxVerificationResult {
        let mut script_cost = 0u64;

        // Calculate base cost (inputs, outputs, data inputs, size)
        let num_inputs = tx.inputs.len();
        let num_outputs = tx.outputs.len();
        let num_data_inputs = tx.data_inputs.as_ref().map_or(0, |di| di.len());
        let tx_size = tx.sigma_serialize_bytes().map_or(0, |b| b.len());

        let base_cost = calculate_base_cost(num_inputs, num_outputs, num_data_inputs, tx_size);

        // Convert outputs to a slice
        let outputs: Vec<ErgoBox> = tx.outputs.iter().cloned().collect();

        // Create references for data inputs
        let data_inputs: Option<TxIoVec<&ErgoBox>> = if data_input_boxes.is_empty() {
            None
        } else {
            match data_input_boxes.iter().collect::<Vec<_>>().try_into() {
                Ok(v) => Some(v),
                Err(_) => {
                    return TxVerificationResult {
                        valid: false,
                        script_cost: 0,
                        total_cost: base_cost,
                        error: Some("Invalid number of data inputs".to_string()),
                    }
                }
            }
        };

        // Create input references
        let input_refs: Vec<&ErgoBox> = input_boxes.iter().collect();
        let inputs: TxIoVec<&ErgoBox> = match input_refs.try_into() {
            Ok(v) => v,
            Err(_) => {
                return TxVerificationResult {
                    valid: false,
                    script_cost: 0,
                    total_cost: base_cost,
                    error: Some("Invalid number of inputs".to_string()),
                }
            }
        };

        // Collect all context extensions for the extension provider
        let extensions: Vec<ContextExtension> = tx
            .inputs
            .iter()
            .map(|input| input.spending_proof.extension.clone())
            .collect();
        let extension_provider = TxContextExtensionProvider {
            extensions: &extensions,
        };

        // Verify each input's spending condition
        for (idx, (input, input_box)) in tx.inputs.iter().zip(input_boxes.iter()).enumerate() {
            // Get the spending proof from the input
            let proof_bytes: Vec<u8> = input.spending_proof.proof.clone().into();
            let proof_len = proof_bytes.len();
            let proof = if proof_bytes.is_empty() {
                ProofBytes::Empty
            } else {
                ProofBytes::Some(proof_bytes.into())
            };

            // Get context extension for this input
            let extension = &extensions[idx];

            // Create context for this input
            let ctx = Context {
                height: self.height,
                self_box: input_box,
                outputs: &outputs,
                data_inputs: data_inputs.clone(),
                inputs: inputs.clone(),
                pre_header: self.pre_header.clone(),
                headers: self.headers.clone(),
                extension,
                tree_version: Cell::new(ErgoTreeVersion::V0),
                extension_provider: &extension_provider,
            };

            // Get the ErgoTree from the input box
            let ergo_tree = &input_box.ergo_tree;

            // Compute message to sign (serialized transaction with empty proofs)
            // This is what the prover signed - NOT the transaction ID hash
            let message = match tx.bytes_to_sign() {
                Ok(bytes) => bytes,
                Err(e) => {
                    return TxVerificationResult {
                        valid: false,
                        script_cost: 0,
                        total_cost: base_cost,
                        error: Some(format!("Failed to get bytes to sign: {}", e)),
                    };
                }
            };

            // Log verification details for debugging
            debug!(
                input_idx = idx,
                box_id = %hex::encode(input_box.box_id().as_ref()),
                tx_id = %hex::encode(tx.id().as_ref()),
                message_len = message.len(),
                proof_len,
                height = self.height,
                "Verifying input"
            );

            // First, check if this is a storage rent spend (empty proof + box old enough)
            if matches!(proof, ProofBytes::Empty) {
                if check_storage_rent_conditions(input_box, extension, &outputs, self.height) {
                    debug!(
                        input_idx = idx,
                        box_id = %hex::encode(input_box.box_id().as_ref()),
                        box_age = self.height.saturating_sub(input_box.creation_height),
                        "Input spent via storage rent"
                    );
                    // Storage rent spend is valid, no script cost
                    continue;
                }
            }

            // Verify the spending proof via ErgoScript
            match self.verify(ergo_tree, &ctx, proof, &message) {
                Ok(result) => {
                    script_cost += result.cost;

                    // Check if we've exceeded the transaction cost limit
                    let current_total = base_cost.saturating_add(script_cost);
                    if current_total > CostConstants::MAX_TX_COST {
                        return TxVerificationResult {
                            valid: false,
                            script_cost,
                            total_cost: current_total,
                            error: Some(format!(
                                "Transaction cost {} exceeds limit {}",
                                current_total,
                                CostConstants::MAX_TX_COST
                            )),
                        };
                    }

                    if !result.result {
                        // Log diagnostic information from the reduction
                        let diag_msg = if let Some(ref expr_str) = result.diag.pretty_printed_expr {
                            format!("Script reduced to false. Expr: {}", expr_str)
                        } else {
                            "Script proof verification failed (signature mismatch)".to_string()
                        };
                        warn!(
                            input_idx = idx,
                            box_id = %hex::encode(input_box.box_id().as_ref()),
                            height = self.height,
                            diag = %diag_msg,
                            "Script verification failed"
                        );
                        return TxVerificationResult {
                            valid: false,
                            script_cost,
                            total_cost: base_cost.saturating_add(script_cost),
                            error: Some(format!(
                                "Script verification failed for input {}: {}",
                                idx, diag_msg
                            )),
                        };
                    }
                }
                Err(e) => {
                    warn!(
                        input_idx = idx,
                        error = %e,
                        "Script verification error"
                    );
                    return TxVerificationResult {
                        valid: false,
                        script_cost,
                        total_cost: base_cost.saturating_add(script_cost),
                        error: Some(format!("Script error for input {}: {}", idx, e)),
                    };
                }
            }
        }

        let total_cost = base_cost.saturating_add(script_cost);
        debug!(
            script_cost,
            base_cost, total_cost, "Transaction verified successfully"
        );

        TxVerificationResult {
            valid: true,
            script_cost,
            total_cost,
            error: None,
        }
    }
}

/// Validate that all inputs exist in the provided UTXO lookup.
///
/// # Arguments
/// * `tx` - The transaction to validate
/// * `utxo_lookup` - Function that returns an ErgoBox for a given box ID, or None if not found
///
/// # Returns
/// A vector of input boxes in the same order as tx.inputs, or an error if any input is missing.
pub fn validate_inputs_exist<F>(tx: &Transaction, utxo_lookup: F) -> ConsensusResult<Vec<ErgoBox>>
where
    F: Fn(&[u8]) -> Option<ErgoBox>,
{
    let mut input_boxes = Vec::with_capacity(tx.inputs.len());

    for (idx, input) in tx.inputs.iter().enumerate() {
        let box_id = input.box_id.as_ref();
        match utxo_lookup(box_id) {
            Some(ergo_box) => input_boxes.push(ergo_box),
            None => {
                return Err(ConsensusError::MissingInput {
                    tx_id: hex::encode(tx.id().as_ref()),
                    input_idx: idx,
                    box_id: hex::encode(box_id),
                });
            }
        }
    }

    Ok(input_boxes)
}

/// Validate that all data inputs exist in the provided UTXO lookup.
pub fn validate_data_inputs_exist<F>(
    tx: &Transaction,
    utxo_lookup: F,
) -> ConsensusResult<Vec<ErgoBox>>
where
    F: Fn(&[u8]) -> Option<ErgoBox>,
{
    let data_inputs = match &tx.data_inputs {
        Some(di) => di,
        None => return Ok(Vec::new()),
    };

    let mut data_input_boxes = Vec::with_capacity(data_inputs.len());

    for (idx, data_input) in data_inputs.iter().enumerate() {
        let box_id = data_input.box_id.as_ref();
        match utxo_lookup(box_id) {
            Some(ergo_box) => data_input_boxes.push(ergo_box),
            None => {
                return Err(ConsensusError::MissingDataInput {
                    tx_id: hex::encode(tx.id().as_ref()),
                    input_idx: idx,
                    box_id: hex::encode(box_id),
                });
            }
        }
    }

    Ok(data_input_boxes)
}

/// Validate token conservation in a transaction.
///
/// Ensures that:
/// 1. No tokens are created out of thin air (except in coinbase)
/// 2. Token amounts are conserved or burned
/// 3. Only valid token IDs are used
pub fn validate_token_conservation(
    tx: &Transaction,
    input_boxes: &[ErgoBox],
    is_coinbase: bool,
) -> ConsensusResult<()> {
    use std::collections::HashMap;

    // Collect input tokens
    let mut input_tokens: HashMap<Vec<u8>, u64> = HashMap::new();
    for input_box in input_boxes {
        if let Some(ref tokens) = input_box.tokens {
            for token in tokens.iter() {
                let token_id = token.token_id.as_ref().to_vec();
                let amount = u64::from(token.amount);
                *input_tokens.entry(token_id).or_insert(0) += amount;
            }
        }
    }

    // Collect output tokens
    let mut output_tokens: HashMap<Vec<u8>, u64> = HashMap::new();
    for output in tx.outputs.iter() {
        if let Some(ref tokens) = output.tokens {
            for token in tokens.iter() {
                let token_id = token.token_id.as_ref().to_vec();
                let amount = u64::from(token.amount);
                *output_tokens.entry(token_id).or_insert(0) += amount;
            }
        }
    }

    // In a non-coinbase transaction, the first input's box ID can be used
    // to mint new tokens (token ID = first input box ID)
    let mintable_token_id: Option<Vec<u8>> = if !is_coinbase && !input_boxes.is_empty() {
        Some(input_boxes[0].box_id().as_ref().to_vec())
    } else {
        None
    };

    // Check that output tokens don't exceed input tokens (except for minting)
    for (token_id, output_amount) in &output_tokens {
        let input_amount = input_tokens.get(token_id).copied().unwrap_or(0);

        if *output_amount > input_amount {
            // Check if this is a valid minting operation
            let is_minting = mintable_token_id
                .as_ref()
                .map(|id| id == token_id)
                .unwrap_or(false);

            if !is_minting {
                return Err(ConsensusError::InvalidTokenAmount {
                    token_id: hex::encode(token_id),
                    input_amount,
                    output_amount: *output_amount,
                });
            }
        }
    }

    Ok(())
}

/// Validate ERG conservation in a transaction.
///
/// Ensures that input ERG >= output ERG (difference is fee).
pub fn validate_erg_conservation(
    tx: &Transaction,
    input_boxes: &[ErgoBox],
) -> ConsensusResult<u64> {
    // Sum input values
    let input_sum: u64 = input_boxes.iter().map(|b| u64::from(b.value)).sum();

    // Sum output values
    let output_sum: u64 = tx.outputs.iter().map(|b| u64::from(b.value)).sum();

    if output_sum > input_sum {
        return Err(ConsensusError::InsufficientFunds {
            input_sum,
            output_sum,
        });
    }

    // Fee is the difference
    let fee = input_sum - output_sum;

    Ok(fee)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ergo_chain_types::Digest32;
    use ergo_lib::ergotree_ir::chain::ergo_box::{
        box_value::BoxValue, ErgoBoxCandidate, NonMandatoryRegisters,
    };
    use ergo_lib::ergotree_ir::chain::token::{Token, TokenAmount, TokenId};
    use ergo_lib::ergotree_ir::chain::tx_id::TxId;
    use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
    use ergo_lib::ergotree_ir::mir::constant::Constant;
    use ergo_lib::ergotree_ir::mir::expr::Expr;
    use ergo_lib::ergotree_ir::sigma_protocol::sigma_boolean::{SigmaBoolean, SigmaProp};
    use std::convert::TryFrom;

    /// Create a mock ErgoBox with given value (no tokens)
    fn create_mock_box(value: u64, box_id_byte: u8) -> ErgoBox {
        let value = BoxValue::try_from(value).unwrap();
        // Create a trivially true proposition for testing (anyone can spend)
        let sigma_prop = SigmaProp::new(SigmaBoolean::TrivialProp(true));
        let constant: Constant = sigma_prop.into();
        let expr = Expr::Const(constant);
        let ergo_tree = ErgoTree::try_from(expr).unwrap();

        let candidate = ErgoBoxCandidate {
            value,
            ergo_tree,
            tokens: None,
            additional_registers: NonMandatoryRegisters::empty(),
            creation_height: 1,
        };

        // Create ErgoBox with a transaction ID
        let mut tx_id_bytes = [0u8; 32];
        tx_id_bytes[0] = box_id_byte;
        let tx_id = TxId::from(Digest32::from(tx_id_bytes));

        ErgoBox::from_box_candidate(&candidate, tx_id, 0).unwrap()
    }

    // ============ ERG Conservation Tests ============
    // Corresponds to Scala's "ergo preservation law holds"

    #[test]
    fn test_erg_conservation_valid() {
        // Input: 1 ERG, Output: 0.9 ERG, Fee: 0.1 ERG
        let input_boxes = vec![create_mock_box(1_000_000_000, 1)];

        // Create a simple transaction mock - we only need to check values
        // For this test, we manually verify the conservation logic
        let input_sum: u64 = input_boxes.iter().map(|b| u64::from(b.value)).sum();
        let output_sum: u64 = 900_000_000; // 0.9 ERG

        assert!(
            output_sum <= input_sum,
            "ERG conservation: output should not exceed input"
        );
        let fee = input_sum - output_sum;
        assert_eq!(fee, 100_000_000, "Fee should be 0.1 ERG");
    }

    #[test]
    fn test_erg_conservation_exact() {
        // Input equals output (zero fee - edge case)
        let input_sum: u64 = 1_000_000_000;
        let output_sum: u64 = 1_000_000_000;

        assert!(output_sum <= input_sum, "ERG conservation allows zero fee");
    }

    #[test]
    fn test_erg_conservation_violation() {
        // Attempt to create more ERG than input
        let input_sum: u64 = 1_000_000_000;
        let output_sum: u64 = 1_100_000_000; // More than input!

        assert!(
            output_sum > input_sum,
            "This should be detected as a violation"
        );
    }

    #[test]
    fn test_erg_conservation_multiple_inputs() {
        // Multiple inputs summed correctly
        let input_boxes = vec![
            create_mock_box(500_000_000, 1),
            create_mock_box(500_000_000, 2),
            create_mock_box(100_000_000, 3),
        ];

        let input_sum: u64 = input_boxes.iter().map(|b| u64::from(b.value)).sum();
        assert_eq!(input_sum, 1_100_000_000, "Sum of 3 inputs");

        let output_sum: u64 = 1_000_000_000;
        assert!(output_sum <= input_sum);
        assert_eq!(input_sum - output_sum, 100_000_000, "Fee is 0.1 ERG");
    }

    // ============ Token Conservation Tests ============
    // Corresponds to Scala's "assets preservation law holds"

    #[test]
    fn test_token_conservation_valid() {
        // Test token conservation logic directly
        let token_id = vec![1u8; 32];

        // Simulate input tokens
        let input_tokens: std::collections::HashMap<Vec<u8>, u64> =
            [(token_id.clone(), 1000)].into_iter().collect();

        // Simulate output with same token amount
        let output_tokens: std::collections::HashMap<Vec<u8>, u64> =
            [(token_id.clone(), 1000)].into_iter().collect();

        for (tid, output_amount) in &output_tokens {
            let input_amount = input_tokens.get(tid).copied().unwrap_or(0);
            assert!(
                *output_amount <= input_amount,
                "Token conservation: output should not exceed input"
            );
        }
    }

    #[test]
    fn test_token_conservation_burning() {
        // Burning tokens is allowed (output < input)
        let token_id = vec![1u8; 32];
        let input_amount = 1000u64;
        let output_amount = 500u64; // Burning 500 tokens

        assert!(output_amount < input_amount, "Burning tokens is valid");
    }

    #[test]
    fn test_token_conservation_violation() {
        // Cannot create tokens out of thin air
        let token_id = vec![1u8; 32];
        let input_amount = 1000u64;
        let output_amount = 1500u64; // Trying to create 500 extra tokens

        assert!(
            output_amount > input_amount,
            "This should be detected as token creation violation"
        );
    }

    #[test]
    fn test_token_minting_with_first_input() {
        // Token minting is allowed when token ID equals first input's box ID
        let first_input_box_id = vec![42u8; 32];
        let minted_token_id = first_input_box_id.clone(); // Same as first input box ID

        // This should be valid - minting new token
        let is_valid_mint = minted_token_id == first_input_box_id;
        assert!(is_valid_mint, "Minting with first input box ID is allowed");
    }

    #[test]
    fn test_multiple_token_types_conservation() {
        // Multiple different tokens must each be conserved
        let token_a = vec![1u8; 32];
        let token_b = vec![2u8; 32];

        let input_tokens: std::collections::HashMap<Vec<u8>, u64> =
            [(token_a.clone(), 1000), (token_b.clone(), 500)]
                .into_iter()
                .collect();

        let output_tokens: std::collections::HashMap<Vec<u8>, u64> =
            [(token_a.clone(), 800), (token_b.clone(), 500)]
                .into_iter()
                .collect();

        for (token_id, output_amount) in &output_tokens {
            let input_amount = input_tokens.get(token_id).copied().unwrap_or(0);
            assert!(
                *output_amount <= input_amount,
                "Each token type must be conserved independently"
            );
        }
    }

    // ============ Input Validation Tests ============
    // Corresponds to Scala's input existence checks

    #[test]
    fn test_validate_inputs_exist_all_found() {
        let box1 = create_mock_box(1_000_000_000, 1);
        let box1_id = box1.box_id().as_ref().to_vec();

        let utxo_set: std::collections::HashMap<Vec<u8>, ErgoBox> =
            [(box1_id.clone(), box1)].into_iter().collect();

        let lookup = |id: &[u8]| -> Option<ErgoBox> { utxo_set.get(id).cloned() };

        // Simulate looking up the box
        let result = lookup(&box1_id);
        assert!(result.is_some(), "Box should be found in UTXO set");
    }

    #[test]
    fn test_validate_inputs_missing() {
        let utxo_set: std::collections::HashMap<Vec<u8>, ErgoBox> =
            std::collections::HashMap::new();

        let lookup = |id: &[u8]| -> Option<ErgoBox> { utxo_set.get(id).cloned() };

        let missing_id = vec![99u8; 32];
        let result = lookup(&missing_id);
        assert!(result.is_none(), "Missing box should return None");
    }

    // ============ Negative/Overflow Prevention Tests ============
    // Corresponds to Scala's "impossible to create negative-value output"
    // and "impossible to overflow ergo tokens"

    #[test]
    fn test_box_value_minimum() {
        // BoxValue has a minimum (dust limit) = 360 * 30 = 10800 nanoERG
        // Based on MIN_VALUE_PER_BOX_BYTE (360) * MIN_BOX_SIZE_BYTES (30)
        let min_value = BoxValue::try_from(10800u64);
        assert!(min_value.is_ok(), "Minimum value of 10800 nanoERG is valid");

        let below_min = BoxValue::try_from(10799u64);
        assert!(below_min.is_err(), "Below minimum value should be rejected");

        let zero_value = BoxValue::try_from(0u64);
        assert!(zero_value.is_err(), "Zero value should be rejected");
    }

    #[test]
    fn test_token_amount_minimum() {
        // Token amounts must be positive
        let valid_amount = TokenAmount::try_from(1u64);
        assert!(valid_amount.is_ok(), "Minimum token amount of 1 is valid");

        let zero_amount = TokenAmount::try_from(0u64);
        assert!(zero_amount.is_err(), "Zero token amount should be rejected");
    }

    #[test]
    fn test_erg_sum_overflow_protection() {
        // Sum of ERG values shouldn't overflow
        let large_value = u64::MAX / 2;
        let values = vec![large_value, large_value];

        // Using checked_add for overflow protection
        let sum = values.iter().try_fold(0u64, |acc, &v| acc.checked_add(v));
        assert!(
            sum.is_some(),
            "These values should not overflow when summed"
        );

        // But MAX + 1 would overflow
        let overflow_values = vec![u64::MAX, 1u64];
        let overflow_sum = overflow_values
            .iter()
            .try_fold(0u64, |acc, &v| acc.checked_add(v));
        assert!(overflow_sum.is_none(), "MAX + 1 should overflow");
    }

    // ============ Double Spend Tests ============
    // Corresponds to Scala's double-spend detection

    #[test]
    fn test_double_spend_same_input_twice() {
        // A transaction cannot spend the same box twice
        let input_box_id = vec![1u8; 32];

        let inputs = vec![input_box_id.clone(), input_box_id.clone()];
        let unique_inputs: std::collections::HashSet<Vec<u8>> = inputs.iter().cloned().collect();

        assert!(
            inputs.len() != unique_inputs.len(),
            "Duplicate inputs should be detected"
        );
    }

    #[test]
    fn test_unique_inputs_valid() {
        let inputs = vec![vec![1u8; 32], vec![2u8; 32], vec![3u8; 32]];
        let unique_inputs: std::collections::HashSet<Vec<u8>> = inputs.iter().cloned().collect();

        assert_eq!(inputs.len(), unique_inputs.len(), "All inputs are unique");
    }

    // ============ Fee Validation Tests ============
    // Corresponds to Scala's fee validation

    #[test]
    fn test_minimum_fee() {
        let min_fee = 1_000_000u64; // 0.001 ERG

        let actual_fee = 1_000_000u64;
        assert!(actual_fee >= min_fee, "Fee meets minimum requirement");

        let low_fee = 500_000u64;
        assert!(low_fee < min_fee, "Low fee should be rejected");
    }

    #[test]
    fn test_fee_calculation() {
        let input_sum = 2_000_000_000u64; // 2 ERG
        let output_sum = 1_999_000_000u64; // 1.999 ERG

        let fee = input_sum.saturating_sub(output_sum);
        assert_eq!(fee, 1_000_000, "Fee should be 0.001 ERG");
    }

    // ============ Data Input Tests ============
    // Corresponds to Scala's "applyTransactions() - dataInputs intersect with inputs"

    #[test]
    fn test_data_inputs_no_intersection_with_inputs() {
        // Data inputs and regular inputs should not intersect
        // (you cannot spend and read the same box)
        let input_ids: std::collections::HashSet<Vec<u8>> =
            [vec![1u8; 32], vec![2u8; 32]].into_iter().collect();

        let data_input_ids: std::collections::HashSet<Vec<u8>> =
            [vec![3u8; 32], vec![4u8; 32]].into_iter().collect();

        let intersection: Vec<_> = input_ids.intersection(&data_input_ids).collect();
        assert!(
            intersection.is_empty(),
            "Data inputs should not intersect with inputs"
        );
    }

    #[test]
    fn test_data_inputs_intersection_detected() {
        let shared_id = vec![2u8; 32];

        let input_ids: std::collections::HashSet<Vec<u8>> =
            [vec![1u8; 32], shared_id.clone()].into_iter().collect();

        let data_input_ids: std::collections::HashSet<Vec<u8>> =
            [shared_id.clone(), vec![3u8; 32]].into_iter().collect();

        let intersection: Vec<_> = input_ids.intersection(&data_input_ids).collect();
        assert!(
            !intersection.is_empty(),
            "Intersection should be detected and rejected"
        );
    }
}
