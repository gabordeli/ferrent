//! Ferrent GUI shell: server profiles, TLS pinning, virtualized torrent
//! table, server-side filtering/sorting, and optimistic actions.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ferrent_gui::actions::{self, TorrentAction};
use ferrent_gui::config::{GuiConfig, PinDecision, ServerProfile};
use ferrent_gui::connection::{self, ListQuery, Page, ServerConfig, ServerStatus};
use ferrent_gui::icons::{self, glyph};
use ferrent_gui::table::{
    self, Column, ContextMenu, MenuAction, ROW_HEIGHT, Table, TableMessage, VISIBLE_ROWS,
};
use ferrent_gui::theme;
use ferrent_protocol::client::AuthClient;
use ferrent_protocol::v1::{SortField, Torrent, TorrentFile, TorrentPeer, TorrentState, TorrentTracker};
use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};
use iced::widget::{
    Column as WidgetColumn, button, column, container, horizontal_space, mouse_area, row,
    scrollable, text, text_input, vertical_space,
};
use iced::{Color, Element, Length, Point, Size, Subscription, Task, mouse};

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
struct ActionUndo {
    page: Page,
    page_offset: Option<usize>,
    scroll: f32,
    selected: BTreeSet<u64>,
    selected_torrents: BTreeMap<u64, Torrent>,
    active: Option<u64>,
}

#[derive(Debug, Clone)]
struct ServerRow {
    name: String,
    server: String,
    status: Option<ServerStatus>,
    error: Option<String>,
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
    Poll(u64),
    StatusPolled(Result<ServerStatus, String>),
    SelectCategory(Option<i32>),
    SelectTab(DetailsTab),
    Resized(f32, f32),
    DetailsLoaded(Box<DetailsPayload>),
    DetailsFailed(String),
    PageLoaded {
        request: u64,
        offset: usize,
        result: Result<Page, String>,
    },
    Reconnect(u64),
    FilterChanged(String),
    FilterSettled(u64),
    Table(TableMessage),
    Refresh,
    Action(TorrentAction),
    ActionFinished {
        undo: Box<ActionUndo>,
        action: TorrentAction,
        errors: Vec<String>,
    },
    AddPathChanged(String),
    AddSavePathChanged(String),
    AddTorrent,
    TorrentAdded(Result<u64, String>),
    FileDropped(PathBuf),
    OpenServers,
    OpenSettings,
    CloseModal,
    ModalDragStart,
    CursorMoved(Point),
    ModalDragEnd,
    ModalBackdrop,
    ServerStatus(String, Result<ServerStatus, String>),
    SelectServer(usize),
    ConnectServer(String),
    DeleteServer(String),
    NewServer,
    TestServer,
    ServerTested {
        profile: ServerProfile,
        fingerprint: String,
        result: Result<ServerStatus, String>,
    },
    KeyPressed(Key, Modifiers),
    KeyReleased(Key, Modifiers),
    ToggleSidebar,
    Frame(Instant),
}

#[derive(Debug, Clone)]
enum DetailsPayload {
    Files(Vec<TorrentFile>),
    Peers(Vec<TorrentPeer>),
    Trackers(Vec<TorrentTracker>),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum DetailsTab {
    #[default]
    General,
    Files,
    Peers,
    Trackers,
    Activity,
}

impl DetailsTab {
    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Files => "Files",
            Self::Peers => "Peers",
            Self::Trackers => "Trackers",
            Self::Activity => "Activity",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Modal {
    Servers,
    Settings,
}

impl Modal {
    fn title(self) -> &'static str {
        match self {
            Self::Servers => "Servers",
            Self::Settings => "Settings",
        }
    }

    fn size(self) -> (f32, f32) {
        match self {
            Self::Servers => (880.0, 620.0),
            Self::Settings => (560.0, 380.0),
        }
    }
}

struct App {
    config: GuiConfig,
    config_path: PathBuf,
    fields: Fields,
    client: Option<AuthClient>,
    status: String,
    state: ConnectionState,
    pending: Option<PendingPin>,
    generation: u64,
    last_config: Option<ServerConfig>,
    retrying: bool,

    filter: String,
    filter_id: text_input::Id,
    filter_settle: u64,
    sort: SortField,
    descending: bool,
    page: Page,
    page_offset: Option<usize>,
    page_request: u64,
    page_in_flight: bool,
    scroll: f32,
    selected: BTreeSet<u64>,
    selected_torrents: BTreeMap<u64, Torrent>,
    active: Option<u64>,
    context: Option<ContextMenu>,
    pending_select: Option<usize>,

    add_path: String,
    add_save_path: String,
    add_id: text_input::Id,
    add_in_flight: bool,
    action_in_flight: bool,
    sidebar_collapsed: bool,

    modal: Option<Modal>,
    modal_offset: (f32, f32),
    modal_drag: Option<(Point, (f32, f32))>,
    cursor: Point,
    server_rows: Vec<ServerRow>,
    server_testing: bool,
    server_message: String,
    server_status: Option<ServerStatus>,
    poll_tick: u32,
    state_filter: Option<i32>,
    details_tab: DetailsTab,
    window_width: f32,
    window_height: f32,
    details_for: Option<u64>,
    details_payload: Option<DetailsPayload>,
    details_error: Option<String>,
    activity: Vec<String>,

    modifiers: Modifiers,
    measure: bool,
    measure_started: Option<Instant>,
    measure_finished: bool,
    frames: u64,
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
            status,
            state: ConnectionState::Idle,
            pending: None,
            generation: 0,
            last_config: None,
            retrying: false,
            filter: String::new(),
            filter_id: text_input::Id::new("filter"),
            filter_settle: 0,
            sort: SortField::Id,
            descending: false,
            page: Page::default(),
            page_offset: None,
            page_request: 0,
            page_in_flight: false,
            scroll: 0.0,
            selected: BTreeSet::new(),
            selected_torrents: BTreeMap::new(),
            active: None,
            context: None,
            pending_select: None,
            add_path: String::new(),
            add_save_path: "./downloads".to_owned(),
            add_id: text_input::Id::new("add-path"),
            add_in_flight: false,
            action_in_flight: false,
            sidebar_collapsed: false,
            modal: None,
            modal_offset: (0.0, 0.0),
            modal_drag: None,
            cursor: Point::ORIGIN,
            server_rows: Vec::new(),
            server_testing: false,
            server_message: String::new(),
            server_status: None,
            poll_tick: 0,
            state_filter: None,
            details_tab: DetailsTab::General,
            window_width: 1440.0,
            window_height: 900.0,
            details_for: None,
            details_payload: None,
            details_error: None,
            activity: Vec::new(),
            modifiers: Modifiers::empty(),
            measure: std::env::args().any(|arg| arg == "--measure" || arg == "--auto"),
            measure_started: None,
            measure_finished: false,
            frames: 0,
        }
    }
}

