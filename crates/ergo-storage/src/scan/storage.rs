//! Scan storage operations.

use super::types::{Scan, ScanBox, ScanId, ScanRequest};
use crate::{ColumnFamily, Storage, StorageResult, WriteBatch};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info};

/// Scan storage manager.
pub struct ScanStorage<S: Storage> {
    /// Underlying storage.
    storage: Arc<S>,
    /// Cached scans (scan_id -> Scan).
    scans: RwLock<HashMap<ScanId, Scan>>,
    /// Next available scan ID.
    next_scan_id: RwLock<ScanId>,
}

impl<S: Storage> ScanStorage<S> {
    /// Create a new scan storage.
    pub fn new(storage: Arc<S>) -> Self {
        let ss = Self {
            storage,
            scans: RwLock::new(HashMap::new()),
            next_scan_id: RwLock::new(1), // Start from 1, 0 is reserved
        };
        // Load existing scans
        if let Err(e) = ss.load_scans() {
            debug!("Failed to load scans: {}", e);
        }
        ss
    }

    /// Load all scans from storage.
    fn load_scans(&self) -> StorageResult<()> {
        let iter = self.storage.iter(ColumnFamily::Scans)?;
        let mut scans = self.scans.write();
        let mut max_id: ScanId = 0;

        for (_key, value) in iter {
            if let Some(scan) = Scan::deserialize(&value) {
                if scan.scan_id > max_id {
                    max_id = scan.scan_id;
                }
                scans.insert(scan.scan_id, scan);
            }
        }

        *self.next_scan_id.write() = max_id + 1;
        info!("Loaded {} scans, next ID: {}", scans.len(), max_id + 1);
        Ok(())
    }

    /// Register a new scan.
    pub fn register(&self, request: ScanRequest) -> Result<Scan, String> {
        let scan_id = {
            let mut next_id = self.next_scan_id.write();
            let id = *next_id;
            *next_id += 1;
            id
        };

        let scan = request.to_scan(scan_id)?;

        // Persist to storage
        let key = scan_id.to_be_bytes();
        let value = scan.serialize();
        self.storage
            .put(ColumnFamily::Scans, &key, &value)
            .map_err(|e| format!("Failed to persist scan: {}", e))?;

        // Update cache
        self.scans.write().insert(scan_id, scan.clone());

        info!("Registered scan {} ({})", scan_id, scan.scan_name);
        Ok(scan)
    }

    /// Deregister (remove) a scan.
    pub fn deregister(&self, scan_id: ScanId) -> Result<(), String> {
        // Check if scan exists
        if !self.scans.read().contains_key(&scan_id) {
            return Err(format!("Scan {} not found", scan_id));
        }

        // Remove from storage
        let key = scan_id.to_be_bytes();
        self.storage
            .delete(ColumnFamily::Scans, &key)
            .map_err(|e| format!("Failed to delete scan: {}", e))?;

        // Remove associated boxes
        self.remove_scan_boxes(scan_id)?;

        // Update cache
        self.scans.write().remove(&scan_id);

        info!("Deregistered scan {}", scan_id);
        Ok(())
    }

    /// List all registered scans.
    pub fn list_all(&self) -> Vec<Scan> {
        self.scans.read().values().cloned().collect()
    }

    /// Get a scan by ID.
    pub fn get(&self, scan_id: ScanId) -> Option<Scan> {
        self.scans.read().get(&scan_id).cloned()
    }

    /// Add a box to a scan.
    pub fn add_box(&self, scan_box: ScanBox) -> StorageResult<()> {
        let key = Self::scan_box_key(&scan_box.box_id);
        let value = scan_box.serialize();
        self.storage.put(ColumnFamily::ScanBoxes, &key, &value)
    }

    /// Add a box to multiple scans.
    pub fn add_box_to_scans(
        &self,
        box_id: &[u8],
        scan_ids: Vec<ScanId>,
        inclusion_height: u32,
        confirmations_num: u32,
        box_data: Vec<u8>,
    ) -> StorageResult<()> {
        let scan_box = ScanBox::new(
            box_id.to_vec(),
            scan_ids,
            inclusion_height,
            confirmations_num,
            box_data,
        );
        self.add_box(scan_box)
    }

