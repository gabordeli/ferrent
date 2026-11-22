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
#include <libtorrent/hex.hpp>
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
#include <filesystem>
#include <fstream>
#include <iterator>
#include <limits>
#include <map>
#include <memory>
#include <optional>
#include <set>
#include <string>
#include <utility>
#include <vector>

namespace lt = libtorrent;

namespace {

/// Splits a newline-separated list (the cxx bridge has no Vec<String> here).
std::vector<std::string> split_lines(std::string const &text) {
    std::vector<std::string> out;
    std::string current;
    for (char character : text) {
        if (character == '\n') {
            out.push_back(current);
            current.clear();
        } else if (character != '\r') {
            current.push_back(character);
        }
    }
    if (!current.empty()) {
        out.push_back(current);
    }
    return out;
}

constexpr int32_t kQueued = 0;
constexpr int32_t kChecking = 1;
constexpr int32_t kDownloading = 2;
constexpr int32_t kSeeding = 3;
constexpr int32_t kPaused = 4;
constexpr int32_t kError = 5;
constexpr int32_t kMoving = 6;

constexpr int32_t kEventStateChanged = 1;
constexpr int32_t kEventProgress = 2;
constexpr int32_t kEventRemoved = 3;
constexpr int32_t kEventFinished = 4;

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
    std::set<uint64_t> moving;
    std::vector<std::string> default_trackers;
    bool seed_mode;
    uint64_t next_id = 1;
    std::string last_error;

    Impl(uint64_t download_rate_limit, uint64_t upload_rate_limit, bool seed_mode,
         uint32_t active_downloads, uint32_t active_seeds, uint32_t active_limit,
         std::string const &listen_interfaces, std::string const &outgoing_interfaces)
        : session(make_settings(download_rate_limit, upload_rate_limit, active_downloads,
                               active_seeds, active_limit, listen_interfaces,
                               outgoing_interfaces)),
          seed_mode(seed_mode) {}

    static lt::settings_pack make_settings(uint64_t download_rate_limit,
                                           uint64_t upload_rate_limit,
                                           uint32_t active_downloads, uint32_t active_seeds,
                                           uint32_t active_limit,
                                           std::string const &listen_interfaces,
                                           std::string const &outgoing_interfaces) {
        lt::settings_pack pack;
        pack.set_int(
            lt::settings_pack::alert_mask,
            static_cast<int>(lt::alert_category::status | lt::alert_category::error |
                             lt::alert_category::storage));
        pack.set_int(lt::settings_pack::download_rate_limit, clamp_rate(download_rate_limit));
        pack.set_int(lt::settings_pack::upload_rate_limit, clamp_rate(upload_rate_limit));
        // Head-room for the 10 000-torrent soak: libtorrent queues anything above
        // active_limit, and a queued torrent never reports as seeding. Zero means
        // "use the soak head-room".
        auto active_or_default = [](uint32_t value) {
            return value == 0 ? 20000 : static_cast<int>(value);
        };
        pack.set_int(lt::settings_pack::active_limit, active_or_default(active_limit));
        pack.set_int(lt::settings_pack::active_seeds, active_or_default(active_seeds));
        pack.set_int(lt::settings_pack::active_downloads, active_or_default(active_downloads));
        pack.set_int(lt::settings_pack::active_checking, 20000);
        // An empty value keeps libtorrent's default, so the packaged daemon
        // behaves as before unless the operator binds an interface.
        if (!listen_interfaces.empty()) {
            pack.set_str(lt::settings_pack::listen_interfaces, listen_interfaces);
        }
        if (!outgoing_interfaces.empty()) {
            pack.set_str(lt::settings_pack::outgoing_interfaces, outgoing_interfaces);
        }
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
        out.infohash = rust::String(lt::aux::to_hex(handle.info_hash().to_string()));
        out.name = rust::String(status.name);
        out.save_path = rust::String(status.save_path);
        out.total_bytes = status.total_wanted > 0
                              ? static_cast<uint64_t>(status.total_wanted)
                              : static_cast<uint64_t>(status.total);
        out.state = (moving.count(id) > 0) ? kMoving : map_status(status);
        out.progress = status.progress;
        out.download_rate = static_cast<uint64_t>(std::max(status.download_rate, 0));
        out.upload_rate = static_cast<uint64_t>(std::max(status.upload_rate, 0));
        // all_time_* survives a restart through the resume data, so the ratio
        // policy keeps its history.
        out.uploaded_bytes = status.all_time_upload;
        out.downloaded_bytes = status.all_time_download;
        // The table column means the swarm the trackers report (what Deluge
        // shows too), not just the peers we happen to be connected to.
        out.seeds = status.num_complete;
        out.peers = status.num_incomplete;
        // libtorrent 2.0 no longer exposes eta; derive it from the wanted bytes.
        auto const remaining = status.total_wanted - status.total_wanted_done;
        out.eta_seconds = (remaining > 0 && status.download_payload_rate > 0)
                              ? remaining / status.download_payload_rate
                              : -1;
        out.error = status.errc ? rust::String(status.errc.message()) : rust::String();
        out.queue_position = static_cast<int>(status.queue_position);
        out.sequential = handle.is_sequential_download();
        out.super_seeding = handle.super_seeding();
        return out;
    }
};

