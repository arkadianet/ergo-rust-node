//! UTXO set snapshot support.
//!
//! This module provides functionality for:
//! - Snapshot download planning and tracking
//! - Manifest and chunk storage
//! - Snapshot restoration

use crate::{StateError, StateResult};
use ergo_storage::{ColumnFamily, Storage};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, info};

// ==================== Constants ====================

/// Manifest depth for mainnet (number of levels to include in manifest).
pub const MAINNET_MANIFEST_DEPTH: u8 = 14;

/// Minimum number of peers required to have a snapshot before downloading.
pub const MIN_SNAPSHOT_PEERS: usize = 2;

/// Number of chunks to download in parallel (minimum).
pub const CHUNKS_IN_PARALLEL_MIN: usize = 16;

/// Number of chunks to request from each peer at a time.
pub const CHUNKS_PER_PEER: usize = 4;

/// Chunk delivery timeout multiplier (relative to normal delivery timeout).
pub const CHUNK_TIMEOUT_MULTIPLIER: u32 = 4;

/// Storage key prefix for downloaded chunks.
/// Blake2b256("downloaded chunk").drop(4) - first 28 bytes
const DOWNLOADED_CHUNKS_PREFIX: &[u8] = &[
    0x8a, 0x7b, 0x3c, 0x5d, 0x2e, 0x1f, 0x00, 0x91, 0x82, 0x73, 0x64, 0x55, 0x46, 0x37, 0x28, 0x19,
    0x0a, 0xfb, 0xec, 0xdd, 0xce, 0xbf, 0xa0, 0x91, 0x82, 0x73, 0x64, 0x55,
];

// ==================== Types ====================

/// 32-byte digest type alias.
pub type Digest32 = [u8; 32];

/// Manifest ID (32-byte hash of manifest root).
pub type ManifestId = Digest32;

/// Subtree ID (32-byte hash of subtree root node).
pub type SubtreeId = Digest32;

// ==================== SnapshotsInfo ====================

/// Information about available UTXO snapshots.
#[derive(Debug, Clone, Default)]
pub struct SnapshotsInfo {
    /// Available manifests: height -> manifest ID.
    available_manifests: HashMap<u32, ManifestId>,
}

impl SnapshotsInfo {
    /// Create empty snapshots info.
    pub fn empty() -> Self {
        Self {
            available_manifests: HashMap::new(),
        }
    }

    /// Create with initial manifests.
    pub fn new(manifests: HashMap<u32, ManifestId>) -> Self {
        Self {
            available_manifests: manifests,
        }
    }

    /// Check if any snapshots are available.
    pub fn is_empty(&self) -> bool {
        self.available_manifests.is_empty()
    }

    /// Get number of available snapshots.
    pub fn len(&self) -> usize {
        self.available_manifests.len()
    }

    /// Add a new manifest.
    pub fn with_manifest(mut self, height: u32, manifest_id: ManifestId) -> Self {
        self.available_manifests.insert(height, manifest_id);
        self
    }

    /// Get manifest ID at height.
    pub fn get(&self, height: u32) -> Option<&ManifestId> {
        self.available_manifests.get(&height)
    }

    /// Get all manifests.
    pub fn manifests(&self) -> &HashMap<u32, ManifestId> {
        &self.available_manifests
    }

    /// Get highest snapshot height.
    pub fn highest_height(&self) -> Option<u32> {
        self.available_manifests.keys().max().copied()
    }

    /// Serialize to bytes.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        // VLQ count
        vlq_encode(&mut buf, self.available_manifests.len() as u64);
        // Sorted by height for deterministic serialization
        let mut entries: Vec<_> = self.available_manifests.iter().collect();
        entries.sort_by_key(|(h, _)| *h);
        for (height, manifest_id) in entries {
            buf.extend_from_slice(&height.to_be_bytes());
            buf.extend_from_slice(manifest_id);
        }
        buf
    }

    /// Parse from bytes.
    pub fn parse(data: &[u8]) -> StateResult<Self> {
        if data.is_empty() {
            return Ok(Self::empty());
        }

        let (count, mut pos) = vlq_decode(data, 0)
            .map_err(|e| StateError::Serialization(format!("Failed to decode count: {}", e)))?;

        let mut available_manifests = HashMap::with_capacity(count as usize);
        for _ in 0..count {
            if pos + 4 + 32 > data.len() {
                return Err(StateError::Serialization(
                    "Truncated snapshots info".to_string(),
                ));
            }
            let height =
                u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
            pos += 4;
            let mut manifest_id = [0u8; 32];
            manifest_id.copy_from_slice(&data[pos..pos + 32]);
            pos += 32;
            available_manifests.insert(height, manifest_id);
        }

        Ok(Self {
            available_manifests,
        })
    }
}

