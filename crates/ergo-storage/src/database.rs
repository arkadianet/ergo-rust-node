//! RocksDB database implementation.

use crate::{Storage, StorageError, StorageResult, WriteBatch};
use parking_lot::RwLock;
use rocksdb::{
    BlockBasedOptions, Cache, ColumnFamilyDescriptor, DBWithThreadMode, MultiThreaded, Options,
};
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, info};

/// Column families for organizing data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColumnFamily {
    /// Block headers indexed by BlockId.
    Headers,
    /// Block transactions indexed by BlockId.
    BlockTransactions,
    /// Block extensions indexed by BlockId.
    Extensions,
    /// Authenticated data proofs indexed by BlockId.
    AdProofs,
    /// Unspent transaction outputs indexed by BoxId.
    Utxo,
    /// UTXO snapshot metadata.
    UtxoSnapshots,
    /// Best chain header mapping (height -> BlockId).
    HeaderChain,
    /// Transaction index (TxId -> BlockId + position).
    TxIndex,
    /// Undo data for state rollback (height -> StateChange).
    UndoData,
    /// ErgoTree hash -> BoxIds index (for address lookups).
    ErgoTreeIndex,
    /// TokenId -> BoxIds index (for token lookups).
    TokenIndex,
    /// BlockId -> Cumulative difficulty (BigInt bytes).
    CumulativeDifficulty,
    /// Node metadata and configuration.
    Metadata,
    /// Extra indexes for addresses, tokens, transactions, boxes.
    ExtraIndex,
    /// Numeric box index (global_index -> box_id).
    BoxIndex,
    /// Numeric transaction index (global_index -> tx_id).
    TxNumericIndex,
    /// Scan definitions (scan_id -> Scan).
    Scans,
    /// Scan boxes (scan_id:box_id -> ScanBox).
    ScanBoxes,
    /// Default column family (required by RocksDB).
    Default,
}

impl ColumnFamily {
    /// Get the string name of the column family.
    pub fn name(&self) -> &'static str {
        match self {
            ColumnFamily::Headers => "headers",
            ColumnFamily::BlockTransactions => "block_transactions",
            ColumnFamily::Extensions => "extensions",
            ColumnFamily::AdProofs => "ad_proofs",
            ColumnFamily::Utxo => "utxo",
            ColumnFamily::UtxoSnapshots => "utxo_snapshots",
            ColumnFamily::HeaderChain => "header_chain",
            ColumnFamily::TxIndex => "tx_index",
            ColumnFamily::UndoData => "undo_data",
            ColumnFamily::ErgoTreeIndex => "ergotree_index",
            ColumnFamily::TokenIndex => "token_index",
            ColumnFamily::CumulativeDifficulty => "cumulative_difficulty",
            ColumnFamily::Metadata => "metadata",
            ColumnFamily::ExtraIndex => "extra_index",
            ColumnFamily::BoxIndex => "box_index",
            ColumnFamily::TxNumericIndex => "tx_numeric_index",
            ColumnFamily::Scans => "scans",
            ColumnFamily::ScanBoxes => "scan_boxes",
            ColumnFamily::Default => "default",
        }
    }

    /// Get all column families.
    pub fn all() -> &'static [ColumnFamily] {
        &[
            ColumnFamily::Headers,
            ColumnFamily::BlockTransactions,
            ColumnFamily::Extensions,
            ColumnFamily::AdProofs,
            ColumnFamily::Utxo,
            ColumnFamily::UtxoSnapshots,
            ColumnFamily::HeaderChain,
            ColumnFamily::TxIndex,
            ColumnFamily::UndoData,
            ColumnFamily::ErgoTreeIndex,
            ColumnFamily::TokenIndex,
            ColumnFamily::CumulativeDifficulty,
            ColumnFamily::Metadata,
            ColumnFamily::ExtraIndex,
            ColumnFamily::BoxIndex,
            ColumnFamily::TxNumericIndex,
            ColumnFamily::Scans,
            ColumnFamily::ScanBoxes,
            ColumnFamily::Default,
        ]
    }
}

/// RocksDB database wrapper.
pub struct Database {
    db: Arc<RwLock<DBWithThreadMode<MultiThreaded>>>,
}

