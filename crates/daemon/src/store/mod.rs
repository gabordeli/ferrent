//! Persistent state for the daemon.
//!
//! Keys are UTF-8 strings (`meta:`, `torrent:`, `label:`, ...), values are
//! opaque byte blobs. The store buffers writes and commits only the dirty
//! records on [`StateStore::flush`].

mod redb_store;

use std::fmt;

pub use redb_store::RedbStore;

/// Schema version written to the database on first open.
pub const SCHEMA_VERSION: u32 = 1;

/// redb table holding the key/value state.
pub const TABLE_NAME: &str = "ferrent_state";

/// One mutation in a [`StateStore::batch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreOp {
    Put { key: String, value: Vec<u8> },
    Delete { key: String },
}

/// Store-level failures. Backend errors are stringified so the type stays
/// cloneable and independent of the redb error hierarchy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    Backend(String),
    UnsupportedSchema { found: u32, expected: u32 },
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backend(message) => formatter.write_str(message),
            Self::UnsupportedSchema { found, expected } => {
                write!(
                    formatter,
                    "unsupported schema version {found} (expected {expected})"
                )
            }
        }
    }
}

impl std::error::Error for StoreError {}

/// Persistent key/value store used by the daemon.
pub trait StateStore: Send {
    /// Reads a value, including buffered (not yet flushed) writes.
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, StoreError>;

    /// Lists values with the given key prefix, including buffered writes.
    fn list(&self, prefix: &str) -> Result<Vec<(String, Vec<u8>)>, StoreError>;

    /// Buffers a write. Nothing reaches the database until [`Self::flush`].
    fn put(&mut self, key: &str, value: &[u8]) -> Result<(), StoreError>;

    /// Buffers a delete.
    fn delete(&mut self, key: &str) -> Result<(), StoreError>;

    /// Applies a batch atomically and commits it in one transaction.
    fn batch(&mut self, ops: Vec<StoreOp>) -> Result<(), StoreError>;

    /// Commits the buffered records. A no-op when nothing is dirty.
    fn flush(&mut self) -> Result<(), StoreError>;

    /// Number of records waiting to be flushed.
    fn dirty_len(&self) -> usize;

    /// Number of records written by the last non-empty flush.
    fn last_flush_writes(&self) -> usize;

    /// Schema version stored in the database.
    fn schema_version(&self) -> Result<u32, StoreError>;
}