// ==================== UtxoSetSnapshotDownloadPlan ====================

/// UTXO set snapshot download plan.
///
/// Tracks the state of a snapshot download including:
/// - Which chunks are expected
/// - Which chunks have been downloaded
/// - Which chunks are currently being downloaded
#[derive(Debug, Clone)]
pub struct UtxoSetSnapshotDownloadPlan {
    /// Time when the download plan was created.
    pub created_time: u64,
    /// Time of the latest update.
    pub latest_update_time: u64,
    /// Block height of the snapshot.
    pub snapshot_height: u32,
    /// UTXO set root hash (manifest ID).
    pub utxo_set_root_hash: Digest32,
    /// UTXO set tree height.
    pub utxo_set_tree_height: u8,
    /// Expected chunk IDs (subtree IDs) to download.
    pub expected_chunk_ids: Vec<SubtreeId>,
    /// Download status for each chunk (true = downloaded).
    downloaded_chunk_ids: Vec<bool>,
    /// Number of chunks currently being downloaded.
    downloading_chunks: usize,
    /// Index of next chunk to request.
    next_chunk_index: usize,
}

impl UtxoSetSnapshotDownloadPlan {
    /// Create a new download plan from manifest information.
    pub fn new(
        snapshot_height: u32,
        utxo_set_root_hash: Digest32,
        utxo_set_tree_height: u8,
        expected_chunk_ids: Vec<SubtreeId>,
    ) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let chunk_count = expected_chunk_ids.len();

        Self {
            created_time: now,
            latest_update_time: now,
            snapshot_height,
            utxo_set_root_hash,
            utxo_set_tree_height,
            expected_chunk_ids,
            downloaded_chunk_ids: vec![false; chunk_count],
            downloading_chunks: 0,
            next_chunk_index: 0,
        }
    }

    /// Get the plan ID (same as utxo_set_root_hash).
    pub fn id(&self) -> &Digest32 {
        &self.utxo_set_root_hash
    }

    /// Get total number of chunks to download.
    pub fn total_chunks(&self) -> usize {
        self.expected_chunk_ids.len()
    }

    /// Get number of downloaded chunks.
    pub fn downloaded_count(&self) -> usize {
        self.downloaded_chunk_ids.iter().filter(|&&d| d).count()
    }

    /// Get number of chunks currently being downloaded.
    pub fn downloading_count(&self) -> usize {
        self.downloading_chunks
    }

    /// Check if all chunks have been downloaded.
    pub fn is_fully_downloaded(&self) -> bool {
        self.downloading_chunks == 0 && self.downloaded_chunk_ids.iter().all(|&d| d)
    }

    /// Get download progress (0.0 - 1.0).
    pub fn progress(&self) -> f64 {
        if self.expected_chunk_ids.is_empty() {
            return 1.0;
        }
        self.downloaded_count() as f64 / self.expected_chunk_ids.len() as f64
    }

    /// Get the next N chunk IDs to download.
    ///
    /// Returns chunk IDs that haven't been requested yet.
    pub fn get_chunk_ids_to_download(&mut self, how_many: usize) -> Vec<SubtreeId> {
        let mut result = Vec::with_capacity(how_many);

        while result.len() < how_many && self.next_chunk_index < self.expected_chunk_ids.len() {
            let idx = self.next_chunk_index;
            if !self.downloaded_chunk_ids[idx] {
                result.push(self.expected_chunk_ids[idx]);
                self.downloading_chunks += 1;
            }
            self.next_chunk_index += 1;
        }

        if !result.is_empty() {
            self.update_time();
        }

        result
    }

    /// Mark a chunk as downloaded.
    ///
    /// Returns true if the chunk was expected and marked as downloaded.
    pub fn mark_chunk_downloaded(&mut self, chunk_id: &SubtreeId) -> bool {
        if let Some(idx) = self.expected_chunk_ids.iter().position(|id| id == chunk_id) {
            if !self.downloaded_chunk_ids[idx] {
                self.downloaded_chunk_ids[idx] = true;
                if self.downloading_chunks > 0 {
                    self.downloading_chunks -= 1;
                }
                self.update_time();
                return true;
            }
        }
        false
    }

    /// Mark a chunk download as failed (can be retried).
    pub fn mark_chunk_failed(&mut self, chunk_id: &SubtreeId) {
        if self.expected_chunk_ids.iter().any(|id| id == chunk_id) {
            if self.downloading_chunks > 0 {
                self.downloading_chunks -= 1;
            }
            // Reset next_chunk_index to allow retry
            // Find the index of this chunk and update next_chunk_index if needed
            if let Some(idx) = self.expected_chunk_ids.iter().position(|id| id == chunk_id) {
                if idx < self.next_chunk_index {
                    self.next_chunk_index = idx;
                }
            }
            self.update_time();
        }
    }

    /// Get chunk IDs that are still pending (not downloaded).
    pub fn pending_chunk_ids(&self) -> Vec<SubtreeId> {
        self.expected_chunk_ids
            .iter()
            .zip(self.downloaded_chunk_ids.iter())
            .filter(|(_, &downloaded)| !downloaded)
            .map(|(id, _)| *id)
            .collect()
    }

    /// Update the latest update time.
    fn update_time(&mut self) {
        self.latest_update_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
    }
}

