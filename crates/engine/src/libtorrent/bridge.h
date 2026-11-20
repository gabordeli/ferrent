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

class Session {
  public:
    Session(uint64_t download_rate_limit, uint64_t upload_rate_limit, bool seed_mode,
            uint32_t active_downloads, uint32_t active_seeds, uint32_t active_limit);
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
    bool move_storage(uint64_t id, rust::Str path);
    bool remove(uint64_t id, bool delete_files);
    bool move_in_queue(uint64_t id, bool up);
    FfiStatus status(uint64_t id) const;
    rust::Vec<FfiFile> files(uint64_t id) const;
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
                                     uint32_t active_seeds, uint32_t active_limit);

rust::Vec<uint8_t> make_torrent(rust::Str name, rust::Slice<const uint8_t> payload);
