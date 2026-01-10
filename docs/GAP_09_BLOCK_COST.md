# GAP_09: Block Cost Validation

## Priority: MEDIUM
## Effort: Medium
## Category: Consensus

---

## Description

The Rust node has transaction validation but lacks complete block cost calculation and enforcement. Block cost limits prevent resource exhaustion attacks by limiting the computational complexity of scripts and total block processing time.

---

## Current State (Rust)

### What Exists

In `crates/ergo-consensus/src/tx_validation.rs`:
- Input/output validation
- Script execution via ergotree-interpreter
- Token and ERG conservation
- Basic validation framework

### What's Missing

1. **Script execution cost tracking** during validation
2. **Transaction cost limits** enforcement
3. **Block cost accumulation** across all transactions
4. **Cost limit constants** (JIT cost, script cost)
5. **Complexity-based mempool ordering**

---

## Scala Reference

### Cost Calculation

```scala
// File: ergo-core/src/main/scala/org/ergoplatform/validation/ValidationRules.scala

object ValidationRules {
  // Maximum script cost per transaction
  val MaxTxScriptCost = 1000000L
  
  // Maximum total cost per block
  val MaxBlockCost = 8000000L
  
  // JIT costing constants
  val JitCostPerByte = 10L
  val JitCostPerOp = 100L
}
```

### Transaction Cost Tracking

```scala
// File: ergotree-interpreter

class CostAccumulator(limit: Long) {
  private var currentCost: Long = 0
  
  def add(cost: Long): Unit = {
    currentCost += cost
    if (currentCost > limit) {
      throw new CostLimitException(currentCost, limit)
    }
  }
  
  def totalCost: Long = currentCost
}

// During script evaluation
def evaluate(expr: SValue, ctx: ErgoContext, costLimit: Long): Try[Any] = {
  val costAccumulator = new CostAccumulator(costLimit)
  
  expr match {
    case If(cond, t, f) =>
      costAccumulator.add(IfCost)
      // evaluate...
      
    case Collection(elems) =>
      costAccumulator.add(CollectionCost * elems.size)
      // evaluate...
  }
}
```

### Block Validation with Costs

```scala
// File: src/main/scala/org/ergoplatform/nodeView/history/BlockValidator.scala

def validateBlock(block: Block, state: UtxoState): ValidationResult = {
  var blockCost = 0L
  
  for (tx <- block.transactions) {
    // Calculate transaction cost
    val txCost = calculateTxCost(tx, state)
    
    // Check individual tx limit
    if (txCost > MaxTxScriptCost) {
      return Invalid(s"Transaction ${tx.id} exceeds cost limit")
    }
    
    blockCost += txCost
    
    // Check block limit
    if (blockCost > MaxBlockCost) {
      return Invalid(s"Block exceeds cost limit at tx ${tx.id}")
    }
  }
  
  Valid
}

def calculateTxCost(tx: Transaction, state: UtxoState): Long = {
  var cost = 0L
  
  // Base cost per input
  cost += tx.inputs.size * InputBaseCost
  
  // Script execution cost
  for (input <- tx.inputs) {
    val box = state.boxById(input.boxId)
    val scriptCost = executeScript(box.ergoTree, createContext(tx, input))
    cost += scriptCost
  }
  
  // Output creation cost
  cost += tx.outputs.size * OutputBaseCost
  
  cost
}
```

### Mempool Cost Ordering

```scala
// File: src/main/scala/org/ergoplatform/nodeView/mempool/ErgoMemPool.scala

def weightedPriority(tx: Transaction): Double = {
  val fee = tx.fee
  val cost = estimateCost(tx)
  val size = tx.bytes.length
  
  // Priority = fee / (cost + size penalty)
  fee.toDouble / (cost + size * SizeWeight)
}
```

---

## Impact

### Why Block Cost Matters

1. **DoS Prevention**: Prevents blocks with expensive scripts
2. **Predictable Validation**: Bounds validation time
3. **Fair Resource Usage**: Miners can estimate block profitability
4. **Network Stability**: Prevents nodes from getting stuck on expensive blocks

### Current Risks

- No protection against script complexity attacks
- Expensive transactions could slow validation
- No cost-based transaction ordering in mempool

---

## Implementation Plan

### Phase 1: Cost Constants (0.5 days)

1. **Define cost constants** in `crates/ergo-consensus/src/cost.rs`:
   ```rust
   /// Block and transaction cost limits
   pub struct CostConstants;
   
   impl CostConstants {
       /// Maximum script execution cost per transaction
       pub const MAX_TX_COST: u64 = 1_000_000;
       
       /// Maximum total cost per block
       pub const MAX_BLOCK_COST: u64 = 8_000_000;
       
       /// Base cost per input
       pub const INPUT_BASE_COST: u64 = 2000;
       
       /// Base cost per output
       pub const OUTPUT_BASE_COST: u64 = 100;
       
       /// Base cost per data input
       pub const DATA_INPUT_COST: u64 = 100;
       
       /// Cost per byte of transaction size
       pub const SIZE_COST_PER_BYTE: u64 = 10;
   }
   ```