Session::Session(uint64_t download_rate_limit, uint64_t upload_rate_limit, bool seed_mode,
                 uint32_t active_downloads, uint32_t active_seeds, uint32_t active_limit,
                 std::string const &listen_interfaces,
                 std::string const &outgoing_interfaces)
    : impl_(std::make_unique<Impl>(download_rate_limit, upload_rate_limit, seed_mode,
                                   active_downloads, active_seeds, active_limit,
                                   listen_interfaces, outgoing_interfaces)) {}

Session::~Session() = default;

FfiAddResult Session::add_impl(std::optional<uint64_t> requested_id,
                               rust::Slice<const uint8_t> metainfo, rust::Str save_path,
                               rust::Slice<const uint8_t> resume_data, bool paused) {
    impl_->last_error.clear();

    lt::error_code ec;
    auto info = std::make_shared<lt::torrent_info>(
        reinterpret_cast<char const *>(metainfo.data()), static_cast<int>(metainfo.size()), ec);
    if (ec) {
        impl_->last_error = ec.message();
        return FfiAddResult{false, false, 0, rust::String(impl_->last_error)};
    }

    // Duplicate detection before add: libtorrent would return a second handle
    // for the same info hash, which would break the id mapping.
    auto const info_hash = info->info_hash();
    if (auto existing = impl_->ids.find(info_hash); existing != impl_->ids.end()) {
        impl_->last_error = "torrent already exists";
        return FfiAddResult{false, true, existing->second, rust::String(impl_->last_error)};
    }

    lt::add_torrent_params params;
    if (resume_data.size() > 0) {
        lt::error_code resume_ec;
        auto restored = lt::read_resume_data(
            {reinterpret_cast<char const *>(resume_data.data()),
             static_cast<int>(resume_data.size())},
            resume_ec);
        // Unreadable resume data must not cost the torrent: fall back to a
        // plain add, which rechecks the files instead of failing the restore.
        if (!resume_ec) {
            params = std::move(restored);
        } else {
            impl_->last_error = resume_ec.message();
        }
    }
    params.ti = std::move(info);
    params.save_path = static_cast<std::string>(save_path);
    // A metainfo without announce URLs gets the configured defaults; torrents
    // that carry their own trackers keep them untouched.
    if (params.ti->trackers().empty()) {
        for (auto const &tracker : impl_->default_trackers) {
            params.trackers.push_back(tracker);
        }
    }
    if (paused) {
        // A restored torrent must come up paused without a follow-up pause call:
        // setting the flag after the add races with the session's queue and the
        // pause can be lost (FERRENT-81).
        params.flags |= lt::torrent_flags::paused;
        params.flags &= ~lt::torrent_flags::auto_managed;
    } else {
        // Auto-managed torrents are the only ones libtorrent keeps a queue
        // position for, which is what the client's up/down buttons move.
        params.flags |= lt::torrent_flags::auto_managed;
    }
    if (impl_->seed_mode) {
        params.flags |= lt::torrent_flags::seed_mode;
    }

    auto handle = impl_->session.add_torrent(params, ec);
    if (ec) {
        impl_->last_error = ec.message();
        return FfiAddResult{false, false, 0, rust::String(impl_->last_error)};
    }

    uint64_t id;
    if (requested_id) {
        id = *requested_id;
        if (impl_->handles.count(id) > 0) {
            impl_->last_error = "torrent id already exists";
            impl_->session.remove_torrent(handle);
            return FfiAddResult{false, false, id, rust::String(impl_->last_error)};
        }
        impl_->next_id = std::max(impl_->next_id, id + 1);
    } else {
        id = impl_->next_id++;
    }

    // v1 info hash; v2-only torrents need info_hashes() once they matter here.
    impl_->ids.emplace(handle.info_hash(), id);
    impl_->handles.emplace(id, handle);
    return FfiAddResult{true, false, id, rust::String()};
}

