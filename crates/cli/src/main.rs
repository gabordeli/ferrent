//! `ferrent-cli`: control a Ferrent daemon over TLS.

mod config;
mod error;
mod output;

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use ferrent_protocol::client::AuthClient;
use ferrent_protocol::v1::{
    AddTorrentRequest, GetConfigRequest, GetNotificationStatusRequest, GetTorrentRequest,
    ListTorrentsRequest, MoveTorrentRequest, PauseTorrentRequest, QueueDirection,
    RemoveTorrentRequest, ResumeTorrentRequest, SendTestNotificationRequest, SetConfigRequest,
    SortField, WatchRequest,
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
        /// Save path on the daemon host (default: the daemon's download dir).
        #[arg(long, default_value = "")]
        save_path: String,
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
    /// Show one torrent.
    Show {
        /// Torrent id.
        id: u64,
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

#[tokio::main]
async fn main() {
    let _telemetry = ferrent_telemetry::init("ferrent-cli");
    let cli = Cli::parse();
    if let Err(error) = run(cli).await {
        ferrent_telemetry::capture_error(&error);
        ferrent_telemetry::flush();
        eprintln!("ferrent-cli: {error}");
        std::process::exit(error.exit_code());
    }
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

    let resolved = config::resolve(server, ca, token, config_path)?;
    let ca_pem = std::fs::read_to_string(&resolved.ca).map_err(|error| {
        CliError::Config(format!(
            "cannot read CA certificate {}: {error}",
            resolved.ca.display()
        ))
    })?;
    let mut client = ferrent_protocol::client::connect(&resolved.server, &ca_pem, &resolved.token)
        .await
        .map_err(|error| CliError::Transport(error.to_string()))?;

    match command {
        Command::Add {
            file,
            name,
            save_path,
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
