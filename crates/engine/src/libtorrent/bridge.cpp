#include "bridge.h"

#include "ferrent-engine/src/libtorrent/bridge.rs.h"

#include <libtorrent/add_torrent_params.hpp>
#include <libtorrent/alert.hpp>
#include <libtorrent/alert_types.hpp>
#include <libtorrent/bencode.hpp>
#include <libtorrent/create_torrent.hpp>
#include <libtorrent/error_code.hpp>
#include <libtorrent/file_storage.hpp>
#include <libtorrent/hasher.hpp>
#include <libtorrent/read_resume_data.hpp>
#include <libtorrent/session.hpp>
#include <libtorrent/settings_pack.hpp>
#include <libtorrent/sha1_hash.hpp>
#include <libtorrent/torrent_flags.hpp>
#include <libtorrent/torrent_handle.hpp>
#include <libtorrent/torrent_info.hpp>
#include <libtorrent/torrent_status.hpp>
#include <libtorrent/write_resume_data.hpp>

#include <algorithm>
#include <iterator>
#include <limits>
#include <map>
#include <memory>
#include <optional>
#include <string>
#include <utility>
#include <vector>

namespace lt = libtorrent;

namespace {

constexpr int32_t kQueued = 0;
constexpr int32_t kChecking = 1;
constexpr int32_t kDownloading = 2;
constexpr int32_t kSeeding = 3;
constexpr int32_t kPaused = 4;
constexpr int32_t kError = 5;

constexpr int32_t kEventStateChanged = 1;
constexpr int32_t kEventProgress = 2;
constexpr int32_t kEventRemoved = 3;

int clamp_rate(uint64_t value) {
    return static_cast<int>(
        std::min<uint64_t>(value, static_cast<uint64_t>(std::numeric_limits<int>::max())));
}

int32_t map_state_t(lt::torrent_status::state_t state) {
    switch (state) {
    case lt::torrent_status::checking_files:
    case lt::torrent_status::checking_resume_data:
        return kChecking;
    case lt::torrent_status::downloading_metadata:
    case lt::torrent_status::downloading:
        return kDownloading;
    case lt::torrent_status::finished:
    case lt::torrent_status::seeding:
        return kSeeding;
    default:
        return kQueued;
    }
}

int32_t map_status(lt::torrent_status const &status) {
    if (status.flags & lt::torrent_flags::paused) {
        return kPaused;
    }
    if (status.errc) {
        return kError;
    }
    return map_state_t(status.state);
}

} // namespace

struct Session::Impl {
    lt::session session;
    std::map<uint64_t, lt::torrent_handle> handles;
    std::map<lt::sha1_hash, uint64_t> ids;
    std::map<uint64_t, std::vector<char>> resume_data;
    bool seed_mode;
    uint64_t next_id = 1;
    std::string last_error;

    Impl(uint64_t download_rate_limit, uint64_t upload_rate_limit, bool seed_mode)
        : session(make_settings(download_rate_limit, upload_rate_limit)),
          seed_mode(seed_mode) {}

    static lt::settings_pack make_settings(uint64_t download_rate_limit,
                                           uint64_t upload_rate_limit) {
        lt::settings_pack pack;
        pack.set_int(
            lt::settings_pack::alert_mask,
            static_cast<int>(lt::alert_category::status | lt::alert_category::error |
                             lt::alert_category::storage));
        pack.set_int(lt::settings_pack::download_rate_limit, clamp_rate(download_rate_limit));
        pack.set_int(lt::settings_pack::upload_rate_limit, clamp_rate(upload_rate_limit));
        // Head-room for the 10 000-torrent soak: libtorrent queues anything above
        // active_limit, and a queued torrent never reports as seeding.
        pack.set_int(lt::settings_pack::active_limit, 20000);
        pack.set_int(lt::settings_pack::active_seeds, 20000);
        pack.set_int(lt::settings_pack::active_downloads, 20000);
        pack.set_int(lt::settings_pack::active_checking, 20000);
        return pack;
    }

    std::optional<uint64_t> id_for(lt::torrent_handle const &handle) const {
        auto it = ids.find(handle.info_hash());
        if (it == ids.end()) {
            return std::nullopt;
        }
        return it->second;
    }

