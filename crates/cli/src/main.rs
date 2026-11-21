//! `ferrent-cli`: control a Ferrent daemon over TLS.

mod bencode;
mod config;
mod error;
mod import_deluge;
mod output;

use std::path::PathBuf;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use ferrent_protocol::client::AuthClient;
use ferrent_protocol::v1::{
    AddTorrentRequest, AddTrackerRequest, CreateLabelRequest, CreateTorrentRequest,
    GetConfigRequest, GetDisksRequest, GetNotificationStatusRequest, GetRssStatusRequest,
    GetStatsRequest, GetStatusRequest, GetTorrentFilesRequest, GetTorrentRequest,
    GetTorrentTrackersRequest, GetWebSeedsRequest, Label as ProtoLabel, ListTorrentsRequest,
    MoveTorrentRequest, PauseTorrentRequest, QueueDirection, RecheckTorrentRequest,
    RefreshBlocklistRequest, RemoveTorrentRequest, RemoveTrackerRequest, RenameTorrentFileRequest,
    ResumeTorrentRequest, SendTestNotificationRequest, SetConfigRequest, SetFilePrioritiesRequest,
    SetWebSeedsRequest, SortField, WatchRequest,
};
use serde_json::json;

use crate::error::CliError;

#[derive(Parser)]
#[command(
    name = "ferrent-cli",
    version,
    about = "Control a Ferrent daemon over TLS",
    long_about = "Control a Ferrent daemon over TLS.\n\n\
        Configuration precedence: flags > FERRENT_SERVER/FERRENT_CA/FERRENT_TOKEN > \
        ~/.config/ferrent/cli.toml.\n\
        Exit codes: 0 ok, 1 runtime, 2 usage/config, 3 not found, 4 auth, 5 transport/TLS."
)]
struct Cli {
    /// Daemon URL (default: https://127.0.0.1:50051).
    #[arg(long, global = true)]
    server: Option<String>,

    /// Path to the daemon's TLS certificate (PEM).
    #[arg(long, global = true)]
    ca: Option<PathBuf>,

    /// Bearer token.
    #[arg(long, global = true)]
    token: Option<String>,