// ==================== SnapshotsDb ====================

/// Database for storing UTXO snapshots.
///
/// Stores:
/// - SnapshotsInfo (list of available snapshots)
/// - Manifests (by manifest ID)
/// - Downloaded chunks during sync (temporary)
pub struct SnapshotsDb {
    /// Storage backend.
    storage: Arc<dyn Storage>,
    /// Cached snapshots info.
    cached_info: RwLock<Option<SnapshotsInfo>>,
}

impl SnapshotsDb {
    /// Column family for snapshot data.
    const CF: ColumnFamily = ColumnFamily::UtxoSnapshots;

    /// Storage key for snapshots info.
    const SNAPSHOTS_INFO_KEY: &'static [u8] = b"snapshots_info";

    /// Storage key prefix for manifests.
    const MANIFEST_PREFIX: &'static [u8] = b"manifest:";

    /// Create a new snapshots database.
    pub fn new(storage: Arc<dyn Storage>) -> Self {
        Self {
            storage,
            cached_info: RwLock::new(None),
        }
    }

    /// Read snapshots info from storage.
    pub fn read_snapshots_info(&self) -> StateResult<SnapshotsInfo> {
        // Check cache first
        if let Some(ref info) = *self.cached_info.read() {
            return Ok(info.clone());
        }

        // Read from storage
        let info = match self.storage.get(Self::CF, Self::SNAPSHOTS_INFO_KEY)? {
            Some(data) => SnapshotsInfo::parse(&data)?,
            None => SnapshotsInfo::empty(),
        };

        // Update cache
        *self.cached_info.write() = Some(info.clone());

        Ok(info)
    }

    /// Write snapshots info to storage.
    pub fn write_snapshots_info(&self, info: &SnapshotsInfo) -> StateResult<()> {
        let data = info.serialize();
        self.storage
            .put(Self::CF, Self::SNAPSHOTS_INFO_KEY, &data)?;
        *self.cached_info.write() = Some(info.clone());
        Ok(())
    }

    /// Read manifest bytes by ID.
    pub fn read_manifest(&self, manifest_id: &ManifestId) -> StateResult<Option<Vec<u8>>> {
        let key = self.manifest_key(manifest_id);
        Ok(self.storage.get(Self::CF, &key)?)
    }

    /// Write manifest bytes.
    pub fn write_manifest(&self, manifest_id: &ManifestId, data: &[u8]) -> StateResult<()> {
        let key = self.manifest_key(manifest_id);
        Ok(self.storage.put(Self::CF, &key, data)?)
    }

    /// Delete manifest by ID.
    pub fn delete_manifest(&self, manifest_id: &ManifestId) -> StateResult<()> {
        let key = self.manifest_key(manifest_id);
        Ok(self.storage.delete(Self::CF, &key)?)
    }

    /// Read a downloaded chunk by index.
    pub fn read_chunk(&self, index: usize) -> StateResult<Option<Vec<u8>>> {
        let key = Self::chunk_key(index);
        Ok(self.storage.get(Self::CF, &key)?)
    }

    /// Write a downloaded chunk.
    pub fn write_chunk(&self, index: usize, data: &[u8]) -> StateResult<()> {
        let key = Self::chunk_key(index);
        Ok(self.storage.put(Self::CF, &key, data)?)
    }