FfiAddResult Session::add_torrent(rust::Slice<const uint8_t> metainfo, rust::Str save_path,
                                  rust::Slice<const uint8_t> resume_data) {
    return add_impl(std::nullopt, metainfo, save_path, resume_data, false);
}

bool Session::restore_torrent(uint64_t id, rust::Slice<const uint8_t> metainfo,
                              rust::Str save_path, rust::Slice<const uint8_t> resume_data,
                              bool paused) {
    return add_impl(id, metainfo, save_path, resume_data, paused).ok;
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
    // An auto-managed torrent is resumed again by the session's queue on the
    // next tick, so clear the flag and set paused in one call: doing it in two
    // steps leaves a window where the queue resumes the torrent.
    it->second.set_flags(lt::torrent_flags::paused, lt::torrent_flags::auto_managed);
    return true;
}

bool Session::resume(uint64_t id) {
    auto it = impl_->handles.find(id);
    if (it == impl_->handles.end()) {
        return false;
    }
    it->second.set_flags(lt::torrent_flags::auto_managed, lt::torrent_flags::paused);
    return true;
}

void Session::set_default_trackers(rust::Str trackers) {
    impl_->default_trackers = split_lines(static_cast<std::string>(trackers));
}

void Session::set_network_settings(rust::Str proxy_type, rust::Str proxy_host,
                                   uint16_t proxy_port, rust::Str proxy_username,
                                   rust::Str proxy_password, bool proxy_peer_connections,
                                   rust::Str encryption, bool upnp, bool natpmp) {
    lt::settings_pack pack;
    auto const kind = static_cast<std::string>(proxy_type);
    int type = lt::settings_pack::none;
    if (kind == "http") {
        type = lt::settings_pack::http;
    } else if (kind == "http_pw") {
        type = lt::settings_pack::http_pw;
    } else if (kind == "socks5") {
        type = lt::settings_pack::socks5;
    } else if (kind == "socks5_pw") {
        type = lt::settings_pack::socks5_pw;
    }
    pack.set_int(lt::settings_pack::proxy_type, type);
    pack.set_str(lt::settings_pack::proxy_hostname, static_cast<std::string>(proxy_host));
    pack.set_int(lt::settings_pack::proxy_port, static_cast<int>(proxy_port));
    pack.set_str(lt::settings_pack::proxy_username, static_cast<std::string>(proxy_username));
    pack.set_str(lt::settings_pack::proxy_password, static_cast<std::string>(proxy_password));
    pack.set_bool(lt::settings_pack::proxy_peer_connections, proxy_peer_connections);

    auto const policy = static_cast<std::string>(encryption);
    int peers = lt::settings_pack::pe_enabled;
    if (policy == "forced") {
        peers = lt::settings_pack::pe_forced;
    } else if (policy == "disabled") {
        peers = lt::settings_pack::pe_disabled;
    }
    pack.set_int(lt::settings_pack::in_enc_policy, peers);
    pack.set_int(lt::settings_pack::out_enc_policy, peers);

    pack.set_bool(lt::settings_pack::enable_upnp, upnp);
    pack.set_bool(lt::settings_pack::enable_natpmp, natpmp);
    impl_->session.apply_settings(pack);
}

