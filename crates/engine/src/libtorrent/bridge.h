#pragma once

#include <cstdint>
#include <memory>

#include "rust/cxx.h"

struct FfiStatus;
struct FfiEvent;

class Session {
  public:
    Session(uint64_t download_rate_limit, uint64_t upload_rate_limit);
    ~Session();

    Session(const Session &) = delete;
    Session &operator=(const Session &) = delete;

    uint64_t add_torrent(rust::Slice<const uint8_t> metainfo, rust::Str save_path);
    bool pause(uint64_t id);
    bool resume(uint64_t id);
    bool remove(uint64_t id);
    FfiStatus status(uint64_t id) const;
    rust::Vec<FfiStatus> list() const;
    rust::Vec<FfiEvent> poll_events();
    void set_settings(uint64_t download_rate_limit, uint64_t upload_rate_limit);
    rust::String last_error() const;

  private:
    struct Impl;
    std::unique_ptr<Impl> impl_;
};

std::unique_ptr<Session> new_session(uint64_t download_rate_limit, uint64_t upload_rate_limit);
