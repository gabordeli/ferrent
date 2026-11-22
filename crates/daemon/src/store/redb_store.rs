use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use redb::{Database, DatabaseError, Durability, ReadableTable, TableDefinition};

use super::{SCHEMA_VERSION, StateStore, StoreError, StoreOp, TABLE_NAME};

const TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new(TABLE_NAME);
const SCHEMA_KEY: &str = "meta:schema_version";

/// Startup waits up to ten seconds for a previous daemon to release the lock.
const LOCK_RETRIES: u32 = 40;
const LOCK_BACKOFF: Duration = Duration::from_millis(250);

/// redb-backed [`StateStore`].
pub struct RedbStore {
    db: Database,
    pending: BTreeMap<String, Option<Vec<u8>>>,
    last_flush_writes: usize,
}

impl RedbStore {
    /// Opens (or creates) the database and ensures the schema table/version.
    ///
    /// redb keeps an exclusive file lock, so a restart that starts before the
    /// previous daemon has fully exited used to abort with "Database already
    /// open"; wait a few seconds instead.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        Self::open_with(path, LOCK_RETRIES, LOCK_BACKOFF)
    }

    /// [`Self::open`] with an explicit retry budget (tests use a short one).
    pub fn open_with(
        path: impl AsRef<Path>,
        retries: u32,
        backoff: Duration,
    ) -> Result<Self, StoreError> {
        let path = path.as_ref();
        let mut attempt = 0;
        let db = loop {
            match Database::create(path) {
                Ok(db) => break db,
                Err(error)
                    if matches!(error, DatabaseError::DatabaseAlreadyOpen) && attempt < retries =>
                {
                    attempt += 1;
                    eprintln!("waiting for the state lock ({attempt}/{retries}): {error}");
                    std::thread::sleep(backoff);
                }
                Err(error) => return Err(backend(error)),
            }
        };
        let mut store = Self {
            db,
            pending: BTreeMap::new(),
            last_flush_writes: 0,
        };
        store.ensure_table()?;
        store.ensure_schema()?;
        Ok(store)
    }

    fn ensure_table(&self) -> Result<(), StoreError> {
        let write = self.db.begin_write().map_err(backend)?;
        {
            let _table = write.open_table(TABLE).map_err(backend)?;
        }
        write.commit().map_err(backend)?;
        Ok(())
    }

    fn read_schema(&self) -> Result<Option<u32>, StoreError> {
        let read = self.db.begin_read().map_err(backend)?;
        let table = read.open_table(TABLE).map_err(backend)?;
        let value = table.get(SCHEMA_KEY).map_err(backend)?;
        match value {
            Some(value) => {
                let bytes: &[u8] = value.value();
                let text = std::str::from_utf8(bytes)
                    .map_err(|error| StoreError::Backend(error.to_string()))?;
                let version = text
                    .parse::<u32>()
                    .map_err(|error| StoreError::Backend(error.to_string()))?;
                Ok(Some(version))
            }
            None => Ok(None),
        }
    }

    fn ensure_schema(&mut self) -> Result<(), StoreError> {
        match self.read_schema()? {
            Some(SCHEMA_VERSION) => Ok(()),
            Some(found) => Err(StoreError::UnsupportedSchema {
                found,
                expected: SCHEMA_VERSION,
            }),
            None => {
                self.pending.insert(
                    SCHEMA_KEY.to_owned(),
                    Some(SCHEMA_VERSION.to_string().into_bytes()),
                );
                self.flush()
            }
        }
    }
}

impl StateStore for RedbStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
        if let Some(value) = self.pending.get(key) {
            return Ok(value.clone());
        }

        let read = self.db.begin_read().map_err(backend)?;
        let table = read.open_table(TABLE).map_err(backend)?;
        let value = table.get(key).map_err(backend)?;
        match value {
            Some(value) => {
                let bytes: &[u8] = value.value();
                Ok(Some(bytes.to_vec()))
            }
            None => Ok(None),
        }
    }

    fn list(&self, prefix: &str) -> Result<Vec<(String, Vec<u8>)>, StoreError> {
        let read = self.db.begin_read().map_err(backend)?;
        let table = read.open_table(TABLE).map_err(backend)?;
        let mut out = BTreeMap::new();

        for item in table.iter().map_err(backend)? {
            let (key, value) = item.map_err(backend)?;
            let key = key.value().to_string();
            if key.starts_with(prefix) {
                let bytes: &[u8] = value.value();
                out.insert(key, bytes.to_vec());
            }
        }

        for (key, value) in &self.pending {
            if !key.starts_with(prefix) {
                continue;
            }
            match value {
                Some(value) => {
                    out.insert(key.clone(), value.clone());
                }
                None => {
                    out.remove(key);
                }
            }
        }

        Ok(out.into_iter().collect())
    }

    fn put(&mut self, key: &str, value: &[u8]) -> Result<(), StoreError> {
        self.pending.insert(key.to_owned(), Some(value.to_vec()));
        Ok(())
    }

    fn delete(&mut self, key: &str) -> Result<(), StoreError> {
        self.pending.insert(key.to_owned(), None);
        Ok(())
    }

    fn batch(&mut self, ops: Vec<StoreOp>) -> Result<(), StoreError> {
        for op in ops {
            match op {
                StoreOp::Put { key, value } => {
                    self.pending.insert(key, Some(value));
                }
                StoreOp::Delete { key } => {
                    self.pending.insert(key, None);
                }
            }
        }
        self.flush()
    }

    fn flush(&mut self) -> Result<(), StoreError> {
        if self.pending.is_empty() {
            self.last_flush_writes = 0;
            return Ok(());
        }

        let mut write = self.db.begin_write().map_err(backend)?;
        // Eventual is the right trade-off for fastresume state: a process
        // crash keeps the commit; a power loss may lose the last seconds,
        // which the daemon can recover from with a recheck.
        write.set_durability(Durability::Eventual);
        {
            let mut table = write.open_table(TABLE).map_err(backend)?;
            for (key, value) in &self.pending {
                match value {
                    Some(value) => {
                        table
                            .insert(key.as_str(), value.as_slice())
                            .map_err(backend)?;
                    }
                    None => {
                        table.remove(key.as_str()).map_err(backend)?;
                    }
                }
            }
        }
        write.commit().map_err(backend)?;

        self.last_flush_writes = self.pending.len();
        self.pending.clear();
        Ok(())
    }

    fn dirty_len(&self) -> usize {
        self.pending.len()
    }

    fn last_flush_writes(&self) -> usize {
        self.last_flush_writes
    }

    fn schema_version(&self) -> Result<u32, StoreError> {
        self.read_schema()?
            .ok_or_else(|| StoreError::Backend("schema version missing".to_owned()))
    }
}