impl App {
    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            iced::keyboard::on_key_press(|key, modifiers| {
                Some(Message::KeyPressed(key, modifiers))
            }),
            iced::keyboard::on_key_release(|key, modifiers| {
                Some(Message::KeyReleased(key, modifiers))
            }),
            iced::event::listen_with(|event, _status, _window| match event {
                iced::Event::Window(iced::window::Event::FileDropped(path)) => {
                    Some(Message::FileDropped(path))
                }
                _ => None,
            }),
        ];
        subscriptions.push(
            iced::window::resize_events()
                .map(|(_id, size)| Message::Resized(size.width, size.height)),
        );
        subscriptions.push(iced::event::listen_with(
            |event, _status, _window| match event {
                iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                    Some(Message::CursorMoved(position))
                }
                iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                    Some(Message::ModalDragEnd)
                }
                _ => None,
            },
        ));
        if self.measure {
            subscriptions.push(iced::window::frames().map(Message::Frame));
        }
        Subscription::batch(subscriptions)
    }

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
                self.reset_page();
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
            Message::Poll(generation) => self.poll(generation),
            Message::StatusPolled(result) => {
                if let Ok(status) = result {
                    self.server_status = Some(status);
                }
                Task::none()
            }
            Message::SelectCategory(filter) => {
                self.state_filter = filter;
                self.scroll = 0.0;
                self.refresh_page()
            }
            Message::SelectTab(tab) => {
                self.details_tab = tab;
                self.load_details()
            }
            Message::Resized(width, height) => {
                self.window_width = width;
                self.window_height = height;
                self.clamp_modal_offset();
                Task::none()
            }
            Message::DetailsLoaded(payload) => {
                if self.details_for == self.selected_torrents.keys().next().copied() {
                    self.details_payload = Some(*payload);
                    self.details_error = None;
                }
                Task::none()
            }
            Message::DetailsFailed(error) => {
                self.details_error = Some(error);
                self.details_payload = None;
                Task::none()
            }
            Message::PageLoaded {
                request,
                offset,
                result,
            } => self.page_loaded(request, offset, result),
            Message::Reconnect(generation) => {
                if generation == self.generation && self.client.is_none() {
                    if let Some(config) = self.last_config.clone() {
                        return self.begin_connect(config, true);
                    }
                }
                Task::none()
            }
            Message::FilterChanged(value) => {
                self.filter = value;
                self.filter_settle += 1;
                let settle = self.filter_settle;
                Task::perform(
                    async move {
                        tokio::time::sleep(Duration::from_millis(150)).await;
                        settle
                    },
                    Message::FilterSettled,
                )
            }
            Message::FilterSettled(settle) => {
                if settle == self.filter_settle {
                    self.reset_page();
                    self.request_page(0)
                } else {
                    Task::none()
                }
            }
            Message::Table(TableMessage::Scroll(delta)) => {
                let max = table::max_scroll(self.page.total as usize);
                self.scroll = (self.scroll + delta).clamp(0.0, max);
                self.ensure_page()
            }
            Message::Table(TableMessage::Pressed(point)) => self.table_pressed(point),
            Message::Table(TableMessage::RightPressed(point)) => self.table_right_pressed(point),
            Message::Refresh => self.refresh_page(),
            Message::Action(action) => self.start_action(action),
            Message::ActionFinished {
                undo,
                action,
                errors,
            } => self.action_finished(*undo, action, errors),
            Message::AddPathChanged(value) => {
                self.add_path = value;
                Task::none()
            }
            Message::AddSavePathChanged(value) => {
                self.add_save_path = value;
                Task::none()
            }
            Message::AddTorrent => self.add_current_torrent(),
            Message::TorrentAdded(result) => self.torrent_added(result),
            Message::FileDropped(path) => self.file_dropped(path),
            Message::OpenServers => self.open_servers(),
            Message::OpenSettings => {
                self.open_modal(Modal::Settings);
                Task::none()
            }
            Message::CloseModal => {
                self.modal = None;
                self.modal_drag = None;
                Task::none()
            }
            Message::ModalDragStart => {
                if self.modal.is_some() {
                    self.modal_drag = Some((self.cursor, self.modal_offset));
                }
                Task::none()
            }
            Message::CursorMoved(point) => {
                self.cursor = point;
                if let Some((from, base)) = self.modal_drag {
                    self.modal_offset = (base.0 + point.x - from.x, base.1 + point.y - from.y);
                    self.clamp_modal_offset();
                }
                Task::none()
            }
            Message::ModalDragEnd => {
                self.modal_drag = None;
                Task::none()
            }
            Message::ModalBackdrop => Task::none(),
            Message::ServerStatus(name, result) => {
                if let Some(row) = self.server_rows.iter_mut().find(|row| row.name == name) {
                    match result {
                        Ok(status) => {
                            row.status = Some(status);
                            row.error = None;
                        }
                        Err(error) => {
                            row.status = None;
                            row.error = Some(error);
                        }
                    }
                }
                if self
                    .server_rows
                    .iter()
                    .all(|row| row.status.is_some() || row.error.is_some())
                {
                    self.server_message.clear();
                }
                Task::none()
            }
            Message::SelectServer(index) => {
                if let Some(profile) = self.config.profiles.get(index) {
                    self.fields = Fields::from_profile(profile);
                    self.server_message = format!("Editing {}", profile.name);
                }
                Task::none()
            }
            Message::ConnectServer(name) => self.connect_server(&name),
            Message::DeleteServer(name) => self.delete_server(&name),
            Message::NewServer => {
                self.fields = Fields::default();
                self.fields.name = "new".to_owned();
                self.server_message = "New profile".to_owned();
                Task::none()
            }
            Message::TestServer => self.test_server(),
            Message::ServerTested {
                profile,
                fingerprint,
                result,
            } => self.server_tested(profile, fingerprint, result),
            Message::KeyPressed(key, modifiers) => self.key_pressed(key, modifiers),
            Message::KeyReleased(key, modifiers) => {
                self.modifiers = modifiers;
                match key.as_ref() {
                    Key::Named(Named::Control) => self.modifiers.remove(Modifiers::CTRL),
                    Key::Named(Named::Shift) => self.modifiers.remove(Modifiers::SHIFT),
                    _ => {}
                }
                Task::none()
            }
            Message::ToggleSidebar => {
                self.sidebar_collapsed = !self.sidebar_collapsed;
                Task::none()
            }
            Message::Frame(now) => self.frame(now),
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

        self.start_profile_connection(profile)
    }

    fn connect_server(&mut self, name: &str) -> Task<Message> {
        let Some(profile) = self
            .config
            .profiles
            .iter()
            .find(|profile| profile.name == name)
            .cloned()
        else {
            self.server_message = format!("Unknown server {name}");
            return Task::none();
        };
        self.config.active = Some(profile.name.clone());
        if let Err(error) = self.config.save(&self.config_path) {
            self.server_message = format!("Cannot save config: {error}");
        }
        self.modal = None;
        self.fields = Fields::from_profile(&profile);
        self.start_profile_connection(profile)
    }

    fn start_profile_connection(&mut self, profile: ServerProfile) -> Task<Message> {
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
        self.reset_page();
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
                self.reset_page();
                self.status = "Connected; loading torrents...".to_owned();
                self.measure_started = self.measure.then(Instant::now);
                self.measure_finished = false;
                self.frames = 0;
                Task::batch([self.request_page(0), self.schedule_poll()])
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
        if self.client.is_none() {
            return Task::none();
        }
        let generation = self.generation;
        Task::perform(
            async move {
                tokio::time::sleep(Duration::from_secs(1)).await;
                generation
            },
            Message::Poll,
        )
    }

    fn poll(&mut self, generation: u64) -> Task<Message> {
        if generation != self.generation {
            return Task::none();
        }
        let refresh = if self.action_in_flight || self.add_in_flight || self.page_in_flight {
            Task::none()
        } else {
            self.refresh_page()
        };
        self.poll_tick = self.poll_tick.wrapping_add(1);
        let status = match (&self.client, self.poll_tick % 5) {
            (Some(client), 0) => {
                let client = client.clone();
                Task::perform(
                    async move { connection::status(&client).await },
                    Message::StatusPolled,
                )
            }
            _ => Task::none(),
        };
        Task::batch([refresh, status, self.schedule_poll()])
    }

    fn page_loaded(
        &mut self,
        request: u64,
        offset: usize,
        result: Result<Page, String>,
    ) -> Task<Message> {
        self.page_in_flight = false;
        if request != self.page_request {
            return self.ensure_page();
        }
        match result {
            Ok(page) => {
                self.page = page;
                self.page_offset = Some(offset);
                let max = table::max_scroll(self.page.total as usize);
                self.scroll = self.scroll.clamp(0.0, max);
                self.update_selected_from_page();
                if let Some(index) = self.pending_select.take() {
                    self.select_index(index);
                }
                self.state = ConnectionState::Connected;
                self.status = self.page_status();
                self.ensure_page()
            }
            Err(error) => self.fail_connection(error),
        }
    }

    fn page_status(&self) -> String {
        let total = self.page.total;
        if total == 0 {
            return if self.filter.trim().is_empty() {
                "0 torrents".to_owned()
            } else {
                "0 matching torrents".to_owned()
            };
        }
        let offset = self.page_offset.unwrap_or(0);
        let end = offset + self.page.torrents.len();
        let what = if self.filter.trim().is_empty() {
            "torrents"
        } else {
            "matching torrents"
        };
        format!("{total} {what} ({}-{end})", offset + 1)
    }

    fn fail_connection(&mut self, error: String) -> Task<Message> {
        self.client = None;
        self.page_in_flight = false;
        self.state = ConnectionState::Reconnecting;
        self.retrying = true;
        self.status = format!("Disconnected: {error}; reconnecting...");
        let generation = self.generation;
        Task::perform(
            async move {
                tokio::time::sleep(Duration::from_secs(1)).await;
                generation
            },
            Message::Reconnect,
        )
    }

    fn request_page(&mut self, offset: usize) -> Task<Message> {
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        self.page_request = self.page_request.wrapping_add(1);
        let request = self.page_request;
        self.page_in_flight = true;
        let query = ListQuery {
            filter: self.filter.clone(),
            state: self.state_filter,
            sort: self.sort,
            descending: self.descending,
            offset: u32::try_from(offset).unwrap_or(u32::MAX),
            limit: connection::PAGE_SIZE,
        };
        Task::perform(
            async move { (request, offset, connection::list(&client, &query).await) },
            |(request, offset, result)| Message::PageLoaded {
                request,
                offset,
                result,
            },
        )
    }

    fn refresh_page(&mut self) -> Task<Message> {
        self.request_page(self.page_offset.unwrap_or(0))
    }

    fn ensure_page(&mut self) -> Task<Message> {
        if self.client.is_none()
            || self.page_in_flight
            || self.action_in_flight
            || self.add_in_flight
        {
            return Task::none();
        }
        let total = self.page.total as usize;
        let range = table::visible_range(self.scroll, total);
        let covered = self.page_offset.is_some_and(|offset| {
            let end = offset + self.page.torrents.len();
            offset <= range.start && end >= range.end
        });
        if covered {
            return Task::none();
        }
        self.request_page(range.start.saturating_sub(5))
    }

    fn reset_page(&mut self) {
        self.page = Page::default();
        self.page_offset = None;
        self.page_request = self.page_request.wrapping_add(1);
        self.page_in_flight = false;
        self.scroll = 0.0;
        self.clear_selection();
        self.pending_select = None;
        self.context = None;
        self.action_in_flight = false;
    }

    fn clear_selection(&mut self) {
        self.selected.clear();
        self.selected_torrents.clear();
        self.active = None;
    }

    fn update_selected_from_page(&mut self) {
        for torrent in &self.page.torrents {
            if self.selected.contains(&torrent.id) {
                self.selected_torrents.insert(torrent.id, torrent.clone());
            }
        }
    }

    fn select_index(&mut self, index: usize) -> bool {
        let Some(offset) = self.page_offset else {
            return false;
        };
        if index < offset || index >= offset + self.page.torrents.len() {
            return false;
        }
        let torrent = self.page.torrents[index - offset].clone();
        self.selected.clear();
        self.selected_torrents.clear();
        self.selected.insert(torrent.id);
        self.selected_torrents.insert(torrent.id, torrent);
        self.active = self.selected.iter().next().copied();
        true
    }

    fn sort_column(&mut self, column: Column) -> Task<Message> {
        let Some(field) = column.sort_field() else {
            return Task::none();
        };
        if self.sort == field {
            self.descending = !self.descending;
        } else {
            self.sort = field;
            self.descending = false;
        }
        self.reset_page();
        self.request_page(0)
    }

    fn table_pressed(&mut self, point: Point) -> Task<Message> {
        if let Some(menu) = self.context {
            if let Some(action) = table::menu_action_at(&menu, point) {
                self.context = None;
                return self.start_action(match action {
                    MenuAction::Pause => TorrentAction::Pause,
                    MenuAction::Resume => TorrentAction::Resume,
                    MenuAction::Remove => TorrentAction::Remove,
                });
            }
        }
        if point.y < table::HEADER_HEIGHT {
            if let Some(column) = table::column_at(point.x, self.window_width - 232.0) {
                return self.sort_column(column);
            }
            self.context = None;
            return Task::none();
        }
        let Some(index) = table::row_at(point, self.scroll, self.page.total as usize) else {
            self.context = None;
            return Task::none();
        };
        let Some(offset) = self.page_offset else {
            return Task::none();
        };
        if index < offset || index >= offset + self.page.torrents.len() {
            return Task::none();
        }
        let torrent = self.page.torrents[index - offset].clone();
        self.context = None;
        self.select_torrent(torrent, index);
        self.load_details()
    }

    fn select_torrent(&mut self, torrent: Torrent, index: usize) {
        let id = torrent.id;
        let ctrl = self.modifiers.control();
        let shift = self.modifiers.shift();
        let clicked = index.saturating_sub(self.page_offset.unwrap_or(0));
        if shift {
            if let Some(active_index) = self.active.and_then(|active| {
                self.page
                    .torrents
                    .iter()
                    .position(|candidate| candidate.id == active)
            }) {
                let start = active_index.min(clicked);
                let end = active_index.max(clicked);
                self.selected.clear();
                self.selected_torrents.clear();
                for offset_index in start..=end {
                    if let Some(candidate) = self.page.torrents.get(offset_index) {
                        self.selected.insert(candidate.id);
                        self.selected_torrents
                            .insert(candidate.id, candidate.clone());
                    }
                }
                self.selected.insert(id);
                self.selected_torrents.insert(id, torrent);
                self.active = Some(id);
                self.status = format!("{} selected", self.selected.len());
                return;
            }
        }
        if ctrl {
            if self.selected.remove(&id) {
                self.selected_torrents.remove(&id);
                if self.active == Some(id) {
                    self.active = self.selected.iter().next().copied();
                }
            } else {
                self.selected.insert(id);
                self.selected_torrents.insert(id, torrent.clone());
                self.active = Some(id);
            }
        } else {
            self.selected.clear();
            self.selected_torrents.clear();
            self.selected.insert(id);
            self.selected_torrents.insert(id, torrent);
            self.active = Some(id);
        }
        self.status = format!("{} selected", self.selected.len());
    }

    fn table_right_pressed(&mut self, point: Point) -> Task<Message> {
        self.context = None;
        if point.y < table::HEADER_HEIGHT {
            return Task::none();
        }
        let Some(index) = table::row_at(point, self.scroll, self.page.total as usize) else {
            return Task::none();
        };
        let Some(offset) = self.page_offset else {
            return Task::none();
        };
        if index < offset || index >= offset + self.page.torrents.len() {
            return Task::none();
        }
        let torrent = self.page.torrents[index - offset].clone();
        if !self.selected.contains(&torrent.id) {
            self.selected.clear();
            self.selected_torrents.clear();
            self.selected.insert(torrent.id);
            self.selected_torrents.insert(torrent.id, torrent.clone());
        }
        self.active = Some(torrent.id);
        let paused = torrent.state == TorrentState::Paused as i32;
        let (x, y) = table::clamp_menu(point.x, point.y);
        self.context = Some(ContextMenu {
            row: torrent.id,
            x,
            y,
            paused,
        });
        self.status = format!("{} selected", self.selected.len());
        Task::none()
    }

    fn select_all_page(&mut self) -> Task<Message> {
        self.selected.clear();
        self.selected_torrents.clear();
        for torrent in &self.page.torrents {
            self.selected.insert(torrent.id);
            self.selected_torrents.insert(torrent.id, torrent.clone());
        }
        self.active = self.page.torrents.first().map(|torrent| torrent.id);
        self.status = format!("{} selected on this page", self.selected.len());
        Task::none()
    }

    fn toggle_pause(&mut self) -> Task<Message> {
        if self.selected.is_empty() {
            self.status = "Select a torrent first".to_owned();
            return Task::none();
        }
        let any_active = self
            .selected_torrents
            .values()
            .any(|torrent| torrent.state != TorrentState::Paused as i32);
        self.start_action(if any_active {
            TorrentAction::Pause
        } else {
            TorrentAction::Resume
        })
    }

    fn start_action(&mut self, action: TorrentAction) -> Task<Message> {
        if self.action_in_flight {
            self.status = "An action is already running".to_owned();
            return Task::none();
        }
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        let ids: Vec<u64> = self.selected.iter().copied().collect();
        if ids.is_empty() {
            self.status = "Select a torrent first".to_owned();
            return Task::none();
        }
        let undo = Box::new(ActionUndo {
            page: self.page.clone(),
            page_offset: self.page_offset,
            scroll: self.scroll,
            selected: self.selected.clone(),
            selected_torrents: self.selected_torrents.clone(),
            active: self.active,
        });
        self.page_request = self.page_request.wrapping_add(1);
        self.page_in_flight = false;
        self.apply_optimistic(action, &ids);
        self.action_in_flight = true;
        self.status = format!("{} {} torrent(s)...", action.label(), ids.len());
        Task::perform(
            async move {
                let errors = actions::apply(&client, action, &ids).await;
                (undo, action, errors)
            },
            |(undo, action, errors)| Message::ActionFinished {
                undo,
                action,
                errors,
            },
        )
    }

    fn apply_optimistic(&mut self, action: TorrentAction, ids: &[u64]) {
        match action {
            TorrentAction::Pause | TorrentAction::Resume => {
                let state = if action == TorrentAction::Pause {
                    TorrentState::Paused
                } else {
                    TorrentState::Downloading
                } as i32;
                for torrent in &mut self.page.torrents {
                    if self.selected.contains(&torrent.id) {
                        torrent.state = state;
                        torrent.download_rate = 0;
                        torrent.upload_rate = 0;
                    }
                }
                for torrent in self.selected_torrents.values_mut() {
                    torrent.state = state;
                    torrent.download_rate = 0;
                    torrent.upload_rate = 0;
                }
            }
            TorrentAction::Remove => {
                self.page
                    .torrents
                    .retain(|torrent| !self.selected.contains(&torrent.id));
                self.page.total = self.page.total.saturating_sub(ids.len() as u32);
                self.selected.clear();
                self.selected_torrents.clear();
                self.active = None;
                self.context = None;
            }
        }
    }

    fn action_finished(
        &mut self,
        undo: ActionUndo,
        action: TorrentAction,
        errors: Vec<String>,
    ) -> Task<Message> {
        self.action_in_flight = false;
        if errors.is_empty() {
            self.status = format!("{} ok ({} torrent(s))", action.label(), undo.selected.len());
            if action == TorrentAction::Remove {
                self.refresh_page()
            } else {
                Task::none()
            }
        } else {
            self.page = undo.page;
            self.page_offset = undo.page_offset;
            self.scroll = undo.scroll;
            self.selected = undo.selected;
            self.selected_torrents = undo.selected_torrents;
            self.active = undo.active;
            self.status = format!("{} failed: {}", action.label(), errors.join("; "));
            self.refresh_page()
        }
    }

    fn add_current_torrent(&mut self) -> Task<Message> {
        if self.add_in_flight {
            return Task::none();
        }
        let path = self.add_path.trim().to_owned();
        let save_path = self.add_save_path.trim().to_owned();
        if path.is_empty() {
            self.status = "Choose a .torrent path first".to_owned();
            return Task::none();
        }
        if save_path.is_empty() {
            self.status = "Save path is required".to_owned();
            return Task::none();
        }
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        self.add_in_flight = true;
        self.status = format!("Adding {path}...");
        let path = PathBuf::from(path);
        Task::perform(
            async move { actions::add(&client, &path, &save_path).await },
            Message::TorrentAdded,
        )
    }

    fn torrent_added(&mut self, result: Result<u64, String>) -> Task<Message> {
        self.add_in_flight = false;
        match result {
            Ok(id) => {
                self.status = format!("Added torrent {id}");
                self.add_path.clear();
                self.reset_page();
                self.request_page(0)
            }
            Err(error) => {
                self.status = format!("Add failed: {error}");
                Task::none()
            }
        }
    }

    fn file_dropped(&mut self, path: PathBuf) -> Task<Message> {
        if self.client.is_none() {
            return Task::none();
        }
        let is_torrent = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("torrent"));
        if !is_torrent {
            self.status = "Only .torrent files can be dropped".to_owned();
            return Task::none();
        }
        self.add_path = path.display().to_string();
        self.add_current_torrent()
    }

    fn open_servers(&mut self) -> Task<Message> {
        self.open_modal(Modal::Servers);
        self.server_rows = self
            .config
            .profiles
            .iter()
            .map(|profile| ServerRow {
                name: profile.name.clone(),
                server: profile.server.clone(),
                status: None,
                error: None,
            })
            .collect();
        self.server_message = "Checking servers...".to_owned();
        let profiles = self.config.profiles.clone();
        Task::batch(profiles.into_iter().map(|profile| {
            let name = profile.name.clone();
            let config = ServerConfig::from(&profile);
            Task::perform(
                async move { (name, connection::probe(&config).await) },
                |(name, result)| Message::ServerStatus(name, result),
            )
        }))
    }

    fn delete_server(&mut self, name: &str) -> Task<Message> {
        self.config.profiles.retain(|profile| profile.name != name);
        if self.config.active.as_deref() == Some(name) {
            self.config.active = None;
        }
        if let Err(error) = self.config.save(&self.config_path) {
            self.server_message = format!("Cannot save config: {error}");
        } else {
            self.server_message = format!("Deleted {name}");
        }
        self.server_rows.retain(|row| row.name != name);
        Task::none()
    }

    fn test_server(&mut self) -> Task<Message> {
        if self.fields.token.trim().is_empty() {
            self.server_message = "API token is required".to_owned();
            return Task::none();
        }
        let profile = self.current_profile();
        if profile.server.is_empty() || profile.ca.as_os_str().is_empty() {
            self.server_message = "Server URL and CA path are required".to_owned();
            return Task::none();
        }
        let config = ServerConfig::from(&profile);
        let fingerprint = match connection::fingerprint(&config) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                self.server_message = error;
                return Task::none();
            }
        };
        self.server_testing = true;
        self.server_message = format!("Testing {}...", profile.server);
        Task::perform(
            async move { (profile, fingerprint, connection::probe(&config).await) },
            |(profile, fingerprint, result)| Message::ServerTested {
                profile,
                fingerprint,
                result,
            },
        )
    }

    fn server_tested(
        &mut self,
        mut profile: ServerProfile,
        fingerprint: String,
        result: Result<ServerStatus, String>,
    ) -> Task<Message> {
        self.server_testing = false;
        match result {
            Ok(status) => {
                if profile
                    .fingerprint
                    .as_deref()
                    .is_some_and(|expected| expected != fingerprint)
                {
                    self.server_message =
                        "Certificate changed; the pinned fingerprint differs".to_owned();
                    return Task::none();
                }
                profile.fingerprint = Some(fingerprint);
                self.config.upsert(profile.clone());
                if let Err(error) = self.config.save(&self.config_path) {
                    self.server_message = format!("Cannot save config: {error}");
                    return Task::none();
                }
                if let Some(row) = self
                    .server_rows
                    .iter_mut()
                    .find(|row| row.name == profile.name)
                {
                    row.status = Some(status);
                    row.error = None;
                } else {
                    self.server_rows.push(ServerRow {
                        name: profile.name.clone(),
                        server: profile.server.clone(),
                        status: Some(status),
                        error: None,
                    });
                }
                self.server_message = format!("Saved and connected to {}", profile.name);
            }
            Err(error) => {
                self.server_message = format!("Test failed: {error}");
                if let Some(row) = self
                    .server_rows
                    .iter_mut()
                    .find(|row| row.name == profile.name)
                {
                    row.status = None;
                    row.error = Some(error);
                }
            }
        }
        Task::none()
    }

    fn key_pressed(&mut self, key: Key, modifiers: Modifiers) -> Task<Message> {
        self.modifiers = modifiers;
        match key.as_ref() {
            Key::Named(Named::Control) => self.modifiers.insert(Modifiers::CTRL),
            Key::Named(Named::Shift) => self.modifiers.insert(Modifiers::SHIFT),
            _ => {}
        }
        if self.modal.is_some() {
            if matches!(key.as_ref(), Key::Named(Named::Escape)) {
                self.modal = None;
                self.modal_drag = None;
            }
            return Task::none();
        }
        if self.client.is_none() {
            return Task::none();
        }
        if modifiers.command() {
            match key.as_ref() {
                Key::Character("f") => return text_input::focus(self.filter_id.clone()),
                Key::Character("r") => return self.refresh_page(),
                Key::Character("a") => return self.select_all_page(),
                Key::Character("o") => return text_input::focus(self.add_id.clone()),
                _ => {}
            }
        }
        match key.as_ref() {
            Key::Named(Named::Space) => self.toggle_pause(),
            Key::Named(Named::Delete) => self.start_action(TorrentAction::Remove),
            Key::Named(Named::Escape) => {
                self.context = None;
                self.clear_selection();
                self.status = "Selection cleared".to_owned();
                Task::none()
            }
            Key::Named(Named::ArrowDown) => self.move_active(1),
            Key::Named(Named::ArrowUp) => self.move_active(-1),
            Key::Named(Named::PageDown) => self.move_active(VISIBLE_ROWS as isize),
            Key::Named(Named::PageUp) => self.move_active(-(VISIBLE_ROWS as isize)),
            Key::Named(Named::Home) => self.jump_to(0),
            Key::Named(Named::End) => {
                let last = self.page.total.saturating_sub(1) as usize;
                self.jump_to(last)
            }
            _ => Task::none(),
        }
    }

    fn move_active(&mut self, delta: isize) -> Task<Message> {
        let total = self.page.total as usize;
        if total == 0 {
            return Task::none();
        }
        let current = self
            .active
            .and_then(|id| {
                self.page
                    .torrents
                    .iter()
                    .position(|torrent| torrent.id == id)
            })
            .map_or_else(
                || table::first_visible(self.scroll),
                |position| self.page_offset.unwrap_or(0) + position,
            );
        let target = (current as isize + delta).clamp(0, total as isize - 1) as usize;
        self.jump_to(target)
    }

    fn jump_to(&mut self, target: usize) -> Task<Message> {
        let total = self.page.total as usize;
        if total == 0 {
            return Task::none();
        }
        let target = target.min(total - 1);
        let range = table::visible_range(self.scroll, total);
        if target < range.start || target >= range.end {
            self.scroll = (target as f32 * ROW_HEIGHT).clamp(0.0, table::max_scroll(total));
        }
        self.pending_select = Some(target);
        if self.select_index(target) {
            self.pending_select = None;
        }
        self.ensure_page()
    }

    fn frame(&mut self, now: Instant) -> Task<Message> {
        if !self.measure || self.measure_finished || self.client.is_none() {
            return Task::none();
        }
        let start = *self.measure_started.get_or_insert(now);
        self.frames += 1;
        let total = self.page.total as usize;
        let max = table::max_scroll(total);
        if max > 0.0 {
            self.scroll = if self.scroll >= max {
                0.0
            } else {
                (self.scroll + 18.0).min(max)
            };
        }
        let task = self.ensure_page();
        let elapsed = start.elapsed().as_secs_f64();
        if elapsed >= 5.0 {
            self.measure_finished = true;
            println!("fps={:.1}", self.frames as f64 / elapsed);
            println!("rss_kb={}", read_rss_kb());
            println!("rows={total}");
            println!("visible_rows={VISIBLE_ROWS}");
            let _ = std::io::stdout().flush();
            std::process::exit(0);
        }
        task
    }

    fn view(&self) -> Element<'_, Message> {
        let content = if self.pending.is_some() {
            self.fingerprint_view()
        } else if self.client.is_some() {
            self.main_view()
        } else {
            self.connect_view()
        };
        let base: Element<Message> = column![content, self.status_bar()].into();
        let Some(modal) = self.modal else {
            return base;
        };
        let (panel, size) = match modal {
            Modal::Servers => (self.server_manager_view(), modal.size()),
            Modal::Settings => (self.settings_view(), modal.size()),
        };
        let overlay = self.modal_overlay(panel, size);
        iced::widget::stack![base, overlay].into()
    }

    fn open_modal(&mut self, modal: Modal) {
        self.modal = Some(modal);
        self.modal_offset = (0.0, 0.0);
        self.modal_drag = None;
    }

    fn clamp_modal_offset(&mut self) {
        let Some(modal) = self.modal else {
            return;
        };
        let (width, height) = modal.size();
        let margin = 8.0;
        let limit_x = ((self.window_width - width) / 2.0 - margin).max(0.0);
        let limit_y = ((self.window_height - height) / 2.0 - margin).max(0.0);
        self.modal_offset.0 = self.modal_offset.0.clamp(-limit_x, limit_x);
        self.modal_offset.1 = self.modal_offset.1.clamp(-limit_y, limit_y);
    }

    fn modal_overlay<'a>(
        &self,
        panel: Element<'a, Message>,
        size: (f32, f32),
    ) -> Element<'a, Message> {
        let (width, height) = size;
        let max_x = (self.window_width - width).max(0.0);
        let max_y = (self.window_height - height).max(0.0);
        let x = ((max_x / 2.0) + self.modal_offset.0).clamp(0.0, max_x);
        let y = ((max_y / 2.0) + self.modal_offset.1).clamp(0.0, max_y);
        let layer = container(
            column![
                vertical_space().height(y),
                row![horizontal_space().width(x), panel],
            ]
            .width(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_theme: &iced::Theme| container::Style {
            background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.45).into()),
            ..container::Style::default()
        });
        mouse_area(layer).on_press(Message::ModalBackdrop).into()
    }

    fn settings_view(&self) -> Element<'_, Message> {
        let active = self.config.active.clone().unwrap_or_else(|| "—".to_owned());
        let body = column![
            text("Connection").size(12).font(theme::semibold()),
            detail_row("Profile", active, theme::TEXT_MUTED),
            detail_row(
                "Config",
                self.config_path.display().to_string(),
                theme::TEXT_MUTED,
            ),
            text("More options land here as we build them out.")
                .size(12)
                .color(theme::TEXT_MUTED),
        ]
        .spacing(8);
        modal_panel(Modal::Settings, body.into())
    }

    fn server_manager_view(&self) -> Element<'_, Message> {
        let active = self.config.active.clone();
        let rows: Vec<Element<Message>> = self
            .server_rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let (icon, color, state, detail) = match (&row.status, &row.error) {
                    (Some(status), _) => (
                        glyph::CHECK,
                        theme::DOWNLOADING,
                        "available",
                        format!(
                            "{} torrents · ↓ {} · ↑ {} · {} free",
                            status.torrent_count,
                            table::format_rate(status.download_rate),
                            table::format_rate(status.upload_rate),
                            format_size(status.free_space_bytes)
                        ),
                    ),
                    (None, Some(error)) => (glyph::ERROR, theme::ERROR, "error", error.clone()),
                    (None, None) => (
                        glyph::HOURGLASS,
                        theme::QUEUED,
                        "checking",
                        "querying the daemon…".to_owned(),
                    ),
                };
                let is_active = active.as_deref() == Some(row.name.as_str());
                let background = if is_active {
                    theme::SURFACE
                } else {
                    theme::BG_SECONDARY
                };
                let border = if is_active {
                    theme::PRIMARY
                } else {
                    theme::BORDER
                };
                container(
                    row![
                        text(icon).font(icons::solid()).size(12.0).color(color),
                        column![
                            row![
                                text(row.name.clone()).size(13).font(theme::medium()),
                                text(row.server.clone()).size(11).color(theme::TEXT_MUTED),
                            ]
                            .spacing(8)
                            .align_y(iced::Alignment::Center),
                            row![
                                text(state).size(11).color(color),
                                text(detail).size(11).color(theme::TEXT_SECONDARY),
                            ]
                            .spacing(8),
                        ]
                        .width(Length::Fill)
                        .spacing(2),
                        small_action(
                            glyph::PLAY,
                            if is_active { "Reconnect" } else { "Connect" },
                            Message::ConnectServer(row.name.clone()),
                        ),
                        small_action(glyph::EDIT, "Edit", Message::SelectServer(index),),
                        small_action(
                            glyph::TRASH,
                            "Delete",
                            Message::DeleteServer(row.name.clone()),
                        ),
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
                )
                .padding([7, 9])
                .width(Length::Fill)
                .style(move |_theme: &iced::Theme| container::Style {
                    background: Some(background.into()),
                    border: iced::Border {
                        color: border,
                        width: 1.0,
                        radius: 5.0.into(),
                    },
                    ..container::Style::default()
                })
                .into()
            })
            .collect();

        let form = container(
            column![
                text("Add / edit server").size(13).font(theme::semibold()),
                row![
                    text_input("Name", &self.fields.name)
                        .on_input(|value| Message::Field(Field::Name, value))
                        .width(Length::Fixed(220.0)),
                    text_input("Server URL", &self.fields.server)
                        .on_input(|value| Message::Field(Field::Server, value))
                        .width(Length::Fill),
                ]
                .spacing(8),
                row![
                    text_input("CA certificate (PEM path)", &self.fields.ca)
                        .on_input(|value| Message::Field(Field::Ca, value))
                        .width(Length::Fill),
                    text_input("API token", &self.fields.token)
                        .secure(true)
                        .on_input(|value| Message::Field(Field::Token, value))
                        .width(Length::Fixed(220.0)),
                ]
                .spacing(8),
                row![
                    button("Test & save").on_press(Message::TestServer),
                    button("New").on_press(Message::NewServer),
                    button("Refresh").on_press(Message::OpenServers),
                ]
                .spacing(8),
            ]
            .spacing(8),
        )
        .padding(10)
        .width(Length::Fill)
        .style(|_theme: &iced::Theme| container::Style {
            background: Some(theme::BG_SECONDARY.into()),
            border: iced::Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 5.0.into(),
            },
            ..container::Style::default()
        });

        let footer = row![
            text(&self.server_message).size(11).color(theme::TEXT_MUTED),
            horizontal_space(),
            button("Close").on_press(Message::CloseModal),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);

        let body = column![
            scrollable(WidgetColumn::with_children(rows).spacing(4)).height(Length::Fill),
            form,
            footer,
        ]
        .spacing(10)
        .height(Length::Fill);

        modal_panel(Modal::Servers, body.into())
    }

    fn connect_view(&self) -> Element<'_, Message> {
        let profiles: Element<Message> = if self.config.profiles.is_empty() {
            text("No profiles yet").into()
        } else {
            WidgetColumn::with_children(
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
        row![
            self.sidebar_view(),
            column![self.toolbar_view(), self.table_view(), self.details_panel()]
                .width(Length::Fill)
                .height(Length::Fill),
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn table_view(&self) -> Element<'_, Message> {
        let table: Element<TableMessage> = iced::widget::canvas::Canvas::new(Table {
            rows: &self.page.torrents,
            page_offset: self.page_offset.unwrap_or(0),
            total: self.page.total as usize,
            scroll: self.scroll,
            selected: &self.selected,
            sort: self.sort,
            descending: self.descending,
            context: self.context,
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
        table.map(Message::Table)
    }

    fn sidebar_view(&self) -> Element<'_, Message> {
        let status = self.server_status.clone().unwrap_or_default();
        let active = self.state_filter;
        let expanded = !self.sidebar_collapsed;

        let mark = container(text("").size(1.0))
            .width(Length::Fixed(4.0))
            .height(Length::Fixed(18.0))
            .style(|_theme: &iced::Theme| container::Style {
                background: Some(theme::PRIMARY.into()),
                border: iced::Border {
                    color: theme::PRIMARY,
                    width: 0.0,
                    radius: 2.0.into(),
                },
                ..container::Style::default()
            });
        let toggle = icon_button(
            if expanded {
                glyph::CHEVRON_LEFT
            } else {
                glyph::CHEVRON_RIGHT
            },
            Message::ToggleSidebar,
        );

        let header: Element<Message> = if expanded {
            row![
                mark,
                text("Ferrent").size(18).font(theme::semibold()),
                horizontal_space(),
                toggle,
            ]
            .spacing(10)
            .align_y(iced::Alignment::Center)
            .into()
        } else {
            row![mark, toggle]
                .spacing(6)
                .align_y(iced::Alignment::Center)
                .into()
        };

        let nav: Element<Message> = nav_button(
            expanded,
            glyph::LIST,
            theme::TEXT_SECONDARY,
            "All torrents",
            status.torrent_count,
            None,
            active,
        );
        let downloading = TorrentState::Downloading as i32;
        let seeding = TorrentState::Seeding as i32;
        let queued = TorrentState::Queued as i32;
        let paused = TorrentState::Paused as i32;
        let error = TorrentState::Error as i32;

        let bottom: Element<Message> = WidgetColumn::with_children(vec![
            icon_nav(
                expanded,
                glyph::SETTINGS,
                theme::TEXT_SECONDARY,
                "Settings",
                Message::OpenSettings,
            ),
            icon_nav(
                expanded,
                glyph::SERVER,
                theme::TEXT_SECONDARY,
                "Servers",
                Message::OpenServers,
            ),
            icon_nav(
                expanded,
                glyph::POWER,
                theme::TEXT_MUTED,
                "Disconnect",
                Message::Disconnect,
            ),
        ])
        .spacing(2)
        .into();

        column![
            container(header).padding(if expanded {
                iced::Padding::from([16, 14])
            } else {
                iced::Padding::from([16, 8])
            }),
            nav,
            nav_button(
                expanded,
                glyph::DOWNLOAD,
                table::state_color(downloading),
                "Downloading",
                status.downloading,
                Some(downloading),
                active,
            ),
            nav_button(
                expanded,
                glyph::UPLOAD,
                table::state_color(seeding),
                "Seeding",
                status.seeding,
                Some(seeding),
                active,
            ),
            nav_button(
                expanded,
                glyph::QUEUED,
                table::state_color(queued),
                "Queued",
                status.queued,
                Some(queued),
                active,
            ),
            nav_button(
                expanded,
                glyph::PAUSED,
                table::state_color(paused),
                "Paused",
                status.paused,
                Some(paused),
                active,
            ),
            nav_stat(
                expanded,
                glyph::ACTIVE,
                theme::CONNECTING,
                "Active",
                status.active,
            ),
            nav_button(
                expanded,
                glyph::ERROR,
                table::state_color(error),
                "Error",
                status.error,
                Some(error),
                active,
            ),
            horizontal_space(),
            container(bottom).padding([10, 8]),
        ]
        .spacing(2)
        .width(Length::Fixed(if expanded { 216.0 } else { 56.0 }))
        .height(Length::Fill)
        .into()
    }

    fn toolbar_view(&self) -> Element<'_, Message> {
        let selected = self.selected.len();
        let search = row![
            text_input("Search torrents…", &self.filter)
                .id(self.filter_id.clone())
                .on_input(Message::FilterChanged)
                .width(Length::Fill),
            action_button("Refresh", Message::Refresh, true),
            action_button("Pause", Message::Action(TorrentAction::Pause), selected > 0),
            action_button(
                "Resume",
                Message::Action(TorrentAction::Resume),
                selected > 0
            ),
            action_button(
                "Remove",
                Message::Action(TorrentAction::Remove),
                selected > 0
            ),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center);

        let add = row![
            text_input("Path to .torrent", &self.add_path)
                .id(self.add_id.clone())
                .on_input(Message::AddPathChanged)
                .width(Length::Fill),
            text_input("Save path", &self.add_save_path)
                .on_input(Message::AddSavePathChanged)
                .width(Length::Fixed(190.0)),
            action_button("Add torrent", Message::AddTorrent, !self.add_in_flight),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center);

        container(column![search, add].spacing(6))
            .padding([8, 12])
            .width(Length::Fill)
            .style(|_theme: &iced::Theme| container::Style {
                background: Some(theme::BG_SECONDARY.into()),
                ..container::Style::default()
            })
            .into()
    }

    fn details_panel(&self) -> Element<'_, Message> {
        let current = self.details_tab;
        let tabs = [
            DetailsTab::General,
            DetailsTab::Files,
            DetailsTab::Peers,
            DetailsTab::Trackers,
            DetailsTab::Activity,
        ]
        .into_iter()
        .map(|tab| {
            let active = tab == current;
            button(text(tab.label()).size(12).font(theme::medium()))
                .on_press(Message::SelectTab(tab))
                .padding([4, 10])
                .style(move |_theme: &iced::Theme, _status| button::Style {
                    background: Some(if active { theme::SURFACE } else { theme::BG }.into()),
                    text_color: if active {
                        theme::BRIGHT
                    } else {
                        theme::TEXT_SECONDARY
                    },
                    border: iced::Border {
                        color: if active { theme::PRIMARY } else { theme::BG },
                        width: 0.0,
                        radius: 4.0.into(),
                    },
                    ..button::Style::default()
                })
                .into()
        })
        .collect::<Vec<Element<Message>>>();

        container(column![row(tabs).spacing(4), self.details_body()].spacing(8))
            .padding(10)
            .width(Length::Fill)
            .height(Length::Fixed(220.0))
            .style(|_theme: &iced::Theme| container::Style {
                background: Some(theme::BG_SECONDARY.into()),
                border: iced::Border {
                    color: theme::BORDER,
                    width: 1.0,
                    radius: 0.0.into(),
                },
                ..container::Style::default()
            })
            .into()
    }

    fn load_details(&mut self) -> Task<Message> {
        let Some(id) = self.selected_torrents.keys().next().copied() else {
            self.details_payload = None;
            return Task::none();
        };
        if self.selected.len() != 1 {
            self.details_payload = None;
            return Task::none();
        }
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        self.details_for = Some(id);
        self.details_payload = None;
        self.details_error = None;
        match self.details_tab {
            DetailsTab::Files => Task::perform(
                async move { connection::files(&client, id).await.map(DetailsPayload::Files) },
                |result| match result {
                    Ok(payload) => Message::DetailsLoaded(Box::new(payload)),
                    Err(error) => Message::DetailsFailed(error),
                },
            ),
            DetailsTab::Peers => Task::perform(
                async move { connection::peers(&client, id).await.map(DetailsPayload::Peers) },
                |result| match result {
                    Ok(payload) => Message::DetailsLoaded(Box::new(payload)),
                    Err(error) => Message::DetailsFailed(error),
                },
            ),
            DetailsTab::Trackers => Task::perform(
                async move { connection::trackers(&client, id).await.map(DetailsPayload::Trackers) },
                |result| match result {
                    Ok(payload) => Message::DetailsLoaded(Box::new(payload)),
                    Err(error) => Message::DetailsFailed(error),
                },
            ),
            DetailsTab::General | DetailsTab::Activity => Task::none(),
        }
    }

    fn details_body(&self) -> Element<'_, Message> {
        let dim = theme::TEXT_SECONDARY;
        if self.selected.len() > 1 {
            let names = self
                .selected_torrents
                .values()
                .take(50)
                .map(|torrent| {
                    text(format!("{}  {}", torrent.id, torrent.name))
                        .size(12)
                        .into()
                })
                .collect::<Vec<Element<Message>>>();
            return column![
                text(format!("{} torrents selected", self.selected.len())).size(13),
                scrollable(WidgetColumn::with_children(names).spacing(3)).height(Length::Fill),
            ]
            .spacing(6)
            .into();
        }
        let Some(torrent) = self.selected_torrents.values().next() else {
            return column![
                text("Nothing selected").size(12).color(dim),
                text("Click a row to inspect it. Space pause/resume · Delete remove")
                    .size(11)
                    .color(theme::TEXT_MUTED),
            ]
            .spacing(4)
            .into();
        };
        match self.details_tab {
            DetailsTab::General => column![
                row![
                    text(&torrent.name).size(13).font(theme::medium()).width(Length::Fill),
                    text(table::state_label(torrent.state))
                        .size(11)
                        .color(table::state_color(torrent.state)),
                ]
                .spacing(12),
                row![
                    detail_row("Size", format_size(torrent.total_bytes), dim),
                    detail_row(
                        "Progress",
                        format!("{:.1}%", torrent.progress * 100.0),
                        dim
                    ),
                    detail_row("Down", table::format_rate(torrent.download_rate), dim),
                    detail_row("Up", table::format_rate(torrent.upload_rate), dim),
                ]
                .spacing(24),
                row![
                    detail_row("Seeds", torrent.seeds.to_string(), dim),
                    detail_row("Peers", torrent.peers.to_string(), dim),
                    detail_row("Torrent ID", torrent.id.to_string(), dim),
                    detail_row(
                        "Save",
                        self.last_config
                            .as_ref()
                            .map_or("-".to_owned(), |_| "./downloads".to_owned()),
                        dim
                    ),
                ]
                .spacing(24),
                if torrent.error.is_empty() {
                    Element::<Message>::from(text(""))
                } else {
                    Element::<Message>::from(
                        text(format!("Error: {}", torrent.error))
                            .size(12)
                            .color(theme::ERROR),
                    )
                },
            ]
            .spacing(8)
            .into(),
            DetailsTab::Files => self.files_body(),
            DetailsTab::Peers => self.peers_body(),
            DetailsTab::Trackers => self.trackers_body(),
            DetailsTab::Activity => self.activity_body(),
        }
    }

    fn files_body(&self) -> Element<'_, Message> {
        let dim = theme::TEXT_SECONDARY;
        match &self.details_payload {
            Some(DetailsPayload::Files(files)) if !files.is_empty() => {
                let rows = files
                    .iter()
                    .map(|file| {
                        row![
                            text(&file.path).size(11).width(Length::Fill),
                            text(format_size(file.size)).size(11).color(dim),
                            text(format!("{:.0}%", file.progress * 100.0)).size(11).color(dim),
                            text(format!("prio {}", file.priority)).size(11).color(dim),
                        ]
                        .spacing(16)
                        .into()
                    })
                    .collect::<Vec<Element<Message>>>();
                scrollable(WidgetColumn::with_children(rows).spacing(2))
                    .height(Length::Fill)
                    .into()
            }
            Some(_) => text("No files reported.").size(12).color(dim).into(),
            None => self.pending_details(dim),
        }
    }

    fn peers_body(&self) -> Element<'_, Message> {
        let dim = theme::TEXT_SECONDARY;
        match &self.details_payload {
            Some(DetailsPayload::Peers(peers)) if !peers.is_empty() => {
                let rows = peers
                    .iter()
                    .map(|peer| {
                        row![
                            text(&peer.endpoint).size(11).width(Length::Fixed(190.0)),
                            text(&peer.client).size(11).width(Length::Fill),
                            text(format!("{:.0}%", peer.progress * 100.0)).size(11).color(dim),
                            text(format!("↓ {}", table::format_rate(peer.download_rate)))
                                .size(11)
                                .color(dim),
                            text(format!("↑ {}", table::format_rate(peer.upload_rate)))
                                .size(11)
                                .color(dim),
                        ]
                        .spacing(14)
                        .into()
                    })
                    .collect::<Vec<Element<Message>>>();
                scrollable(WidgetColumn::with_children(rows).spacing(2))
                    .height(Length::Fill)
                    .into()
            }
            Some(_) => text("No connected peers.").size(12).color(dim).into(),
            None => self.pending_details(dim),
        }
    }

    fn trackers_body(&self) -> Element<'_, Message> {
        let dim = theme::TEXT_SECONDARY;
        match &self.details_payload {
            Some(DetailsPayload::Trackers(trackers)) if !trackers.is_empty() => {
                let rows = trackers
                    .iter()
                    .map(|tracker| {
                        row![
                            text(&tracker.url).size(11).width(Length::Fill),
                            text(&tracker.status).size(11).color(dim),
                        ]
                        .spacing(16)
                        .into()
                    })
                    .collect::<Vec<Element<Message>>>();
                scrollable(WidgetColumn::with_children(rows).spacing(2))
                    .height(Length::Fill)
                    .into()
            }
            Some(_) => text("No trackers reported.").size(12).color(dim).into(),
            None => self.pending_details(dim),
        }
    }

    fn activity_body(&self) -> Element<'_, Message> {
        let dim = theme::TEXT_SECONDARY;
        if self.activity.is_empty() {
            return text("No events observed yet.").size(12).color(dim).into();
        }
        let rows = self
            .activity
            .iter()
            .rev()
            .take(40)
            .map(|line| text(line.clone()).size(11).color(dim).into())
            .collect::<Vec<Element<Message>>>();
        scrollable(WidgetColumn::with_children(rows).spacing(2))
            .height(Length::Fill)
            .into()
    }

    fn pending_details(&self, dim: Color) -> Element<'_, Message> {
        if let Some(error) = &self.details_error {
            text(format!("Load failed: {error}")).size(12).color(theme::ERROR).into()
        } else {
            text("Loading…").size(12).color(dim).into()
        }
    }

    fn status_bar(&self) -> Element<'_, Message> {
        let (label, color) = state_style(self.state);
        let server = self
            .last_config
            .as_ref()
            .map_or("", |config| config.server.as_str());
        let (down, up, active, total) = self.server_status.as_ref().map_or(
            (0, 0, 0, self.page.total),
            |status| {
                (
                    status.download_rate,
                    status.upload_rate,
                    status.active,
                    status.torrent_count,
                )
            },
        );
        container(
            row![
                text("●").size(9).color(color),
                text(label).size(11).color(color),
                text(server).size(11).color(theme::TEXT_SECONDARY),
                horizontal_space(),
                text(format!("↓ {}", table::format_rate(down))).size(11),
                text(format!("↑ {}", table::format_rate(up))).size(11),
                text(format!("{active} active")).size(11).color(theme::TEXT_SECONDARY),
                text(format!("{total} torrents")).size(11).color(theme::TEXT_SECONDARY),
            ]
            .spacing(14)
            .align_y(iced::Alignment::Center),
        )
        .padding([5, 12])
        .width(Length::Fill)
        .style(|_theme: &iced::Theme| container::Style {
            background: Some(theme::BG_SECONDARY.into()),
            border: iced::Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..container::Style::default()
        })
        .into()
    }
}