    /// CLI config file (default: $XDG_CONFIG_HOME/ferrent/cli.toml).
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Emit stable JSON instead of human-readable output.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Add a .torrent file.
    Add {
        /// Path to the .torrent file.
        file: PathBuf,
        /// Display name (defaults to the file name).
        #[arg(long)]
        name: Option<String>,
        /// Save path on the daemon host (default: the label's staging folder,
        /// or the daemon's download dir without a label).
        #[arg(long, default_value = "")]
        save_path: String,
        /// Label of the torrent; its staging folder receives the download.
        #[arg(long, default_value = "")]
        label: String,
        /// Download the pieces in order (streaming-friendly).
        #[arg(long)]
        sequential: bool,
    },
    /// List torrents.
    List {
        /// Case-insensitive substring filter on the name.
        #[arg(long)]
        filter: Option<String>,
        /// Sort key.
        #[arg(long, value_enum, default_value = "id")]
        sort: SortArg,
        /// Sort descending.
        #[arg(long)]
        desc: bool,
        /// Number of matching torrents to skip.
        #[arg(long, default_value_t = 0)]
        offset: u32,
        /// Page size; zero means no limit.
        #[arg(long, default_value_t = 50)]
        limit: u32,
        /// Restrict to one label.
        #[arg(long)]
        label: Option<String>,
    },
    /// Show the storage payload: filesystems, zones and free space.
    Disks,
    /// Import a Deluge state directory (torrents and plugin configs).
    ImportDeluge {
        /// Deluge state directory, the one holding the `.torrent` files.
        #[arg(long)]
        state_dir: PathBuf,
        /// Directory with `label.conf`, `autoadd.conf`, `execute.conf` and
        /// `scheduler.conf` (default: the parent of `--state-dir`).
        #[arg(long)]
        config_dir: Option<PathBuf>,
        /// Only list what was found; do not add anything to the daemon.
        #[arg(long)]
        dry_run: bool,
    },
    /// Show one torrent.
    Show {
        /// Torrent id.
        id: u64,
    },
    /// Build a .torrent from a file or directory on the daemon host.
    Create {
        /// File or directory on the daemon host.
        path: String,
        /// Announce URL; repeatable.
        #[arg(long = "tracker")]
        trackers: Vec<String>,
        /// Piece size in bytes (16 KiB - 16 MiB; 0 picks 256 KiB).
        #[arg(long, default_value_t = 0)]
        piece_length: u32,
        /// Where to write the .torrent (default: <name>.torrent here).
        #[arg(long = "output")]
        out: Option<PathBuf>,
    },
    /// Show the RSS watcher status.
    Rss,
    /// Ask the daemon to verify the existing data against the hashes.
    Recheck {
        /// Torrent id.
        id: u64,
    },
    /// Rename one file inside a torrent.
    RenameFile {
        /// Torrent id.
        id: u64,
        /// Zero-based file index (see `files`).
        index: u32,
        /// New base name; must not contain '/'.
        name: String,
    },
    /// List the trackers of one torrent.
    Trackers {
        /// Torrent id.
        id: u64,
        /// Add this announce URL before listing.
        #[arg(long)]
        add: Option<String>,
        /// Remove this announce URL before listing.
        #[arg(long)]
        remove: Option<String>,
    },
    /// List the web seeds of one torrent; `--set` replaces the list.
    #[command(name = "webseeds", visible_alias = "web-seeds")]
    WebSeeds {
        /// Torrent id.
        id: u64,
        /// Comma-separated URLs; an empty value clears the list.
        #[arg(long)]
        set: Option<String>,
    },
    /// Show the IP blocklist status; `--refresh` reloads the sources.
    Blocklist {
        /// Reload the configured sources before printing.
        #[arg(long)]
        refresh: bool,
    },
    /// Session totals: bytes, ratio, peer count and uptime.
    Stats,
    /// List the files of one torrent; `--set` changes their priorities.
    Files {
        /// Torrent id.
        id: u64,
        /// Comma-separated index=level pairs, e.g. `0=skip,2=high`
        /// (skip | low | normal | high, or 0-7).
        #[arg(long)]
        set: Option<String>,
    },
    /// Pause a torrent.
    Pause {
        /// Torrent id.
        id: u64,
    },
    /// Resume a torrent.
    Resume {
        /// Torrent id.
        id: u64,
    },
    /// Remove a torrent.
    Remove {
        /// Torrent id.
        id: u64,
        /// Also delete the downloaded files from the save path.
        #[arg(long)]
        delete_files: bool,
    },
    /// Move a torrent in the download queue.
    Move {
        /// Torrent id.
        id: u64,
        /// Move towards the back of the queue (default: towards the front).
        #[arg(long)]
        down: bool,
    },
    /// Stream torrent deltas until Ctrl-C.
    Watch {
        /// Stop after N events.
        #[arg(long)]
        count: Option<usize>,
    },
    /// Read or replace the daemon configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Notifications: outbox status and a test delivery.
    Notify {
        #[command(subcommand)]
        command: NotifyCommand,
    },
}

