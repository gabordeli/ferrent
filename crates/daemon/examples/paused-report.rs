//! Counts the persisted torrents that are flagged as paused by the user.
//!
//! `cargo run -q -p ferrent-daemon --example paused-report -- <state.redb>`
//!
//! Tells a store problem from an engine problem: if the count is zero the
//! pause flag never reached the store; if it is non-zero the restore hands it
//! to the engine and the engine drops it.

use ferrent_daemon::persist;
use ferrent_daemon::store::RedbStore;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: paused-report <state.redb>")?;
    let store = RedbStore::open(path)?;
    let torrents = persist::load_torrents(&store)?;
    let paused = torrents
        .iter()
        .filter(|torrent| torrent.paused_by_user)
        .count();
    println!("torrents={} paused_by_user={paused}", torrents.len());
    for torrent in torrents.iter().take(3) {
        println!(
            "  id={} paused={} name={}",
            torrent.id, torrent.paused_by_user, torrent.name
        );
    }
    Ok(())
}
