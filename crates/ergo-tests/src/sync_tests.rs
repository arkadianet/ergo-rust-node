//! Sync protocol tests for ergo-sync crate.
//!
//! Tests for block download, synchronization state, and sync protocol.

use ergo_network::PeerId;
use ergo_sync::{
    BlockDownloader, DownloadConfig, DownloadTask, SyncConfig, SyncState, HEADERS_BATCH_SIZE,
    MAX_DOWNLOAD_QUEUE, PARALLEL_DOWNLOADS,
};
use std::time::Duration;

// ============================================================================
// Download Task Tests
// ============================================================================

#[test]
fn test_download_task_creation() {
    let id = vec![1, 2, 3, 4];
    let task = DownloadTask::new(id.clone(), 1);

    assert_eq!(task.id, id);
    assert_eq!(task.type_id, 1);
    assert!(task.peer.is_none());
    assert!(task.requested_at.is_none());
    assert_eq!(task.retries, 0);
}

#[test]
fn test_download_task_failed_peers_initially_empty() {
    let task = DownloadTask::new(vec![1, 2, 3], 1);
    assert!(task.failed_peers.is_empty());
}

#[test]
fn test_download_task_peer_failed_check() {
    let mut task = DownloadTask::new(vec![1, 2, 3], 1);
    let peer_id = PeerId(vec![1, 2, 3]);

    assert!(!task.peer_failed(&peer_id));

    task.failed_peers.insert(peer_id.clone());
    assert!(task.peer_failed(&peer_id));
}

#[test]
fn test_download_task_not_timed_out_when_not_requested() {
    let task = DownloadTask::new(vec![1, 2, 3], 1);

    assert!(!task.is_timed_out(Duration::from_secs(10)));
}

#[test]
fn test_download_task_different_type_ids() {
    // Header type
    let header_task = DownloadTask::new(vec![1], 101);
    assert_eq!(header_task.type_id, 101);

    // Block transactions type
    let block_task = DownloadTask::new(vec![2], 102);
    assert_eq!(block_task.type_id, 102);
}

#[test]
fn test_download_task_clone() {
    let task = DownloadTask::new(vec![1, 2, 3], 5);
    let cloned = task.clone();

    assert_eq!(task.id, cloned.id);
    assert_eq!(task.type_id, cloned.type_id);
    assert_eq!(task.retries, cloned.retries);
}

#[test]
fn test_download_task_debug() {
    let task = DownloadTask::new(vec![1, 2, 3], 1);
    let debug_str = format!("{:?}", task);

    assert!(debug_str.contains("DownloadTask"));
}

// ============================================================================
// Download Config Tests
// ============================================================================

#[test]
fn test_download_config_default() {
    let config = DownloadConfig::default();

    assert_eq!(config.max_parallel, PARALLEL_DOWNLOADS);
    assert_eq!(config.timeout, Duration::from_secs(10));
    assert_eq!(config.max_retries, 5);
}

#[test]
fn test_download_config_custom() {
    let config = DownloadConfig {
        max_parallel: 64,
        timeout: Duration::from_secs(30),
        max_retries: 10,
    };

    assert_eq!(config.max_parallel, 64);
    assert_eq!(config.timeout, Duration::from_secs(30));
    assert_eq!(config.max_retries, 10);
}

#[test]
fn test_download_config_clone() {
    let config = DownloadConfig {
        max_parallel: 32,
        timeout: Duration::from_secs(15),
        max_retries: 3,
    };

    let cloned = config.clone();
    assert_eq!(config.max_parallel, cloned.max_parallel);
    assert_eq!(config.timeout, cloned.timeout);
    assert_eq!(config.max_retries, cloned.max_retries);
}

#[test]
fn test_download_config_debug() {
    let config = DownloadConfig::default();
    let debug_str = format!("{:?}", config);

    assert!(debug_str.contains("DownloadConfig"));
}

// ============================================================================
// Block Downloader Tests
// ============================================================================

#[test]
fn test_block_downloader_creation() {
    let config = DownloadConfig::default();
    let downloader = BlockDownloader::new(config);

    let stats = downloader.stats();
    assert_eq!(stats.pending, 0);
    assert_eq!(stats.in_flight, 0);
    assert_eq!(stats.completed, 0);
    assert_eq!(stats.failed, 0);
}

