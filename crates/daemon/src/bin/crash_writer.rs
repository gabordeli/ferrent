//! Test helper: keeps a redb write transaction open until the parent sends
//! SIGKILL, so the crash-recovery test can verify that a killed process leaves
//! a readable database.

use std::io::{self, Write};
use std::time::Duration;

use ferrent_daemon::store::{RedbStore, StateStore, TABLE_NAME};
use redb::{Database, TableDefinition};

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: crash_writer <db-path>");

    let mut store = RedbStore::open(&path).expect("open store");
    store.put("baseline", b"ok").expect("put baseline");
    store.flush().expect("flush baseline");
    drop(store);

    let db = Database::create(&path).expect("open raw db");
    let definition: TableDefinition<&str, &[u8]> = TableDefinition::new(TABLE_NAME);
    let write = db.begin_write().expect("begin write");
    {
        let mut table = write.open_table(definition).expect("open table");
        let payload = vec![0u8; 8 * 1024 * 1024];
        table
            .insert("uncommitted", payload.as_slice())
            .expect("insert");
    }

    println!("writing");
    io::stdout().flush().expect("flush stdout");
    std::thread::sleep(Duration::from_secs(60));
}