### Phase 2: Cost Accumulator (1 day)

2. **Implement cost accumulator** in `crates/ergo-consensus/src/cost.rs`:
   ```rust
   use thiserror::Error;
   
   #[derive(Debug, Error)]
   #[error("Cost limit exceeded: {current} > {limit}")]
   pub struct CostLimitError {
       pub current: u64,
       pub limit: u64,
   }
   
   pub struct CostAccumulator {
       current: u64,
       limit: u64,
   }
   
   impl CostAccumulator {
       pub fn new(limit: u64) -> Self {
           Self { current: 0, limit }
       }
       
       pub fn add(&mut self, cost: u64) -> Result<(), CostLimitError> {
           self.current = self.current.saturating_add(cost);
           if self.current > self.limit {
               Err(CostLimitError {
                   current: self.current,
                   limit: self.limit,
               })
           } else {
               Ok(())
           }
       }
       
       pub fn total(&self) -> u64 {
           self.current
       }
       
       pub fn remaining(&self) -> u64 {
           self.limit.saturating_sub(self.current)
       }
   }
   ```

### Phase 3: Transaction Cost Calculation (2 days)

3. **Add cost calculation to tx validation** in `crates/ergo-consensus/src/tx_validation.rs`:
   ```rust
   use crate::cost::{CostAccumulator, CostConstants, CostLimitError};
   
   pub struct TransactionCostResult {
       pub total_cost: u64,
       pub script_costs: Vec<u64>,
       pub is_valid: bool,
   }
   
   pub fn calculate_transaction_cost(
       tx: &Transaction,
       inputs: &[ErgoBox],
       context: &ErgoContext,
   ) -> Result<TransactionCostResult, ValidationError> {
       let mut accumulator = CostAccumulator::new(CostConstants::MAX_TX_COST);
       let mut script_costs = Vec::new();
       
       // Base costs
       accumulator.add(tx.inputs.len() as u64 * CostConstants::INPUT_BASE_COST)?;
       accumulator.add(tx.outputs.len() as u64 * CostConstants::OUTPUT_BASE_COST)?;
       accumulator.add(tx.data_inputs.len() as u64 * CostConstants::DATA_INPUT_COST)?;
       
       // Size cost
       let tx_size = tx.sigma_serialize_bytes()?.len() as u64;
       accumulator.add(tx_size * CostConstants::SIZE_COST_PER_BYTE)?;
       
       // Script execution costs
       for (input, input_box) in tx.inputs.iter().zip(inputs.iter()) {
           let script_cost = execute_script_with_cost(
               &input_box.ergo_tree,
               context,
               accumulator.remaining(),
           )?;
           
           accumulator.add(script_cost)?;
           script_costs.push(script_cost);
       }
       
       Ok(TransactionCostResult {
           total_cost: accumulator.total(),
           script_costs,
           is_valid: true,
       })
   }
   
   fn execute_script_with_cost(
       script: &ErgoTree,
       context: &ErgoContext,
       cost_limit: u64,
   ) -> Result<u64, ValidationError> {
       // Use ergotree-interpreter with cost tracking
       let verifier = Verifier::new();
       let result = verifier.verify_with_cost(
           script,
           context,
           cost_limit,
       )?;
       
       Ok(result.cost)
   }
   ```

### Phase 4: Block Cost Validation (1.5 days)

4. **Add block cost validation** in `crates/ergo-consensus/src/block_validation.rs`:
   ```rust
   pub fn validate_block_cost(
       block: &Block,
       state: &UtxoState,
   ) -> Result<BlockCostResult, ValidationError> {
       let mut block_accumulator = CostAccumulator::new(CostConstants::MAX_BLOCK_COST);
       let mut tx_costs = Vec::new();
       
       for tx in block.transactions() {
           // Get input boxes
           let inputs = get_input_boxes(tx, state)?;
           let context = create_tx_context(block, tx, &inputs)?;
           
           // Calculate transaction cost
           let tx_result = calculate_transaction_cost(tx, &inputs, &context)?;
           
           // Check individual transaction limit
           if tx_result.total_cost > CostConstants::MAX_TX_COST {
               return Err(ValidationError::TransactionCostExceeded {
                   tx_id: tx.id(),
                   cost: tx_result.total_cost,
                   limit: CostConstants::MAX_TX_COST,
               });
           }
           
           // Add to block total
           block_accumulator.add(tx_result.total_cost)
               .map_err(|e| ValidationError::BlockCostExceeded {
                   cost: e.current,
                   limit: e.limit,
                   at_tx: tx.id(),
               })?;
           
           tx_costs.push((tx.id(), tx_result.total_cost));
       }
       
       Ok(BlockCostResult {
           total_cost: block_accumulator.total(),
           transaction_costs: tx_costs,
       })
   }
   ```

### Phase 5: Mempool Integration (1 day)

