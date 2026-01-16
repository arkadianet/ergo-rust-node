//! Write batch for atomic operations.

use crate::ColumnFamily;

/// Kind of batch operation.
#[derive(Debug, Clone)]
pub enum OperationKind {
    /// Put a key-value pair.
    Put { value: Vec<u8> },
    /// Delete a key.
    Delete,
}

/// A single batch operation.
#[derive(Debug, Clone)]
pub struct BatchOperation {
    /// Target column family.
    pub cf: ColumnFamily,
    /// Key to operate on.
    pub key: Vec<u8>,
    /// Kind of operation.
    pub kind: OperationKind,
}

/// A batch of write operations to be executed atomically.
#[derive(Debug, Default)]
pub struct WriteBatch {
    /// Collected operations.
    pub(crate) operations: Vec<BatchOperation>,
}

impl WriteBatch {
    /// Create a new empty batch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a batch with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            operations: Vec::with_capacity(capacity),
        }
    }

    /// Add a put operation to the batch.
    pub fn put(&mut self, cf: ColumnFamily, key: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) {
        self.operations.push(BatchOperation {
            cf,
            key: key.into(),
            kind: OperationKind::Put {
                value: value.into(),
            },
        });
    }

    /// Add a delete operation to the batch.
    pub fn delete(&mut self, cf: ColumnFamily, key: impl Into<Vec<u8>>) {
        self.operations.push(BatchOperation {
            cf,
            key: key.into(),
            kind: OperationKind::Delete,
        });
    }

    /// Get the number of operations in the batch.
    pub fn len(&self) -> usize {
        self.operations.len()
    }

    /// Check if the batch is empty.
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    /// Clear all operations from the batch.
    pub fn clear(&mut self) {
        self.operations.clear();
    }

    /// Merge another batch into this one.
    pub fn merge(&mut self, other: WriteBatch) {
        self.operations.extend(other.operations);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_batch() {
        let mut batch = WriteBatch::new();
        assert!(batch.is_empty());

        batch.put(ColumnFamily::Headers, b"key1", b"value1");
        batch.put(ColumnFamily::Utxo, b"key2", b"value2");
        batch.delete(ColumnFamily::Headers, b"key3");

        assert_eq!(batch.len(), 3);
        assert!(!batch.is_empty());

        batch.clear();
        assert!(batch.is_empty());
    }
}
