#[cxx::bridge]
pub(crate) mod ffi {
    struct FfiStatus {
        found: bool,
        id: u64,
        infohash: String,
        name: String,
        save_path: String,
        total_bytes: u64,
        state: i32,
        progress: f32,
        download_rate: u64,
        upload_rate: u64,
        uploaded_bytes: u64,
        downloaded_bytes: u64,
        seeds: i32,
        peers: i32,
        eta_seconds: i64,
        error: String,
        queue_position: i32,
        sequential: bool,
        super_seeding: bool,
    }

    struct FfiAddResult {
        ok: bool,
        duplicate: bool,
        id: u64,
        error: String,
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
        message: String,
        next_announce: u32,
        fails: u32,
    }

    struct FfiCreateResult {
        ok: bool,
        metainfo: Vec<u8>,
        infohash: String,
        error: String,
    }

    struct FfiEvent {
        kind: i32,
        id: u64,
        state: i32,
        progress: f32,
        download_rate: u64,
        upload_rate: u64,
        queue_position: i32,
        seeds: i32,
        peers: i32,
    }

    unsafe extern "C++" {
        include!("ferrent-engine/src/libtorrent/bridge.h");

        type Session;

        // The bridge is a flat mirror of the C++ constructor; grouping the
        // scalars into a struct would only add conversion code.
        #[allow(clippy::too_many_arguments)]
        fn new_session(
            download_rate_limit: u64,
            upload_rate_limit: u64,
            seed_mode: bool,
            active_downloads: u32,
            active_seeds: u32,
            active_limit: u32,
            listen_interfaces: &str,
            outgoing_interfaces: &str,
        ) -> UniquePtr<Session>;
        fn add_torrent(
            self: Pin<&mut Session>,
            metainfo: &[u8],
            save_path: &str,
            resume_data: &[u8],
        ) -> FfiAddResult;
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
        fn set_torrent_limits(
            self: Pin<&mut Session>,
            id: u64,
            download_rate_limit: u64,
            upload_rate_limit: u64,
            connections_limit: u32,
            upload_slots_limit: u32,
        ) -> bool;
        fn set_default_trackers(self: Pin<&mut Session>, trackers: &str);
        #[allow(clippy::too_many_arguments)]
        fn set_network_settings(
            self: Pin<&mut Session>,
            proxy_type: &str,
            proxy_host: &str,
            proxy_port: u16,
            proxy_username: &str,
            proxy_password: &str,
            proxy_peer_connections: bool,
            encryption: &str,
            upnp: bool,
            natpmp: bool,
        );
        fn move_storage(self: Pin<&mut Session>, id: u64, path: &str) -> bool;
        fn remove(self: Pin<&mut Session>, id: u64, delete_files: bool) -> bool;
        fn move_in_queue(self: Pin<&mut Session>, id: u64, up: bool) -> bool;
        fn status(self: &Session, id: u64) -> FfiStatus;
        fn list(self: &Session) -> Vec<FfiStatus>;
        fn files(self: &Session, id: u64) -> Vec<FfiFile>;
        fn force_recheck(self: Pin<&mut Session>, id: u64) -> bool;
        fn set_sequential(self: Pin<&mut Session>, id: u64, on: bool) -> bool;
        fn set_super_seeding(self: Pin<&mut Session>, id: u64, on: bool) -> bool;
        fn add_tracker(self: Pin<&mut Session>, id: u64, url: &str) -> bool;
        fn remove_tracker(self: Pin<&mut Session>, id: u64, url: &str) -> bool;
        fn web_seeds(self: &Session, id: u64) -> Vec<String>;
        fn set_web_seeds(self: Pin<&mut Session>, id: u64, urls: &str) -> bool;
        fn rename_file(self: Pin<&mut Session>, id: u64, index: i32, name: &str) -> bool;
        fn set_file_priorities(self: Pin<&mut Session>, id: u64, priorities: &[i32]) -> bool;
        fn peers(self: &Session, id: u64) -> Vec<FfiPeer>;
        fn trackers(self: &Session, id: u64) -> Vec<FfiTracker>;
        fn poll_events(self: Pin<&mut Session>) -> Vec<FfiEvent>;
        fn request_updates(self: Pin<&mut Session>);
        fn set_settings(
            self: Pin<&mut Session>,
            download_rate_limit: u64,
            upload_rate_limit: u64,
            active_downloads: u32,
            active_seeds: u32,
            active_limit: u32,
            seed_mode: bool,
        );
        fn last_error(self: &Session) -> String;

        fn make_torrent(name: &str, payload: &[u8]) -> Vec<u8>;
        fn make_torrent_from_path(path: &str, piece_length: u32, trackers: &str)
        -> FfiCreateResult;
    }
}