5. **Add cost-based ordering to mempool** in `crates/ergo-mempool/src/pool.rs`:
   ```rust
   impl PooledTransaction {
       /// Calculate priority including cost
       pub fn cost_adjusted_priority(&self) -> f64 {
           let fee = self.fee();
           let cost = self.estimated_cost.unwrap_or(CostConstants::INPUT_BASE_COST);
           let size = self.size_bytes as u64;
           
           // Higher fee per unit cost = higher priority
           fee as f64 / (cost + size) as f64
       }
   }
   
   impl Mempool {
       pub fn add_with_cost_estimation(
           &mut self,
           tx: Transaction,
       ) -> Result<(), MempoolError> {
           // Estimate transaction cost before adding
           let estimated_cost = self.estimate_cost(&tx)?;
           
           // Check if we would accept this cost
           if estimated_cost > CostConstants::MAX_TX_COST {
               return Err(MempoolError::TransactionTooExpensive {
                   estimated_cost,
               });
           }
           
           let pooled = PooledTransaction {
               tx,
               estimated_cost: Some(estimated_cost),
               // ... other fields
           };
           
           self.add_pooled(pooled)
       }
   }
   ```

---

## Files to Create/Modify

### New Files
```
crates/ergo-consensus/src/cost.rs     # Cost constants and accumulator
```

### Modified Files
```
crates/ergo-consensus/src/lib.rs           # Export cost module
crates/ergo-consensus/src/tx_validation.rs # Add cost calculation
crates/ergo-consensus/src/block_validation.rs # Add block cost validation
crates/ergo-mempool/src/pool.rs            # Add cost-based ordering
crates/ergo-mempool/src/ordering.rs        # Update priority calculation
```

---

## Estimated Effort

| Task | Time |
|------|------|
| Cost constants | 0.5 days |
| Cost accumulator | 1 day |
| Transaction cost calculation | 2 days |
| Block cost validation | 1.5 days |
| Mempool integration | 1 day |
| Testing | 1 day |
| **Total** | **7 days** |

---

## Dependencies

- Script execution via ergotree-interpreter
- Transaction validation framework (exists)
- Block validation framework (exists)

---

## Test Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_cost_accumulator_limits() {
        let mut acc = CostAccumulator::new(1000);
        
        assert!(acc.add(500).is_ok());
        assert_eq!(acc.total(), 500);
        assert_eq!(acc.remaining(), 500);
        
        assert!(acc.add(500).is_ok());
        assert_eq!(acc.total(), 1000);
        
        // Should fail
        assert!(acc.add(1).is_err());
    }
    
    #[test]
    fn test_transaction_cost_calculation() {
        let tx = create_simple_transaction(2, 3); // 2 inputs, 3 outputs
        let inputs = create_input_boxes(2);
        let context = create_test_context();
        
        let result = calculate_transaction_cost(&tx, &inputs, &context).unwrap();
        
        // Base cost: 2 inputs * 2000 + 3 outputs * 100
        assert!(result.total_cost >= 4300);
    }
    
    #[test]
    fn test_expensive_transaction_rejected() {
        let tx = create_transaction_with_expensive_script();
        let inputs = create_input_boxes(1);
        let context = create_test_context();
        
        let result = calculate_transaction_cost(&tx, &inputs, &context);
        
        assert!(matches!(
            result,
            Err(ValidationError::TransactionCostExceeded { .. })
        ));
    }
    
    #[test]
    fn test_block_cost_accumulation() {
        let mut block = create_empty_block();
        let state = create_test_state();
        
        // Add transactions until we exceed limit
        for i in 0..100 {
            let tx = create_medium_cost_transaction();
            block.add_transaction(tx);
        }
        
        let result = validate_block_cost(&block, &state);
        
        // Should eventually exceed block limit
        assert!(result.is_err() || result.unwrap().total_cost <= CostConstants::MAX_BLOCK_COST);
    }
}
```

### Integration Tests

```rust
#[tokio::test]
async fn test_mempool_rejects_expensive_tx() {
    let mut mempool = create_test_mempool();
    
    let expensive_tx = create_transaction_exceeding_cost_limit();
    let result = mempool.add_with_cost_estimation(expensive_tx);
    
    assert!(matches!(
        result,
        Err(MempoolError::TransactionTooExpensive { .. })
    ));
}
```

---

## Success Metrics

1. **Cost Accuracy**: Calculated costs match expected values
2. **Limit Enforcement**: Transactions/blocks exceeding limits rejected
3. **Performance**: Cost calculation doesn't significantly slow validation
4. **Compatibility**: Cost limits match Scala node behavior

---

## Scala File References

| Feature | Scala File |
|---------|------------|
| Cost limits | `ergo-core/src/main/scala/org/ergoplatform/validation/ValidationRules.scala` |
| Cost tracking | `ergotree-interpreter: CostAccumulator.scala` |
| Block validation | `src/main/scala/org/ergoplatform/nodeView/history/BlockValidator.scala` |
| Tx validation | `src/main/scala/org/ergoplatform/nodeView/state/ErgoStateContext.scala` |