fn modal_panel<'a>(modal: Modal, body: Element<'a, Message>) -> Element<'a, Message> {
    let (width, height) = modal.size();
    let close = button(
        text(glyph::CLOSE)
            .font(icons::solid())
            .size(12.0)
            .color(theme::TEXT_SECONDARY),
    )
    .on_press(Message::CloseModal)
    .padding([4, 7])
    .style(|_theme: &iced::Theme, _status| button::Style {
        background: Some(theme::BG.into()),
        border: iced::Border {
            color: theme::BORDER,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..button::Style::default()
    });

    let title_bar = mouse_area(
        container(
            row![
                text(modal.title()).size(15).font(theme::semibold()),
                horizontal_space(),
                close,
            ]
            .align_y(iced::Alignment::Center),
        )
        .padding([8, 12])
        .width(Length::Fill)
        .style(|_theme: &iced::Theme| container::Style {
            background: Some(theme::BG_SECONDARY.into()),
            border: iced::Border {
                color: theme::BORDER,
                width: 0.0,
                radius: iced::border::Radius {
                    top_left: 8.0,
                    top_right: 8.0,
                    bottom_right: 0.0,
                    bottom_left: 0.0,
                },
            },
            ..container::Style::default()
        }),
    )
    .on_press(Message::ModalDragStart)
    .interaction(mouse::Interaction::Grab);

    container(column![
        title_bar,
        container(body).padding(14).height(Length::Fill),
    ])
    .width(Length::Fixed(width))
    .height(Length::Fixed(height))
    .style(|_theme: &iced::Theme| container::Style {
        background: Some(theme::BG.into()),
        border: iced::Border {
            color: theme::BORDER,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..container::Style::default()
    })
    .into()
}

fn small_action(icon: char, label: &'static str, message: Message) -> Element<'static, Message> {
    button(
        row![
            text(icon)
                .font(icons::solid())
                .size(10.0)
                .color(theme::TEXT_SECONDARY),
            text(label).size(11).color(theme::TEXT_SECONDARY),
        ]
        .spacing(5)
        .align_y(iced::Alignment::Center),
    )
    .on_press(message)
    .padding([4, 8])
    .style(|_theme: &iced::Theme, _status| button::Style {
        background: Some(theme::BG.into()),
        border: iced::Border {
            color: theme::BORDER,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..button::Style::default()
    })
    .into()
}

fn nav_button(
    expanded: bool,
    icon: char,
    icon_color: Color,
    label: &'static str,
    count: u32,
    filter: Option<i32>,
    active: Option<i32>,
) -> Element<'static, Message> {
    let selected = active == filter;
    let content: Element<Message> = if expanded {
        row![
            text(icon)
                .font(icons::solid())
                .size(13.0)
                .color(icon_color),
            text(label).size(13),
            horizontal_space(),
            text(count.to_string()).size(11).color(theme::TEXT_MUTED),
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center)
        .into()
    } else {
        container(
            text(icon)
                .font(icons::solid())
                .size(14.0)
                .color(icon_color),
        )
        .width(Length::Fill)
        .center_x(Length::Fill)
        .into()
    };
    button(content)
        .on_press(Message::SelectCategory(filter))
        .width(Length::Fill)
        .padding(nav_padding(expanded))
        .style(move |_theme: &iced::Theme, _status| button::Style {
            background: Some(if selected {
                theme::SURFACE
            } else {
                theme::BG_SECONDARY
            }
            .into()),
            text_color: if selected {
                theme::TEXT
            } else {
                theme::TEXT_SECONDARY
            },
            border: iced::Border {
                color: if selected {
                    theme::PRIMARY
                } else {
                    theme::BG_SECONDARY
                },
                width: 0.0,
                radius: 3.0.into(),
            },
            ..button::Style::default()
        })
        .into()
}

fn nav_stat(
    expanded: bool,
    icon: char,
    icon_color: Color,
    label: &'static str,
    count: u32,
) -> Element<'static, Message> {
    let content: Element<Message> = if expanded {
        row![
            text(icon)
                .font(icons::solid())
                .size(13.0)
                .color(icon_color),
            text(label).size(13).color(theme::TEXT_SECONDARY),
            horizontal_space(),
            text(count.to_string()).size(11).color(theme::TEXT_MUTED),
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center)
        .into()
    } else {
        container(
            text(icon)
                .font(icons::solid())
                .size(14.0)
                .color(icon_color),
        )
        .width(Length::Fill)
        .center_x(Length::Fill)
        .into()
    };
    container(content)
        .padding(nav_padding(expanded))
        .width(Length::Fill)
        .into()
}

fn nav_padding(expanded: bool) -> iced::Padding {
    if expanded {
        iced::Padding::from([7, 12])
    } else {
        iced::Padding::from([8, 0])
    }
}

fn icon_button(icon: char, message: Message) -> Element<'static, Message> {
    button(
        text(icon)
            .font(icons::solid())
            .size(11.0)
            .color(theme::TEXT_SECONDARY),
    )
    .on_press(message)
    .padding([4, 6])
    .style(|_theme: &iced::Theme, _status| button::Style {
        background: Some(theme::BG_SECONDARY.into()),
        border: iced::Border {
            color: theme::BORDER,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..button::Style::default()
    })
    .into()
}

fn icon_nav(
    expanded: bool,
    icon: char,
    icon_color: Color,
    label: &'static str,
    message: Message,
) -> Element<'static, Message> {
    let content: Element<Message> = if expanded {
        row![
            text(icon)
                .font(icons::solid())
                .size(13.0)
                .color(icon_color),
            text(label).size(13).color(theme::TEXT_SECONDARY),
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center)
        .into()
    } else {
        container(
            text(icon)
                .font(icons::solid())
                .size(14.0)
                .color(icon_color),
        )
        .width(Length::Fill)
        .center_x(Length::Fill)
        .into()
    };
    button(content)
        .on_press(message)
        .width(Length::Fill)
        .padding(nav_padding(expanded))
        .style(|_theme: &iced::Theme, _status| button::Style {
            background: Some(theme::BG_SECONDARY.into()),
            border: iced::Border {
                color: theme::BG_SECONDARY,
                width: 0.0,
                radius: 3.0.into(),
            },
            ..button::Style::default()
        })
        .into()
}