    FfiStatus make_status(uint64_t id, lt::torrent_handle const &handle) const {
        FfiStatus out{};
        auto status = handle.status(lt::torrent_handle::query_name |
                                    lt::torrent_handle::query_save_path);
        out.found = true;
        out.id = id;
        out.name = rust::String(status.name);
        out.save_path = rust::String(status.save_path);
        out.total_bytes = status.total_wanted > 0
                              ? static_cast<uint64_t>(status.total_wanted)
                              : static_cast<uint64_t>(status.total);
        out.state = map_status(status);
        out.progress = status.progress;
        out.download_rate = static_cast<uint64_t>(std::max(status.download_rate, 0));
        out.upload_rate = static_cast<uint64_t>(std::max(status.upload_rate, 0));
        out.seeds = status.num_seeds;
        out.peers = status.num_peers;
        // libtorrent 2.0 no longer exposes eta; derive it from the wanted bytes.
        auto const remaining = status.total_wanted - status.total_wanted_done;
        out.eta_seconds = (remaining > 0 && status.download_payload_rate > 0)
                              ? remaining / status.download_payload_rate
                              : -1;
        out.error = status.errc ? rust::String(status.errc.message()) : rust::String();
        return out;
    }
};

Session::Session(uint64_t download_rate_limit, uint64_t upload_rate_limit, bool seed_mode)
    : impl_(std::make_unique<Impl>(download_rate_limit, upload_rate_limit, seed_mode)) {}

Session::~Session() = default;

std::optional<uint64_t> Session::add_impl(std::optional<uint64_t> requested_id,
                                          rust::Slice<const uint8_t> metainfo,
                                          rust::Str save_path,
                                          rust::Slice<const uint8_t> resume_data) {
    impl_->last_error.clear();

    lt::error_code ec;
    auto info = std::make_shared<lt::torrent_info>(
        reinterpret_cast<char const *>(metainfo.data()), static_cast<int>(metainfo.size()), ec);
    if (ec) {
        impl_->last_error = ec.message();
        return std::nullopt;
    }

    lt::add_torrent_params params;
    if (resume_data.size() > 0) {
        lt::error_code resume_ec;
        params = lt::read_resume_data(
            {reinterpret_cast<char const *>(resume_data.data()),
             static_cast<int>(resume_data.size())},
            resume_ec);
        if (resume_ec) {
            impl_->last_error = resume_ec.message();
            return std::nullopt;
        }
    }
    params.ti = std::move(info);
    params.save_path = static_cast<std::string>(save_path);
    if (impl_->seed_mode) {
        params.flags |= lt::torrent_flags::seed_mode;
    }

    auto handle = impl_->session.add_torrent(params, ec);
    if (ec) {
        impl_->last_error = ec.message();
        return std::nullopt;
    }

    uint64_t id;
    if (requested_id) {
        id = *requested_id;
        if (impl_->handles.count(id) > 0) {
            impl_->last_error = "torrent id already exists";
            impl_->session.remove_torrent(handle);
            return std::nullopt;
        }
        impl_->next_id = std::max(impl_->next_id, id + 1);
    } else {
        id = impl_->next_id++;
    }

    // v1 info hash; v2-only torrents need info_hashes() once they matter here.
    impl_->ids.emplace(handle.info_hash(), id);
    impl_->handles.emplace(id, handle);
    return id;
}

uint64_t Session::add_torrent(rust::Slice<const uint8_t> metainfo, rust::Str save_path,
                              rust::Slice<const uint8_t> resume_data) {
    return add_impl(std::nullopt, metainfo, save_path, resume_data).value_or(0);
}

bool Session::restore_torrent(uint64_t id, rust::Slice<const uint8_t> metainfo,
                              rust::Str save_path, rust::Slice<const uint8_t> resume_data) {
    return add_impl(id, metainfo, save_path, resume_data).has_value();
}

bool Session::request_resume(uint64_t id) {
    auto it = impl_->handles.find(id);
    if (it == impl_->handles.end()) {
        return false;
    }
    it->second.save_resume_data();
    return true;
}

rust::Vec<uint8_t> Session::take_resume(uint64_t id) {
    rust::Vec<uint8_t> out;
    auto it = impl_->resume_data.find(id);
    if (it == impl_->resume_data.end()) {
        return out;
    }
    out.reserve(it->second.size());
    for (char byte : it->second) {
        out.push_back(static_cast<uint8_t>(byte));
    }
    impl_->resume_data.erase(it);
    return out;
}

