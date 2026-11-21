//! Prints the persisted torrent records of a state database: id, name length
//! and metainfo size. Used to diagnose a restore that came back without
//! metadata.
//!
//! `cargo run -p ferrent-daemon --example store_dump -- /tmp/ferrent-live/state.redb`

use ferrent_daemon::persist::load_torrents;
use ferrent_daemon::store::{RedbStore, StateStore};

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: store_dump <state.redb>");
    let store = RedbStore::open(&path).expect("open store");
    let records = load_torrents(&store).expect("load");
    println!("{} record(s)", records.len());
    for record in records {
        println!(
            "id={:<3} name={:<44} metainfo={:<8} resume={:<7} path={}",
            record.id,
            record.name,
            record.metainfo.len(),
            record.resume_data.as_ref().map_or(0, Vec::len),
            record.save_path.display()
        );
    }
    let keys = store.list("torrent:").expect("list");
    println!("{} raw key(s)", keys.len());
    for (key, value) in keys.iter().take(40) {
        println!("  {key} = {} bytes", value.len());
    }
}
