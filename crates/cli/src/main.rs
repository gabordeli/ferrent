//! `ferrent-cli`: control a Ferrent daemon over TLS.

mod config;
mod error;
mod output;

use std::path::PathBuf;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use ferrent_protocol::client::AuthClient;
use ferrent_protocol::v1::{
    AddTorrentRequest, AddTrackerRequest, CreateLabelRequest, CreateTorrentRequest,
    DeleteLabelRequest, GetAutoAddStatusRequest, GetConfigRequest, GetDisksRequest,
    GetHookStatusRequest, GetNotificationStatusRequest, GetRssStatusRequest, GetStatsRequest,
    GetStatusRequest, GetTorrentFilesRequest, GetTorrentPeersRequest, GetTorrentRequest,
    GetTorrentTrackersRequest, GetWebSeedsRequest, ImportStateRequest, ImportStateResponse, Label,
    ListLabelsRequest, ListTorrentsRequest, MoveTorrentRequest, PauseTorrentRequest,
    PreviewRssFeedRequest, QueueDirection, RecheckTorrentRequest, RefreshBlocklistRequest,
    RemoveTorrentRequest, RemoveTrackerRequest, RenameTorrentFileRequest, ResumeTorrentRequest,
    ScanAutoAddRequest, SendTestNotificationRequest, SetConfigRequest, SetFilePrioritiesRequest,
    SetSequentialRequest, SetSuperSeedingRequest, SetTorrentLabelRequest, SetTorrentLimitsRequest,
    SetWebSeedsRequest, SortField, UpdateLabelRequest, WatchRequest,
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
    /// One-shot health summary: version, uptime, torrents, free space, pin.
    Status,
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
    /// Import a uTorrent app-data directory (resume.dat and its .torrent files).
    ImportUtorrent {
        /// uTorrent's app-data directory, the one holding `resume.dat`.
        #[arg(long)]
        app_dir: PathBuf,
        /// Replace every torrent's save path on the daemon host.
        #[arg(long)]
        save_root: Option<String>,
        /// Map a client path prefix to a daemon path (`FROM=TO`; repeatable).
        #[arg(long = "path-map")]
        path_map: Vec<String>,
        /// Only list what was found; do not add anything to the daemon.
        #[arg(long)]
        dry_run: bool,
    },
    /// Show one torrent.
    Show {
        /// Torrent id.
        id: u64,
    },
    /// List the peers of one torrent.
    Peers {
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
    /// Show the RSS watcher status; `--preview` fetches one feed.
    Rss {
        /// Feed URL to fetch with the watcher's user agent and timeout.
        #[arg(long)]
        preview: Option<String>,
        /// Title regex applied to the preview; empty means every torrent item.
        #[arg(long, default_value = "")]
        filter: String,
    },
    /// Watch directories: configured folders and their scanner counters.
    Autoadd {
        /// Scan now instead of only printing the counters.
        #[arg(long)]
        scan: bool,
        /// Limit the scan to one watch path (default: every enabled folder).
        #[arg(long, requires = "scan")]
        path: Option<String>,
    },
    /// Hook journal: pending events and the most recent runs.
    Hooks {
        /// Number of recent runs to show (server default: 20).
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
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
    /// Labels: list, create, update, delete, or assign one to a torrent.
    Labels {
        #[command(subcommand)]
        command: Option<LabelsCommand>,
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
    /// Set per-torrent limits; zero inherits the label or session policy.
    Limits {
        /// Torrent id.
        id: u64,
        /// Download limit in bytes per second.
        #[arg(long, default_value_t = 0)]
        download: u64,
        /// Upload limit in bytes per second.
        #[arg(long, default_value_t = 0)]
        upload: u64,
        /// Maximum peer connections.
        #[arg(long, default_value_t = 0)]
        connections: u32,
        /// Maximum upload slots.
        #[arg(long, default_value_t = 0)]
        upload_slots: u32,
        /// Drop the manual override.
        #[arg(long)]
        clear: bool,
    },
    /// Toggle sequential download or super seeding on one torrent.
    Set {
        /// Torrent id.
        id: u64,
        /// Download the pieces in order.
        #[arg(long, overrides_with = "no_sequential")]
        sequential: bool,
        /// Turn sequential download off.
        #[arg(long = "no-sequential", overrides_with = "sequential")]
        no_sequential: bool,
        /// Seed without advertising a full copy.
        #[arg(long = "super-seeding", overrides_with = "no_super_seeding")]
        super_seeding: bool,
        /// Turn super seeding off.
        #[arg(long = "no-super-seeding", overrides_with = "super_seeding")]
        no_super_seeding: bool,
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
    /// Saved daemons in cli.toml: list, add, remove, or select one.
    Servers {
        #[command(subcommand)]
        command: Option<ServersCommand>,
    },
    /// Notifications: outbox status and a test delivery.
    Notify {
        #[command(subcommand)]
        command: NotifyCommand,
    },
}

#[derive(Subcommand)]
enum LabelsCommand {
    /// List labels with their assignment counts.
    List,
    /// Create a label.
    Create {
        /// Lowercase name; unique.
        name: String,
        #[command(flatten)]
        fields: LabelFields,
    },
    /// Update a label, keeping the fields that are not given.
    Update {
        /// Label to update.
        name: String,
        #[command(flatten)]
        fields: LabelFields,
    },
    /// Delete a label and clear its assignments.
    Delete {
        /// Label to delete.
        name: String,
    },
    /// Assign a label to a torrent; an empty name clears it.
    Set {
        /// Torrent id.
        id: u64,
        /// Label name; empty clears the assignment.
        name: String,
    },
}

/// The mutable label attributes; every CLI flag is optional so an update can
/// leave the rest of the label untouched.
#[derive(clap::Args, Clone, Default)]
struct LabelFields {
    /// #RRGGBB or #RRGGBBAA colour.
    #[arg(long)]
    color: Option<String>,
    /// Move the payload here when the torrent completes.
    #[arg(long)]
    move_completed: Option<String>,
    /// Download limit in bytes per second; zero inherits the session limit.
    #[arg(long)]
    download_limit: Option<u64>,
    /// Upload limit in bytes per second; zero inherits the session limit.
    #[arg(long)]
    upload_limit: Option<u64>,
    /// Maximum peer connections.
    #[arg(long)]
    connections: Option<u32>,
    /// Maximum upload slots.
    #[arg(long)]
    upload_slots: Option<u32>,
    /// Tracker URL substring for auto-assignment; repeatable.
    #[arg(long = "match-tracker")]
    match_trackers: Vec<String>,
    /// Name regex for auto-assignment.
    #[arg(long)]
    match_name: Option<String>,
}

impl LabelFields {
    /// Applies the given flags over `label`; an absent flag keeps the value.
    fn apply(&self, label: &mut Label) {
        if let Some(color) = &self.color {
            label.color.clone_from(color);
        }
        if let Some(path) = &self.move_completed {
            label.move_completed_path.clone_from(path);
        }
        if let Some(limit) = self.download_limit {
            label.download_rate_limit = limit;
        }
        if let Some(limit) = self.upload_limit {
            label.upload_rate_limit = limit;
        }
        if let Some(limit) = self.connections {
            label.connections_limit = limit;
        }
        if let Some(limit) = self.upload_slots {
            label.upload_slots_limit = limit;
        }
        if !self.match_trackers.is_empty() {
            label.match_trackers.clone_from(&self.match_trackers);
        }
        if let Some(pattern) = &self.match_name {
            label.match_name.clone_from(pattern);
        }
    }

    fn into_label(self, name: &str) -> Label {
        let mut label = Label {
            name: name.to_owned(),
            ..Label::default()
        };
        self.apply(&mut label);
        label
    }
}

#[derive(Subcommand, Clone)]
enum ServersCommand {
    /// List the saved daemons.
    List,
    /// Save a daemon under a name (replaces an existing entry).
    Add {
        /// Profile name, usable as `--server <name>`.
        name: String,
        /// Daemon URL, e.g. https://192.168.2.27:50051.
        #[arg(long)]
        server: String,
        /// CA certificate used to pin the daemon.
        #[arg(long)]
        ca: Option<PathBuf>,
        /// Bearer token stored in the config (mode 0600).
        #[arg(long)]
        token: Option<String>,
        /// Also make it the default for later commands.
        #[arg(long = "use")]
        use_it: bool,
    },
    /// Remove a saved daemon.
    Remove {
        /// Profile name.
        name: String,
    },
    /// Make a saved daemon the default for later commands.
    Use {
        /// Profile name.
        name: String,
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
    if let Command::ImportUtorrent {
        app_dir,
        save_root,
        path_map,
        dry_run: true,
    } = &command
    {
        return import_utorrent_report(app_dir, save_root.as_deref(), path_map, json);
    }

    // `servers` edits the local config, so it runs before the daemon resolves.
    if let Command::Servers { command } = &command {
        return servers_report(
            command.clone().unwrap_or(ServersCommand::List),
            config_path,
            json,
        );
    }

    let resolved = config::resolve(server, ca, token, config_path)?;

    let ca_pem = std::fs::read_to_string(&resolved.ca).map_err(|error| {
        CliError::Config(format!(
            "cannot read CA certificate {}: {error}",
            resolved.ca.display()
        ))
    })?;
    let ca_path = resolved.ca.clone();
    let server = resolved.server.clone();
    let mut client = ferrent_protocol::client::connect(&server, &ca_pem, &resolved.token)
        .await
        .map_err(|error| CliError::Transport {
            server: server.clone(),
            error,
        })?;

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
                    resume_data: Vec::new(),
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
            let response = client
                .import_state(ImportStateRequest {
                    source: "deluge".to_owned(),
                    state_dir: state_dir.to_string_lossy().into_owned(),
                    config_dir: config_dir
                        .map(|path| path.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    paused: true,
                    // The CLI imports everything; the GUI sends a selection.
                    info_hashes: Vec::new(),
                    options: std::collections::HashMap::new(),
                })
                .await?
                .into_inner();
            report_import_state(&response, json);
        }
        Command::ImportUtorrent {
            app_dir,
            save_root,
            path_map,
            dry_run: true,
        } => {
            import_utorrent_report(&app_dir, save_root.as_deref(), &path_map, json)?;
        }
        Command::ImportUtorrent {
            app_dir,
            save_root,
            path_map,
            dry_run: false,
        } => {
            let mut options = std::collections::HashMap::new();
            if let Some(root) = save_root {
                options.insert("save_root".to_owned(), root);
            }
            if !path_map.is_empty() {
                options.insert("path_map".to_owned(), path_map.join(";"));
            }
            let response = client
                .import_state(ImportStateRequest {
                    source: "utorrent".to_owned(),
                    state_dir: app_dir.to_string_lossy().into_owned(),
                    config_dir: String::new(),
                    paused: true,
                    info_hashes: Vec::new(),
                    options,
                })
                .await?
                .into_inner();
            report_import_state(&response, json);
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
        Command::Peers { id } => {
            let peers = client
                .get_torrent_peers(GetTorrentPeersRequest { id })
                .await?
                .into_inner()
                .peers;
            if json {
                let peers: Vec<_> = peers
                    .iter()
                    .map(|peer| {
                        json!({
                            "endpoint": peer.endpoint,
                            "client": peer.client,
                            "progress": peer.progress,
                            "download_rate": peer.download_rate,
                            "upload_rate": peer.upload_rate,
                        })
                    })
                    .collect();
                output::print_json(&json!({ "id": id, "peers": peers }));
            } else if peers.is_empty() {
                println!("no connected peers");
            } else {
                for peer in &peers {
                    println!(
                        "{:<24} {:<20} {:>6.1}%  {} down  {} up",
                        peer.endpoint,
                        if peer.client.is_empty() {
                            "unknown"
                        } else {
                            &peer.client
                        },
                        peer.progress * 100.0,
                        output::format_rate(peer.download_rate),
                        output::format_rate(peer.upload_rate)
                    );
                }
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
        Command::Rss { preview, filter } => {
            if let Some(url) = preview {
                let response = client
                    .preview_rss_feed(PreviewRssFeedRequest { url, filter })
                    .await?
                    .into_inner();
                if json {
                    let items: Vec<_> = response
                        .items
                        .iter()
                        .map(|item| {
                            json!({
                                "title": item.title,
                                "torrent_url": item.torrent_url,
                                "matched": item.matched,
                                "reason": item.reason,
                            })
                        })
                        .collect();
                    output::print_json(&json!({
                        "total": response.total,
                        "items": items,
                    }));
                } else {
                    println!(
                        "{} items, {} matched",
                        response.total,
                        response.items.iter().filter(|item| item.matched).count()
                    );
                    for item in &response.items {
                        println!(
                            "  {}  {}{}",
                            if item.matched { "yes" } else { "no " },
                            item.title,
                            if item.reason.is_empty() {
                                String::new()
                            } else {
                                format!("  ({})", item.reason)
                            }
                        );
                    }
                }
            } else {
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
        }
        Command::Autoadd { scan, path } => {
            if scan {
                let response = client
                    .scan_auto_add(ScanAutoAddRequest {
                        path: path.unwrap_or_default(),
                    })
                    .await?
                    .into_inner();
                if json {
                    output::print_json(&json!({
                        "added": response.added,
                        "duplicates": response.duplicates,
                        "failed": response.failed,
                        "invalid": response.invalid,
                        "deferred": response.deferred,
                    }));
                } else {
                    println!(
                        "added {}, duplicates {}, deferred {}, invalid {}, failed {}",
                        response.added,
                        response.duplicates,
                        response.deferred,
                        response.invalid,
                        response.failed
                    );
                }
            } else {
                let dirs = client
                    .get_auto_add_status(GetAutoAddStatusRequest {})
                    .await?
                    .into_inner()
                    .dirs;
                if json {
                    let dirs: Vec<_> = dirs
                        .iter()
                        .map(|dir| {
                            json!({
                                "path": dir.path,
                                "enabled": dir.enabled,
                                "label": dir.label,
                                "move_to": dir.move_to,
                                "pending": dir.pending,
                                "added": dir.added,
                                "duplicates": dir.duplicates,
                                "invalid": dir.invalid,
                                "deferred": dir.deferred,
                                "last_error": dir.last_error,
                                "last_scan_unix": dir.last_scan_unix,
                            })
                        })
                        .collect();
                    output::print_json(&json!({ "dirs": dirs }));
                } else if dirs.is_empty() {
                    println!("no watch directories configured");
                } else {
                    for dir in &dirs {
                        println!(
                            "{}{}  {}  pending {}  added {}  duplicates {}{}",
                            if dir.enabled { "" } else { "(off) " },
                            dir.path,
                            if dir.label.is_empty() {
                                "-"
                            } else {
                                &dir.label
                            },
                            dir.pending,
                            dir.added,
                            dir.duplicates,
                            if dir.move_to.is_empty() {
                                String::new()
                            } else {
                                format!("  move {}", dir.move_to)
                            }
                        );
                    }
                }
            }
        }
        Command::Hooks { limit } => {
            let response = client
                .get_hook_status(GetHookStatusRequest { limit })
                .await?
                .into_inner();
            if json {
                let runs: Vec<_> = response
                    .runs
                    .iter()
                    .map(|run| {
                        json!({
                            "event": run.event,
                            "torrent_id": run.torrent_id,
                            "torrent_name": run.torrent_name,
                            "command": run.command,
                            "succeeded": run.succeeded,
                            "exit_code": run.exit_code,
                            "timed_out": run.timed_out,
                            "duration_ms": run.duration_ms,
                            "stdout": run.stdout,
                            "stderr": run.stderr,
                            "at_unix": run.at_unix,
                        })
                    })
                    .collect();
                output::print_json(&json!({
                    "pending": response.pending,
                    "runs": runs,
                }));
            } else {
                println!("pending events: {}", response.pending);
                if response.runs.is_empty() {
                    println!("no hook runs yet");
                }
                for run in &response.runs {
                    println!(
                        "{}  {}  {}  {}  {}ms{}",
                        run.event,
                        run.torrent_name,
                        run.command,
                        if run.timed_out {
                            "timeout".to_owned()
                        } else {
                            format!("exit {}", run.exit_code)
                        },
                        run.duration_ms,
                        if run.stderr.is_empty() {
                            String::new()
                        } else {
                            format!("  stderr: {}", run.stderr.trim())
                        }
                    );
                }
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
        Command::Labels { command } => match command.unwrap_or(LabelsCommand::List) {
            LabelsCommand::List => {
                let entries = client
                    .list_labels(ListLabelsRequest {})
                    .await?
                    .into_inner()
                    .labels;
                if json {
                    let labels: Vec<_> = entries
                        .iter()
                        .map(|entry| {
                            let label = entry.label.clone().unwrap_or_default();
                            json!({
                                "name": label.name,
                                "color": label.color,
                                "move_completed_path": label.move_completed_path,
                                "download_rate_limit": label.download_rate_limit,
                                "upload_rate_limit": label.upload_rate_limit,
                                "connections_limit": label.connections_limit,
                                "upload_slots_limit": label.upload_slots_limit,
                                "match_trackers": label.match_trackers,
                                "match_name": label.match_name,
                                "torrents": entry.torrent_count,
                            })
                        })
                        .collect();
                    output::print_json(&json!({ "labels": labels }));
                } else if entries.is_empty() {
                    println!("no labels");
                } else {
                    for entry in &entries {
                        let label = entry.label.clone().unwrap_or_default();
                        println!(
                            "{:<16} {:>5}  {}{}{}",
                            label.name,
                            entry.torrent_count,
                            if label.color.is_empty() {
                                ""
                            } else {
                                &label.color
                            },
                            if label.move_completed_path.is_empty() {
                                String::new()
                            } else {
                                format!("  move {}", label.move_completed_path)
                            },
                            if label.match_name.is_empty() {
                                String::new()
                            } else {
                                format!("  match /{}/", label.match_name)
                            }
                        );
                    }
                }
            }
            LabelsCommand::Create { name, fields } => {
                let label = fields.into_label(&name);
                let created = client
                    .create_label(CreateLabelRequest { label: Some(label) })
                    .await?
                    .into_inner()
                    .label
                    .unwrap_or_default();
                if json {
                    output::print_json(&json!({ "name": created.name, "color": created.color }));
                } else {
                    println!("created label {}", created.name);
                }
            }
            LabelsCommand::Update { name, fields } => {
                let entries = client
                    .list_labels(ListLabelsRequest {})
                    .await?
                    .into_inner()
                    .labels;
                let mut label = entries
                    .into_iter()
                    .find_map(|entry| entry.label.filter(|label| label.name == name))
                    .ok_or_else(|| {
                        CliError::Status(tonic::Status::not_found(format!(
                            "label {name} not found"
                        )))
                    })?;
                fields.apply(&mut label);
                let updated = client
                    .update_label(UpdateLabelRequest { label: Some(label) })
                    .await?
                    .into_inner()
                    .label
                    .unwrap_or_default();
                if json {
                    output::print_json(&json!({ "name": updated.name, "color": updated.color }));
                } else {
                    println!("updated label {}", updated.name);
                }
            }
            LabelsCommand::Delete { name } => {
                let response = client
                    .delete_label(DeleteLabelRequest { name: name.clone() })
                    .await?
                    .into_inner();
                if json {
                    output::print_json(&json!({
                        "name": name,
                        "removed": response.removed,
                        "cleared_assignments": response.cleared_assignments,
                    }));
                } else if response.removed {
                    println!(
                        "removed label {name} ({} assignments cleared)",
                        response.cleared_assignments
                    );
                } else {
                    println!("label {name} not found");
                }
            }
            LabelsCommand::Set { id, name } => {
                let response = client
                    .set_torrent_label(SetTorrentLabelRequest { id, label: name })
                    .await?
                    .into_inner();
                if json {
                    output::print_json(&json!({ "id": id, "label": response.label }));
                } else if response.label.is_empty() {
                    println!("torrent {id}: label cleared");
                } else {
                    println!("torrent {id}: label {}", response.label);
                }
            }
        },
        Command::Limits {
            id,
            download,
            upload,
            connections,
            upload_slots,
            clear,
        } => {
            client
                .set_torrent_limits(SetTorrentLimitsRequest {
                    id,
                    download_rate_limit: if clear { 0 } else { download },
                    upload_rate_limit: if clear { 0 } else { upload },
                    connections_limit: if clear { 0 } else { connections },
                    upload_slots_limit: if clear { 0 } else { upload_slots },
                    clear,
                })
                .await?;
            if json {
                output::print_json(&json!({ "ok": true, "id": id, "cleared": clear }));
            } else if clear {
                println!("torrent {id}: limits cleared");
            } else {
                println!("torrent {id}: limits set");
            }
        }
        Command::Set {
            id,
            sequential,
            no_sequential,
            super_seeding,
            no_super_seeding,
        } => {
            let sequential = match (sequential, no_sequential) {
                (true, _) => Some(true),
                (_, true) => Some(false),
                _ => None,
            };
            let super_seeding = match (super_seeding, no_super_seeding) {
                (true, _) => Some(true),
                (_, true) => Some(false),
                _ => None,
            };
            if sequential.is_none() && super_seeding.is_none() {
                return Err(CliError::Config(
                    "nothing to set: pass --sequential/--no-sequential or \
                     --super-seeding/--no-super-seeding"
                        .to_owned(),
                ));
            }
            let mut applied = serde_json::Map::new();
            if let Some(on) = sequential {
                client
                    .set_sequential(SetSequentialRequest { id, on })
                    .await?;
                applied.insert("sequential".to_owned(), json!(on));
            }
            if let Some(on) = super_seeding {
                client
                    .set_super_seeding(SetSuperSeedingRequest { id, on })
                    .await?;
                applied.insert("super_seeding".to_owned(), json!(on));
            }
            if json {
                output::print_json(&json!({ "ok": true, "id": id, "set": applied }));
            } else {
                let mut parts = Vec::new();
                if let Some(on) = sequential {
                    parts.push(format!("sequential {}", if on { "on" } else { "off" }));
                }
                if let Some(on) = super_seeding {
                    parts.push(format!("super seeding {}", if on { "on" } else { "off" }));
                }
                println!("torrent {id}: {}", parts.join(", "));
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
        Command::Status => {
            let status = client.get_status(GetStatusRequest {}).await?.into_inner();
            let fingerprint =
                ferrent_protocol::tls::fingerprint_from_pem(&ca_pem).map_err(|error| {
                    CliError::Config(format!(
                        "invalid CA certificate {}: {error}",
                        ca_path.display()
                    ))
                })?;
            if json {
                output::print_json(&json!({
                    "server": server,
                    "daemon_version": status.daemon_version,
                    "uptime_secs": status.uptime_secs,
                    "torrent_count": status.torrent_count,
                    "active_count": status.active_count,
                    "downloading_count": status.downloading_count,
                    "seeding_count": status.seeding_count,
                    "paused_count": status.paused_count,
                    "error_count": status.error_count,
                    "free_space_bytes": status.free_space_bytes,
                    "download_rate": status.download_rate,
                    "upload_rate": status.upload_rate,
                    "import_enabled": status.import_enabled,
                    "fingerprint": fingerprint,
                }));
            } else {
                let version = if status.daemon_version.is_empty() {
                    "unknown".to_owned()
                } else {
                    status.daemon_version
                };
                println!("server:      {server}");
                println!("daemon:      ferrent {version}");
                println!(
                    "uptime:      {}",
                    output::format_duration(status.uptime_secs)
                );
                println!(
                    "torrents:    {} ({} active, {} paused, {} error)",
                    status.torrent_count,
                    status.active_count,
                    status.paused_count,
                    status.error_count
                );
                println!(
                    "rate:        {} down / {} up",
                    output::format_rate(status.download_rate),
                    output::format_rate(status.upload_rate)
                );
                println!(
                    "free space:  {}",
                    output::format_bytes(status.free_space_bytes)
                );
                println!("tls sha256:  {fingerprint}");
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
        Command::Servers { .. } => {
            // Answered before the connection; kept for exhaustiveness.
        }
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

/// `servers`: the saved daemons live in cli.toml, not on the daemon.
fn servers_report(
    command: ServersCommand,
    config_path: Option<PathBuf>,
    json: bool,
) -> Result<(), CliError> {
    match command {
        ServersCommand::List => {
            let (servers, active) = config::profiles(config_path)?;
            if json {
                let servers: Vec<_> = servers
                    .iter()
                    .map(|entry| {
                        json!({
                            "name": entry.name,
                            "server": entry.server,
                            "ca": entry.ca,
                            "token_set": entry.token.is_some(),
                            "active": active.as_deref() == Some(entry.name.as_str()),
                        })
                    })
                    .collect();
                output::print_json(&json!({ "active": active, "servers": servers }));
            } else if servers.is_empty() {
                println!(
                    "no saved servers; add one with `ferrent-cli servers add <name> --server <url>`"
                );
            } else {
                for entry in &servers {
                    println!(
                        "{}{:<12} {}{}",
                        if active.as_deref() == Some(entry.name.as_str()) {
                            "* "
                        } else {
                            "  "
                        },
                        entry.name,
                        entry.server,
                        if entry.token.is_some() { "  token" } else { "" }
                    );
                }
            }
        }
        ServersCommand::Add {
            name,
            server,
            ca,
            token,
            use_it,
        } => {
            let path = config::add_profile(
                config_path,
                config::ServerEntry {
                    name: name.clone(),
                    server: server.clone(),
                    ca,
                    token,
                },
                use_it,
            )?;
            if json {
                output::print_json(&json!({
                    "name": name, "server": server, "active": use_it, "config": path
                }));
            } else {
                println!(
                    "saved {name} -> {server}{} ({})",
                    if use_it { " (active)" } else { "" },
                    path.display()
                );
            }
        }
        ServersCommand::Remove { name } => {
            let (path, removed) = config::remove_profile(config_path, &name)?;
            if json {
                output::print_json(&json!({ "name": name, "removed": removed, "config": path }));
            } else if removed {
                println!("removed {name} ({})", path.display());
            } else {
                println!("no saved server named {name}");
            }
        }
        ServersCommand::Use { name } => {
            let path = config::use_profile(config_path, &name)?;
            if json {
                output::print_json(&json!({ "active": name, "config": path }));
            } else {
                println!("active server: {name} ({})", path.display());
            }
        }
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
    let options = ferrent_import::Options {
        config_dir: config_dir.map_or_else(
            || ferrent_import::default_config_dir(state_dir),
            std::path::Path::to_path_buf,
        ),
        state_dir: state_dir.to_path_buf(),
    };
    let report = ferrent_import::scan(&options).map_err(CliError::Config)?;
    print_import_report(&report, json);
    Ok(())
}

/// `import-utorrent --dry-run`: reads the uTorrent resume state without a
/// daemon. `save_root` and `path_map` are the same keys the RPC receives.
fn import_utorrent_report(
    app_dir: &std::path::Path,
    save_root: Option<&str>,
    path_map: &[String],
    json: bool,
) -> Result<(), CliError> {
    let mut options = std::collections::BTreeMap::new();
    options.insert("app_dir".to_owned(), app_dir.to_string_lossy().into_owned());
    if let Some(root) = save_root {
        options.insert("save_root".to_owned(), root.to_owned());
    }
    if !path_map.is_empty() {
        options.insert("path_map".to_owned(), path_map.join(";"));
    }
    let report = ferrent_import::build_source("utorrent", &options)
        .map_err(CliError::Config)?
        .scan()
        .map_err(CliError::Config)?;
    print_import_report(&report, json);
    Ok(())
}

fn print_import_report(report: &ferrent_import::Report, json: bool) {
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
        return;
    }

    output::print_import_table(report);
}

fn report_import_state(response: &ImportStateResponse, json: bool) {
    if json {
        let outcomes: Vec<_> = response
            .outcomes
            .iter()
            .map(|outcome| {
                json!({
                    "name": outcome.name,
                    "status": outcome.status,
                    "detail": outcome.detail,
                })
            })
            .collect();
        output::print_json(&json!({
            "found": response.outcomes.len(),
            "added": response.added,
            "already_present": response.skipped,
            "failed": response.failed,
            "warnings": response.warnings,
            "outcomes": outcomes,
        }));
        return;
    }

    println!(
        "imported {}, already present {}, failed {} of {} torrents",
        response.added,
        response.skipped,
        response.failed,
        response.outcomes.len()
    );
    for outcome in &response.outcomes {
        if outcome.status == "error" {
            println!("  {}: {}", outcome.name, outcome.detail);
        }
    }
    print_import_warnings(&response.warnings);
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

    fn command_strings(command: &clap::Command, out: &mut Vec<String>) {
        for text in [command.get_about(), command.get_long_about()]
            .into_iter()
            .flatten()
        {
            out.push(text.to_string());
        }
        for arg in command.get_arguments() {
            for text in [arg.get_help(), arg.get_long_help()].into_iter().flatten() {
                out.push(text.to_string());
            }
        }
        for sub in command.get_subcommands() {
            command_strings(sub, out);
        }
    }

    #[test]
    fn every_help_string_has_a_translation() {
        let known: std::collections::HashSet<&str> = ferrent_i18n::keys().collect();
        let mut strings = Vec::new();
        command_strings(&Cli::command(), &mut strings);
        strings.sort();
        strings.dedup();
        let missing: Vec<&String> = strings
            .iter()
            .filter(|text| !known.contains(text.as_str()))
            .collect();
        assert!(missing.is_empty(), "missing translations: {missing:#?}");
    }

    #[test]
    fn docs_list_every_subcommand() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/cli.md");
        let docs = std::fs::read_to_string(path).expect("docs/cli.md");
        let mut documented = std::collections::HashSet::new();
        for line in docs.lines().filter(|line| line.starts_with("### ")) {
            let mut rest = line;
            while let Some(position) = rest.find("ferrent-cli ") {
                let after = &rest[position + "ferrent-cli ".len()..];
                let name = after
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .trim_end_matches('`');
                if !name.is_empty() {
                    documented.insert(name);
                }
                rest = after;
            }
        }
        let commands: Vec<String> = Cli::command()
            .get_subcommands()
            .map(|sub| sub.get_name().to_owned())
            .collect();
        for command in &commands {
            assert!(
                documented.contains(command.as_str()),
                "docs/cli.md is missing a `### `ferrent-cli {command}`` section"
            );
        }
        for name in &documented {
            assert!(
                commands.iter().any(|command| command == name),
                "docs/cli.md documents the unknown command {name}"
            );
        }
    }

    #[test]
    fn the_man_page_lists_every_subcommand() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../packaging/debian/ferrent-cli.1"
        );
        let man = std::fs::read_to_string(path).expect("ferrent-cli.1");
        let mut documented = std::collections::HashSet::new();
        let mut in_commands = false;
        for line in man.lines() {
            if line.starts_with(".SH ") {
                in_commands = line.trim() == ".SH COMMANDS";
                continue;
            }
            if in_commands && let Some(name) = line.strip_prefix(".B ") {
                let name = name.trim();
                if name.chars().all(|ch| ch.is_ascii_lowercase() || ch == '-') {
                    documented.insert(name);
                }
            }
        }
        let commands: Vec<String> = Cli::command()
            .get_subcommands()
            .map(|sub| sub.get_name().to_owned())
            .collect();
        for command in &commands {
            assert!(
                documented.contains(command.as_str()),
                "ferrent-cli.1 is missing {command}"
            );
        }
        assert_eq!(documented.len(), commands.len(), "man page drift");
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