fn state_style(state: ConnectionState) -> (&'static str, Color) {
    match state {
        ConnectionState::Idle => ("idle", theme::TEXT_MUTED),
        ConnectionState::Connecting => ("connecting", theme::CONNECTING),
        ConnectionState::Connected => ("connected", theme::DOWNLOADING),
        ConnectionState::Reconnecting => ("reconnecting", theme::CONNECTING),
    }
}

fn detail_row<'a>(label: &'a str, value: String, dim: Color) -> Element<'a, Message> {
    row![
        text(label)
            .size(11)
            .width(Length::Fixed(84.0))
            .color(dim),
        text(value).size(12),
    ]
    .spacing(8)
    .into()
}

fn action_button<'a>(label: &'a str, message: Message, enabled: bool) -> Element<'a, Message> {
    let button = button(text(label));
    if enabled {
        button.on_press(message).into()
    } else {
        button.into()
    }
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn read_rss_kb() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for line in status.lines() {
        if let Some(value) = line.strip_prefix("VmRSS:") {
            return value
                .split_whitespace()
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
        }
    }
    0
}

fn main() -> iced::Result {
    let _telemetry = ferrent_telemetry::init("ferrent-gui");
    let app = iced::application("Ferrent", App::update, App::view)
        .theme(|_state| theme::iced_theme())
        .default_font(theme::regular())
        .font(theme::REGULAR_BYTES)
        .font(theme::MEDIUM_BYTES)
        .font(theme::SEMIBOLD_BYTES)
        .font(icons::SOLID_BYTES)
        .subscription(App::subscription)
        .window_size(Size::new(1440.0, 900.0));

    let result =
        if std::env::args().any(|argument| argument == "--connect" || argument == "--measure") {
            app.run_with(|| (App::default(), Task::done(Message::Connect)))
        } else {
            app.run()
        };
    if let Err(error) = &result {
        ferrent_telemetry::capture_error(error);
    }
    result
}