#[test]
fn test_block_downloader_queue_single() {
    let downloader = BlockDownloader::new(DownloadConfig::default());

    let task = DownloadTask::new(vec![1, 2, 3, 4], 1);
    downloader.queue(vec![task]);

    let stats = downloader.stats();
    assert_eq!(stats.pending, 1);
}

#[test]
fn test_block_downloader_queue_multiple() {
    let downloader = BlockDownloader::new(DownloadConfig::default());

    let tasks: Vec<_> = (0..10).map(|i| DownloadTask::new(vec![i], 1)).collect();
    downloader.queue(tasks);

    let stats = downloader.stats();
    assert_eq!(stats.pending, 10);
}

#[test]
fn test_block_downloader_get_ready_tasks() {
    let downloader = BlockDownloader::new(DownloadConfig::default());

    let tasks: Vec<_> = (0..10).map(|i| DownloadTask::new(vec![i], 1)).collect();
    downloader.queue(tasks);

    let ready = downloader.get_ready_tasks(5);
    assert_eq!(ready.len(), 5);
}

#[test]
fn test_block_downloader_get_ready_tasks_for_peer() {
    let downloader = BlockDownloader::new(DownloadConfig::default());

    let tasks: Vec<_> = (0..10).map(|i| DownloadTask::new(vec![i], 1)).collect();
    downloader.queue(tasks);

    let peer = PeerId(vec![1, 2, 3]);
    let ready = downloader.get_ready_tasks_for_peer(5, &peer);
    assert!(ready.len() <= 5);
}

#[test]
fn test_block_downloader_dispatch_and_complete() {
    let downloader = BlockDownloader::new(DownloadConfig::default());

    let id = vec![1, 2, 3, 4];
    let task = DownloadTask::new(id.clone(), 1);
    downloader.queue(vec![task]);

    let peer = PeerId(vec![1, 2, 3]);
    downloader.dispatch(&id, peer);

    let stats = downloader.stats();
    assert_eq!(stats.in_flight, 1);

    downloader.complete(&id);

    let stats = downloader.stats();
    assert_eq!(stats.completed, 1);
    assert_eq!(stats.in_flight, 0);
}

#[test]
fn test_block_downloader_fail() {
    let downloader = BlockDownloader::new(DownloadConfig::default());

    let id = vec![1, 2, 3, 4];
    let task = DownloadTask::new(id.clone(), 1);
    downloader.queue(vec![task]);

    let peer = PeerId(vec![1, 2, 3]);
    downloader.dispatch(&id, peer.clone());

    downloader.fail(&id, &peer);

    let stats = downloader.stats();
    assert_eq!(stats.in_flight, 0);
}

#[test]
fn test_block_downloader_is_complete() {
    let downloader = BlockDownloader::new(DownloadConfig::default());

    // Empty downloader is complete
    assert!(downloader.is_complete());

    let task = DownloadTask::new(vec![1, 2, 3], 1);
    downloader.queue(vec![task]);

    // With pending tasks, not complete
    assert!(!downloader.is_complete());
}

#[test]
fn test_block_downloader_uncomplete() {
    let downloader = BlockDownloader::new(DownloadConfig::default());

    let id = vec![1, 2, 3, 4];
    let task = DownloadTask::new(id.clone(), 1);
    downloader.queue(vec![task]);

    let peer = PeerId(vec![1, 2, 3]);
    downloader.dispatch(&id, peer);
    downloader.complete(&id);

    let stats = downloader.stats();
    assert_eq!(stats.completed, 1);

    // uncomplete removes a specific ID from completed set
    downloader.uncomplete(&id);

    let stats = downloader.stats();
    assert_eq!(stats.completed, 0);
}

// ============================================================================
// Sync Config Tests
// ============================================================================

#[test]
fn test_sync_config_default() {
    let config = SyncConfig::default();

    assert!(config.headers_batch > 0);
    assert!(config.parallel_downloads > 0);
}

#[test]
fn test_sync_config_custom() {
    let config = SyncConfig {
        headers_batch: 200,
        parallel_downloads: 50,
        ..Default::default()
    };

    assert_eq!(config.headers_batch, 200);
    assert_eq!(config.parallel_downloads, 50);
}

#[test]
fn test_sync_config_utxo_bootstrap_disabled_by_default() {
    let config = SyncConfig::default();
    assert!(!config.utxo_bootstrap);
}