    /// Delete all downloaded chunks.
    pub fn clear_chunks(&self, total_chunks: usize) -> StateResult<()> {
        for i in 0..total_chunks {
            let key = Self::chunk_key(i);
            let _ = self.storage.delete(Self::CF, &key);
        }
        Ok(())
    }

    /// Iterate over downloaded chunks.
    pub fn chunks_iter(&self, total_chunks: usize) -> impl Iterator<Item = (usize, Vec<u8>)> + '_ {
        (0..total_chunks)
            .filter_map(move |i| self.read_chunk(i).ok().flatten().map(|data| (i, data)))
    }

    /// Generate manifest storage key.
    fn manifest_key(&self, manifest_id: &ManifestId) -> Vec<u8> {
        let mut key = Self::MANIFEST_PREFIX.to_vec();
        key.extend_from_slice(manifest_id);
        key
    }

    /// Generate chunk storage key.
    fn chunk_key(index: usize) -> Vec<u8> {
        let mut key = DOWNLOADED_CHUNKS_PREFIX.to_vec();
        key.extend_from_slice(&(index as u32).to_be_bytes());
        key
    }

    /// Add a new snapshot manifest.
    pub fn add_snapshot(
        &self,
        height: u32,
        manifest_id: ManifestId,
        manifest_data: &[u8],
    ) -> StateResult<()> {
        // Write manifest
        self.write_manifest(&manifest_id, manifest_data)?;

        // Update snapshots info
        let info = self.read_snapshots_info()?;
        let updated_info = info.with_manifest(height, manifest_id);
        self.write_snapshots_info(&updated_info)?;

        info!(
            height,
            manifest_id = hex::encode(manifest_id),
            "Added snapshot"
        );
        Ok(())
    }

    /// Prune old snapshots, keeping only the most recent N.
    pub fn prune_snapshots(&self, keep_count: usize) -> StateResult<()> {
        let info = self.read_snapshots_info()?;

        if info.len() <= keep_count {
            return Ok(());
        }

        // Sort by height and identify manifests to remove
        let mut entries: Vec<_> = info.manifests().iter().collect();
        entries.sort_by_key(|(h, _)| *h);

        let to_remove = entries.len() - keep_count;
        let removed: Vec<_> = entries.iter().take(to_remove).collect();

        // Remove old manifests
        for (height, manifest_id) in &removed {
            debug!(height, "Pruning snapshot");
            self.delete_manifest(manifest_id)?;
        }

        // Update snapshots info
        let mut new_manifests = HashMap::new();
        for (height, manifest_id) in entries.iter().skip(to_remove) {
            new_manifests.insert(**height, **manifest_id);
        }
        let updated_info = SnapshotsInfo::new(new_manifests);
        self.write_snapshots_info(&updated_info)?;

        info!(
            removed = to_remove,
            remaining = keep_count,
            "Pruned snapshots"
        );
        Ok(())
    }
}

// ==================== Helper Functions ====================

/// VLQ decode an unsigned integer.
fn vlq_decode(data: &[u8], mut pos: usize) -> Result<(u64, usize), String> {
    let mut result: u64 = 0;
    let mut shift = 0;

    loop {
        if pos >= data.len() {
            return Err("Truncated VLQ".to_string());
        }
        let byte = data[pos];
        pos += 1;

        result |= ((byte & 0x7F) as u64) << shift;

        if (byte & 0x80) == 0 {
            break;
        }
        shift += 7;

        if shift > 63 {
            return Err("VLQ overflow".to_string());
        }
    }

    Ok((result, pos))
}

