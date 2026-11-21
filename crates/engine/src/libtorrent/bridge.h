#pragma once

#include <cstdint>
#include <memory>
#include <optional>

#include "rust/cxx.h"

struct FfiStatus;
struct FfiAddResult;
struct FfiEvent;
struct FfiFile;
struct FfiPeer;
struct FfiTracker;
struct FfiCreateResult;

class Session {
  public:
    Session(uint64_t download_rate_limit, uint64_t upload_rate_limit, bool seed_mode,
            uint32_t active_downloads, uint32_t active_seeds, uint32_t active_limit,
            std::string const &listen_interfaces,
            std::string const &outgoing_interfaces);
    ~Session();

    Session(const Session &) = delete;
    Session &operator=(const Session &) = delete;

    FfiAddResult add_torrent(rust::Slice<const uint8_t> metainfo, rust::Str save_path,
                             rust::Slice<const uint8_t> resume_data);
    bool restore_torrent(uint64_t id, rust::Slice<const uint8_t> metainfo,
                         rust::Str save_path, rust::Slice<const uint8_t> resume_data);
    bool request_resume(uint64_t id);
    rust::Vec<uint8_t> take_resume(uint64_t id);
    bool pause(uint64_t id);
    bool resume(uint64_t id);
    bool set_torrent_limits(uint64_t id, uint64_t download_rate_limit,
                            uint64_t upload_rate_limit, uint32_t connections_limit,
                            uint32_t upload_slots_limit);
    void set_default_trackers(rust::Str trackers);

    /// Applies proxy, encryption and UPnP/NAT-PMP settings. Some of them only
    /// take effect on a fresh session, which is why a config save restarts the
    /// daemon.
    void set_network_settings(rust::Str proxy_type, rust::Str proxy_host,
                              uint16_t proxy_port, rust::Str proxy_username,
                              rust::Str proxy_password, bool proxy_peer_connections,
                              rust::Str encryption, bool upnp, bool natpmp);
    bool move_storage(uint64_t id, rust::Str path);
    bool remove(uint64_t id, bool delete_files);
    bool move_in_queue(uint64_t id, bool up);
    FfiStatus status(uint64_t id) const;
    rust::Vec<FfiFile> files(uint64_t id) const;
    bool force_recheck(uint64_t id);
    bool set_sequential(uint64_t id, bool on);
    bool set_super_seeding(uint64_t id, bool on);
    bool add_tracker(uint64_t id, rust::Str url);
    bool remove_tracker(uint64_t id, rust::Str url);
    rust::Vec<rust::String> web_seeds(uint64_t id) const;
    /// Replaces every web seed; the URLs are separated by newlines.
    bool set_web_seeds(uint64_t id, rust::Str urls);
    bool rename_file(uint64_t id, int32_t index, rust::Str name);
    bool set_file_priorities(uint64_t id, rust::Slice<const int32_t> priorities);
    rust::Vec<FfiPeer> peers(uint64_t id) const;
    rust::Vec<FfiTracker> trackers(uint64_t id) const;
    rust::Vec<FfiStatus> list() const;
    rust::Vec<FfiEvent> poll_events();
    void request_updates();
    void set_settings(uint64_t download_rate_limit, uint64_t upload_rate_limit,
                      uint32_t active_downloads, uint32_t active_seeds, uint32_t active_limit,
                      bool seed_mode);
    rust::String last_error() const;

  private:
    FfiAddResult add_impl(std::optional<uint64_t> requested_id,
                          rust::Slice<const uint8_t> metainfo,
                          rust::Str save_path,
                          rust::Slice<const uint8_t> resume_data);

    struct Impl;
    std::unique_ptr<Impl> impl_;
};

std::unique_ptr<Session> new_session(uint64_t download_rate_limit, uint64_t upload_rate_limit,
                                     bool seed_mode, uint32_t active_downloads,
                                     uint32_t active_seeds, uint32_t active_limit,
                                     rust::Str listen_interfaces,
                                     rust::Str outgoing_interfaces);

rust::Vec<uint8_t> make_torrent(rust::Str name, rust::Slice<const uint8_t> payload);

/// Builds a .torrent for a file or directory on the daemon host. Trackers are
/// a newline-separated list.
FfiCreateResult make_torrent_from_path(rust::Str path, uint32_t piece_length,
                                       rust::Str trackers);