#[test]
fn test_sync_config_enable_utxo_bootstrap() {
    let config = SyncConfig {
        utxo_bootstrap: true,
        ..Default::default()
    };
    assert!(config.utxo_bootstrap);
}

// ============================================================================
// Sync State Tests
// ============================================================================

#[test]
fn test_sync_state_idle() {
    let state = SyncState::Idle;
    assert!(matches!(state, SyncState::Idle));
}

#[test]
fn test_sync_state_awaiting_snapshots_info() {
    let state = SyncState::AwaitingSnapshotsInfo {
        requested_from: 5,
        started_at_ms: 1000,
    };
    if let SyncState::AwaitingSnapshotsInfo {
        requested_from,
        started_at_ms,
    } = state
    {
        assert_eq!(requested_from, 5);
        assert_eq!(started_at_ms, 1000);
    } else {
        panic!("Expected AwaitingSnapshotsInfo state");
    }
}

#[test]
fn test_sync_state_syncing_headers() {
    let state = SyncState::SyncingHeaders {
        from: 100,
        to: 1000,
    };
    if let SyncState::SyncingHeaders { from, to } = state {
        assert_eq!(from, 100);
        assert_eq!(to, 1000);
    } else {
        panic!("Expected SyncingHeaders state");
    }
}

#[test]
fn test_sync_state_syncing_blocks() {
    let state = SyncState::SyncingBlocks { pending_count: 50 };
    if let SyncState::SyncingBlocks { pending_count } = state {
        assert_eq!(pending_count, 50);
    } else {
        panic!("Expected SyncingBlocks state");
    }
}

#[test]
fn test_sync_state_synchronized() {
    let state = SyncState::Synchronized;
    assert!(matches!(state, SyncState::Synchronized));
}

#[test]
fn test_sync_state_debug() {
    let state = SyncState::Idle;
    let debug_str = format!("{:?}", state);
    assert!(debug_str.contains("Idle"));
}

#[test]
fn test_sync_state_clone() {
    let state = SyncState::SyncingHeaders { from: 0, to: 500 };
    let cloned = state.clone();
    assert!(matches!(
        cloned,
        SyncState::SyncingHeaders { from: 0, to: 500 }
    ));
}

#[test]
fn test_sync_state_downloading_chunks() {
    let state = SyncState::DownloadingChunks {
        manifest_id: [0u8; 32],
        height: 1000,
        total_chunks: 100,
        downloaded_chunks: 50,
    };
    if let SyncState::DownloadingChunks {
        height,
        total_chunks,
        downloaded_chunks,
        ..
    } = state
    {
        assert_eq!(height, 1000);
        assert_eq!(total_chunks, 100);
        assert_eq!(downloaded_chunks, 50);
    } else {
        panic!("Expected DownloadingChunks state");
    }
}

#[test]
fn test_sync_state_applying_snapshot() {
    let state = SyncState::ApplyingSnapshot { height: 500000 };
    if let SyncState::ApplyingSnapshot { height } = state {
        assert_eq!(height, 500000);
    } else {
        panic!("Expected ApplyingSnapshot state");
    }
}

// ============================================================================
// Constants Tests
// ============================================================================

#[test]
fn test_headers_batch_size_reasonable() {
    assert!(HEADERS_BATCH_SIZE >= 10);
    assert!(HEADERS_BATCH_SIZE <= 1000);
}

#[test]
fn test_parallel_downloads_reasonable() {
    assert!(PARALLEL_DOWNLOADS >= 1);
    assert!(PARALLEL_DOWNLOADS <= 1000);
}

#[test]
fn test_max_download_queue_reasonable() {
    assert!(MAX_DOWNLOAD_QUEUE >= PARALLEL_DOWNLOADS);
}

// ============================================================================
// Download Stats Tests
// ============================================================================

#[test]
fn test_download_stats_initial() {
    let downloader = BlockDownloader::new(DownloadConfig::default());
    let stats = downloader.stats();

    assert_eq!(stats.pending, 0);
    assert_eq!(stats.in_flight, 0);
    assert_eq!(stats.completed, 0);
    assert_eq!(stats.failed, 0);
}

#[test]
fn test_download_stats_after_queue() {
    let downloader = BlockDownloader::new(DownloadConfig::default());

    let tasks: Vec<_> = (0..5).map(|i| DownloadTask::new(vec![i], 1)).collect();
    downloader.queue(tasks);

    let stats = downloader.stats();
    assert_eq!(stats.pending, 5);
    assert_eq!(stats.in_flight, 0);
}