#[derive(Subcommand)]
enum NotifyCommand {
    /// Queue a test delivery to every enabled webhook.
    Test,
    /// Print the outbox counters.
    Status,
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Print the current TOML document and revision.
    Get,
    /// Replace the configuration from a TOML file.
    Set {
        /// TOML file to upload.
        file: PathBuf,
        /// Revision from `config get`; fetched automatically when omitted.
        #[arg(long)]
        revision: Option<u64>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum SortArg {
    Id,
    Name,
    Progress,
    Download,
    Upload,
    Label,
}

impl SortArg {
    fn as_proto(self) -> i32 {
        match self {
            Self::Id => SortField::Id as i32,
            Self::Name => SortField::Name as i32,
            Self::Progress => SortField::Progress as i32,
            Self::Download => SortField::DownloadRate as i32,
            Self::Upload => SortField::UploadRate as i32,
            Self::Label => SortField::Label as i32,
        }
    }
}

enum Action {
    Pause,
    Resume,
    Remove { delete_files: bool },
}

impl Action {
    fn past(&self) -> &'static str {
        match self {
            Self::Pause => "paused",
            Self::Resume => "resumed",
            Self::Remove { .. } => "removed",
        }
    }
}

/// Parses `0=skip,2=high` into one priority per file.
///
/// Unlisted files keep libtorrent's normal priority, so a single skip does not
/// silently reset the rest of the torrent.
fn parse_file_priorities(spec: &str, file_count: usize) -> Result<Vec<i32>, CliError> {
    let mut priorities = vec![4_i32; file_count];
    for part in spec.split(',').filter(|part| !part.trim().is_empty()) {
        let (index, level) = part
            .split_once('=')
            .ok_or_else(|| CliError::Config(format!("expected index=level, got {part:?}")))?;
        let index: usize = index
            .trim()
            .parse()
            .map_err(|_| CliError::Config(format!("invalid file index {index:?}")))?;
        let level = level.trim();
        let value = match level {
            "skip" => 0,
            "low" => 1,
            "normal" => 4,
            "high" => 7,
            other => other
                .parse::<i32>()
                .ok()
                .filter(|value| (0..=7).contains(value))
                .ok_or_else(|| {
                    CliError::Config(format!(
                        "unknown priority {other:?} (skip | low | normal | high, or 0-7)"
                    ))
                })?,
        };
        if index >= file_count {
            return Err(CliError::Config(format!(
                "file index {index} is out of range (0..{file_count})"
            )));
        }
        priorities[index] = value;
    }
    Ok(priorities)
}

#[tokio::main]
async fn main() {
    let _telemetry = ferrent_telemetry::init("ferrent-cli");
    ferrent_i18n::Lang::from_env();
    // clap bakes the derive-d strings in, so the command tree is rebuilt and
    // translated before parsing; `--json` keys stay untouched.
    let command = localize_command(Cli::command());
    let cli = Cli::from_arg_matches(&command.get_matches()).unwrap_or_else(|error| error.exit());
    if let Err(error) = run(cli).await {
        // Only defects are reported; a stopped daemon or a mistyped id is a
        // normal answer and would otherwise flood the issue tracker.
        if error.reportable() {
            ferrent_telemetry::capture_error(&error);
            ferrent_telemetry::flush();
        }
        eprintln!("ferrent-cli: {error}");
        std::process::exit(error.exit_code());
    }
}

/// Translates a command tree in place: about/long-about, argument help and
/// value names, recursively for every subcommand.
fn localize_command(command: clap::Command) -> clap::Command {
    let mut command = command.mut_args(localize_arg);
    if let Some(about) = command.get_about().map(ToString::to_string) {
        command = command.about(ferrent_i18n::translate(&about));
    }
    if let Some(long) = command.get_long_about().map(ToString::to_string) {
        command = command.long_about(ferrent_i18n::translate(&long));
    }
    let names: Vec<String> = command
        .get_subcommands()
        .map(|sub| sub.get_name().to_owned())
        .collect();
    for name in names {
        command = command.mut_subcommand(name, localize_command);
    }
    command
}

fn localize_arg(arg: clap::Arg) -> clap::Arg {
    let help = arg.get_help().map(ToString::to_string);
    let long_help = arg.get_long_help().map(ToString::to_string);
    let names: Option<Vec<String>> = arg
        .get_value_names()
        .map(|names| names.iter().map(ToString::to_string).collect());
    let mut arg = arg;
    if let Some(help) = help {
        arg = arg.help(ferrent_i18n::translate(&help));
    }
    if let Some(long_help) = long_help {
        arg = arg.long_help(ferrent_i18n::translate(&long_help));
    }
    if let Some(names) = names {
        // clap only accepts `Str`-convertible names, and a translated name is
        // owned; the command lives for the rest of the process, so leaking it
        // is bounded by the number of arguments.
        let translated: Vec<&'static str> = names
            .iter()
            .map(|name| &*Box::leak(ferrent_i18n::translate(name).into_boxed_str()))
            .collect();
        arg = arg.value_names(translated);
    }
    arg
}

async fn run(cli: Cli) -> Result<(), CliError> {
    let Cli {
        server,
        ca,
        token,
        config: config_path,
        json,
        command,
    } = cli;

    // A dry run only reads the Deluge state directory, so it must work without
    // a daemon (or even a configured connection).
    if let Command::ImportDeluge {
        state_dir,
        config_dir,
        dry_run: true,
    } = &command
    {
        return import_deluge_report(state_dir, config_dir.as_deref(), json);
    }

    let resolved = config::resolve(server, ca, token, config_path)?;

    let ca_pem = std::fs::read_to_string(&resolved.ca).map_err(|error| {
        CliError::Config(format!(
            "cannot read CA certificate {}: {error}",
            resolved.ca.display()
        ))
    })?;
    let server = resolved.server.clone();
    let mut client = ferrent_protocol::client::connect(&server, &ca_pem, &resolved.token)
        .await
        .map_err(|error| CliError::Transport { server, error })?;

    match command {
        Command::Add {
            file,
            name,
            save_path,
            label,
            sequential,
        } => {
            let metainfo = std::fs::read(&file)?;
            let name = name.unwrap_or_else(|| {
                file.file_name()
                    .and_then(|name| name.to_str())
                    .map_or_else(|| "torrent".to_owned(), str::to_owned)
            });
            let response = client
                .add_torrent(AddTorrentRequest {
                    name,
                    metainfo,
                    save_path,
                    total_bytes: 0,
                    label,
                    magnet: String::new(),
                    sequential,
                })
                .await?
                .into_inner();
            if json {
                output::print_json(&json!({
                    "id": response.id,
                    "duplicate": response.duplicate,
                }));
            } else if response.duplicate {
                println!("torrent {} is already present", response.id);
            } else {
                println!("added torrent {}", response.id);
            }
        }
        Command::ImportDeluge {
            state_dir,
            config_dir,
            dry_run: true,
        } => {
            import_deluge_report(&state_dir, config_dir.as_deref(), json)?;
        }
        Command::ImportDeluge {
            state_dir,
            config_dir,
            dry_run: false,
        } => {
            let options = import_deluge::Options {
                config_dir: config_dir
                    .unwrap_or_else(|| import_deluge::default_config_dir(&state_dir)),
                state_dir,
            };
            let report = import_deluge::scan(&options).map_err(CliError::Config)?;
            // A torrent can only be added under an existing label, so the
            // declared ones are created first (AlreadyExists is fine).
            for label in &report.plugins.labels {
                let request = CreateLabelRequest {
                    label: Some(ProtoLabel {
                        name: label.name.clone(),
                        color: label.color.clone(),
                        ..ProtoLabel::default()
                    }),
                };
                if let Err(error) = client.create_label(request).await {
                    if error.code() != tonic::Code::AlreadyExists {
                        eprintln!("label {}: {error}", label.name);
                    }
                }
            }
            let mut added = 0u32;
            let mut skipped = 0u32;
            let mut failed = 0u32;
            for torrent in &report.torrents {
                let metainfo = std::fs::read(&torrent.path)?;
                let response = client
                    .add_torrent(AddTorrentRequest {
                        name: torrent.name.clone(),
                        metainfo,
                        save_path: String::new(),
                        total_bytes: torrent.total_bytes,
                        label: torrent.label.clone().unwrap_or_default(),
                        magnet: String::new(),
                        sequential: false,
                    })
                    .await;
                match response {
                    Ok(response) => {
                        if response.into_inner().duplicate {
                            skipped += 1;
                        } else {
                            added += 1;
                        }
                    }
                    Err(error) => {
                        failed += 1;
                        eprintln!("{}: {error}", torrent.name);
                    }
                }
            }
            if json {
                output::print_json(&json!({
                    "found": report.torrents.len(),
                    "added": added,
                    "already_present": skipped,
                    "failed": failed,
                    "warnings": report.warnings,
                }));
            } else {
                println!(
                    "imported {added}, already present {skipped}, failed {failed} of {} torrents",
                    report.torrents.len()
                );
                print_import_warnings(&report.warnings);
            }
        }
        Command::List {
            filter,
            sort,
            desc,
            offset,
            limit,
            label,
        } => {
            let response = client
                .list_torrents(ListTorrentsRequest {
                    filter: filter.unwrap_or_default(),
                    sort: sort.as_proto(),
                    descending: desc,
                    offset,
                    limit,
                    state_filter: 0,
                    label_filter: label.unwrap_or_default(),
                })
                .await?
                .into_inner();
            if json {
                let torrents: Vec<_> = response.torrents.iter().map(output::torrent_json).collect();
                output::print_json(&json!({
                    "total": response.total,
                    "torrents": torrents,
                }));
            } else {
                output::print_torrents(&response.torrents);
                // A paged answer must say so: 50 rows of 10 000 otherwise
                // looks like the daemon lost the rest.
                let shown = response.torrents.len() as u32;
                if shown < response.total {
                    let first = offset + 1;
                    let last = offset + shown;
                    output::print_page_note(first, last, response.total);
                }
            }
        }
        Command::Show { id } => {
            let response = client
                .get_torrent(GetTorrentRequest { id })
                .await?
                .into_inner();
            let torrent = response.torrent.ok_or_else(|| {
                CliError::Status(tonic::Status::not_found(format!("torrent {id} not found")))
            })?;
            if json {
                output::print_json(&output::torrent_json(&torrent));
            } else {
                output::print_torrent(&torrent);
            }
        }
        Command::Create {
            path,
            trackers,
            piece_length,
            out,
        } => {
            let response = client
                .create_torrent(CreateTorrentRequest {
                    path: path.clone(),
                    piece_length,
                    trackers,
                })
                .await?
                .into_inner();
            let target = out.unwrap_or_else(|| {
                let name = std::path::Path::new(&path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "torrent".to_owned());
                PathBuf::from(format!("{name}.torrent"))
            });
            std::fs::write(&target, &response.metainfo)?;
            if json {
                output::print_json(&json!({
                    "output": target,
                    "infohash": response.infohash,
                    "bytes": response.metainfo.len(),
                }));
            } else {
                println!(
                    "{} ({} bytes, {})",
                    target.display(),
                    response.metainfo.len(),
                    response.infohash
                );
            }
        }
        Command::Rss => {
            let feeds = client
                .get_rss_status(GetRssStatusRequest {})
                .await?
                .into_inner()
                .feeds;
            if json {
                output::print_json(&output::rss_json(&feeds));
            } else {
                output::print_rss(&feeds);
            }
        }
        Command::Recheck { id } => {
            client.recheck_torrent(RecheckTorrentRequest { id }).await?;
            if !json {
                println!("recheck requested for torrent {id}");
            }
        }
        Command::RenameFile { id, index, name } => {
            let files = client
                .rename_torrent_file(RenameTorrentFileRequest { id, index, name })
                .await?
                .into_inner()
                .files;
            if json {
                output::print_json(&output::files_json(&files));
            } else {
                output::print_files(&files);
            }
        }
        Command::Trackers { id, add, remove } => {
            if let Some(url) = add {
                client.add_tracker(AddTrackerRequest { id, url }).await?;
            }
            if let Some(url) = remove {
                client
                    .remove_tracker(RemoveTrackerRequest { id, url })
                    .await?;
            }
            let trackers = client
                .get_torrent_trackers(GetTorrentTrackersRequest { id })
                .await?
                .into_inner()
                .trackers;
            if json {
                output::print_json(&output::trackers_json(&trackers));
            } else {
                output::print_trackers(&trackers);
            }
        }
        Command::WebSeeds { id, set } => {
            let urls = match set {
                Some(spec) => {
                    let urls: Vec<String> = spec
                        .split(',')
                        .map(str::trim)
                        .filter(|url| !url.is_empty())
                        .map(str::to_owned)
                        .collect();
                    client
                        .set_web_seeds(SetWebSeedsRequest { id, urls })
                        .await?
                        .into_inner()
                        .urls
                }
                None => {
                    client
                        .get_web_seeds(GetWebSeedsRequest { id })
                        .await?
                        .into_inner()
                        .urls
                }
            };
            if json {
                output::print_json(&json!({ "id": id, "web_seeds": urls }));
            } else if urls.is_empty() {
                println!("no web seeds");
            } else {
                for url in urls {
                    println!("{url}");
                }
            }
        }
        Command::Blocklist { refresh } => {
            let (ranges, refreshed, errors) = if refresh {
                let response = client
                    .refresh_blocklist(RefreshBlocklistRequest {})
                    .await?
                    .into_inner();
                (response.ranges, response.refreshed_unix, response.errors)
            } else {
                let status = client.get_status(GetStatusRequest {}).await?.into_inner();
                (
                    status.blocklist_ranges,
                    status.blocklist_refreshed_unix,
                    Vec::new(),
                )
            };
            if json {
                output::print_json(&json!({
                    "ranges": ranges,
                    "refreshed_unix": refreshed,
                    "errors": errors,
                }));
            } else {
                println!("ranges: {ranges}");
                if refreshed > 0 {
                    println!("refreshed_unix: {refreshed}");
                }
                for error in &errors {
                    println!("warning: {error}");
                }
            }
        }
        Command::Stats => {
            let stats = client.get_stats(GetStatsRequest {}).await?.into_inner();
            if json {
                output::print_json(&json!({
                    "downloaded_bytes": stats.downloaded_bytes,
                    "uploaded_bytes": stats.uploaded_bytes,
                    "ratio": stats.ratio,
                    "torrents": stats.torrent_count,
                    "peers": stats.peers,
                    "uptime_secs": stats.uptime_secs,
                    "download_rate": stats.download_rate,
                    "upload_rate": stats.upload_rate,
                }));
            } else {
                println!(
                    "downloaded: {}",
                    output::format_bytes(stats.downloaded_bytes)
                );
                println!("uploaded:   {}", output::format_bytes(stats.uploaded_bytes));
                println!("ratio:      {:.3}", stats.ratio);
                println!("torrents:   {}", stats.torrent_count);
                println!("peers:      {}", stats.peers);
                println!("uptime:     {}s", stats.uptime_secs);
            }
        }
        Command::Files { id, set } => {
            let files = match set {
                Some(spec) => {
                    let current = client
                        .get_torrent_files(GetTorrentFilesRequest { id })
                        .await?
                        .into_inner()
                        .files;
                    let priorities = parse_file_priorities(&spec, current.len())?;
                    client
                        .set_file_priorities(SetFilePrioritiesRequest { id, priorities })
                        .await?
                        .into_inner()
                        .files
                }
                None => {
                    client
                        .get_torrent_files(GetTorrentFilesRequest { id })
                        .await?
                        .into_inner()
                        .files
                }
            };
            if json {
                output::print_json(&output::files_json(&files));
            } else {
                output::print_files(&files);
            }
        }
        Command::Pause { id } => {
            simple_action(&mut client, id, Action::Pause, json).await?;
        }
        Command::Resume { id } => {
            simple_action(&mut client, id, Action::Resume, json).await?;
        }
        Command::Remove { id, delete_files } => {
            simple_action(&mut client, id, Action::Remove { delete_files }, json).await?;
        }
        Command::Move { id, down } => {
            let direction = if down {
                QueueDirection::Down
            } else {
                QueueDirection::Up
            };
            client
                .move_torrent(MoveTorrentRequest {
                    id,
                    direction: direction as i32,
                })
                .await?;
            if json {
                output::print_json(&json!({ "ok": true, "id": id }));
            } else {
                println!("moved {id} {}", if down { "down" } else { "up" });
            }
        }
        Command::Watch { count } => {
            let mut stream = client.watch(WatchRequest {}).await?.into_inner();
            let mut seen = 0usize;
            loop {
                tokio::select! {
                    signal = tokio::signal::ctrl_c() => {
                        signal?;
                        break;
                    }
                    message = stream.message() => {
                        match message? {
                            Some(event) => {
                                output::print_event(&event, json)?;
                                seen += 1;
                                if count.is_some_and(|limit| seen >= limit) {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                }
            }
        }
        Command::Disks => {
            let response = client.get_disks(GetDisksRequest {}).await?.into_inner();
            if json {
                output::print_json(&json!({
                    "enabled": response.enabled,
                    "min_free_bytes": response.min_free_bytes,
                    "margin_bytes": response.margin_bytes,
                    "total_free_bytes": response.total_free_bytes,
                    "unknown": response.unknown,
                    "disks": response.disks.iter().map(|disk| json!({
                        "mount": disk.mount,
                        "device": disk.device,
                        "fs": disk.fs,
                        "total_bytes": disk.total_bytes,
                        "free_bytes": disk.free_bytes,
                        "used_bytes": disk.used_bytes,
                        "low": disk.low,
                        "torrents": disk.torrents,
                        "labels": disk.labels,
                        "zones": disk.zones.iter().map(|zone| json!({
                            "path": zone.path,
                            "label": zone.label,
                            "staging": zone.staging,
                            "enabled": zone.enabled,
                        })).collect::<Vec<_>>(),
                    })).collect::<Vec<_>>(),
                }));
            } else {
                for disk in &response.disks {
                    println!(
                        "{}  {}  {}  {} free / {}  {} torrents{}",
                        disk.mount,
                        disk.fs,
                        disk.device,
                        output::format_bytes(disk.free_bytes),
                        output::format_bytes(disk.total_bytes),
                        disk.torrents,
                        if disk.low { "  LOW" } else { "" }
                    );
                    for zone in &disk.zones {
                        println!(
                            "  {} {}",
                            if zone.staging { "staging" } else { "library" },
                            zone.path
                        );
                    }
                }
                for path in &response.unknown {
                    println!("unknown  {path}");
                }
            }
        }
        Command::Notify { command } => match command {
            NotifyCommand::Test => {
                let response = client
                    .send_test_notification(SendTestNotificationRequest {})
                    .await?
                    .into_inner();
                if json {
                    output::print_json(&json!({ "queued": response.queued }));
                } else {
                    println!("queued {} test delivery(ies)", response.queued);
                }
            }
            NotifyCommand::Status => {
                let response = client
                    .get_notification_status(GetNotificationStatusRequest {})
                    .await?
                    .into_inner();
                if json {
                    output::print_json(&json!({
                        "pending": response.pending,
                        "dead": response.dead,
                        "channels": response.channels,
                    }));
                } else {
                    println!(
                        "pending {} dead {} channels {}",
                        response.pending, response.dead, response.channels
                    );
                }
            }
        },
        Command::Config { command } => match command {
            ConfigCommand::Get => {
                let response = client.get_config(GetConfigRequest {}).await?.into_inner();
                if json {
                    output::print_json(&json!({
                        "revision": response.revision,
                        "env_locked": response.env_locked,
                        "toml": response.toml,
                    }));
                } else {
                    print!("{}", response.toml);
                }
            }
            ConfigCommand::Set { file, revision } => {
                let toml = std::fs::read_to_string(&file)?;
                let revision = match revision {
                    Some(revision) => revision,
                    None => {
                        client
                            .get_config(GetConfigRequest {})
                            .await?
                            .into_inner()
                            .revision
                    }
                };
                let response = client
                    .set_config(SetConfigRequest { toml, revision })
                    .await?
                    .into_inner();
                if json {
                    output::print_json(&json!({ "revision": response.revision }));
                } else {
                    println!("config updated to revision {}", response.revision);
                }
            }
        },
    }

    Ok(())
}

async fn simple_action(
    client: &mut AuthClient,
    id: u64,
    action: Action,
    json: bool,
) -> Result<(), CliError> {
    match action {
        Action::Pause => {
            client.pause_torrent(PauseTorrentRequest { id }).await?;
        }
        Action::Resume => {
            client.resume_torrent(ResumeTorrentRequest { id }).await?;
        }
        Action::Remove { delete_files } => {
            client
                .remove_torrent(RemoveTorrentRequest { id, delete_files })
                .await?;
        }
    }

    if json {
        output::print_json(&json!({ "ok": true, "id": id }));
    } else {
        println!("{} {id}", action.past());
    }
    Ok(())
}

/// `import-deluge --dry-run`: lists the torrents and the plugin configs the
/// import would use, without needing a daemon.
fn import_deluge_report(
    state_dir: &std::path::Path,
    config_dir: Option<&std::path::Path>,
    json: bool,
) -> Result<(), CliError> {
    let options = import_deluge::Options {
        config_dir: config_dir.map_or_else(
            || import_deluge::default_config_dir(state_dir),
            std::path::Path::to_path_buf,
        ),
        state_dir: state_dir.to_path_buf(),
    };
    let report = import_deluge::scan(&options).map_err(CliError::Config)?;

    if json {
        let torrents: Vec<_> = report
            .torrents
            .iter()
            .map(|torrent| {
                json!({
                    "infohash": torrent.info_hash,
                    "name": torrent.name,
                    "total_bytes": torrent.total_bytes,
                    "label": torrent.label,
                    "path": torrent.path.display().to_string(),
                })
            })
            .collect();
        output::print_json(&json!({
            "torrents": torrents,
            "labels": report
                .plugins
                .labels
                .iter()
                .map(|label| json!({ "name": label.name, "color": label.color }))
                .collect::<Vec<_>>(),
            "assignments": report.plugins.assignments.len(),
            "watch_dirs": report.plugins.watch_dirs.len(),
            "commands": report.plugins.commands.len(),
            "scheduler": {
                "cells": report.plugins.scheduler_cells,
                "red": report.plugins.scheduler_red,
            },
            "warnings": report.warnings,
        }));
        return Ok(());
    }

    output::print_import_table(&report);
    Ok(())
}

fn print_import_warnings(warnings: &[String]) {
    for warning in warnings {
        eprintln!("warning: {warning}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_renders_in_both_languages() {
        ferrent_i18n::install(ferrent_i18n::Lang::Hu);
        let hungarian = localize_command(Cli::command()).render_help().to_string();
        assert!(
            hungarian.contains("Torrentek listázása"),
            "hungarian help missing: {hungarian}"
        );

        ferrent_i18n::install(ferrent_i18n::Lang::En);
        let english = localize_command(Cli::command()).render_help().to_string();
        assert!(
            english.contains("List torrents"),
            "english help missing: {english}"
        );
        ferrent_i18n::install(ferrent_i18n::Lang::Hu);
    }

    #[test]
    fn file_priority_spec_keeps_unlisted_files_normal() {
        assert_eq!(
            parse_file_priorities("1=skip", 3).expect("parse"),
            vec![4, 0, 4]
        );
        assert_eq!(
            parse_file_priorities("0=high,2=1", 3).expect("parse"),
            vec![7, 4, 1]
        );
    }

    #[test]
    fn file_priority_spec_rejects_bad_input() {
        assert!(parse_file_priorities("3=skip", 3).is_err(), "index range");
        assert!(parse_file_priorities("0=urgent", 1).is_err(), "level name");
        assert!(parse_file_priorities("nope", 1).is_err(), "missing '='");
        assert!(parse_file_priorities("0=9", 1).is_err(), "level range");
    }
}