    /// Get a box by ID.
    pub fn get_box(&self, box_id: &[u8]) -> StorageResult<Option<ScanBox>> {
        let key = Self::scan_box_key(box_id);
        match self.storage.get(ColumnFamily::ScanBoxes, &key)? {
            Some(data) => Ok(ScanBox::deserialize(&data)),
            None => Ok(None),
        }
    }

    /// Get unspent boxes for a scan.
    pub fn get_unspent_boxes(&self, scan_id: ScanId) -> StorageResult<Vec<ScanBox>> {
        let iter = self.storage.iter(ColumnFamily::ScanBoxes)?;
        let mut boxes = Vec::new();

        for (_key, value) in iter {
            if let Some(scan_box) = ScanBox::deserialize(&value) {
                if !scan_box.spent && scan_box.scan_ids.contains(&scan_id) {
                    boxes.push(scan_box);
                }
            }
        }

        Ok(boxes)
    }

    /// Get spent boxes for a scan.
    pub fn get_spent_boxes(&self, scan_id: ScanId) -> StorageResult<Vec<ScanBox>> {
        let iter = self.storage.iter(ColumnFamily::ScanBoxes)?;
        let mut boxes = Vec::new();

        for (_key, value) in iter {
            if let Some(scan_box) = ScanBox::deserialize(&value) {
                if scan_box.spent && scan_box.scan_ids.contains(&scan_id) {
                    boxes.push(scan_box);
                }
            }
        }

        Ok(boxes)
    }

    /// Stop tracking a box for a specific scan.
    pub fn stop_tracking(&self, scan_id: ScanId, box_id: &[u8]) -> StorageResult<()> {
        let key = Self::scan_box_key(box_id);

        if let Some(data) = self.storage.get(ColumnFamily::ScanBoxes, &key)? {
            if let Some(mut scan_box) = ScanBox::deserialize(&data) {
                scan_box.scan_ids.retain(|&id| id != scan_id);

                if scan_box.scan_ids.is_empty() {
                    // No more scans tracking this box, remove it
                    self.storage.delete(ColumnFamily::ScanBoxes, &key)?;
                } else {
                    // Update with remaining scan IDs
                    let value = scan_box.serialize();
                    self.storage.put(ColumnFamily::ScanBoxes, &key, &value)?;
                }
            }
        }

        Ok(())
    }

    /// Mark a box as spent.
    pub fn mark_box_spent(
        &self,
        box_id: &[u8],
        spending_tx_id: Vec<u8>,
        spending_height: u32,
    ) -> StorageResult<()> {
        let key = Self::scan_box_key(box_id);

        if let Some(data) = self.storage.get(ColumnFamily::ScanBoxes, &key)? {
            if let Some(mut scan_box) = ScanBox::deserialize(&data) {
                scan_box.mark_spent(spending_tx_id, spending_height);
                let value = scan_box.serialize();
                self.storage.put(ColumnFamily::ScanBoxes, &key, &value)?;
            }
        }

        Ok(())
    }

    /// Remove all boxes associated with a scan.
    fn remove_scan_boxes(&self, scan_id: ScanId) -> Result<(), String> {
        let iter = self
            .storage
            .iter(ColumnFamily::ScanBoxes)
            .map_err(|e| format!("Failed to iterate scan boxes: {}", e))?;

        let mut batch = WriteBatch::new();

        for (key, value) in iter {
            if let Some(mut scan_box) = ScanBox::deserialize(&value) {
                if scan_box.scan_ids.contains(&scan_id) {
                    scan_box.scan_ids.retain(|&id| id != scan_id);

                    if scan_box.scan_ids.is_empty() {
                        batch.delete(ColumnFamily::ScanBoxes, key);
                    } else {
                        let value = scan_box.serialize();
                        batch.put(ColumnFamily::ScanBoxes, key, value);
                    }
                }
            }
        }

        self.storage
            .write_batch(batch)
            .map_err(|e| format!("Failed to remove scan boxes: {}", e))?;

        Ok(())
    }

    /// Create a key for a scan box.
    fn scan_box_key(box_id: &[u8]) -> Vec<u8> {
        box_id.to_vec()
    }