bool Session::pause(uint64_t id) {
    auto it = impl_->handles.find(id);
    if (it == impl_->handles.end()) {
        return false;
    }
    it->second.pause();
    return true;
}

bool Session::resume(uint64_t id) {
    auto it = impl_->handles.find(id);
    if (it == impl_->handles.end()) {
        return false;
    }
    it->second.resume();
    return true;
}

bool Session::remove(uint64_t id) {
    auto it = impl_->handles.find(id);
    if (it == impl_->handles.end()) {
        return false;
    }
    auto handle = it->second;
    impl_->ids.erase(handle.info_hash());
    impl_->resume_data.erase(id);
    impl_->handles.erase(it);
    impl_->session.remove_torrent(handle);
    return true;
}

rust::Vec<FfiFile> Session::files(uint64_t id) const {
    rust::Vec<FfiFile> out;
    auto it = impl_->handles.find(id);
    if (it == impl_->handles.end()) {
        return out;
    }
    auto info = it->second.torrent_file();
    if (!info) {
        return out;
    }
    auto const &storage = info->files();
    auto priorities = it->second.file_priorities();
    auto progress = it->second.file_progress();
    for (lt::file_index_t index{0}; index < storage.end_file(); ++index) {
        auto const slot = static_cast<std::size_t>(static_cast<int>(index));
        FfiFile file{};
        file.path = rust::String(storage.file_path(index));
        file.size = static_cast<uint64_t>(storage.file_size(index));
        file.priority = slot < priorities.size()
                             ? static_cast<int32_t>(priorities[slot])
                             : 0;
        auto done = slot < progress.size() ? progress[slot] : 0;
        file.progress = storage.file_size(index) > 0
                             ? static_cast<float>(done) / static_cast<float>(storage.file_size(index))
                             : 0.0f;
        out.push_back(std::move(file));
    }
    return out;
}

rust::Vec<FfiPeer> Session::peers(uint64_t id) const {
    rust::Vec<FfiPeer> out;
    auto it = impl_->handles.find(id);
    if (it == impl_->handles.end()) {
        return out;
    }
    std::vector<lt::peer_info> peers;
    it->second.get_peer_info(peers);
    for (auto const &peer : peers) {
        FfiPeer item{};
        item.endpoint = rust::String(peer.ip.address().to_string() + ":" +
                                     std::to_string(peer.ip.port()));
        item.client = rust::String(peer.client);
        item.progress = peer.progress;
        item.download_rate = static_cast<uint64_t>(std::max(peer.down_speed, 0));
        item.upload_rate = static_cast<uint64_t>(std::max(peer.up_speed, 0));
        out.push_back(std::move(item));
    }
    return out;
}

rust::Vec<FfiTracker> Session::trackers(uint64_t id) const {
    rust::Vec<FfiTracker> out;
    auto it = impl_->handles.find(id);
    if (it == impl_->handles.end()) {
        return out;
    }
    for (auto const &tracker : it->second.trackers()) {
        FfiTracker item{};
        item.url = rust::String(tracker.url);
        bool updating = false;
        bool failed = false;
        int seeds = -1;
        int leechers = -1;
        for (auto const &endpoint : tracker.endpoints) {
            for (auto const &infohash : endpoint.info_hashes) {
                updating = updating || infohash.updating;
                failed = failed || static_cast<bool>(infohash.last_error);
                seeds = std::max(seeds, infohash.scrape_complete);
                leechers = std::max(leechers, infohash.scrape_incomplete);
            }
        }
        // Report the live announce state: a tracker that answered is "working"
        // even while it holds no scrape data, and one that failed is "error".
        if (updating) {
            item.status = rust::String("updating");
        } else if (failed) {
            item.status = rust::String("error");
        } else if (tracker.verified) {
            item.status = rust::String("working");
        } else {
            item.status = rust::String("unverified");
        }
        item.seeds = seeds > 0 ? static_cast<uint32_t>(seeds) : 0;
        item.leechers = leechers > 0 ? static_cast<uint32_t>(leechers) : 0;
        out.push_back(std::move(item));
    }
    return out;
}

