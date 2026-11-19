//! Ferrent GUI shell: server profiles, TLS pinning, virtualized torrent
//! table, server-side filtering/sorting, and optimistic actions.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ferrent_gui::actions::{self, TorrentAction};
use ferrent_gui::config::{GuiConfig, PinDecision, ServerProfile};
use ferrent_gui::connection::{self, ListQuery, Page, ServerConfig};
use ferrent_gui::table::{
    self, Column, ContextMenu, MenuAction, ROW_HEIGHT, TABLE_HEIGHT, TABLE_WIDTH, Table,
    TableMessage, VISIBLE_ROWS,
};
use ferrent_protocol::client::AuthClient;
use ferrent_protocol::v1::{SortField, Torrent, TorrentState};
use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};
use iced::widget::{
    Column as WidgetColumn, button, column, container, row, scrollable, text, text_input,
};
use iced::{Element, Length, Point, Size, Subscription, Task};

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
    KeyPressed(Key, Modifiers),
    KeyReleased(Key, Modifiers),
    Frame(Instant),
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
        Task::batch([refresh, self.schedule_poll()])
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
            if let Some(column) = table::column_at(point.x) {
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
        Task::none()
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

    fn key_pressed(&mut self, key: Key, modifiers: Modifiers) -> Task<Message> {
        self.modifiers = modifiers;
        match key.as_ref() {
            Key::Named(Named::Control) => self.modifiers.insert(Modifiers::CTRL),
            Key::Named(Named::Shift) => self.modifiers.insert(Modifiers::SHIFT),
            _ => {}
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
        column![content, self.status_bar()].into()
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

        let selected_count = self.selected.len();
        let toolbar = row![
            text_input("Filter torrents...", &self.filter)
                .id(self.filter_id.clone())
                .on_input(Message::FilterChanged)
                .width(Length::Fill),
            button("Refresh").on_press(Message::Refresh),
            action_button(
                "Pause",
                Message::Action(TorrentAction::Pause),
                selected_count > 0
            ),
            action_button(
                "Resume",
                Message::Action(TorrentAction::Resume),
                selected_count > 0
            ),
            action_button(
                "Remove",
                Message::Action(TorrentAction::Remove),
                selected_count > 0
            ),
            text(format!("{selected_count} selected")).size(12),
        ]
        .spacing(8);

        let add_row = row![
            text_input("Path to .torrent", &self.add_path)
                .id(self.add_id.clone())
                .on_input(Message::AddPathChanged)
                .width(Length::Fill),
            text_input("Save path on daemon", &self.add_save_path)
                .on_input(Message::AddSavePathChanged)
                .width(Length::Fixed(220.0)),
            action_button("Add .torrent", Message::AddTorrent, !self.add_in_flight),
        ]
        .spacing(8);

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
        .width(Length::Fixed(TABLE_WIDTH))
        .height(Length::Fixed(TABLE_HEIGHT))
        .into();
        let table = table.map(Message::Table);

        let body = row![table, self.details_view()].spacing(10);
        column![header, toolbar, add_row, body]
            .spacing(8)
            .padding(12)
            .height(Length::Fill)
            .into()
    }

    fn details_view(&self) -> Element<'_, Message> {
        let content: Element<Message> = if self.selected.is_empty() {
            column![
                text("Details").size(18),
                text("Click a row to select it. Right-click for actions.").size(12),
                text("Space: pause/resume  Delete: remove").size(11),
                text("Ctrl+F: filter  Ctrl+R: refresh  Ctrl+A: select page").size(11),
            ]
            .spacing(8)
            .into()
        } else if self.selected.len() == 1 {
            let torrent = self.selected_torrents.values().next();
            match torrent {
                Some(torrent) => column![
                    text("Details").size(18),
                    text(&torrent.name).size(15),
                    row![text("ID").size(12), text(torrent.id.to_string())].spacing(8),
                    row![
                        text("State").size(12),
                        text(table::state_label(torrent.state))
                    ]
                    .spacing(8),
                    row![
                        text("Progress").size(12),
                        text(format!("{:.1}%", torrent.progress * 100.0))
                    ]
                    .spacing(8),
                    row![
                        text("Size").size(12),
                        text(format_size(torrent.total_bytes))
                    ]
                    .spacing(8),
                    row![
                        text("Down").size(12),
                        text(table::format_rate(torrent.download_rate))
                    ]
                    .spacing(8),
                    row![
                        text("Up").size(12),
                        text(table::format_rate(torrent.upload_rate))
                    ]
                    .spacing(8),
                    if torrent.error.is_empty() {
                        Element::<Message>::from(text(""))
                    } else {
                        Element::<Message>::from(text(format!("Error: {}", torrent.error)).size(12))
                    },
                ]
                .spacing(7)
                .into(),
                None => column![text("Details").size(18), text("Loading...").size(12)]
                    .spacing(8)
                    .into(),
            }
        } else {
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
            column![
                text("Details").size(18),
                text(format!("{} torrents selected", self.selected.len())).size(13),
                text("Space: pause/resume  Delete: remove").size(11),
                scrollable(WidgetColumn::with_children(names).spacing(3)).height(Length::Fill),
            ]
            .spacing(8)
            .into()
        };

        container(content)
            .padding(12)
            .width(Length::Fixed(320.0))
            .height(Length::Fixed(TABLE_HEIGHT))
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
    let app = iced::application("Ferrent", App::update, App::view)
        .subscription(App::subscription)
        .window_size(Size::new(1100.0, 700.0));

    if std::env::args().any(|argument| argument == "--connect" || argument == "--measure") {
        app.run_with(|| (App::default(), Task::done(Message::Connect)))
    } else {
        app.run()
    }
}