    /// Process a new block and update scan boxes.
    ///
    /// This should be called when a new block is applied to check for
    /// boxes matching registered scans.
    pub fn process_block(
        &self,
        _height: u32,
        _created_boxes: &[(Vec<u8>, Vec<u8>)], // (box_id, serialized_box)
        spent_box_ids: &[Vec<u8>],
        spending_tx_id: &[u8],
    ) -> StorageResult<()> {
        // Mark spent boxes
        for box_id in spent_box_ids {
            if self.get_box(box_id)?.is_some() {
                self.mark_box_spent(box_id, spending_tx_id.to_vec(), 0)?;
            }
        }

        // Note: Matching new boxes against scan predicates would require
        // deserializing box data and extracting tokens/registers.
        // This is left as a TODO for full integration.

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::predicate::ScanningPredicate;
    use crate::Database;
    use tempfile::TempDir;

    fn create_test_storage() -> (ScanStorage<Database>, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db = Database::open(tmp.path()).unwrap();
        let storage = ScanStorage::new(Arc::new(db));
        (storage, tmp)
    }

    #[test]
    fn test_register_scan() {
        let (storage, _tmp) = create_test_storage();

        let request = ScanRequest {
            scan_name: "Test Scan".to_string(),
            tracking_rule: ScanningPredicate::ContainsAsset {
                asset_id: vec![1, 2, 3],
            },
            wallet_interaction: None,
            remove_offchain: None,
        };

        let scan = storage.register(request).unwrap();
        assert_eq!(scan.scan_id, 1);
        assert_eq!(scan.scan_name, "Test Scan");

        // List should contain the scan
        let scans = storage.list_all();
        assert_eq!(scans.len(), 1);
        assert_eq!(scans[0].scan_id, 1);
    }

    #[test]
    fn test_deregister_scan() {
        let (storage, _tmp) = create_test_storage();

        let request = ScanRequest {
            scan_name: "Test Scan".to_string(),
            tracking_rule: ScanningPredicate::ContainsAsset {
                asset_id: vec![1, 2, 3],
            },
            wallet_interaction: None,
            remove_offchain: None,
        };

        let scan = storage.register(request).unwrap();
        assert_eq!(storage.list_all().len(), 1);

        storage.deregister(scan.scan_id).unwrap();
        assert_eq!(storage.list_all().len(), 0);
    }

    #[test]
    fn test_add_and_get_box() {
        let (storage, _tmp) = create_test_storage();

        let box_id = vec![1, 2, 3, 4];
        let scan_box = ScanBox::new(box_id.clone(), vec![1], 100, 5, vec![0xAB, 0xCD]);

        storage.add_box(scan_box).unwrap();

        let retrieved = storage.get_box(&box_id).unwrap().unwrap();
        assert_eq!(retrieved.box_id, box_id);
        assert_eq!(retrieved.scan_ids, vec![1]);
        assert!(!retrieved.spent);
    }

    #[test]
    fn test_unspent_and_spent_boxes() {
        let (storage, _tmp) = create_test_storage();

        let box1 = ScanBox::new(vec![1], vec![1], 100, 5, vec![]);
        let box2 = ScanBox::new(vec![2], vec![1], 101, 4, vec![]);

        storage.add_box(box1).unwrap();
        storage.add_box(box2).unwrap();

        // Both unspent
        assert_eq!(storage.get_unspent_boxes(1).unwrap().len(), 2);
        assert_eq!(storage.get_spent_boxes(1).unwrap().len(), 0);

        // Mark one as spent
        storage.mark_box_spent(&[1], vec![0xAA], 150).unwrap();

        assert_eq!(storage.get_unspent_boxes(1).unwrap().len(), 1);
        assert_eq!(storage.get_spent_boxes(1).unwrap().len(), 1);
    }

    #[test]
    fn test_stop_tracking() {
        let (storage, _tmp) = create_test_storage();

        let box_id = vec![1, 2, 3];
        let scan_box = ScanBox::new(box_id.clone(), vec![1, 2], 100, 5, vec![]);

        storage.add_box(scan_box).unwrap();

        // Stop tracking for scan 1
        storage.stop_tracking(1, &box_id).unwrap();

        let retrieved = storage.get_box(&box_id).unwrap().unwrap();
        assert_eq!(retrieved.scan_ids, vec![2]);

        // Stop tracking for scan 2 (should remove the box)
        storage.stop_tracking(2, &box_id).unwrap();

        assert!(storage.get_box(&box_id).unwrap().is_none());
    }
}