FfiStatus Session::status(uint64_t id) const {
    auto it = impl_->handles.find(id);
    if (it == impl_->handles.end()) {
        FfiStatus out{};
        out.found = false;
        out.id = id;
        return out;
    }
    return impl_->make_status(id, it->second);
}

rust::Vec<FfiStatus> Session::list() const {
    rust::Vec<FfiStatus> out;
    out.reserve(impl_->handles.size());
    for (auto const &[id, handle] : impl_->handles) {
        out.push_back(impl_->make_status(id, handle));
    }
    return out;
}

rust::Vec<FfiEvent> Session::poll_events() {
    std::vector<lt::alert *> alerts;
    impl_->session.pop_alerts(&alerts);

    rust::Vec<FfiEvent> events;
    events.reserve(alerts.size());
    for (lt::alert *alert : alerts) {
        if (auto *state = lt::alert_cast<lt::state_changed_alert>(alert)) {
            if (auto id = impl_->id_for(state->handle)) {
                events.push_back(FfiEvent{kEventStateChanged, *id,
                                          map_state_t(state->state), 0.0f, 0, 0});
            }
        } else if (auto *finished = lt::alert_cast<lt::torrent_finished_alert>(alert)) {
            if (auto id = impl_->id_for(finished->handle)) {
                events.push_back(
                    FfiEvent{kEventStateChanged, *id, kSeeding, 1.0f, 0, 0});
            }
        } else if (auto *error = lt::alert_cast<lt::torrent_error_alert>(alert)) {
            if (auto id = impl_->id_for(error->handle)) {
                events.push_back(
                    FfiEvent{kEventStateChanged, *id, kError, 0.0f, 0, 0});
            }
        } else if (auto *removed = lt::alert_cast<lt::torrent_removed_alert>(alert)) {
            if (auto id = impl_->id_for(removed->handle)) {
                events.push_back(
                    FfiEvent{kEventRemoved, *id, kQueued, 0.0f, 0, 0});
            }
        } else if (auto *updates = lt::alert_cast<lt::state_update_alert>(alert)) {
            for (auto const &status : updates->status) {
                if (auto id = impl_->id_for(status.handle)) {
                    events.push_back(FfiEvent{
                        kEventProgress, *id, map_status(status), status.progress,
                        static_cast<uint64_t>(std::max(status.download_rate, 0)),
                        static_cast<uint64_t>(std::max(status.upload_rate, 0))});
                }
            }
        } else if (auto *resume = lt::alert_cast<lt::save_resume_data_alert>(alert)) {
            if (auto id = impl_->id_for(resume->handle)) {
                impl_->resume_data[*id] = lt::write_resume_data_buf(resume->params);
            }
        }
    }
    return events;
}

void Session::request_updates() {
    impl_->session.post_torrent_updates(lt::torrent_handle::status_flags_t{});
}

void Session::set_settings(uint64_t download_rate_limit, uint64_t upload_rate_limit) {
    lt::settings_pack pack;
    pack.set_int(lt::settings_pack::download_rate_limit, clamp_rate(download_rate_limit));
    pack.set_int(lt::settings_pack::upload_rate_limit, clamp_rate(upload_rate_limit));
    impl_->session.apply_settings(pack);
}

rust::String Session::last_error() const {
    return rust::String(impl_->last_error);
}

std::unique_ptr<Session> new_session(uint64_t download_rate_limit,
                                     uint64_t upload_rate_limit, bool seed_mode) {
    return std::make_unique<Session>(download_rate_limit, upload_rate_limit, seed_mode);
}

rust::Vec<uint8_t> make_torrent(rust::Str name, rust::Slice<const uint8_t> payload) {
    lt::file_storage files;
    files.add_file(static_cast<std::string>(name), static_cast<std::int64_t>(payload.size()));

    lt::create_torrent torrent(files, 16 * 1024);
    lt::hasher hasher;
    hasher.update(reinterpret_cast<char const *>(payload.data()),
                  static_cast<int>(payload.size()));
    torrent.set_hash(lt::piece_index_t(0), hasher.final());

    std::vector<char> encoded;
    lt::bencode(std::back_inserter(encoded), torrent.generate());

    rust::Vec<uint8_t> out;
    out.reserve(encoded.size());
    for (char byte : encoded) {
        out.push_back(static_cast<uint8_t>(byte));
    }
    return out;
}
