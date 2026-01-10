//! Test harness for integration tests.
//!
//! Provides utilities for creating test nodes, databases, and managing
//! test fixtures.

use ergo_storage::{ColumnFamily, Database, Storage};
use std::path::PathBuf;
use tempfile::TempDir;

/// Test database wrapper that cleans up on drop.
pub struct TestDatabase {
    db: Database,
    _temp_dir: TempDir,
}

impl TestDatabase {
    /// Create a new test database in a temporary directory.
    pub fn new() -> Self {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");
        Self {
            db,
            _temp_dir: temp_dir,
        }
    }

    /// Get the path to the database.
    pub fn path(&self) -> PathBuf {
        self._temp_dir.path().to_path_buf()
    }

    /// Get a reference to the database.
    pub fn db(&self) -> &Database {
        &self.db
    }

    /// Get a clone of the database (shares underlying connection).
    pub fn db_clone(&self) -> Database {
        self.db.clone()
    }
}

impl Default for TestDatabase {
    fn default() -> Self {
        Self::new()
    }
}

impl std::ops::Deref for TestDatabase {
    type Target = Database;

    fn deref(&self) -> &Self::Target {
        &self.db
    }
}

/// Test context containing shared resources for integration tests.
pub struct TestContext {
    /// The test database.
    pub db: TestDatabase,
}

impl TestContext {
    /// Create a new test context.
    pub fn new() -> Self {
        Self {
            db: TestDatabase::new(),
        }
    }
}

impl Default for TestContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper trait for test assertions.
pub trait TestAssertions {
    /// Assert that a value is Some and return the inner value.
    fn assert_some<T>(opt: Option<T>, msg: &str) -> T {
        opt.expect(msg)
    }

    /// Assert that a value is None.
    fn assert_none<T>(opt: Option<T>, msg: &str) {
        assert!(opt.is_none(), "{}", msg);
    }

    /// Assert that a Result is Ok and return the inner value.
    fn assert_ok<T, E: std::fmt::Debug>(result: Result<T, E>, msg: &str) -> T {
        result.expect(msg)
    }

    /// Assert that a Result is Err.
    fn assert_err<T: std::fmt::Debug, E>(result: Result<T, E>, msg: &str) {
        assert!(result.is_err(), "{}", msg);
    }
}

impl TestAssertions for TestContext {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_database_creation() {
        let test_db = TestDatabase::new();

        // Should be able to write and read
        test_db
            .put(ColumnFamily::Metadata, b"test_key", b"test_value")
            .unwrap();
        let value = test_db.get(ColumnFamily::Metadata, b"test_key").unwrap();

        assert_eq!(value, Some(b"test_value".to_vec()));
    }

    #[test]
    fn test_database_persistence_within_session() {
        let test_db = TestDatabase::new();

        // Write data
        test_db
            .put(ColumnFamily::Headers, b"header1", b"data1")
            .unwrap();

        // Clone and verify data is visible
        let db_clone = test_db.db_clone();
        let value = db_clone.get(ColumnFamily::Headers, b"header1").unwrap();

        assert_eq!(value, Some(b"data1".to_vec()));
    }

    #[test]
    fn test_context_creation() {
        let ctx = TestContext::new();

        // Should have a working database
        ctx.db
            .put(ColumnFamily::Metadata, b"ctx_key", b"ctx_value")
            .unwrap();
        let value = ctx.db.get(ColumnFamily::Metadata, b"ctx_key").unwrap();

        assert_eq!(value, Some(b"ctx_value".to_vec()));
    }
}
