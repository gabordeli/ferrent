#[cxx::bridge]
pub(crate) mod ffi {
    struct FfiStatus {
        found: bool,
        id: u64,
        name: String,
        save_path: String,
        total_bytes: u64,
        state: i32,
        progress: f32,
        download_rate: u64,
        upload_rate: u64,
        seeds: i32,
        peers: i32,
        eta_seconds: i64,
        error: String,
        queue_position: i32,
    }

    struct FfiFile {
        path: String,
        size: u64,
        progress: f32,
        priority: i32,
    }

    struct FfiPeer {
        endpoint: String,
        client: String,
        progress: f32,
        download_rate: u64,
        upload_rate: u64,
    }

    struct FfiTracker {
        url: String,
        status: String,
        seeds: u32,
        leechers: u32,
    }

    struct FfiEvent {
        kind: i32,
        id: u64,
        state: i32,
        progress: f32,
        download_rate: u64,
        upload_rate: u64,
        queue_position: i32,
    }

    unsafe extern "C++" {
        include!("ferrent-engine/src/libtorrent/bridge.h");

        type Session;

        fn new_session(
            download_rate_limit: u64,
            upload_rate_limit: u64,
            seed_mode: bool,
        ) -> UniquePtr<Session>;
        fn add_torrent(
            self: Pin<&mut Session>,
            metainfo: &[u8],
            save_path: &str,
            resume_data: &[u8],
        ) -> u64;
        fn restore_torrent(
            self: Pin<&mut Session>,
            id: u64,
            metainfo: &[u8],
            save_path: &str,
            resume_data: &[u8],
        ) -> bool;
        fn request_resume(self: Pin<&mut Session>, id: u64) -> bool;
        fn take_resume(self: Pin<&mut Session>, id: u64) -> Vec<u8>;
        fn pause(self: Pin<&mut Session>, id: u64) -> bool;
        fn resume(self: Pin<&mut Session>, id: u64) -> bool;
        fn remove(self: Pin<&mut Session>, id: u64, delete_files: bool) -> bool;
        fn move_in_queue(self: Pin<&mut Session>, id: u64, up: bool) -> bool;
        fn status(self: &Session, id: u64) -> FfiStatus;
        fn list(self: &Session) -> Vec<FfiStatus>;
        fn files(self: &Session, id: u64) -> Vec<FfiFile>;
        fn peers(self: &Session, id: u64) -> Vec<FfiPeer>;
        fn trackers(self: &Session, id: u64) -> Vec<FfiTracker>;
        fn poll_events(self: Pin<&mut Session>) -> Vec<FfiEvent>;
        fn request_updates(self: Pin<&mut Session>);
        fn set_settings(self: Pin<&mut Session>, download_rate_limit: u64, upload_rate_limit: u64);
        fn last_error(self: &Session) -> String;

        fn make_torrent(name: &str, payload: &[u8]) -> Vec<u8>;
    }
}