bool Session::set_torrent_limits(uint64_t id, uint64_t download_rate_limit,
                                 uint64_t upload_rate_limit, uint32_t connections_limit,
                                 uint32_t upload_slots_limit) {
    auto it = impl_->handles.find(id);
    if (it == impl_->handles.end()) {
        return false;
    }
    it->second.set_download_limit(clamp_rate(download_rate_limit));
    it->second.set_upload_limit(clamp_rate(upload_rate_limit));
    auto limit_or_unlimited = [](uint32_t value) {
        return value == 0 ? std::numeric_limits<int>::max()
                          : static_cast<int>(std::min<uint32_t>(
                                value, static_cast<uint32_t>(std::numeric_limits<int>::max())));
    };
    it->second.set_max_connections(limit_or_unlimited(connections_limit));
    it->second.set_max_uploads(limit_or_unlimited(upload_slots_limit));
    return true;
}

bool Session::move_storage(uint64_t id, rust::Str path) {
    auto it = impl_->handles.find(id);
    if (it == impl_->handles.end()) {
        return false;
    }
    impl_->moving.insert(id);
    it->second.move_storage(static_cast<std::string>(path));
    return true;
}

bool Session::remove(uint64_t id, bool delete_files) {
    auto it = impl_->handles.find(id);
    if (it == impl_->handles.end()) {
        return false;
    }
    auto handle = it->second;
    impl_->ids.erase(handle.info_hash());
    impl_->resume_data.erase(id);
    impl_->moving.erase(id);
    impl_->handles.erase(it);
    impl_->session.remove_torrent(
        handle, delete_files ? lt::session::delete_files : lt::remove_flags_t{});
    return true;
}

