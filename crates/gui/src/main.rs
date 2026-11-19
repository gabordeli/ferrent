//! Ferrent GUI shell: server profiles, TLS pinning, reconnect, status bar.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use ferrent_gui::config::{GuiConfig, PinDecision, ServerProfile};
use ferrent_gui::connection::{self, ServerConfig};
use ferrent_protocol::client::AuthClient;
use ferrent_protocol::v1::{Torrent, TorrentState};
use iced::widget::{Column, button, column, container, row, scrollable, text, text_input};
use iced::{Element, Length, Size, Task};

#[derive(Debug, Clone)]
struct Fields {
    name: String,
    server: String,
    ca: String,
    token: String,
}

impl Default for Fields {
    fn default() -> Self {
        Self {
            name: "local".to_owned(),
            server: "https://127.0.0.1:50051".to_owned(),
            ca: "./ferrent-data/tls-cert.pem".to_owned(),
            token: String::new(),
        }
    }
}

impl Fields {
    fn from_profile(profile: &ServerProfile) -> Self {
        Self {
            name: profile.name.clone(),
            server: profile.server.clone(),
            ca: profile.ca.display().to_string(),
            token: profile.token.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Field {
    Name,
    Server,
    Ca,
    Token,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionState {
    Idle,
    Connecting,
    Connected,
    Reconnecting,
}

struct PendingPin {
    config: ServerConfig,
    fingerprint: String,
    profile_name: String,
    expected: Option<String>,
}

#[derive(Clone)]
struct ClientHandle(AuthClient);

impl fmt::Debug for ClientHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ClientHandle")
    }
}

#[derive(Debug, Clone)]
enum Message {
    SelectProfile(usize),
    Field(Field, String),
    SaveProfile,
    Connect,
    Disconnect,
    TrustFingerprint,
    CancelFingerprint,
    Connected(u64, Result<ClientHandle, String>),
    Polled(u64, Result<Vec<Torrent>, String>),
    Reconnect(u64),
}

struct App {
    config: GuiConfig,
    config_path: PathBuf,
    fields: Fields,
    client: Option<AuthClient>,
    torrents: Vec<Torrent>,
    status: String,
    state: ConnectionState,
    pending: Option<PendingPin>,
    generation: u64,
    last_config: Option<ServerConfig>,
    retrying: bool,
}

impl Default for App {
    fn default() -> Self {
        let config_path = GuiConfig::default_path();
        let (config, mut status) = match GuiConfig::load(&config_path) {
            Ok(config) => (config, "Not connected".to_owned()),
            Err(error) => (GuiConfig::default(), format!("Config error: {error}")),
        };
        let selected = if config.profiles.is_empty() {
            None
        } else {
            Some(
                config
                    .profiles
                    .iter()
                    .position(|profile| config.active.as_deref() == Some(profile.name.as_str()))
                    .unwrap_or(0),
            )
        };
        let fields = selected
            .and_then(|index| config.profiles.get(index))
            .map(Fields::from_profile)
            .unwrap_or_default();
        if selected.is_none() && status == "Not connected" {
            status = "No profiles yet; fill in the fields and save".to_owned();
        }

        Self {
            config,
            config_path,
            fields,
            client: None,
            torrents: Vec::new(),
            status,
            state: ConnectionState::Idle,
            pending: None,
            generation: 0,
            last_config: None,
            retrying: false,
        }
    }
}

impl App {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SelectProfile(index) => {
                if let Some(profile) = self.config.profiles.get(index) {
                    self.fields = Fields::from_profile(profile);
                    self.status = format!("Loaded profile {}", profile.name);
                }
                Task::none()
            }
            Message::Field(field, value) => {
                match field {
                    Field::Name => self.fields.name = value,
                    Field::Server => self.fields.server = value,
                    Field::Ca => self.fields.ca = value,
                    Field::Token => self.fields.token = value,
                }
                Task::none()
            }
            Message::SaveProfile => {
                self.save_current_profile();
                Task::none()
            }
            Message::Connect => self.connect_clicked(),
            Message::Disconnect => {
                self.generation += 1;
                self.client = None;
                self.torrents.clear();
                self.state = ConnectionState::Idle;
                self.status = "Disconnected".to_owned();
                Task::none()
            }
            Message::TrustFingerprint => self.trust_fingerprint(),
            Message::CancelFingerprint => {
                self.pending = None;
                self.status = "Connection cancelled".to_owned();
                Task::none()
            }
            Message::Connected(generation, result) => self.connected(generation, result),
            Message::Polled(generation, result) => self.polled(generation, result),
            Message::Reconnect(generation) => {
                if generation == self.generation && self.client.is_none() {
                    if let Some(config) = self.last_config.clone() {
                        return self.begin_connect(config, true);
                    }
                }
                Task::none()
            }
        }
    }

