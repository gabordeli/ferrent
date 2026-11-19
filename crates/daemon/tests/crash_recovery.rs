#![cfg(unix)]

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::Duration;

use ferrent_daemon::store::{RedbStore, StateStore};

#[test]
fn kill_during_open_write_keeps_db_readable() {
    let path = std::env::temp_dir().join(format!("ferrent-crash-{}.redb", std::process::id()));
    let _ = std::fs::remove_file(&path);

    let mut child = Command::new(env!("CARGO_BIN_EXE_crash_writer"))
        .arg(&path)
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn crash writer");

    let stdout = child.stdout.take().expect("stdout");
    let mut lines = BufReader::new(stdout).lines();
    let mut reached_write = false;
    for line in &mut lines {
        let line = line.expect("read line");
        if line == "writing" {
            reached_write = true;
            break;
        }
    }
    assert!(
        reached_write,
        "crash writer did not reach the open write transaction"
    );

    std::thread::sleep(Duration::from_millis(100));
    child.kill().expect("kill child");
    let _ = child.wait();

    let store = RedbStore::open(&path).expect("reopen store after SIGKILL");
    assert_eq!(
        store.get("baseline").expect("read baseline"),
        Some(b"ok".to_vec())
    );
    assert_eq!(store.get("uncommitted").expect("read uncommitted"), None);

    let _ = std::fs::remove_file(&path);
}