#[test]
fn test_download_stats_after_dispatch() {
    let downloader = BlockDownloader::new(DownloadConfig::default());

    let id = vec![1, 2, 3];
    let task = DownloadTask::new(id.clone(), 1);
    downloader.queue(vec![task]);

    let peer = PeerId(vec![9, 9, 9]);
    downloader.dispatch(&id, peer);

    let stats = downloader.stats();
    // Task stays in pending (with peer assigned) and also tracked in in_flight
    assert_eq!(stats.pending, 1);
    assert_eq!(stats.in_flight, 1);
}

// ============================================================================
// Edge Case Tests
// ============================================================================

#[test]
fn test_download_empty_id() {
    let task = DownloadTask::new(vec![], 1);
    assert!(task.id.is_empty());
}

#[test]
fn test_download_large_id() {
    let large_id = vec![0u8; 1000];
    let task = DownloadTask::new(large_id.clone(), 1);
    assert_eq!(task.id.len(), 1000);
}

#[test]
fn test_downloader_complete_unknown_id() {
    let downloader = BlockDownloader::new(DownloadConfig::default());

    // Complete an ID that was never added - should not panic
    // Note: complete() always adds to completed set
    downloader.complete(&vec![99, 99, 99]);

    let stats = downloader.stats();
    // ID is added to completed set even if not previously known
    assert_eq!(stats.completed, 1);
}

#[test]
fn test_downloader_get_ready_more_than_available() {
    let downloader = BlockDownloader::new(DownloadConfig::default());

    let tasks: Vec<_> = (0..3).map(|i| DownloadTask::new(vec![i], 1)).collect();
    downloader.queue(tasks);

    // Request more than available
    let ready = downloader.get_ready_tasks(100);
    assert_eq!(ready.len(), 3);
}

#[test]
fn test_downloader_empty_queue_get_ready() {
    let downloader = BlockDownloader::new(DownloadConfig::default());

    let ready = downloader.get_ready_tasks(5);
    assert!(ready.is_empty());
}

// ============================================================================
// PeerId Tests
// ============================================================================

#[test]
fn test_peer_id_creation() {
    let peer = PeerId(vec![1, 2, 3, 4]);
    assert_eq!(peer.0, vec![1, 2, 3, 4]);
}

#[test]
fn test_peer_id_clone() {
    let peer = PeerId(vec![1, 2, 3, 4]);
    let cloned = peer.clone();
    assert_eq!(peer.0, cloned.0);
}

#[test]
fn test_peer_id_equality() {
    let peer1 = PeerId(vec![1, 2, 3]);
    let peer2 = PeerId(vec![1, 2, 3]);
    let peer3 = PeerId(vec![4, 5, 6]);

    assert_eq!(peer1, peer2);
    assert_ne!(peer1, peer3);
}

#[test]
fn test_peer_id_hash() {
    use std::collections::HashSet;

    let peer1 = PeerId(vec![1, 2, 3]);
    let peer2 = PeerId(vec![1, 2, 3]);
    let peer3 = PeerId(vec![4, 5, 6]);

    let mut set = HashSet::new();
    set.insert(peer1);

    assert!(set.contains(&peer2));
    assert!(!set.contains(&peer3));
}

// ============================================================================
// Retry and Timeout Tests
// ============================================================================

#[test]
fn test_download_task_increment_retries() {
    let mut task = DownloadTask::new(vec![1, 2, 3], 1);

    assert_eq!(task.retries, 0);

    task.retries += 1;
    assert_eq!(task.retries, 1);

    task.retries += 1;
    assert_eq!(task.retries, 2);
}

#[test]
fn test_download_config_max_retries() {
    let config = DownloadConfig {
        max_retries: 10,
        ..Default::default()
    };

    assert_eq!(config.max_retries, 10);
}

#[test]
fn test_download_task_add_failed_peer() {
    let mut task = DownloadTask::new(vec![1, 2, 3], 1);

    let peer1 = PeerId(vec![1]);
    let peer2 = PeerId(vec![2]);

    task.failed_peers.insert(peer1.clone());
    task.failed_peers.insert(peer2.clone());

    assert!(task.peer_failed(&peer1));
    assert!(task.peer_failed(&peer2));
    assert!(!task.peer_failed(&PeerId(vec![3])));
}
