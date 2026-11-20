//! Persisted torrent records on top of [`StateStore`].
//!
//! Keys: `torrent:<id>:{name,save_path,metainfo,fastresume}`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use ferrent_engine::{AddTorrent, TorrentId, TorrentSource};

use crate::store::{StateStore, StoreError, StoreOp};

const PREFIX: &str = "torrent:";
const NAME: &str = "name";
const SAVE_PATH: &str = "save_path";
const METAINFO: &str = "metainfo";
const FAST_RESUME: &str = "fastresume";
const PAUSED: &str = "paused";
const LABEL: &str = "label";
/// Single record holding the download queue order (ids, little endian u64).
const QUEUE_ORDER: &str = "queue:order";

/// Persists the download queue order.
pub fn save_queue_order(store: &mut dyn StateStore, ids: &[TorrentId]) -> Result<(), StoreError> {
    let mut value = Vec::with_capacity(ids.len() * 8);
    for id in ids {
        value.extend_from_slice(&id.to_le_bytes());
    }
    store.batch(vec![StoreOp::Put {
        key: QUEUE_ORDER.to_owned(),
        value,
    }])
}

/// Loads the persisted download queue order.
pub fn load_queue_order(store: &dyn StateStore) -> Result<Vec<TorrentId>, StoreError> {
    let Some(bytes) = store.get(QUEUE_ORDER)? else {
        return Ok(Vec::new());
    };
    Ok(bytes
        .chunks_exact(8)
        .filter_map(|chunk| <[u8; 8]>::try_from(chunk).ok())
        .map(u64::from_le_bytes)
        .collect())
}

/// One torrent record persisted across daemon restarts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedTorrent {
    pub id: TorrentId,
    pub name: String,
    pub save_path: PathBuf,
    pub metainfo: Vec<u8>,
    pub resume_data: Option<Vec<u8>>,
    /// Manual pause survives a daemon restart; scheduler/guard reasons do not.
    pub paused_by_user: bool,
    /// Label assignment; `None` means unlabelled.
    pub label: Option<String>,
}

impl PersistedTorrent {
    /// Converts the record into an engine add request.
    #[must_use]
    pub fn into_add_torrent(self) -> AddTorrent {
        AddTorrent {
            name: self.name,
            total_bytes: 0,
            source: TorrentSource::Metainfo(self.metainfo),
            save_path: self.save_path,
            resume_data: self.resume_data,
        }
    }
}

/// Saves one torrent record.
pub fn save_torrent(
    store: &mut dyn StateStore,
    torrent: &PersistedTorrent,
) -> Result<(), StoreError> {
    save_torrents(store, std::slice::from_ref(torrent))
}

/// Saves several torrent records in one transaction.
pub fn save_torrents(
    store: &mut dyn StateStore,
    torrents: &[PersistedTorrent],
) -> Result<(), StoreError> {
    let mut ops = Vec::with_capacity(torrents.len() * 4);
    for torrent in torrents {
        ops.push(StoreOp::Put {
            key: key(torrent.id, NAME),
            value: torrent.name.as_bytes().to_vec(),
        });
        ops.push(StoreOp::Put {
            key: key(torrent.id, SAVE_PATH),
            value: torrent.save_path.to_string_lossy().as_bytes().to_vec(),
        });
        ops.push(StoreOp::Put {
            key: key(torrent.id, METAINFO),
            value: torrent.metainfo.clone(),
        });
        ops.push(StoreOp::Put {
            key: key(torrent.id, PAUSED),
            value: vec![u8::from(torrent.paused_by_user)],
        });
        ops.push(StoreOp::Put {
            key: key(torrent.id, LABEL),
            value: torrent.label.clone().unwrap_or_default().into_bytes(),
        });
        if let Some(data) = &torrent.resume_data {
            ops.push(StoreOp::Put {
                key: key(torrent.id, FAST_RESUME),
                value: data.clone(),
            });
        }
    }
    store.batch(ops)
}

/// Saves fastresume blobs for several torrents in one transaction.
pub fn save_fastresumes(
    store: &mut dyn StateStore,
    entries: &[(TorrentId, Vec<u8>)],
) -> Result<(), StoreError> {
    let ops = entries
        .iter()
        .map(|(id, data)| StoreOp::Put {
            key: key(*id, FAST_RESUME),
            value: data.clone(),
        })
        .collect();
    store.batch(ops)
}

/// Saves the manual pause flag for one torrent.
pub fn save_paused(
    store: &mut dyn StateStore,
    id: TorrentId,
    paused: bool,
) -> Result<(), StoreError> {
    store.batch(vec![StoreOp::Put {
        key: key(id, PAUSED),
        value: vec![u8::from(paused)],
    }])
}

/// Saves (or clears) the label assignment for one torrent.
pub fn save_torrent_label(
    store: &mut dyn StateStore,
    id: TorrentId,
    label: Option<&str>,
) -> Result<(), StoreError> {
    store.batch(vec![StoreOp::Put {
        key: key(id, LABEL),
        value: label.unwrap_or_default().as_bytes().to_vec(),
    }])
}