bool Session::move_in_queue(uint64_t id, bool up) {
    auto it = impl_->handles.find(id);
    if (it == impl_->handles.end()) {
        return false;
    }
    if (up) {
        it->second.queue_position_up();
    } else {
        it->second.queue_position_down();
    }
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
    auto priorities = it->second.get_file_priorities();
    auto progress = it->second.file_progress();
    for (lt::file_index_t index{0}; index < storage.end_file(); ++index) {
        auto const slot = static_cast<std::size_t>(static_cast<int>(index));
        FfiFile file{};
        file.path = rust::String(storage.file_path(index));
        file.size = static_cast<uint64_t>(storage.file_size(index));
        file.priority = slot < priorities.size()
                             ? static_cast<int32_t>(priorities[slot])
                             : 0;        auto done = slot < progress.size() ? progress[slot] : 0;
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
        int next_announce = 0;
        int fails = 0;
        std::string message;
        for (auto const &endpoint : tracker.endpoints) {
            for (auto const &infohash : endpoint.info_hashes) {
                updating = updating || infohash.updating;
                failed = failed || static_cast<bool>(infohash.last_error);
                seeds = std::max(seeds, infohash.scrape_complete);
                leechers = std::max(leechers, infohash.scrape_incomplete);
                fails = std::max(fails, static_cast<int>(infohash.fails));
                if (static_cast<bool>(infohash.last_error)) {
                    message = infohash.last_error.message();
                } else if (!infohash.message.empty()) {
                    message = infohash.message;
                }
                auto due = infohash.next_announce -
                           libtorrent::time_point_cast<libtorrent::seconds32>(
                               libtorrent::clock_type::now());
                int seconds = static_cast<int>(due.count());
                if (seconds > 0 && (next_announce == 0 || seconds < next_announce)) {
                    next_announce = seconds;
                }
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
        item.message = rust::String(message);
        item.next_announce = next_announce > 0 ? static_cast<uint32_t>(next_announce) : 0;
        item.fails = fails > 0 ? static_cast<uint32_t>(fails) : 0;
        out.push_back(std::move(item));
    }
    return out;
}

bool Session::force_recheck(uint64_t id) {
    auto it = impl_->handles.find(id);
    if (it == impl_->handles.end()) {
        return false;
    }
    it->second.force_recheck();
    return true;
}

bool Session::set_sequential(uint64_t id, bool on) {
    auto it = impl_->handles.find(id);
    if (it == impl_->handles.end()) {
        return false;
    }
    it->second.set_sequential_download(on);
    return true;
}

bool Session::set_super_seeding(uint64_t id, bool on) {
    auto it = impl_->handles.find(id);
    if (it == impl_->handles.end()) {
        return false;
    }
    it->second.super_seeding(on);
    return true;
}

bool Session::add_tracker(uint64_t id, rust::Str url) {
    auto it = impl_->handles.find(id);
    if (it == impl_->handles.end()) {
        return false;
    }
    std::string text(url);
    if (text.empty()) {
        return false;
    }
    it->second.add_tracker(lt::announce_entry(text));
    return true;
}

bool Session::remove_tracker(uint64_t id, rust::Str url) {
    auto it = impl_->handles.find(id);
    if (it == impl_->handles.end()) {
        return false;
    }
    std::string target(url);
    std::vector<lt::announce_entry> kept;
    bool removed = false;
    for (auto const &entry : it->second.trackers()) {
        if (entry.url == target) {
            removed = true;
            continue;
        }
        kept.push_back(entry);
    }
    if (!removed) {
        return false;
    }
    it->second.replace_trackers(kept);
    return true;
}

rust::Vec<rust::String> Session::web_seeds(uint64_t id) const {
    rust::Vec<rust::String> out;
    auto it = impl_->handles.find(id);
    if (it == impl_->handles.end()) {
        return out;
    }
    for (auto const &seed : it->second.url_seeds()) {
        out.push_back(rust::String(seed));
    }
    return out;
}

bool Session::set_web_seeds(uint64_t id, rust::Str urls) {
    auto it = impl_->handles.find(id);
    if (it == impl_->handles.end()) {
        return false;
    }
    // The whole set is replaced: drop what libtorrent holds, then add the
    // requested URLs in order.
    for (auto const &seed : it->second.url_seeds()) {
        it->second.remove_url_seed(seed);
    }
    std::string text(urls);
    std::size_t start = 0;
    while (start <= text.size()) {
        auto end = text.find('\n', start);
        if (end == std::string::npos) {
            end = text.size();
        }
        auto url = text.substr(start, end - start);
        if (!url.empty()) {
            it->second.add_url_seed(url);
        }
        if (end == text.size()) {
            break;
        }
        start = end + 1;
    }
    return true;
}

bool Session::set_ip_filter(rust::Str ranges) {
    lt::ip_filter filter;
    std::string text(ranges);
    std::size_t start = 0;
    while (start < text.size()) {
        auto end = text.find('\n', start);
        if (end == std::string::npos) {
            end = text.size();
        }
        auto line = text.substr(start, end - start);
        auto dash = line.find('-');
        if (dash != std::string::npos) {
            lt::error_code from_error;
            lt::error_code to_error;
            auto first = lt::make_address(line.substr(0, dash), from_error);
            auto last = lt::make_address(line.substr(dash + 1), to_error);
            if (!from_error && !to_error) {
                filter.add_rule(first, last, lt::ip_filter::blocked);
            }
        }
        if (end == text.size()) {
            break;
        }
        start = end + 1;
    }
    impl_->session.set_ip_filter(filter);
    return true;
}

bool Session::rename_file(uint64_t id, int32_t index, rust::Str name) {
    auto it = impl_->handles.find(id);
    if (it == impl_->handles.end()) {
        return false;
    }
    it->second.rename_file(lt::file_index_t(index), static_cast<std::string>(name));
    return true;
}

bool Session::set_file_priorities(uint64_t id, rust::Slice<const int32_t> priorities) {
    auto it = impl_->handles.find(id);
    if (it == impl_->handles.end()) {
        return false;
    }
    auto info = it->second.torrent_file();
    if (!info) {
        return false;
    }
    std::vector<lt::download_priority_t> values;
    values.reserve(priorities.size());
    for (auto value : priorities) {
        auto clamped = value < 0 ? 0 : (value > 7 ? 7 : value);
        values.push_back(static_cast<lt::download_priority_t>(clamped));
    }
    it->second.prioritize_files(values);
    return true;
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
                                          map_state_t(state->state), 0.0f, 0, 0, -1, 0, 0});
            }
        } else if (auto *finished = lt::alert_cast<lt::torrent_finished_alert>(alert)) {
            if (auto id = impl_->id_for(finished->handle)) {
                events.push_back(
                    FfiEvent{kEventStateChanged, *id, kSeeding, 1.0f, 0, 0, -1, 0, 0});
                events.push_back(
                    FfiEvent{kEventFinished, *id, kSeeding, 1.0f, 0, 0, -1, 0, 0});
            }
        } else if (auto *moved = lt::alert_cast<lt::storage_moved_alert>(alert)) {
            if (auto id = impl_->id_for(moved->handle)) {
                impl_->moving.erase(*id);
                auto status = moved->handle.status(lt::torrent_handle::query_name |
                                                   lt::torrent_handle::query_save_path);
                events.push_back(FfiEvent{
                    kEventStateChanged, *id, map_status(status), status.progress,
                    static_cast<uint64_t>(std::max(status.download_rate, 0)),
                    static_cast<uint64_t>(std::max(status.upload_rate, 0)),
                    static_cast<int>(status.queue_position)});
            }
        } else if (auto *failed = lt::alert_cast<lt::storage_moved_failed_alert>(alert)) {
            if (auto id = impl_->id_for(failed->handle)) {
                impl_->moving.erase(*id);
                impl_->last_error = failed->error.message();
                events.push_back(
                    FfiEvent{kEventStateChanged, *id, kError, 0.0f, 0, 0, -1, 0, 0});
            }
        } else if (auto *error = lt::alert_cast<lt::torrent_error_alert>(alert)) {
            if (auto id = impl_->id_for(error->handle)) {
                events.push_back(
                    FfiEvent{kEventStateChanged, *id, kError, 0.0f, 0, 0, -1, 0, 0});
            }
        } else if (auto *removed = lt::alert_cast<lt::torrent_removed_alert>(alert)) {
            if (auto id = impl_->id_for(removed->handle)) {
                events.push_back(
                    FfiEvent{kEventRemoved, *id, kQueued, 0.0f, 0, 0, -1, 0, 0});
            }
        } else if (auto *updates = lt::alert_cast<lt::state_update_alert>(alert)) {
            for (auto const &status : updates->status) {
                if (auto id = impl_->id_for(status.handle)) {
                    events.push_back(FfiEvent{
                        kEventProgress, *id, map_status(status), status.progress,
                        static_cast<uint64_t>(std::max(status.download_rate, 0)),
                        static_cast<uint64_t>(std::max(status.upload_rate, 0)),
                        static_cast<int>(status.queue_position), status.num_complete,
                        status.num_incomplete});
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

void Session::set_settings(uint64_t download_rate_limit, uint64_t upload_rate_limit,
                           uint32_t active_downloads, uint32_t active_seeds,
                           uint32_t active_limit, bool seed_mode) {
    impl_->seed_mode = seed_mode;
    lt::settings_pack pack;
    pack.set_int(lt::settings_pack::download_rate_limit, clamp_rate(download_rate_limit));
    pack.set_int(lt::settings_pack::upload_rate_limit, clamp_rate(upload_rate_limit));
    auto active_or_default = [](uint32_t value) {
        return value == 0 ? 20000 : static_cast<int>(value);
    };
    pack.set_int(lt::settings_pack::active_limit, active_or_default(active_limit));
    pack.set_int(lt::settings_pack::active_seeds, active_or_default(active_seeds));
    pack.set_int(lt::settings_pack::active_downloads, active_or_default(active_downloads));
    impl_->session.apply_settings(pack);
}

rust::String Session::last_error() const {
    return rust::String(impl_->last_error);
}

std::unique_ptr<Session> new_session(uint64_t download_rate_limit,
                                     uint64_t upload_rate_limit, bool seed_mode,
                                     uint32_t active_downloads, uint32_t active_seeds,
                                     uint32_t active_limit, rust::Str listen_interfaces,
                                     rust::Str outgoing_interfaces) {
    return std::make_unique<Session>(download_rate_limit, upload_rate_limit, seed_mode,
                                     active_downloads, active_seeds, active_limit,
                                     static_cast<std::string>(listen_interfaces),
                                     static_cast<std::string>(outgoing_interfaces));
}

FfiCreateResult make_torrent_from_path_impl(FfiCreateResult &result, rust::Str path,
                                            uint32_t piece_length, rust::Str trackers);

FfiCreateResult make_torrent_from_path(rust::Str path, uint32_t piece_length,
                                       rust::Str trackers) {
    FfiCreateResult result{};
    try {
        return make_torrent_from_path_impl(result, path, piece_length, trackers);
    } catch (std::exception const &error) {
        // libtorrent throws here (e.g. a missing piece hash); a C++ exception
        // must not take the daemon down.
        FfiCreateResult failed{};
        failed.error = rust::String(error.what());
        return failed;
    }
}

FfiCreateResult make_torrent_from_path_impl(FfiCreateResult &result, rust::Str path,
                                            uint32_t piece_length, rust::Str trackers) {
    std::string root(path);
    // std::filesystem fills a std::error_code. lt::error_code is
    // boost::system::error_code in the static build (/opt/lt), and passing it
    // here would bind a temporary after the implicit conversion.
    std::error_code fs_ec;
    bool const directory = std::filesystem::is_directory(root, fs_ec);
    if (fs_ec) {
        result.error = rust::String(fs_ec.message());
        return result;
    }

    lt::file_storage files;
    if (directory) {
        lt::add_files(files, root, [](std::string const &) { return true; });
        if (files.num_files() == 0) {
            result.error = rust::String("the directory has no files");
            return result;
        }
    } else {
        auto size = std::filesystem::file_size(root, fs_ec);
        if (fs_ec) {
            result.error = rust::String(fs_ec.message());
            return result;
        }
        files.add_file(std::filesystem::path(root).filename().string(),
                       static_cast<std::int64_t>(size));
    }

    int piece = piece_length == 0 ? 256 * 1024 : static_cast<int>(piece_length);
    piece = std::clamp(piece, 16 * 1024, 16 * 1024 * 1024);
    // v1-only: we hash the pieces ourselves, and v2 metadata would demand
    // piece-aligned files with pad files this walker does not produce.
    lt::create_torrent torrent(files, piece, lt::create_torrent::v1_only);
    for (auto const &entry : split_lines(static_cast<std::string>(trackers))) {
        if (!entry.empty()) {
            torrent.add_tracker(entry);
        }
    }

    // Hash the pieces by hand: set_piece_hashes posts to an io_context, which
    // this worker thread does not run, and a 64 KiB buffer keeps the loop
    // aligned with the piece boundaries.
    auto const base = std::filesystem::path(root).parent_path();
    std::vector<char> buffer(static_cast<std::size_t>(torrent.piece_length()));
    lt::hasher hasher;
    lt::piece_index_t piece_index{0};
    std::int64_t filled = 0;
    for (lt::file_index_t index{0}; index < files.end_file(); ++index) {
        if (files.pad_file_at(index)) {
            continue;
        }
        auto const file = base / files.file_path(index);
        std::ifstream input(file, std::ios::binary);
        if (!input) {
            result.error = rust::String("cannot read " + file.string());
            return result;
        }
        while (input) {
            // Fill the current piece exactly: a piece can span two files, so
            // the read size is whatever is still missing.
            auto const want =
                static_cast<std::streamsize>(torrent.piece_length() - filled);
            input.read(buffer.data(), want);
            auto const got = input.gcount();
            if (got <= 0) {
                break;
            }
            hasher.update(buffer.data(), static_cast<int>(got));
            filled += got;
            if (filled == torrent.piece_length()) {
                torrent.set_hash(piece_index, hasher.final());
                piece_index = lt::piece_index_t(static_cast<int>(piece_index) + 1);
                filled = 0;
            }
        }
    }
    if (filled > 0) {
        torrent.set_hash(piece_index, hasher.final());
    }

    auto meta = [&]() {
        try {
            return torrent.generate();
        } catch (std::exception const &error) {
            result.error = rust::String(std::string("generate: ") + error.what() + " after " +
                                        std::to_string(static_cast<int>(piece_index)) + " of " +
                                        std::to_string(torrent.num_pieces()) + " pieces");
            return lt::entry{};
        }
    }();
    if (!result.error.empty()) {
        return result;
    }
    std::vector<char> info_encoded;
    lt::bencode(std::back_inserter(info_encoded), meta.dict().at("info"));
    lt::hasher info_hasher;
    info_hasher.update(info_encoded.data(), static_cast<int>(info_encoded.size()));
    result.infohash = rust::String(lt::aux::to_hex(info_hasher.final()));

    std::vector<char> encoded;
    lt::bencode(std::back_inserter(encoded), meta);
    result.metainfo.reserve(encoded.size());
    for (char byte : encoded) {
        result.metainfo.push_back(static_cast<uint8_t>(byte));
    }
    result.ok = true;
    return result;
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