    fn current_profile(&self) -> ServerProfile {
        let name = if self.fields.name.trim().is_empty() {
            "local".to_owned()
        } else {
            self.fields.name.trim().to_owned()
        };
        let fingerprint = self
            .config
            .profile(&name)
            .and_then(|profile| profile.fingerprint.clone());
        ServerProfile {
            name,
            server: self.fields.server.trim().to_owned(),
            ca: PathBuf::from(self.fields.ca.trim()),
            token: self.fields.token.clone(),
            fingerprint,
        }
    }

    fn save_current_profile(&mut self) {
        let profile = self.current_profile();
        if profile.server.is_empty() || profile.ca.as_os_str().is_empty() {
            self.status = "Server URL and CA path are required".to_owned();
            return;
        }
        self.config.upsert(profile.clone());
        match self.config.save(&self.config_path) {
            Ok(()) => {
                self.status = format!("Saved profile {}", profile.name);
                self.fields.name = profile.name;
            }
            Err(error) => self.status = format!("Cannot save config: {error}"),
        }
    }

    fn connect_clicked(&mut self) -> Task<Message> {
        if self.fields.token.trim().is_empty() {
            self.status = "API token is required".to_owned();
            return Task::none();
        }
        let profile = self.current_profile();
        if profile.server.is_empty() || profile.ca.as_os_str().is_empty() {
            self.status = "Server URL and CA path are required".to_owned();
            return Task::none();
        }

        self.config.upsert(profile.clone());
        if let Err(error) = self.config.save(&self.config_path) {
            self.status = format!("Cannot save config: {error}");
            return Task::none();
        }

        let config = ServerConfig::from(&profile);
        let fingerprint = match connection::fingerprint(&config) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                self.status = error;
                return Task::none();
            }
        };

        match self.config.pin_decision(&profile, &fingerprint) {
            PinDecision::Match => self.begin_connect(config, false),
            PinDecision::First => {
                self.pending = Some(PendingPin {
                    config,
                    fingerprint,
                    profile_name: profile.name,
                    expected: None,
                });
                self.status = "Confirm the server certificate".to_owned();
                Task::none()
            }
            PinDecision::Mismatch { expected } => {
                self.status = format!("Certificate changed (was {expected})");
                self.pending = Some(PendingPin {
                    config,
                    fingerprint,
                    profile_name: profile.name,
                    expected: Some(expected),
                });
                Task::none()
            }
        }
    }

    fn trust_fingerprint(&mut self) -> Task<Message> {
        let Some(pending) = self.pending.take() else {
            return Task::none();
        };
        if let Some(profile) = self
            .config
            .profiles
            .iter_mut()
            .find(|profile| profile.name == pending.profile_name)
        {
            profile.fingerprint = Some(pending.fingerprint.clone());
        }
        self.config.active = Some(pending.profile_name.clone());
        if let Err(error) = self.config.save(&self.config_path) {
            self.status = format!("Cannot save pin: {error}");
        }
        self.begin_connect(pending.config, false)
    }

    fn begin_connect(&mut self, config: ServerConfig, retrying: bool) -> Task<Message> {
        self.generation += 1;
        let generation = self.generation;
        self.client = None;
        self.torrents.clear();
        self.state = ConnectionState::Connecting;
        self.retrying = retrying;
        self.last_config = Some(config.clone());
        self.status = format!("Connecting to {}...", config.server);

        Task::perform(
            async move { (generation, connection::connect(&config).await) },
            |(generation, result)| Message::Connected(generation, result.map(ClientHandle)),
        )
    }

    fn connected(
        &mut self,
        generation: u64,
        result: Result<ClientHandle, String>,
    ) -> Task<Message> {
        if generation != self.generation {
            return Task::none();
        }
        match result {
            Ok(handle) => {
                self.client = Some(handle.0);
                self.state = ConnectionState::Connected;
                self.retrying = false;
                self.status = "Connected".to_owned();
                self.schedule_poll()
            }
            Err(error) => {
                self.client = None;
                self.status = format!("Connection failed: {error}");
                if self.retrying {
                    self.state = ConnectionState::Reconnecting;
                    Task::perform(
                        async move {
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            generation
                        },
                        Message::Reconnect,
                    )
                } else {
                    self.state = ConnectionState::Idle;
                    Task::none()
                }
            }
        }
    }

    fn schedule_poll(&mut self) -> Task<Message> {
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        let generation = self.generation;
        Task::perform(
            async move {
                tokio::time::sleep(Duration::from_secs(1)).await;
                (generation, connection::list(&client).await)
            },
            |(generation, result)| Message::Polled(generation, result),
        )
    }

    fn polled(&mut self, generation: u64, result: Result<Vec<Torrent>, String>) -> Task<Message> {
        if generation != self.generation {
            return Task::none();
        }
        match result {
            Ok(torrents) => {
                self.torrents = torrents;
                self.state = ConnectionState::Connected;
                self.status = format!("{} torrents", self.torrents.len());
                self.schedule_poll()
            }
            Err(error) => {
                self.client = None;
                self.state = ConnectionState::Reconnecting;
                self.retrying = true;
                self.status = format!("Disconnected: {error}; reconnecting...");
                Task::perform(
                    async move {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        generation
                    },
                    Message::Reconnect,
                )
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let content = if self.pending.is_some() {
            self.fingerprint_view()
        } else if self.client.is_some() {
            self.main_view()
        } else {
            self.connect_view()
        };
        column![content, self.status_bar()].into()
    }

    fn connect_view(&self) -> Element<'_, Message> {
        let profiles: Element<Message> = if self.config.profiles.is_empty() {
            text("No profiles yet").into()
        } else {
            Column::with_children(
                self.config
                    .profiles
                    .iter()
                    .enumerate()
                    .map(|(index, profile)| {
                        button(text(profile.name.clone()))
                            .on_press(Message::SelectProfile(index))
                            .width(Length::Fill)
                            .into()
                    })
                    .collect::<Vec<Element<Message>>>(),
            )
            .spacing(6)
            .into()
        };

        let fields = column![
            text("Server profile").size(18),
            text_input("Name", &self.fields.name)
                .on_input(|value| Message::Field(Field::Name, value))
                .width(Length::Fill),
            text("Server URL").size(12),
            text_input("https://host:50051", &self.fields.server)
                .on_input(|value| Message::Field(Field::Server, value))
                .width(Length::Fill),
            text("CA certificate (PEM path)").size(12),
            text_input("/path/to/tls-cert.pem", &self.fields.ca)
                .on_input(|value| Message::Field(Field::Ca, value))
                .width(Length::Fill),
            text("API token").size(12),
            text_input("token", &self.fields.token)
                .secure(true)
                .on_input(|value| Message::Field(Field::Token, value))
                .width(Length::Fill),
            row![
                button("Save profile").on_press(Message::SaveProfile),
                button("Connect").on_press(Message::Connect),
            ]
            .spacing(8),
        ]
        .spacing(8)
        .width(Length::Fill);

        let body = row![
            container(profiles).width(Length::Fixed(220.0)).padding(10),
            container(fields).width(Length::Fill).padding(10),
        ]
        .spacing(10);

        container(body)
            .padding(20)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn fingerprint_view(&self) -> Element<'_, Message> {
        let Some(pending) = &self.pending else {
            return text("").into();
        };
        let (title, warning) = if pending.expected.is_some() {
            (
                "Server certificate changed",
                "The fingerprint differs from the pinned value. Only trust it if you rotated the daemon certificate.",
            )
        } else {
            (
                "New server certificate",
                "Verify this fingerprint out-of-band before trusting the daemon.",
            )
        };
        let pinned: Element<Message> = match &pending.expected {
            Some(expected) => {
                column![text("Pinned fingerprint").size(12), text(expected).size(14),]
                    .spacing(4)
                    .into()
            }
            None => column![].into(),
        };

        container(
            column![
                text(title).size(22),
                text(warning),
                text("Server").size(12),
                text(&pending.config.server),
                text("Presented SHA-256 fingerprint").size(12),
                text(&pending.fingerprint).size(16),
                pinned,
                row![
                    button("Trust and connect").on_press(Message::TrustFingerprint),
                    button("Cancel").on_press(Message::CancelFingerprint),
                ]
                .spacing(8),
            ]
            .spacing(10),
        )
        .padding(24)
        .max_width(760)
        .into()
    }

    fn main_view(&self) -> Element<'_, Message> {
        let server = self
            .last_config
            .as_ref()
            .map_or("", |config| config.server.as_str());
        let header = row![
            text("Ferrent").size(24),
            text(server),
            button("Disconnect").on_press(Message::Disconnect),
        ]
        .spacing(16);

        let rows: Vec<Element<Message>> = self
            .torrents
            .iter()
            .take(100)
            .map(|torrent| {
                text(format!(
                    "{:>4}  {:<12}  {:>6.1}%  {:>12}  {}",
                    torrent.id,
                    state_label(torrent.state),
                    torrent.progress * 100.0,
                    format_rate(torrent.download_rate),
                    torrent.name
                ))
                .into()
            })
            .collect();

        let list: Element<Message> = if rows.is_empty() {
            text("No torrents yet").into()
        } else {
            scrollable(Column::with_children(rows).spacing(2))
                .height(Length::Fill)
                .into()
        };

        column![header, list]
            .spacing(10)
            .padding(16)
            .height(Length::Fill)
            .into()
    }

    fn status_bar(&self) -> Element<'_, Message> {
        let state = match self.state {
            ConnectionState::Idle => "idle",
            ConnectionState::Connecting => "connecting",
            ConnectionState::Connected => "connected",
            ConnectionState::Reconnecting => "reconnecting",
        };
        container(text(format!("{state}  |  {}", self.status)).size(12))
            .padding(8)
            .width(Length::Fill)
            .into()
    }
}

fn state_label(state: i32) -> &'static str {
    match TorrentState::try_from(state).unwrap_or(TorrentState::Unspecified) {
        TorrentState::Unspecified => "unknown",
        TorrentState::Queued => "queued",
        TorrentState::Checking => "checking",
        TorrentState::Downloading => "downloading",
        TorrentState::Seeding => "seeding",
        TorrentState::Paused => "paused",
        TorrentState::Error => "error",
    }
}

fn format_rate(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B/s")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB/s", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MiB/s", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn main() -> iced::Result {
    let app =
        iced::application("Ferrent", App::update, App::view).window_size(Size::new(1100.0, 700.0));

    if std::env::args().any(|argument| argument == "--connect") {
        app.run_with(|| (App::default(), Task::done(Message::Connect)))
    } else {
        app.run()
    }
}