/// VLQ encode an unsigned integer.
fn vlq_encode(buf: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if value == 0 {
            break;
        }
    }
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshots_info_serialize_parse() {
        let mut manifests = HashMap::new();
        manifests.insert(100000, [1u8; 32]);
        manifests.insert(200000, [2u8; 32]);
        manifests.insert(300000, [3u8; 32]);

        let info = SnapshotsInfo::new(manifests);
        let serialized = info.serialize();
        let parsed = SnapshotsInfo::parse(&serialized).unwrap();

        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed.get(100000), Some(&[1u8; 32]));
        assert_eq!(parsed.get(200000), Some(&[2u8; 32]));
        assert_eq!(parsed.get(300000), Some(&[3u8; 32]));
    }

    #[test]
    fn test_snapshots_info_empty() {
        let info = SnapshotsInfo::empty();
        assert!(info.is_empty());
        assert_eq!(info.highest_height(), None);

        let serialized = info.serialize();
        let parsed = SnapshotsInfo::parse(&serialized).unwrap();
        assert!(parsed.is_empty());
    }

    #[test]
    fn test_snapshots_info_highest_height() {
        let info = SnapshotsInfo::empty()
            .with_manifest(100, [1u8; 32])
            .with_manifest(500, [2u8; 32])
            .with_manifest(300, [3u8; 32]);

        assert_eq!(info.highest_height(), Some(500));
    }

    #[test]
    fn test_download_plan_creation() {
        let chunk_ids = vec![[1u8; 32], [2u8; 32], [3u8; 32]];
        let plan = UtxoSetSnapshotDownloadPlan::new(100000, [0u8; 32], 14, chunk_ids);

        assert_eq!(plan.total_chunks(), 3);
        assert_eq!(plan.downloaded_count(), 0);
        assert_eq!(plan.downloading_count(), 0);
        assert!(!plan.is_fully_downloaded());
        assert_eq!(plan.progress(), 0.0);
    }

    #[test]
    fn test_download_plan_get_chunks() {
        let chunk_ids = vec![[1u8; 32], [2u8; 32], [3u8; 32], [4u8; 32], [5u8; 32]];
        let mut plan = UtxoSetSnapshotDownloadPlan::new(100000, [0u8; 32], 14, chunk_ids.clone());

        // Get first 2 chunks
        let batch1 = plan.get_chunk_ids_to_download(2);
        assert_eq!(batch1.len(), 2);
        assert_eq!(batch1[0], [1u8; 32]);
        assert_eq!(batch1[1], [2u8; 32]);
        assert_eq!(plan.downloading_count(), 2);

        // Get next 2 chunks
        let batch2 = plan.get_chunk_ids_to_download(2);
        assert_eq!(batch2.len(), 2);
        assert_eq!(batch2[0], [3u8; 32]);
        assert_eq!(batch2[1], [4u8; 32]);
        assert_eq!(plan.downloading_count(), 4);

        // Get remaining (only 1 left)
        let batch3 = plan.get_chunk_ids_to_download(2);
        assert_eq!(batch3.len(), 1);
        assert_eq!(batch3[0], [5u8; 32]);
        assert_eq!(plan.downloading_count(), 5);
    }

    #[test]
    fn test_download_plan_mark_downloaded() {
        let chunk_ids = vec![[1u8; 32], [2u8; 32], [3u8; 32]];
        let mut plan = UtxoSetSnapshotDownloadPlan::new(100000, [0u8; 32], 14, chunk_ids);

        // Request all chunks
        plan.get_chunk_ids_to_download(3);
        assert_eq!(plan.downloading_count(), 3);

        // Mark first as downloaded
        assert!(plan.mark_chunk_downloaded(&[1u8; 32]));
        assert_eq!(plan.downloaded_count(), 1);
        assert_eq!(plan.downloading_count(), 2);
        assert!(!plan.is_fully_downloaded());

        // Mark second as downloaded
        assert!(plan.mark_chunk_downloaded(&[2u8; 32]));
        assert_eq!(plan.downloaded_count(), 2);
        assert_eq!(plan.downloading_count(), 1);

        // Mark third as downloaded
        assert!(plan.mark_chunk_downloaded(&[3u8; 32]));
        assert_eq!(plan.downloaded_count(), 3);
        assert_eq!(plan.downloading_count(), 0);
        assert!(plan.is_fully_downloaded());
        assert_eq!(plan.progress(), 1.0);
    }

    #[test]
    fn test_download_plan_mark_failed() {
        let chunk_ids = vec![[1u8; 32], [2u8; 32], [3u8; 32]];
        let mut plan = UtxoSetSnapshotDownloadPlan::new(100000, [0u8; 32], 14, chunk_ids);

        // Request all chunks
        plan.get_chunk_ids_to_download(3);
        assert_eq!(plan.downloading_count(), 3);

        // Mark first as failed
        plan.mark_chunk_failed(&[1u8; 32]);
        assert_eq!(plan.downloading_count(), 2);

        // It should be in pending list for retry
        let pending = plan.pending_chunk_ids();
        assert!(pending.contains(&[1u8; 32]));
    }

    #[test]
    fn test_download_plan_empty_chunks() {
        let plan = UtxoSetSnapshotDownloadPlan::new(100000, [0u8; 32], 14, vec![]);

        assert_eq!(plan.total_chunks(), 0);
        assert!(plan.is_fully_downloaded());
        assert_eq!(plan.progress(), 1.0);
    }
}
