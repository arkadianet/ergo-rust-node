//! Block download management.

use crate::{SyncError, SyncResult, PARALLEL_DOWNLOADS};
use ergo_network::PeerId;
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use tracing::{debug, warn};

/// A download task.
#[derive(Debug, Clone)]
pub struct DownloadTask {
    /// Block/modifier ID.
    pub id: Vec<u8>,
    /// Modifier type.
    pub type_id: u8,
    /// Assigned peer.
    pub peer: Option<PeerId>,
    /// Request time.
    pub requested_at: Option<Instant>,
    /// Retry count.
    pub retries: u32,
}

impl DownloadTask {
    /// Create a new download task.
    pub fn new(id: Vec<u8>, type_id: u8) -> Self {
        Self {
            id,
            type_id,
            peer: None,
            requested_at: None,
            retries: 0,
        }
    }

    /// Check if task has timed out.
    pub fn is_timed_out(&self, timeout: Duration) -> bool {
        self.requested_at
            .map(|t| t.elapsed() > timeout)
            .unwrap_or(false)
    }
}

/// Download configuration.
#[derive(Debug, Clone)]
pub struct DownloadConfig {
    /// Maximum parallel downloads.
    pub max_parallel: usize,
    /// Request timeout.
    pub timeout: Duration,
    /// Maximum retries.
    pub max_retries: u32,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            max_parallel: PARALLEL_DOWNLOADS,
            timeout: Duration::from_secs(30),
            max_retries: 3,
        }
    }
}

/// Block download manager.
pub struct BlockDownloader {
    /// Configuration.
    config: DownloadConfig,
    /// Pending tasks (id -> task).
    pending: RwLock<HashMap<Vec<u8>, DownloadTask>>,
    /// In-flight requests (id -> peer).
    in_flight: RwLock<HashMap<Vec<u8>, PeerId>>,
    /// Completed IDs.
    completed: RwLock<HashSet<Vec<u8>>>,
    /// Failed IDs.
    failed: RwLock<HashSet<Vec<u8>>>,
}

impl BlockDownloader {
    /// Create a new block downloader.
    pub fn new(config: DownloadConfig) -> Self {
        Self {
            config,
            pending: RwLock::new(HashMap::new()),
            in_flight: RwLock::new(HashMap::new()),
            completed: RwLock::new(HashSet::new()),
            failed: RwLock::new(HashSet::new()),
        }
    }

    /// Add tasks to download queue.
    pub fn queue(&self, tasks: Vec<DownloadTask>) {
        let mut pending = self.pending.write();
        for task in tasks {
            if !self.completed.read().contains(&task.id) {
                pending.insert(task.id.clone(), task);
            }
        }
        debug!(queued = pending.len(), "Tasks queued for download");
    }

    /// Get tasks ready to dispatch.
    pub fn get_ready_tasks(&self, count: usize) -> Vec<DownloadTask> {
        let in_flight_count = self.in_flight.read().len();
        let available = self.config.max_parallel.saturating_sub(in_flight_count);
        let take = count.min(available);

        let pending = self.pending.read();
        pending
            .values()
            .filter(|t| t.peer.is_none())
            .take(take)
            .cloned()
            .collect()
    }

    /// Mark a task as dispatched to a peer.
    pub fn dispatch(&self, id: &[u8], peer: PeerId) {
        if let Some(mut task) = self.pending.write().get_mut(id) {
            task.peer = Some(peer.clone());
            task.requested_at = Some(Instant::now());
        }
        self.in_flight.write().insert(id.to_vec(), peer);
    }

    /// Mark a task as completed.
    pub fn complete(&self, id: &[u8]) {
        self.pending.write().remove(id);
        self.in_flight.write().remove(id);
        self.completed.write().insert(id.to_vec());
        debug!(id = hex::encode(id), "Download completed");
    }

    /// Handle a failed download.
    pub fn fail(&self, id: &[u8], peer: &PeerId) {
        self.in_flight.write().remove(id);

        if let Some(mut task) = self.pending.write().get_mut(id) {
            task.retries += 1;
            task.peer = None;
            task.requested_at = None;

            if task.retries >= self.config.max_retries {
                warn!(
                    id = hex::encode(id),
                    retries = task.retries,
                    "Download failed permanently"
                );
                self.pending.write().remove(id);
                self.failed.write().insert(id.to_vec());
            } else {
                debug!(
                    id = hex::encode(id),
                    retries = task.retries,
                    "Download will retry"
                );
            }
        }
    }

    /// Check for timed out requests.
    pub fn check_timeouts(&self) -> Vec<(Vec<u8>, PeerId)> {
        let mut timed_out = Vec::new();

        let pending = self.pending.read();
        for (id, task) in pending.iter() {
            if task.is_timed_out(self.config.timeout) {
                if let Some(peer) = &task.peer {
                    timed_out.push((id.clone(), peer.clone()));
                }
            }
        }

        timed_out
    }

    /// Get download statistics.
    pub fn stats(&self) -> DownloadStats {
        DownloadStats {
            pending: self.pending.read().len(),
            in_flight: self.in_flight.read().len(),
            completed: self.completed.read().len(),
            failed: self.failed.read().len(),
        }
    }

    /// Check if all downloads are complete.
    pub fn is_complete(&self) -> bool {
        self.pending.read().is_empty() && self.in_flight.read().is_empty()
    }

    /// Clear completed set (to free memory).
    pub fn clear_completed(&self) {
        self.completed.write().clear();
    }
}

/// Download statistics.
#[derive(Debug, Clone, Default)]
pub struct DownloadStats {
    /// Pending downloads.
    pub pending: usize,
    /// In-flight requests.
    pub in_flight: usize,
    /// Completed downloads.
    pub completed: usize,
    /// Failed downloads.
    pub failed: usize,
}

impl Default for BlockDownloader {
    fn default() -> Self {
        Self::new(DownloadConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_download_queue() {
        let downloader = BlockDownloader::default();

        let tasks = vec![
            DownloadTask::new(vec![1; 32], 1),
            DownloadTask::new(vec![2; 32], 1),
            DownloadTask::new(vec![3; 32], 1),
        ];

        downloader.queue(tasks);

        let stats = downloader.stats();
        assert_eq!(stats.pending, 3);
        assert_eq!(stats.in_flight, 0);
    }

    #[test]
    fn test_dispatch_and_complete() {
        let downloader = BlockDownloader::default();

        let task = DownloadTask::new(vec![1; 32], 1);
        downloader.queue(vec![task]);

        let peer = PeerId::from_bytes(vec![9, 9, 9]);
        downloader.dispatch(&[1; 32], peer);

        assert_eq!(downloader.stats().in_flight, 1);

        downloader.complete(&[1; 32]);

        assert_eq!(downloader.stats().in_flight, 0);
        assert_eq!(downloader.stats().completed, 1);
    }
}