impl Database {
    /// Open or create a database at the given path.
    pub fn open<P: AsRef<Path>>(path: P) -> StorageResult<Self> {
        let path = path.as_ref();
        info!("Opening database at {:?}", path);

        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        opts.set_max_open_files(256);
        opts.set_keep_log_file_num(1);
        opts.set_max_total_wal_size(64 * 1024 * 1024); // 64MB WAL (reduced from 128MB)

        // Performance optimizations for sync workload
        // Larger buffers reduce write amplification by delaying compaction
        opts.set_write_buffer_size(64 * 1024 * 1024); // 64MB write buffer
        opts.set_max_write_buffer_number(4); // Keep 4 write buffers for better batching
        opts.set_min_write_buffer_number_to_merge(2); // Merge when 2 are full
        opts.set_level_zero_file_num_compaction_trigger(8); // Delay L0 compaction
        opts.set_max_background_jobs(4); // Background compaction threads

        // Reduce write amplification by increasing level size multiplier
        // Default is 10x between levels; 20x means fewer levels and less frequent compaction
        opts.set_max_bytes_for_level_multiplier(20.0);

        // Disable fsync on every write for better performance during sync
        // Data is still durable via WAL, just not synced immediately
        opts.set_manual_wal_flush(true);

        // Create a SHARED block cache for all column families.
        // This is critical for bounding memory - without explicit configuration,
        // each CF gets an unbounded default cache that can grow to gigabytes.
        // 256MB is enough for good read performance while keeping memory bounded.
        let block_cache = Cache::new_lru_cache(256 * 1024 * 1024); // 256MB shared

        // Create column family descriptors
        let cf_descriptors: Vec<ColumnFamilyDescriptor> = ColumnFamily::all()
            .iter()
            .map(|cf| {
                let mut cf_opts = Options::default();
                cf_opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
                // Larger buffer per CF reduces flush frequency
                // 19 CFs * 32MB = ~600MB total for CF write buffers
                // This significantly reduces compaction during sync
                cf_opts.set_write_buffer_size(32 * 1024 * 1024); // 32MB per CF

                // Configure block-based table with shared cache
                let mut block_opts = BlockBasedOptions::default();
                block_opts.set_block_cache(&block_cache);
                // Keep index and filter blocks in cache for better read performance
                block_opts.set_cache_index_and_filter_blocks(true);
                block_opts.set_pin_l0_filter_and_index_blocks_in_cache(true);
                cf_opts.set_block_based_table_factory(&block_opts);

                ColumnFamilyDescriptor::new(cf.name(), cf_opts)
            })
            .collect();

        let db =
            DBWithThreadMode::<MultiThreaded>::open_cf_descriptors(&opts, path, cf_descriptors)?;

        debug!("Database opened successfully");

        Ok(Self {
            db: Arc::new(RwLock::new(db)),
        })
    }

    /// Open a database in read-only mode.
    pub fn open_read_only<P: AsRef<Path>>(path: P) -> StorageResult<Self> {
        let path = path.as_ref();
        info!("Opening database in read-only mode at {:?}", path);

        let opts = Options::default();
        let cf_names: Vec<&str> = ColumnFamily::all().iter().map(|cf| cf.name()).collect();

        let db =
            DBWithThreadMode::<MultiThreaded>::open_cf_for_read_only(&opts, path, cf_names, false)?;

        Ok(Self {
            db: Arc::new(RwLock::new(db)),
        })
    }

    /// Get a reference to the column family handle.
    fn cf_handle(&self, cf: ColumnFamily) -> StorageResult<()> {
        let db = self.db.read();
        if db.cf_handle(cf.name()).is_some() {
            Ok(())
        } else {
            Err(StorageError::ColumnFamilyNotFound(cf.name().to_string()))
        }
    }

    /// Flush all pending writes to disk.
    pub fn flush(&self) -> StorageResult<()> {
        let db = self.db.read();
        for cf in ColumnFamily::all() {
            if let Some(handle) = db.cf_handle(cf.name()) {
                db.flush_cf(&handle)?;
            }
        }
        Ok(())
    }

    /// Compact the database.
    pub fn compact(&self) -> StorageResult<()> {
        let db = self.db.read();
        for cf in ColumnFamily::all() {
            if let Some(handle) = db.cf_handle(cf.name()) {
                db.compact_range_cf(&handle, None::<&[u8]>, None::<&[u8]>);
            }
        }
        Ok(())
    }
}

