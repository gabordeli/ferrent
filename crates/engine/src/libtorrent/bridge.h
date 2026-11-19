#pragma once

#include <cstdint>
#include <memory>
#include <optional>

#include "rust/cxx.h"

struct FfiStatus;
struct FfiEvent;

class Session {
  public:
    Session(uint64_t download_rate_limit, uint64_t upload_rate_limit, bool seed_mode);
    ~Session();

    Session(const Session &) = delete;
    Session &operator=(const Session &) = delete;

    uint64_t add_torrent(rust::Slice<const uint8_t> metainfo, rust::Str save_path,
                         rust::Slice<const uint8_t> resume_data);
    bool restore_torrent(uint64_t id, rust::Slice<const uint8_t> metainfo,
                         rust::Str save_path, rust::Slice<const uint8_t> resume_data);
    bool request_resume(uint64_t id);
    rust::Vec<uint8_t> take_resume(uint64_t id);
    bool pause(uint64_t id);
    bool resume(uint64_t id);
    bool remove(uint64_t id);
    FfiStatus status(uint64_t id) const;
    rust::Vec<FfiStatus> list() const;
    rust::Vec<FfiEvent> poll_events();
    void request_updates();
    void set_settings(uint64_t download_rate_limit, uint64_t upload_rate_limit);
    rust::String last_error() const;

  private:
    std::optional<uint64_t> add_impl(std::optional<uint64_t> requested_id,
                                     rust::Slice<const uint8_t> metainfo,
                                     rust::Str save_path,
                                     rust::Slice<const uint8_t> resume_data);

    struct Impl;
    std::unique_ptr<Impl> impl_;
};

std::unique_ptr<Session> new_session(uint64_t download_rate_limit, uint64_t upload_rate_limit,
                                     bool seed_mode);

rust::Vec<uint8_t> make_torrent(rust::Str name, rust::Slice<const uint8_t> payload);