/// Deletes all stored keys for one torrent.
pub fn remove_torrent(store: &mut dyn StateStore, id: TorrentId) -> Result<(), StoreError> {
    let ops = [NAME, SAVE_PATH, METAINFO, FAST_RESUME, PAUSED, LABEL]
        .into_iter()
        .map(|field| StoreOp::Delete {
            key: key(id, field),
        })
        .collect();
    store.batch(ops)
}

/// Loads every complete torrent record, ordered by id.
pub fn load_torrents(store: &dyn StateStore) -> Result<Vec<PersistedTorrent>, StoreError> {
    let entries = store.list(PREFIX)?;
    let mut records: BTreeMap<TorrentId, PartialRecord> = BTreeMap::new();

    for (key, value) in entries {
        let rest = key.strip_prefix(PREFIX).ok_or_else(|| {
            StoreError::Backend(format!("unexpected store key without prefix: {key}"))
        })?;
        let (id, field) = rest
            .split_once(':')
            .ok_or_else(|| StoreError::Backend(format!("malformed torrent key: {key}")))?;
        let id: TorrentId = id.parse().map_err(|error| {
            StoreError::Backend(format!("malformed torrent id in {key}: {error}"))
        })?;
        let record = records.entry(id).or_default();
        match field {
            NAME => {
                record.name = Some(String::from_utf8(value).map_err(|error| {
                    StoreError::Backend(format!("invalid name for torrent {id}: {error}"))
                })?);
            }
            SAVE_PATH => {
                record.save_path = Some(PathBuf::from(String::from_utf8(value).map_err(
                    |error| StoreError::Backend(format!("invalid save path for {id}: {error}")),
                )?));
            }
            METAINFO => record.metainfo = Some(value),
            FAST_RESUME => record.resume_data = Some(value),
            PAUSED => record.paused_by_user = value.first().is_some_and(|byte| *byte != 0),
            LABEL => {
                record.label = if value.is_empty() {
                    None
                } else {
                    Some(String::from_utf8(value).map_err(|error| {
                        StoreError::Backend(format!("invalid label for torrent {id}: {error}"))
                    })?)
                };
            }
            _ => {}
        }
    }

    let mut out = Vec::new();
    for (id, record) in records {
        if let (Some(metainfo), Some(save_path)) = (record.metainfo, record.save_path) {
            out.push(PersistedTorrent {
                id,
                name: record.name.unwrap_or_else(|| format!("torrent-{id}")),
                save_path,
                metainfo,
                resume_data: record.resume_data,
                paused_by_user: record.paused_by_user,
                label: record.label,
            });
        }
    }
    Ok(out)
}

#[derive(Default)]
struct PartialRecord {
    name: Option<String>,
    save_path: Option<PathBuf>,
    metainfo: Option<Vec<u8>>,
    resume_data: Option<Vec<u8>>,
    paused_by_user: bool,
    label: Option<String>,
}

fn key(id: TorrentId, field: &str) -> String {
    format!("{PREFIX}{id}:{field}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::RedbStore;

    fn temp_db(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "ferrent-persist-{name}-{}.redb",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        path
    }

    fn record(id: TorrentId, name: &str) -> PersistedTorrent {
        PersistedTorrent {
            id,
            name: name.to_owned(),
            save_path: PathBuf::from("/data"),
            metainfo: vec![1, 2, 3],
            resume_data: None,
            paused_by_user: false,
            label: None,
        }
    }

    #[test]
    fn roundtrip_fastresume_and_remove() {
        let path = temp_db("roundtrip");
        let mut store = RedbStore::open(&path).expect("open");

        save_torrent(&mut store, &record(0, "zero")).expect("save 0");
        save_torrent(&mut store, &record(1, "one")).expect("save 1");

        let loaded = load_torrents(&store).expect("load");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].name, "zero");
        assert_eq!(loaded[1].name, "one");
        assert!(loaded[0].resume_data.is_none());

        save_fastresumes(&mut store, &[(1, b"resume".to_vec())]).expect("save resume");
        save_paused(&mut store, 1, true).expect("save paused");
        save_torrent_label(&mut store, 1, Some("movies")).expect("save label");
        let loaded = load_torrents(&store).expect("load resume");
        assert_eq!(loaded[1].resume_data.as_deref(), Some(b"resume".as_slice()));
        assert!(loaded[1].paused_by_user);
        assert_eq!(loaded[1].label.as_deref(), Some("movies"));

        remove_torrent(&mut store, 0).expect("remove 0");
        let loaded = load_torrents(&store).expect("load after remove");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn incomplete_records_are_skipped() {
        let path = temp_db("incomplete");
        let mut store = RedbStore::open(&path).expect("open");
        store
            .put("torrent:7:name", b"missing metainfo")
            .expect("put");
        store.flush().expect("flush");

        let loaded = load_torrents(&store).expect("load");
        assert!(loaded.is_empty());
        let _ = std::fs::remove_file(&path);
    }
}
