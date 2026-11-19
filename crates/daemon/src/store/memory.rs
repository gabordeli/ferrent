//! In-memory [`StateStore`] for tests and ephemeral runs.

use std::collections::BTreeMap;

use super::{SCHEMA_VERSION, StateStore, StoreError, StoreOp};

#[derive(Debug, Default)]
pub struct MemoryStore {
    data: BTreeMap<String, Vec<u8>>,
    pending: BTreeMap<String, Option<Vec<u8>>>,
    last_flush_writes: usize,
}

impl MemoryStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl StateStore for MemoryStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
        if let Some(value) = self.pending.get(key) {
            return Ok(value.clone());
        }
        Ok(self.data.get(key).cloned())
    }

    fn list(&self, prefix: &str) -> Result<Vec<(String, Vec<u8>)>, StoreError> {
        let mut out = BTreeMap::new();
        for (key, value) in &self.data {
            if key.starts_with(prefix) {
                out.insert(key.clone(), value.clone());
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
        self.last_flush_writes = self.pending.len();
        for (key, value) in std::mem::take(&mut self.pending) {
            match value {
                Some(value) => {
                    self.data.insert(key, value);
                }
                None => {
                    self.data.remove(&key);
                }
            }
        }
        Ok(())
    }

    fn dirty_len(&self) -> usize {
        self.pending.len()
    }

    fn last_flush_writes(&self) -> usize {
        self.last_flush_writes
    }

    fn schema_version(&self) -> Result<u32, StoreError> {
        Ok(SCHEMA_VERSION)
    }
}
