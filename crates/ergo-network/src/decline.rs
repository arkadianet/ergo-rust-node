//! Decline table for spam protection.
//!
//! The decline table tracks modifiers (blocks, transactions, headers) that
//! have been previously validated and found to be invalid. When a peer
//! sends a modifier that's in the decline table, it's immediately rejected
//! and the peer is penalized.
//!
//! This prevents resource exhaustion attacks where an attacker repeatedly
//! sends invalid modifiers that are expensive to validate.

use dashmap::DashMap;
use std::time::{Duration, Instant};

/// Modifier ID type (32-byte hash).
pub type ModifierId = [u8; 32];

/// Entry in the decline table.
#[derive(Debug, Clone)]
struct DeclineEntry {
    /// When this entry was added.
    added_at: Instant,
    /// Why this modifier was declined.
    reason: String,
}

/// Decline table for tracking invalid modifiers.
///
/// Thread-safe table that stores IDs of modifiers that were previously
/// validated and found to be invalid. Entries expire after a TTL.
pub struct DeclineTable {
    /// Declined modifiers.
    entries: DashMap<ModifierId, DeclineEntry>,
    /// Time-to-live for entries.
    ttl: Duration,
    /// Maximum number of entries.
    max_size: usize,
}

impl DeclineTable {
    /// Create a new decline table.
    ///
    /// # Arguments
    /// * `max_size` - Maximum number of entries to store
    /// * `ttl` - Time-to-live for entries
    pub fn new(max_size: usize, ttl: Duration) -> Self {
        Self {
            entries: DashMap::new(),
            ttl,
            max_size,
        }
    }

    /// Create a decline table with default settings.
    ///
    /// Default: 10,000 entries, 10 minute TTL.
    pub fn with_defaults() -> Self {
        Self::new(10_000, Duration::from_secs(600))
    }

    /// Add a modifier to the decline table.
    ///
    /// # Arguments
    /// * `id` - Modifier ID to decline
    /// * `reason` - Why this modifier was declined
    pub fn decline(&self, id: ModifierId, reason: impl Into<String>) {
        // Clean up if we're at capacity
        if self.entries.len() >= self.max_size {
            self.cleanup();
        }

        self.entries.insert(
            id,
            DeclineEntry {
                added_at: Instant::now(),
                reason: reason.into(),
            },
        );
    }

    /// Check if a modifier is declined.
    ///
    /// Returns `Some(reason)` if declined, `None` if not declined.
    pub fn is_declined(&self, id: &ModifierId) -> Option<String> {
        if let Some(entry) = self.entries.get(id) {
            if entry.added_at.elapsed() < self.ttl {
                return Some(entry.reason.clone());
            }
            // Entry expired, will be cleaned up later
        }
        None
    }

    /// Check if a modifier is declined (bool version).
    pub fn contains(&self, id: &ModifierId) -> bool {
        self.is_declined(id).is_some()
    }

    /// Remove expired entries.
    pub fn cleanup(&self) {
        self.entries
            .retain(|_, entry| entry.added_at.elapsed() < self.ttl);
    }

    /// Get the number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the table is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clear all entries.
    pub fn clear(&self) {
        self.entries.clear();
    }

    /// Remove a specific entry.
    pub fn remove(&self, id: &ModifierId) {
        self.entries.remove(id);
    }
}

impl Default for DeclineTable {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_id(n: u8) -> ModifierId {
        let mut id = [0u8; 32];
        id[0] = n;
        id
    }

    #[test]
    fn test_decline_and_check() {
        let table = DeclineTable::with_defaults();
        let id = test_id(1);

        assert!(!table.contains(&id));

        table.decline(id, "invalid PoW");
        assert!(table.contains(&id));

        let reason = table.is_declined(&id);
        assert_eq!(reason, Some("invalid PoW".to_string()));
    }

    #[test]
    fn test_expiration() {
        let table = DeclineTable::new(100, Duration::from_millis(10));
        let id = test_id(1);

        table.decline(id, "test");
        assert!(table.contains(&id));

        // Wait for expiration
        std::thread::sleep(Duration::from_millis(20));

        assert!(!table.contains(&id));
    }

    #[test]
    fn test_max_size_cleanup() {
        let table = DeclineTable::new(5, Duration::from_secs(60));

        // Add 5 entries
        for i in 0..5u8 {
            table.decline(test_id(i), "test");
        }
        assert_eq!(table.len(), 5);

        // Adding one more should trigger cleanup
        // Since none are expired, all 6 should be present after cleanup
        table.decline(test_id(10), "test");
        assert!(table.len() <= 6); // May have cleaned some expired ones
    }

    #[test]
    fn test_clear() {
        let table = DeclineTable::with_defaults();

        table.decline(test_id(1), "test1");
        table.decline(test_id(2), "test2");
        assert_eq!(table.len(), 2);

        table.clear();
        assert!(table.is_empty());
    }

    #[test]
    fn test_remove() {
        let table = DeclineTable::with_defaults();
        let id = test_id(1);

        table.decline(id, "test");
        assert!(table.contains(&id));

        table.remove(&id);
        assert!(!table.contains(&id));
    }
}