fn backend<E: std::fmt::Display>(error: E) -> StoreError {
    StoreError::Backend(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use super::*;

    fn temp_db(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("ferrent-store-{name}-{}.redb", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn a_locked_state_waits_for_the_previous_owner() {
        let path = temp_db("lock");
        let first = RedbStore::open(&path).expect("first open");

        // The lock is held: a short budget gives up...
        let error = RedbStore::open_with(&path, 2, Duration::from_millis(10))
            .err()
            .expect("the lock must be reported");
        assert!(matches!(error, StoreError::Backend(_)), "{error:?}");

        // ...and a patient open succeeds once the first store is gone.
        drop(first);
        let second =
            RedbStore::open_with(&path, 100, Duration::from_millis(10)).expect("after the release");
        drop(second);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn roundtrip_and_schema_version() {
        let path = temp_db("roundtrip");
        let mut store = RedbStore::open(&path).expect("open");
        assert_eq!(store.schema_version().expect("schema"), SCHEMA_VERSION);

        store.put("torrent:1", b"fastresume").expect("put");
        assert_eq!(
            store.get("torrent:1").expect("get"),
            Some(b"fastresume".to_vec())
        );
        store.flush().expect("flush");
        drop(store);

        let mut store = RedbStore::open(&path).expect("reopen");
        assert_eq!(
            store.get("torrent:1").expect("get"),
            Some(b"fastresume".to_vec())
        );
        store.delete("torrent:1").expect("delete");
        assert_eq!(store.get("torrent:1").expect("pending delete"), None);
        store.flush().expect("flush delete");
        assert_eq!(store.get("torrent:1").expect("deleted"), None);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn batch_writes_only_dirty_records_and_loads_under_budget() {
        let path = temp_db("batch");
        let mut store = RedbStore::open(&path).expect("open");

        let ops: Vec<StoreOp> = (0..4_000)
            .map(|index| StoreOp::Put {
                key: format!("torrent:{index:04}"),
                value: vec![index as u8; 32],
            })
            .collect();
        let start = Instant::now();
        store.batch(ops).expect("batch");
        let batch_elapsed = start.elapsed();
        // The debug budget has slack for a busy workstation (a running daemon,
        // GUI and browser) and for the shared GitHub runner, which runs the
        // other 148 tests in parallel on a few cores (measured 1.03 s there);
        // release keeps the tight one. A regression that rewrites every
        // record still lands far above this.
        let batch_budget = if cfg!(debug_assertions) {
            Duration::from_secs(5)
        } else {
            Duration::from_millis(100)
        };
        println!("redb batch 4000: {batch_elapsed:?} (budget {batch_budget:?})");
        assert!(
            batch_elapsed < batch_budget,
            "batch took {batch_elapsed:?} (budget {batch_budget:?})"
        );
        assert_eq!(store.last_flush_writes(), 4_000);
        assert_eq!(store.dirty_len(), 0);

        store.put("torrent:0001", b"changed").expect("put");
        assert_eq!(store.dirty_len(), 1);
        store.flush().expect("flush");
        assert_eq!(store.last_flush_writes(), 1);

        drop(store);
        let start = Instant::now();
        let store = RedbStore::open(&path).expect("reopen");
        let all = store.list("torrent:").expect("list");
        let load_elapsed = start.elapsed();
        println!("redb open+load 4000: {load_elapsed:?}");
        assert_eq!(all.len(), 4_000);
        assert_eq!(
            store.get("torrent:0001").expect("get changed"),
            Some(b"changed".to_vec())
        );
        assert_eq!(
            store.get("torrent:0000").expect("get original"),
            Some(vec![0u8; 32])
        );
        assert!(
            load_elapsed < Duration::from_secs(2),
            "load took {load_elapsed:?}"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn list_prefix_includes_pending_overlay() {
        let path = temp_db("prefix");
        let mut store = RedbStore::open(&path).expect("open");
        store.put("torrent:1", b"a").expect("put");
        store.put("torrent:2", b"b").expect("put");
        store.put("label:linux", b"l").expect("put");
        store.flush().expect("flush");

        store.put("torrent:3", b"c").expect("put");
        store.delete("torrent:1").expect("delete");

        let torrents = store.list("torrent:").expect("list");
        assert_eq!(torrents.len(), 2);
        assert_eq!(torrents[0].0, "torrent:2");
        assert_eq!(torrents[1].0, "torrent:3");
        let labels = store.list("label:").expect("list");
        assert_eq!(labels.len(), 1);

        let _ = std::fs::remove_file(&path);
    }
}