impl Storage for Database {
    fn get(&self, cf: ColumnFamily, key: &[u8]) -> StorageResult<Option<Vec<u8>>> {
        let db = self.db.read();
        let handle = db
            .cf_handle(cf.name())
            .ok_or_else(|| StorageError::ColumnFamilyNotFound(cf.name().to_string()))?;

        Ok(db.get_cf(&handle, key)?)
    }

    fn put(&self, cf: ColumnFamily, key: &[u8], value: &[u8]) -> StorageResult<()> {
        let db = self.db.read();
        let handle = db
            .cf_handle(cf.name())
            .ok_or_else(|| StorageError::ColumnFamilyNotFound(cf.name().to_string()))?;

        db.put_cf(&handle, key, value)?;
        Ok(())
    }

    fn delete(&self, cf: ColumnFamily, key: &[u8]) -> StorageResult<()> {
        let db = self.db.read();
        let handle = db
            .cf_handle(cf.name())
            .ok_or_else(|| StorageError::ColumnFamilyNotFound(cf.name().to_string()))?;

        db.delete_cf(&handle, key)?;
        Ok(())
    }

    fn write_batch(&self, batch: WriteBatch) -> StorageResult<()> {
        let db = self.db.read();
        let mut rocks_batch = rocksdb::WriteBatch::default();

        for op in batch.operations {
            let handle = db
                .cf_handle(op.cf.name())
                .ok_or_else(|| StorageError::ColumnFamilyNotFound(op.cf.name().to_string()))?;

            match op.kind {
                crate::batch::OperationKind::Put { value } => {
                    rocks_batch.put_cf(&handle, &op.key, &value);
                }
                crate::batch::OperationKind::Delete => {
                    rocks_batch.delete_cf(&handle, &op.key);
                }
            }
        }

        // Use non-sync write for better performance during sync
        // WAL provides durability, we just skip the fsync
        let mut write_opts = rocksdb::WriteOptions::default();
        write_opts.disable_wal(false); // Keep WAL for durability
        write_opts.set_sync(false); // Don't fsync on every write

        db.write_opt(rocks_batch, &write_opts)?;
        Ok(())
    }

    fn iter(
        &self,
        cf: ColumnFamily,
    ) -> StorageResult<Box<dyn Iterator<Item = (Vec<u8>, Vec<u8>)> + '_>> {
        let db = self.db.read();
        let handle = db
            .cf_handle(cf.name())
            .ok_or_else(|| StorageError::ColumnFamilyNotFound(cf.name().to_string()))?;

        let iter = db.iterator_cf(&handle, rocksdb::IteratorMode::Start);

        // Note: This is a simplified implementation. In production, we'd need
        // to handle the lifetime properly with a wrapper type.
        let collected: Vec<_> = iter
            .filter_map(|r| r.ok())
            .map(|(k, v)| (k.to_vec(), v.to_vec()))
            .collect();

        Ok(Box::new(collected.into_iter()))
    }
}

impl Clone for Database {
    fn clone(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_database_open_and_write() {
        let tmp = TempDir::new().unwrap();
        let db = Database::open(tmp.path()).unwrap();

        // Test basic operations
        db.put(ColumnFamily::Metadata, b"key1", b"value1").unwrap();
        let value = db.get(ColumnFamily::Metadata, b"key1").unwrap();
        assert_eq!(value, Some(b"value1".to_vec()));

        // Test delete
        db.delete(ColumnFamily::Metadata, b"key1").unwrap();
        let value = db.get(ColumnFamily::Metadata, b"key1").unwrap();
        assert_eq!(value, None);
    }

    #[test]
    fn test_write_batch() {
        let tmp = TempDir::new().unwrap();
        let db = Database::open(tmp.path()).unwrap();

        let mut batch = WriteBatch::new();
        batch.put(ColumnFamily::Headers, b"h1", b"header1");
        batch.put(ColumnFamily::Headers, b"h2", b"header2");
        batch.put(ColumnFamily::Utxo, b"box1", b"boxdata");

        db.write_batch(batch).unwrap();

        assert_eq!(
            db.get(ColumnFamily::Headers, b"h1").unwrap(),
            Some(b"header1".to_vec())
        );
        assert_eq!(
            db.get(ColumnFamily::Headers, b"h2").unwrap(),
            Some(b"header2".to_vec())
        );
        assert_eq!(
            db.get(ColumnFamily::Utxo, b"box1").unwrap(),
            Some(b"boxdata".to_vec())
        );
    }
}
