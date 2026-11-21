//! Ferrent GUI shell: server profiles, TLS pinning, virtualized torrent
//! table, server-side filtering/sorting, and optimistic actions.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use ferrent_gui::actions::{self, TorrentAction};
use ferrent_gui::activity::{
    ActivityLog, Change, Event as ActivityEvent, Source as ActivitySource,
};
use ferrent_gui::config::{GuiConfig, PinDecision, ServerProfile};
use ferrent_gui::connection::{self, ListQuery, Page, ServerConfig, ServerStatus};
use ferrent_gui::icons::{self, glyph};
use ferrent_gui::settings::{
    self, ConfigField, DigestField, DiskRow, HookEvent, HookField, LabelField, SchedulerLevel,
    Settings, StorageField, SuccessAction, Tab, WatchField, WebhookField,
};
use ferrent_gui::table::{
    self, Column, ContextMenu, MenuAction, ROW_HEIGHT, Table, TableMessage, VISIBLE_ROWS,
};
use ferrent_gui::theme;
use ferrent_gui::tracker;
use ferrent_protocol::client::AuthClient;
use ferrent_protocol::v1::{
    Label as ProtoLabel, SortField, Torrent, TorrentFile, TorrentPeer, TorrentState, TorrentTracker,
};
use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};
use iced::widget::{
    Column as WidgetColumn, Row as WidgetRow, button, checkbox, column, container,
    horizontal_space, mouse_area, row, scrollable, text, text_input, tooltip, vertical_space,
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
    /// No client; the status bar must not show the last poll's numbers.
    Offline,
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
    AddSavePathChanged(String),
    AddNameChanged(String),
    AddConfirmed,
    PickTorrent,
    TorrentPicked(Option<PathBuf>),
    OpenRemove,
    RemoveConfirmed,
    ToggleRemoveFiles(bool),
    QueueMove(bool),
    QueueMoved(Vec<String>),
    ToggleSearch,
    TorrentAdded(Result<u64, String>),
    FileDropped(PathBuf),
    OpenServers,
    OpenSettings,
    OpenStorage,
    SettingsTab(Tab),
    SettingsField(ConfigField, String),
    SettingsSeedMode(bool),
    SettingsQueueEnabled(bool),
    SettingsReload,
    SettingsSave,
    SettingsLoaded {
        request: u64,
        config: Result<connection::ConfigSnapshot, String>,
        labels: Result<Vec<settings::LabelRow>, String>,
        auto_add: Result<Vec<settings::AutoAddStatusRow>, String>,
        hooks: Result<connection::HookStatus, String>,
    },
    SettingsSaved {
        request: u64,
        toml: String,
        result: Result<u64, String>,
    },
    LabelSelected(String),
    LabelNew,
    LabelField(LabelField, String),
    LabelSave,
    LabelSaved {
        count: u32,
        result: Result<ProtoLabel, String>,
    },
    LabelDeleteRequested,
    LabelDeleteCancelled,
    LabelDeleteConfirmed,
    LabelDeleted {
        name: String,
        result: Result<u32, String>,
    },
    LabelsLoaded(Result<Vec<settings::LabelRow>, String>),
    AutoAddSelected(usize),
    AutoAddAdd,
    AutoAddRemove,
    AutoAddField(WatchField, String),
    AutoAddEnabled(bool),
    AutoAddPaused(bool),
    AutoAddAction(SuccessAction),
    AutoAddScanNow,
    AutoAddStatusLoaded(Result<Vec<settings::AutoAddStatusRow>, String>),
    AutoAddScanned(Result<connection::ScanTotals, String>),
    HooksSelected(usize),
    HooksAdd,
    HooksRemove,
    HooksField(HookField, String),
    HooksEvent(HookEvent),
    HooksShell(bool),
    HooksEnabled(bool),
    HooksRefresh,
    HooksStatusLoaded(Result<connection::HookStatus, String>),
    SchedulerEnabled(bool),
    SchedulerPauseSeeding(bool),
    SchedulerField(SchedulerField, String),
    SchedulerCell(usize, usize),
    NotifySelected(usize),
    NotifyAdd,
    NotifyRemove,
    NotifyField(WebhookField, String),
    NotifyEnabled(bool),
    NotifyWebhookEnabled(bool),
    NotifyTest,
    NotifyTested(Result<u32, String>),
    NotifyStatusLoaded(Result<connection::NotifyStatus, String>),
    DisksLoaded(Result<Vec<DiskRow>, String>),
    AddLabelChanged(String),
    StorageEnabled(bool),
    StorageOnLow(bool),
    StorageRequireSpace(bool),
    StorageNumber(StorageField, String),
    ZonePath(usize, String),
    ZoneLabel(usize, String),
    ZoneStaging(usize, bool),
    ZoneEnabled(usize, bool),
    ZoneAdd,
    ZoneRemove(usize),
    NotifyTelegramSelected(usize),
    NotifyTelegramAdd,
    NotifyTelegramRemove,
    NotifyTelegramField(settings::TelegramField, String),
    NotifyTelegramEnabled(bool),
    DigestChanged(settings::DigestField, String),
    SelectLabelFilter(String),
    ChooseLabel(Option<String>),
    LabelsAssigned(Result<usize, String>),
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
    AddTorrent,
    Remove,
    SetLabel,
}

impl Modal {
    fn title(self) -> &'static str {
        match self {
            Self::Servers => "Servers",
            Self::Settings => "Settings",
            Self::AddTorrent => "Add torrent",
            Self::Remove => "Remove torrents",
            Self::SetLabel => "Set label",
        }
    }

    fn size(self) -> (f32, f32) {
        match self {
            Self::Servers => (880.0, 620.0),
            Self::Settings => (settings::MODAL_WIDTH, settings::MODAL_HEIGHT),
            Self::AddTorrent => (620.0, 470.0),
            Self::Remove => (480.0, 260.0),
            Self::SetLabel => (420.0, 420.0),
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
    label_filter: String,
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
    columns: table::ColumnLayout,
    column_menu: Option<Point>,
    /// (column, grab x, width when the drag started)
    column_resize: Option<(table::Column, f32, f32)>,
    pending_select: Option<usize>,

    add_path: String,
    add_save_path: String,
    add_name: String,
    add_label: String,
    disks: Vec<DiskRow>,
    add_in_flight: bool,
    action_in_flight: bool,
    sidebar_collapsed: bool,
    search_visible: bool,
    remove_delete_files: bool,

    modal: Option<Modal>,
    modal_offset: (f32, f32),
    modal_drag: Option<(Point, (f32, f32))>,
    cursor: Point,
    settings: Settings,
    settings_request: u64,
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
    activity: ActivityLog,

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
            label_filter: String::new(),
            sort: SortField::Queue,
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
            columns: table::ColumnLayout::new(),
            column_menu: None,
            column_resize: None,
            pending_select: None,
            add_path: String::new(),
            add_save_path: String::new(),
            add_name: String::new(),
            add_label: String::new(),
            disks: Vec::new(),
            add_in_flight: false,
            action_in_flight: false,
            sidebar_collapsed: false,
            search_visible: false,
            remove_delete_files: false,
            modal: None,
            modal_offset: (0.0, 0.0),
            modal_drag: None,
            cursor: Point::ORIGIN,
            settings: Settings::default(),
            settings_request: 0,
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
            activity: ActivityLog::new(),
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
                self.server_status = None;
                self.reset_page();
                self.state = ConnectionState::Offline;
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
                self.label_filter.clear();
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
            Message::Table(TableMessage::ToggleColumn(column)) => {
                self.columns.toggle(column);
                self.column_menu = None;
                Task::none()
            }
            Message::Table(TableMessage::ColumnMenu(origin)) => {
                self.column_menu = origin;
                self.context = None;
                Task::none()
            }
            Message::Table(TableMessage::ResizeStart(column, x)) => {
                self.column_resize = Some((column, x, self.columns.width_of(column)));
                Task::none()
            }
            Message::Table(TableMessage::Resize(column, x)) => {
                if let Some((dragging, start_x, start_width)) = self.column_resize {
                    if dragging == column {
                        self.columns.set_width(column, start_width + (x - start_x));
                    }
                }
                Task::none()
            }
            Message::Table(TableMessage::ResizeEnd) => {
                self.column_resize = None;
                Task::none()
            }
            Message::Table(TableMessage::RightPressed(point)) => self.table_right_pressed(point),
            Message::Refresh => self.refresh_page(),
            Message::Action(action) => self.start_action(action),
            Message::ActionFinished {
                undo,
                action,
                errors,
            } => self.action_finished(*undo, action, errors),
            Message::AddSavePathChanged(value) => {
                self.add_save_path = value;
                Task::none()
            }
            Message::AddNameChanged(value) => {
                self.add_name = value;
                Task::none()
            }
            Message::AddConfirmed => self.confirm_add_torrent(),
            Message::PickTorrent => self.pick_torrent(),
            Message::TorrentPicked(path) => self.torrent_picked(path),
            Message::OpenRemove => self.open_remove_modal(),
            Message::RemoveConfirmed => self.confirm_remove(),
            Message::ToggleRemoveFiles(value) => {
                self.remove_delete_files = value;
                Task::none()
            }
            Message::QueueMove(up) => self.queue_move(up),
            Message::QueueMoved(errors) => self.queue_moved(errors),
            Message::ToggleSearch => self.toggle_search(),
            Message::TorrentAdded(result) => self.torrent_added(result),
            Message::FileDropped(path) => self.file_dropped(path),
            Message::OpenServers => self.open_servers(),
            Message::OpenSettings => self.open_settings(),
            Message::OpenStorage => {
                self.settings.tab = Tab::Storage;
                Task::batch([self.open_settings(), self.load_disks()])
            }
            Message::SettingsTab(tab) => {
                self.settings.tab = tab;
                self.settings.message.clear();
                if tab == Tab::Notifications {
                    return self.load_notify_status();
                }
                if tab == Tab::Storage {
                    return self.load_disks();
                }
                Task::none()
            }
            Message::DisksLoaded(result) => {
                match result {
                    Ok(rows) => {
                        self.disks = rows.clone();
                        self.settings.storage_disks = rows;
                    }
                    Err(error) => {
                        if self.settings.error.is_none() {
                            self.settings.error = Some(format!("storage: {error}"));
                        }
                    }
                }
                Task::none()
            }
            Message::AddLabelChanged(label) => {
                self.add_label = label.clone();
                if let Some(staging) = self.staging_for(&label) {
                    self.add_save_path = staging;
                }
                Task::none()
            }
            Message::StorageEnabled(value) => {
                self.settings.storage.enabled = value;
                Task::none()
            }
            Message::StorageOnLow(notify) => {
                self.settings.storage.on_low_notify = notify;
                Task::none()
            }
            Message::StorageRequireSpace(value) => {
                self.settings.storage.require_space = value;
                Task::none()
            }
            Message::StorageNumber(field, value) => {
                self.settings.storage.set_text(field, value);
                Task::none()
            }
            Message::ZonePath(index, value) => {
                if let Some(zone) = self.settings.storage.zones.get_mut(index) {
                    zone.path = value;
                }
                Task::none()
            }
            Message::ZoneLabel(index, value) => {
                if let Some(zone) = self.settings.storage.zones.get_mut(index) {
                    zone.label = value;
                }
                Task::none()
            }
            Message::ZoneStaging(index, value) => {
                if let Some(zone) = self.settings.storage.zones.get_mut(index) {
                    zone.staging = value;
                }
                Task::none()
            }
            Message::ZoneEnabled(index, value) => {
                if let Some(zone) = self.settings.storage.zones.get_mut(index) {
                    zone.enabled = value;
                }
                Task::none()
            }
            Message::ZoneAdd => {
                self.settings.zone_selected = Some(self.settings.storage.add_zone());
                Task::none()
            }
            Message::ZoneRemove(index) => {
                self.settings.storage.remove_zone(index);
                self.settings.zone_selected = None;
                Task::none()
            }
            Message::SettingsField(field, value) => {
                if !self.settings.locked(field) {
                    self.settings.form.set_text(field, value);
                }
                Task::none()
            }
            Message::SettingsSeedMode(value) => {
                self.settings.form.seed_mode = value;
                Task::none()
            }
            Message::SettingsQueueEnabled(value) => {
                self.settings.form.queue_enabled = value;
                Task::none()
            }
            Message::SettingsReload => Task::batch([self.load_settings(), self.load_disks()]),
            Message::SettingsSave => self.save_settings(),
            Message::SettingsLoaded {
                request,
                config,
                labels,
                auto_add,
                hooks,
            } => self.settings_loaded(request, config, labels, auto_add, hooks),
            Message::SettingsSaved {
                request,
                toml,
                result,
            } => self.settings_saved(request, toml, result),
            Message::LabelSelected(name) => {
                self.settings.select_label(&name);
                Task::none()
            }
            Message::LabelNew => {
                self.settings.new_label();
                Task::none()
            }
            Message::LabelField(field, value) => {
                self.settings.label_form.set(field, value);
                Task::none()
            }
            Message::LabelSave => self.save_label(),
            Message::LabelSaved { count, result } => self.label_saved(count, result),
            Message::LabelDeleteRequested => {
                self.settings.confirming_delete = true;
                Task::none()
            }
            Message::LabelDeleteCancelled => {
                self.settings.confirming_delete = false;
                Task::none()
            }
            Message::LabelDeleteConfirmed => self.delete_label(),
            Message::LabelDeleted { name, result } => self.label_deleted(name, result),
            Message::LabelsLoaded(result) => self.labels_loaded(result),
            Message::AutoAddSelected(index) => {
                if index < self.settings.autoadd.rows.len() {
                    self.settings.autoadd_selected = Some(index);
                }
                Task::none()
            }
            Message::AutoAddAdd => {
                self.settings.add_watch();
                Task::none()
            }
            Message::AutoAddRemove => {
                self.settings.remove_watch();
                Task::none()
            }
            Message::AutoAddField(field, value) => {
                if let Some(row) = self.settings.selected_watch_mut() {
                    row.set(field, value);
                }
                Task::none()
            }
            Message::AutoAddEnabled(value) => {
                if let Some(row) = self.settings.selected_watch_mut() {
                    row.enabled = value;
                }
                Task::none()
            }
            Message::AutoAddPaused(value) => {
                if let Some(row) = self.settings.selected_watch_mut() {
                    row.add_paused = value;
                }
                Task::none()
            }
            Message::AutoAddAction(action) => {
                if let Some(row) = self.settings.selected_watch_mut() {
                    row.on_success = action;
                }
                Task::none()
            }
            Message::HooksSelected(index) => {
                if index < self.settings.hooks.rows.len() {
                    self.settings.hooks_selected = Some(index);
                }
                Task::none()
            }
            Message::HooksAdd => {
                self.settings.add_hook();
                Task::none()
            }
            Message::HooksRemove => {
                self.settings.remove_hook();
                Task::none()
            }
            Message::HooksField(field, value) => {
                if let Some(row) = self.settings.selected_hook_mut() {
                    row.set(field, value);
                }
                Task::none()
            }
            Message::HooksEvent(event) => {
                if let Some(row) = self.settings.selected_hook_mut() {
                    row.event = event;
                }
                Task::none()
            }
            Message::HooksShell(value) => {
                if let Some(row) = self.settings.selected_hook_mut() {
                    row.shell = value;
                }
                Task::none()
            }
            Message::HooksEnabled(value) => {
                if let Some(row) = self.settings.selected_hook_mut() {
                    row.enabled = value;
                }
                Task::none()
            }
            Message::SchedulerEnabled(value) => {
                self.settings.scheduler.enabled = value;
                Task::none()
            }
            Message::SchedulerPauseSeeding(value) => {
                self.settings.scheduler.pause_seeding = value;
                Task::none()
            }
            Message::SchedulerField(field, value) => {
                match field {
                    SchedulerField::Timezone => self.settings.scheduler.timezone = value,
                    SchedulerField::DownloadLimit => {
                        self.settings.scheduler.download_rate_limit = value;
                    }
                    SchedulerField::UploadLimit => {
                        self.settings.scheduler.upload_rate_limit = value;
                    }
                }
                Task::none()
            }
            Message::SchedulerCell(day, hour) => {
                self.settings.scheduler.cycle(day, hour);
                Task::none()
            }
            Message::NotifySelected(index) => {
                if index < self.settings.notify.webhooks.len() {
                    self.settings.notify_selected = Some(index);
                    self.settings.notify_telegram = None;
                }
                Task::none()
            }
            Message::NotifyTelegramSelected(index) => {
                if index < self.settings.notify.telegram.len() {
                    self.settings.notify_telegram = Some(index);
                    self.settings.notify_selected = None;
                }
                Task::none()
            }
            Message::NotifyTelegramAdd => {
                self.settings.notify_telegram = Some(self.settings.notify.add_telegram());
                self.settings.notify_selected = None;
                Task::none()
            }
            Message::NotifyTelegramRemove => {
                if let Some(index) = self.settings.notify_telegram {
                    self.settings.notify.remove_telegram(index);
                    self.settings.notify_telegram = if self.settings.notify.telegram.is_empty() {
                        None
                    } else {
                        Some(index.min(self.settings.notify.telegram.len() - 1))
                    };
                }
                Task::none()
            }
            Message::NotifyTelegramField(field, value) => {
                if let Some(index) = self.settings.notify_telegram {
                    if let Some(row) = self.settings.notify.telegram.get_mut(index) {
                        row.set(field, value);
                    }
                }
                Task::none()
            }
            Message::NotifyTelegramEnabled(value) => {
                if let Some(index) = self.settings.notify_telegram {
                    if let Some(row) = self.settings.notify.telegram.get_mut(index) {
                        row.enabled = value;
                    }
                }
                Task::none()
            }
            Message::DigestChanged(field, value) => {
                self.settings.notify.set_digest(field, value);
                Task::none()
            }
            Message::NotifyAdd => {
                self.settings.notify_selected = Some(self.settings.notify.add());
                Task::none()
            }
            Message::NotifyRemove => {
                if let Some(index) = self.settings.notify_selected {
                    self.settings.notify.remove(index);
                    self.settings.notify_selected = if self.settings.notify.webhooks.is_empty() {
                        None
                    } else {
                        Some(index.min(self.settings.notify.webhooks.len() - 1))
                    };
                }
                Task::none()
            }
            Message::NotifyField(field, value) => {
                if let Some(index) = self.settings.notify_selected {
                    if let Some(row) = self.settings.notify.webhooks.get_mut(index) {
                        row.set(field, value);
                    }
                }
                Task::none()
            }
            Message::NotifyEnabled(value) => {
                self.settings.notify.enabled = value;
                Task::none()
            }
            Message::NotifyWebhookEnabled(value) => {
                if let Some(index) = self.settings.notify_selected {
                    if let Some(row) = self.settings.notify.webhooks.get_mut(index) {
                        row.enabled = value;
                    }
                }
                Task::none()
            }
            Message::NotifyTest => self.send_notification_test(),
            Message::NotifyTested(result) => {
                match result {
                    Ok(queued) => {
                        self.settings.message = format!("Queued {queued} test delivery(ies)")
                    }
                    Err(error) => self.settings.error = Some(format!("Notification test: {error}")),
                }
                Task::batch([self.load_notify_status()])
            }
            Message::NotifyStatusLoaded(result) => {
                match result {
                    Ok(status) => {
                        self.settings.notify_status =
                            Some((status.pending, status.dead, status.channels));
                    }
                    Err(error) => self.settings.error = Some(format!("Notifications: {error}")),
                }
                Task::none()
            }
            Message::HooksRefresh => self.load_hook_status(),
            Message::HooksStatusLoaded(result) => {
                match result {
                    Ok(status) => {
                        self.settings.hook_pending = status.pending;
                        self.settings.hook_runs = status.runs;
                    }
                    Err(error) => self.settings.error = Some(format!("Hooks: {error}")),
                }
                Task::none()
            }
            Message::AutoAddScanNow => self.run_auto_add_scan(),
            Message::AutoAddStatusLoaded(result) => {
                match result {
                    Ok(rows) => self.settings.autoadd_status = rows,
                    Err(error) => self.settings.error = Some(format!("AutoAdd: {error}")),
                }
                Task::none()
            }
            Message::AutoAddScanned(result) => match result {
                Ok(totals) => {
                    self.settings.loading = false;
                    self.settings.message = format!(
                        "Scan: {} added, {} duplicate, {} failed, {} invalid, {} deferred",
                        totals.added,
                        totals.duplicates,
                        totals.failed,
                        totals.invalid,
                        totals.deferred
                    );
                    Task::batch([
                        self.load_auto_add_status(),
                        self.refresh_page(),
                        self.load_labels(),
                    ])
                }
                Err(error) => {
                    self.settings.loading = false;
                    self.settings.error = Some(error);
                    Task::none()
                }
            },
            Message::SelectLabelFilter(name) => {
                self.label_filter = name;
                self.state_filter = None;
                self.reset_page();
                self.request_page(0)
            }
            Message::ChooseLabel(choice) => self.assign_label(choice),
            Message::LabelsAssigned(result) => self.labels_assigned(result),
            Message::CloseModal => {
                self.modal = None;
                self.modal_drag = None;
                self.settings.confirming_delete = false;
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
                // The Servers modal opened from the connect screen must give
                // way to the torrent list once the connection is up.
                if matches!(self.modal, Some(Modal::Servers)) {
                    self.modal = None;
                }
                self.reset_page();
                self.status = "Connected; loading torrents...".to_owned();
                self.measure_started = self.measure.then(Instant::now);
                self.measure_finished = false;
                self.frames = 0;
                Task::batch([
                    self.request_page(0),
                    self.load_labels(),
                    self.schedule_poll(),
                    self.load_disks(),
                ])
            }
            Err(error) => {
                self.client = None;
                self.server_status = None;
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
                    self.state = ConnectionState::Offline;
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
                self.activity.observe(&page.torrents, SystemTime::now());
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
        self.server_status = None;
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
            label: self.label_filter.clone(),
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
                let action = match action {
                    MenuAction::Pause => TorrentAction::Pause,
                    MenuAction::Resume => TorrentAction::Resume,
                    MenuAction::Label => return self.open_set_label(),
                    MenuAction::Remove => return self.open_remove_modal(),
                };
                return self.start_action(action);
            }
        }
        if point.y < table::HEADER_HEIGHT {
            if let Some(column) = self
                .columns
                .column_at(point.x, self.window_width - 232.0 - 14.0)
            {
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
            TorrentAction::Remove { .. } => {
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
            let at = SystemTime::now();
            match action {
                TorrentAction::Pause => {
                    self.activity.client_states(
                        &undo.selected_torrents,
                        TorrentState::Paused as i32,
                        at,
                    );
                }
                TorrentAction::Resume => {
                    self.activity.client_states(
                        &undo.selected_torrents,
                        TorrentState::Downloading as i32,
                        at,
                    );
                }
                TorrentAction::Remove { .. } => {
                    self.activity.client_removed(&undo.selected_torrents, at);
                }
            }
            if matches!(action, TorrentAction::Remove { .. }) {
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
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        self.add_in_flight = true;
        self.status = format!("Adding {path}...");
        let path = PathBuf::from(path);
        let name = self.add_name.trim().to_owned();
        let label = self.add_label.trim().to_owned();
        Task::perform(
            async move { actions::add(&client, &path, &save_path, &name, &label).await },
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
        self.prepare_add(path);
        Task::none()
    }

    /// Prefills the add form and opens the confirmation modal.
    /// The staging folder of the label's filesystem, from the disk payload.
    fn staging_for(&self, label: &str) -> Option<String> {
        if label.trim().is_empty() {
            return None;
        }
        for disk in &self.disks {
            let library = disk.zones.iter().any(|zone| {
                !zone.staging && zone.enabled && zone.label.eq_ignore_ascii_case(label.trim())
            });
            if !library {
                continue;
            }
            if let Some(staging) = disk.zones.iter().find(|zone| zone.staging && zone.enabled) {
                return Some(staging.path.clone());
            }
        }
        None
    }

    fn prepare_add(&mut self, path: PathBuf) {
        self.add_name = path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
        self.add_path = path.display().to_string();
        self.open_modal(Modal::AddTorrent);
    }

    fn pick_torrent(&mut self) -> Task<Message> {
        if self.client.is_none() {
            return Task::none();
        }
        Task::perform(
            async {
                rfd::AsyncFileDialog::new()
                    .add_filter("BitTorrent", &["torrent"])
                    .pick_file()
                    .await
                    .map(|handle| handle.path().to_path_buf())
            },
            Message::TorrentPicked,
        )
    }

    fn torrent_picked(&mut self, path: Option<PathBuf>) -> Task<Message> {
        if let Some(path) = path {
            self.prepare_add(path);
        }
        Task::none()
    }

    fn confirm_add_torrent(&mut self) -> Task<Message> {
        self.modal = None;
        self.add_current_torrent()
    }

    fn open_remove_modal(&mut self) -> Task<Message> {
        if self.selected.is_empty() {
            self.status = "Select torrents to remove".to_owned();
            return Task::none();
        }
        self.remove_delete_files = false;
        self.open_modal(Modal::Remove);
        Task::none()
    }

    fn open_set_label(&mut self) -> Task<Message> {
        if self.selected.is_empty() {
            self.status = "Select a torrent first".to_owned();
            return Task::none();
        }
        self.open_modal(Modal::SetLabel);
        Task::none()
    }

    fn confirm_remove(&mut self) -> Task<Message> {
        self.modal = None;
        self.start_action(TorrentAction::Remove {
            delete_files: self.remove_delete_files,
        })
    }

    /// Moves the selected unfinished torrents as a block; completed torrents
    /// keep their place because the queue has no meaning for them.
    fn queue_move(&mut self, up: bool) -> Task<Message> {
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        let mut ids: Vec<(i32, u64)> = self
            .selected_torrents
            .values()
            .filter(|torrent| torrent.state != TorrentState::Seeding as i32)
            .map(|torrent| (torrent.queue_position, torrent.id))
            .collect();
        if ids.is_empty() {
            self.status = "Only unfinished torrents can be reordered".to_owned();
            return Task::none();
        }
        ids.sort_by_key(|(position, id)| (if *position < 0 { i32::MAX } else { *position }, *id));
        if !up {
            ids.reverse();
        }
        self.sort = SortField::Queue;
        self.descending = false;
        Task::perform(
            async move {
                let mut errors = Vec::new();
                for (_, id) in ids {
                    if let Err(error) = actions::move_in_queue(&client, id, up).await {
                        errors.push(error);
                    }
                }
                errors
            },
            Message::QueueMoved,
        )
    }

    fn queue_moved(&mut self, errors: Vec<String>) -> Task<Message> {
        if errors.is_empty() {
            self.status = "Queue order updated".to_owned();
        } else {
            self.status = format!("Queue move failed: {}", errors.join("; "));
        }
        self.reset_page();
        self.request_page(0)
    }

    fn toggle_search(&mut self) -> Task<Message> {
        self.search_visible = !self.search_visible;
        if self.search_visible {
            return text_input::focus(self.filter_id.clone());
        }
        if self.filter.is_empty() {
            return Task::none();
        }
        self.filter.clear();
        self.reset_page();
        self.request_page(0)
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
                self.settings.confirming_delete = false;
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
                Key::Character("o") => return self.pick_torrent(),
                _ => {}
            }
        }
        match key.as_ref() {
            Key::Named(Named::Space) => self.toggle_pause(),
            Key::Named(Named::Delete) => self.open_remove_modal(),
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
            Modal::AddTorrent => (self.add_torrent_view(), modal.size()),
            Modal::Remove => (self.remove_view(), modal.size()),
            Modal::SetLabel => (self.set_label_view(), modal.size()),
        };
        let overlay = self.modal_overlay(panel, size);
        iced::widget::stack![base, overlay].into()
    }

    fn open_modal(&mut self, modal: Modal) {
        self.modal = Some(modal);
        self.modal_offset = (0.0, 0.0);
        self.modal_drag = None;
    }

    fn open_settings(&mut self) -> Task<Message> {
        self.open_modal(Modal::Settings);
        self.load_settings()
    }

    /// Fetches the configuration document and the label list.
    fn load_settings(&mut self) -> Task<Message> {
        self.settings.loading = true;
        self.settings.error = None;
        self.settings.message.clear();
        self.settings_request = self.settings_request.wrapping_add(1);
        let request = self.settings_request;
        let Some(client) = self.client.clone() else {
            self.settings.loading = false;
            self.settings.error = Some("Not connected".to_owned());
            return Task::none();
        };
        Task::perform(
            async move {
                let config = connection::config(&client).await;
                let labels = connection::labels(&client)
                    .await
                    .map(|entries| entries.into_iter().map(settings::LabelRow::from).collect());
                let auto_add = connection::auto_add_status(&client).await;
                let hooks = connection::hook_status(&client, 30).await;
                (request, config, labels, auto_add, hooks)
            },
            |(request, config, labels, auto_add, hooks)| Message::SettingsLoaded {
                request,
                config,
                labels,
                auto_add,
                hooks,
            },
        )
    }

    fn settings_loaded(
        &mut self,
        request: u64,
        config: Result<connection::ConfigSnapshot, String>,
        labels: Result<Vec<settings::LabelRow>, String>,
        auto_add: Result<Vec<settings::AutoAddStatusRow>, String>,
        hooks: Result<connection::HookStatus, String>,
    ) -> Task<Message> {
        if request != self.settings_request {
            return Task::none();
        }
        self.settings.loading = false;
        match config {
            Ok(snapshot) => {
                match self.settings.apply_snapshot(
                    &snapshot.toml,
                    snapshot.revision,
                    snapshot.env_locked,
                ) {
                    Ok(()) => {
                        self.settings.message = format!("Loaded revision {}", snapshot.revision);
                    }
                    Err(error) => self.settings.error = Some(error),
                }
            }
            Err(error) => self.settings.error = Some(error),
        }
        match labels {
            Ok(rows) => self.settings.set_rows(rows),
            Err(error) => {
                if self.settings.error.is_none() {
                    self.settings.error = Some(format!("labels: {error}"));
                }
            }
        }
        match auto_add {
            Ok(rows) => self.settings.autoadd_status = rows,
            Err(error) => {
                if self.settings.error.is_none() {
                    self.settings.error = Some(format!("autoadd: {error}"));
                }
            }
        }
        match hooks {
            Ok(status) => {
                self.settings.hook_pending = status.pending;
                self.settings.hook_runs = status.runs;
            }
            Err(error) => {
                if self.settings.error.is_none() {
                    self.settings.error = Some(format!("hooks: {error}"));
                }
            }
        }
        Task::none()
    }

    fn load_disks(&mut self) -> Task<Message> {
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { connection::disks(&client).await },
            Message::DisksLoaded,
        )
    }

    fn load_notify_status(&mut self) -> Task<Message> {
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { connection::notification_status(&client).await },
            Message::NotifyStatusLoaded,
        )
    }

    fn send_notification_test(&mut self) -> Task<Message> {
        let Some(client) = self.client.clone() else {
            self.settings.error = Some("Not connected".to_owned());
            return Task::none();
        };
        self.settings.error = None;
        self.settings.message = "Queueing a test delivery...".to_owned();
        Task::perform(
            async move { connection::send_test_notification(&client).await },
            Message::NotifyTested,
        )
    }

    fn load_hook_status(&mut self) -> Task<Message> {
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { connection::hook_status(&client, 30).await },
            Message::HooksStatusLoaded,
        )
    }

    fn load_auto_add_status(&mut self) -> Task<Message> {
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        Task::perform(
            async move { connection::auto_add_status(&client).await },
            Message::AutoAddStatusLoaded,
        )
    }

    fn run_auto_add_scan(&mut self) -> Task<Message> {
        if self.settings.loading {
            return Task::none();
        }
        let Some(client) = self.client.clone() else {
            self.settings.error = Some("Not connected".to_owned());
            return Task::none();
        };
        self.settings.loading = true;
        self.settings.error = None;
        self.settings.message = "Scanning watch directories...".to_owned();
        Task::perform(
            async move { connection::scan_auto_add(&client, "").await },
            Message::AutoAddScanned,
        )
    }

    fn save_settings(&mut self) -> Task<Message> {
        if self.settings.loading {
            return Task::none();
        }
        let Some(client) = self.client.clone() else {
            self.settings.error = Some("Not connected".to_owned());
            return Task::none();
        };
        let toml = match self.settings.render() {
            Ok(toml) => toml,
            Err(error) => {
                self.settings.error = Some(error);
                return Task::none();
            }
        };
        self.settings.loading = true;
        self.settings.error = None;
        let revision = self.settings.revision;
        let request = self.settings_request;
        Task::perform(
            async move {
                let sent = toml.clone();
                (
                    request,
                    sent,
                    connection::set_config(&client, &toml, revision).await,
                )
            },
            |(request, toml, result)| Message::SettingsSaved {
                request,
                toml,
                result,
            },
        )
    }

    fn settings_saved(
        &mut self,
        request: u64,
        toml: String,
        result: Result<u64, String>,
    ) -> Task<Message> {
        if request != self.settings_request {
            return Task::none();
        }
        self.settings.loading = false;
        match result {
            Ok(revision) => {
                self.settings.revision = revision;
                self.settings.raw = toml;
                self.settings.baseline = self.settings.form.clone();
                self.settings.autoadd_baseline = self.settings.autoadd.clone();
                self.settings.hooks_baseline = self.settings.hooks.clone();
                self.settings.scheduler_baseline = self.settings.scheduler.clone();
                self.settings.notify_baseline = self.settings.notify.clone();
                self.settings.error = None;
                self.settings.message = format!("Saved revision {revision}");
                Task::batch([self.refresh_page(), self.load_labels()])
            }
            Err(error) => {
                self.settings.error = Some(error);
                self.settings.message = "Save failed; Reload re-reads the live document".to_owned();
                Task::none()
            }
        }
    }

    fn load_labels(&mut self) -> Task<Message> {
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        Task::perform(
            async move {
                connection::labels(&client).await.map(|entries| {
                    entries
                        .into_iter()
                        .map(settings::LabelRow::from)
                        .collect::<Vec<_>>()
                })
            },
            Message::LabelsLoaded,
        )
    }

    fn labels_loaded(&mut self, result: Result<Vec<settings::LabelRow>, String>) -> Task<Message> {
        match result {
            Ok(rows) => self.settings.set_rows(rows),
            Err(error) => {
                self.status = format!("Cannot load labels: {error}");
            }
        }
        Task::none()
    }

    fn save_label(&mut self) -> Task<Message> {
        if self.settings.loading {
            return Task::none();
        }
        let label = match self.settings.label_form.to_label() {
            Ok(label) => label,
            Err(error) => {
                self.settings.error = Some(error);
                return Task::none();
            }
        };
        let create = self.settings.label_new || self.settings.label_selected.is_none();
        if create
            && self
                .settings
                .labels
                .iter()
                .any(|row| row.label.name == label.name)
        {
            self.settings.error = Some(format!("A label named {:?} already exists", label.name));
            return Task::none();
        }
        let Some(client) = self.client.clone() else {
            self.settings.error = Some("Not connected".to_owned());
            return Task::none();
        };
        let count = self
            .settings
            .current_label()
            .map_or(0, |row| row.torrent_count);
        self.settings.loading = true;
        self.settings.error = None;
        Task::perform(
            async move { connection::save_label(&client, label, create).await },
            move |result| Message::LabelSaved { count, result },
        )
    }

    fn label_saved(&mut self, count: u32, result: Result<ProtoLabel, String>) -> Task<Message> {
        self.settings.loading = false;
        match result {
            Ok(label) => {
                self.settings.message = format!("Saved label {}", label.name);
                self.settings.upsert_row(label, count);
                Task::batch([self.load_labels(), self.refresh_page()])
            }
            Err(error) => {
                self.settings.error = Some(error);
                Task::none()
            }
        }
    }

    fn delete_label(&mut self) -> Task<Message> {
        if self.settings.loading {
            return Task::none();
        }
        let Some(name) = self.settings.label_selected.clone() else {
            return Task::none();
        };
        let Some(client) = self.client.clone() else {
            self.settings.error = Some("Not connected".to_owned());
            return Task::none();
        };
        self.settings.loading = true;
        self.settings.error = None;
        let sent = name.clone();
        Task::perform(
            async move { connection::delete_label(&client, &name).await },
            move |result| Message::LabelDeleted {
                name: sent.clone(),
                result,
            },
        )
    }

    fn label_deleted(&mut self, name: String, result: Result<u32, String>) -> Task<Message> {
        self.settings.loading = false;
        match result {
            Ok(cleared) => {
                self.settings.remove_row(&name);
                if self.label_filter == name {
                    self.label_filter.clear();
                }
                self.settings.message =
                    format!("Deleted label {name}; cleared {cleared} assignment(s)");
                Task::batch([self.load_labels(), self.refresh_page()])
            }
            Err(error) => {
                self.settings.error = Some(error);
                Task::none()
            }
        }
    }

    fn assign_label(&mut self, choice: Option<String>) -> Task<Message> {
        self.modal = None;
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        let ids: Vec<u64> = self.selected.iter().copied().collect();
        if ids.is_empty() {
            return Task::none();
        }
        let label = choice.unwrap_or_default();
        self.status = format!("Assigning label to {} torrent(s)...", ids.len());
        Task::perform(
            async move {
                let mut applied = 0;
                let mut errors = Vec::new();
                for id in ids {
                    match connection::assign_label(&client, id, &label).await {
                        Ok(_) => applied += 1,
                        Err(error) => errors.push(error),
                    }
                }
                if errors.is_empty() {
                    Ok(applied)
                } else {
                    Err(errors.join("; "))
                }
            },
            Message::LabelsAssigned,
        )
    }

    fn labels_assigned(&mut self, result: Result<usize, String>) -> Task<Message> {
        match result {
            Ok(count) => self.status = format!("Label applied to {count} torrent(s)"),
            Err(error) => self.status = format!("Label update failed: {error}"),
        }
        Task::batch([self.refresh_page(), self.load_labels()])
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
        let settings = &self.settings;
        let rail = container(
            column![
                text("SETTINGS")
                    .size(10)
                    .color(theme::TEXT_MUTED)
                    .font(theme::semibold()),
                WidgetColumn::with_children(
                    Tab::ALL
                        .into_iter()
                        .map(|tab| settings_tab(tab, settings.tab == tab))
                        .collect::<Vec<Element<Message>>>(),
                )
                .spacing(2),
                vertical_space(),
                if settings.loading {
                    Element::from(text("loading…").size(11).color(theme::TEXT_MUTED))
                } else {
                    Element::from(text("").size(1))
                },
            ]
            .spacing(8),
        )
        .width(Length::Fixed(176.0))
        .height(Length::Fill);

        let body: Element<Message> = match settings.tab {
            Tab::Connection => self.settings_connection_view(),
            Tab::Session => self.settings_session_view(),
            Tab::AutoAdd => self.settings_auto_add_view(),
            Tab::Hooks => self.settings_hooks_view(),
            Tab::Scheduler => self.settings_scheduler_view(),
            Tab::Notifications => self.settings_notifications_view(),
            Tab::Storage => self.settings_storage_view(),
            Tab::Labels => self.settings_labels_view(),
        };
        let dirty = settings.dirty_tab(settings.tab);

        let mut summary = text("")
            .size(11)
            .color(theme::TEXT_MUTED)
            .width(Length::Fill);
        if let Some(error) = &settings.error {
            summary = text(error.clone())
                .size(11)
                .color(theme::ERROR)
                .width(Length::Fill);
        } else if !settings.message.is_empty() {
            summary = text(settings.message.clone())
                .size(11)
                .color(theme::TEXT_SECONDARY)
                .width(Length::Fill);
        } else if dirty {
            summary = text("Unsaved changes; Reload discards them")
                .size(11)
                .color(theme::QUEUED)
                .width(Length::Fill);
        }

        let save = match settings.tab {
            Tab::Labels => Message::LabelSave,
            Tab::Connection
            | Tab::Session
            | Tab::AutoAdd
            | Tab::Hooks
            | Tab::Scheduler
            | Tab::Notifications
            | Tab::Storage => Message::SettingsSave,
        };
        let can_save = !settings.loading
            && match settings.tab {
                Tab::Labels => settings.label_new || settings.label_selected.is_some(),
                Tab::Connection
                | Tab::Session
                | Tab::AutoAdd
                | Tab::Hooks
                | Tab::Scheduler
                | Tab::Notifications
                | Tab::Storage => true,
            };
        let footer = row![
            summary,
            action_button("Reload", Message::SettingsReload, !settings.loading),
            action_button("Save", save, can_save),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);

        let content = column![
            text(settings.tab.label()).size(16).font(theme::semibold()),
            text(settings.tab.hint()).size(11).color(theme::TEXT_MUTED),
            container(body).height(Length::Fill).padding([6, 0]),
            footer,
        ]
        .spacing(6)
        .width(Length::Fill)
        .height(Length::Fill);

        let body = row![
            rail,
            container(content)
                .padding([0, 14])
                .width(Length::Fill)
                .height(Length::Fill),
        ]
        .height(Length::Fill);

        modal_panel(Modal::Settings, body.into())
    }

    fn settings_connection_view(&self) -> Element<'_, Message> {
        let form = &self.settings.form;
        let field = |field| {
            settings_field(
                &self.settings,
                form,
                field,
                if field == ConfigField::Bind {
                    "127.0.0.1:50051 (default)"
                } else {
                    "daemon default"
                },
            )
        };
        column![
            field(ConfigField::DataDir),
            field(ConfigField::DownloadDir),
            field(ConfigField::Bind),
            field(ConfigField::TokenFile),
            detail_row(
                "Profile",
                self.config.active.clone().unwrap_or_else(|| "—".to_owned()),
                theme::TEXT_MUTED
            ),
            detail_row(
                "Client config",
                self.config_path.display().to_string(),
                theme::TEXT_MUTED
            ),
        ]
        .spacing(10)
        .max_width(520.0)
        .into()
    }

    fn settings_session_view(&self) -> Element<'_, Message> {
        let form = &self.settings.form;
        let field = |field| settings_field(&self.settings, form, field, "0 = unlimited");
        column![
            field(ConfigField::DownloadRateLimit),
            field(ConfigField::UploadRateLimit),
            row![
                field(ConfigField::ActiveDownloads),
                field(ConfigField::ActiveSeeds),
                field(ConfigField::ActiveLimit),
            ]
            .spacing(10),
            checkbox(
                "Treat newly added torrents as complete (seed mode)",
                form.seed_mode
            )
            .on_toggle(Message::SettingsSeedMode)
            .size(14)
            .text_size(12),
            checkbox(
                "Apply the configured active limits (queue)",
                form.queue_enabled
            )
            .on_toggle(Message::SettingsQueueEnabled)
            .size(14)
            .text_size(12),
        ]
        .spacing(10)
        .max_width(560.0)
        .into()
    }

    fn settings_auto_add_view(&self) -> Element<'_, Message> {
        let settings = &self.settings;
        let rows: Vec<Element<Message>> = settings
            .autoadd
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let selected = settings.autoadd_selected == Some(index);
                let status = settings
                    .autoadd_status
                    .iter()
                    .find(|status| status.path == row.path.trim());
                let detail: Element<Message> = match status {
                    Some(status) if !status.last_error.is_empty() => {
                        text(status.last_error.clone())
                            .size(10)
                            .color(theme::ERROR)
                            .into()
                    }
                    Some(status) => text(status.summary())
                        .size(10)
                        .color(theme::TEXT_MUTED)
                        .into(),
                    None if row.enabled => text("not scanned yet")
                        .size(10)
                        .color(theme::TEXT_MUTED)
                        .into(),
                    None => text("disabled").size(10).color(theme::TEXT_MUTED).into(),
                };
                let path = if row.path.trim().is_empty() {
                    "(no path)".to_owned()
                } else {
                    row.path.trim().to_owned()
                };
                let label = if row.label.trim().is_empty() {
                    String::new()
                } else {
                    format!("#{}", row.label.trim())
                };
                button(
                    column![
                        row![
                            text(glyph::TAG).font(icons::solid()).size(11.0).color(
                                if row.enabled {
                                    theme::PRIMARY
                                } else {
                                    theme::TEXT_MUTED
                                }
                            ),
                            text(path).size(12),
                            horizontal_space(),
                            text(label).size(10).color(theme::TEXT_MUTED),
                        ]
                        .spacing(8)
                        .align_y(iced::Alignment::Center),
                        detail,
                    ]
                    .spacing(2),
                )
                .on_press(Message::AutoAddSelected(index))
                .width(Length::Fill)
                .padding([5, 8])
                .style(move |_theme: &iced::Theme, _status| button::Style {
                    background: Some(
                        if selected {
                            theme::SURFACE
                        } else {
                            theme::BG_SECONDARY
                        }
                        .into(),
                    ),
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
                        radius: 4.0.into(),
                    },
                    ..button::Style::default()
                })
                .into()
            })
            .collect();

        let list = column![
            row![
                text("Watch directories").size(12).font(theme::semibold()),
                horizontal_space(),
                action_button("Add", Message::AutoAddAdd, !settings.loading),
                action_button(
                    "Remove",
                    Message::AutoAddRemove,
                    settings.autoadd_selected.is_some() && !settings.loading,
                ),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            container(
                scrollable(WidgetColumn::with_children(rows).spacing(2)).height(Length::Fill)
            )
            .height(Length::Fill),
        ]
        .spacing(6)
        .width(Length::Fixed(330.0))
        .height(Length::Fill);

        let editor: Element<Message> = if settings.selected_watch().is_some() {
            self.auto_add_editor_view()
        } else {
            column![
                text("Select a watch directory, or add one.")
                    .size(12)
                    .color(theme::TEXT_MUTED),
            ]
            .into()
        };

        let added: u32 = settings
            .autoadd_status
            .iter()
            .map(|status| status.added)
            .sum();
        let pending: u32 = settings
            .autoadd_status
            .iter()
            .map(|status| status.pending)
            .sum();
        let invalid: u32 = settings
            .autoadd_status
            .iter()
            .map(|status| status.invalid)
            .sum();
        let deferred: u32 = settings
            .autoadd_status
            .iter()
            .map(|status| status.deferred)
            .sum();
        let totals = text(format!(
            "{added} added · {pending} pending · {invalid} invalid · {deferred} deferred"
        ))
        .size(11)
        .color(theme::TEXT_MUTED);

        row![
            list,
            container(
                column![
                    scrollable(editor).height(Length::Fill),
                    totals,
                    row![
                        horizontal_space(),
                        container(action_button(
                            "Scan now",
                            Message::AutoAddScanNow,
                            !settings.loading,
                        ))
                        .width(Length::Fixed(110.0)),
                    ],
                ]
                .spacing(4)
                .height(Length::Fill)
            )
            .padding([0, 12])
            .width(Length::Fill)
            .height(Length::Fill),
        ]
        .height(Length::Fill)
        .into()
    }

    fn auto_add_editor_view(&self) -> Element<'_, Message> {
        let settings = &self.settings;
        let Some(row) = settings.selected_watch() else {
            return text("").size(1).into();
        };
        let mut editor = column![
            watch_field_view(row, WatchField::Path, "/data/watch"),
            row![
                watch_field_view(row, WatchField::Label, "movies"),
                watch_field_view(row, WatchField::SavePath, "session default"),
            ]
            .spacing(8),
            row![
                checkbox("Enabled", row.enabled)
                    .on_toggle(Message::AutoAddEnabled)
                    .size(14)
                    .text_size(12),
                checkbox("Add paused", row.add_paused)
                    .on_toggle(Message::AutoAddPaused)
                    .size(14)
                    .text_size(12),
            ]
            .spacing(16),
            column![
                text("After adding").size(11).color(theme::TEXT_MUTED),
                row![
                    watch_action(SuccessAction::Delete, row.on_success),
                    watch_action(SuccessAction::Keep, row.on_success),
                    watch_action(SuccessAction::Move, row.on_success),
                ]
                .spacing(6),
            ]
            .spacing(3),
        ]
        .spacing(10);
        editor = editor.push(watch_field_view(row, WatchField::MinFree, "0 (disabled)"));
        if row.on_success == SuccessAction::Move {
            editor = editor.push(watch_field_view(
                row,
                WatchField::MoveTo,
                "/data/watch/done",
            ));
        }
        editor.max_width(560.0).into()
    }

    fn settings_hooks_view(&self) -> Element<'_, Message> {
        let settings = &self.settings;
        let rows: Vec<Element<Message>> = settings
            .hooks
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let selected = settings.hooks_selected == Some(index);
                let command = row
                    .command
                    .rsplit('/')
                    .next()
                    .filter(|name| !name.is_empty())
                    .unwrap_or("(no command)")
                    .to_owned();
                let detail = if row.enabled {
                    format!("{} · {}s", row.event.label(), row.timeout_secs)
                } else {
                    format!("{} · disabled", row.event.label())
                };
                button(
                    column![
                        row![
                            text(glyph::PLAY).font(icons::solid()).size(10.0).color(
                                if row.enabled {
                                    theme::PRIMARY
                                } else {
                                    theme::TEXT_MUTED
                                }
                            ),
                            text(command).size(12),
                        ]
                        .spacing(8)
                        .align_y(iced::Alignment::Center),
                        text(detail).size(10).color(theme::TEXT_MUTED),
                    ]
                    .spacing(2),
                )
                .on_press(Message::HooksSelected(index))
                .width(Length::Fill)
                .padding([5, 8])
                .style(move |_theme: &iced::Theme, _status| button::Style {
                    background: Some(
                        if selected {
                            theme::SURFACE
                        } else {
                            theme::BG_SECONDARY
                        }
                        .into(),
                    ),
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
                        radius: 4.0.into(),
                    },
                    ..button::Style::default()
                })
                .into()
            })
            .collect();

        let list = column![
            row![
                text("Hooks").size(12).font(theme::semibold()),
                horizontal_space(),
                action_button("Add", Message::HooksAdd, !settings.loading),
                action_button(
                    "Remove",
                    Message::HooksRemove,
                    settings.hooks_selected.is_some() && !settings.loading,
                ),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            container(
                scrollable(WidgetColumn::with_children(rows).spacing(2)).height(Length::Fill)
            )
            .height(Length::Fill),
        ]
        .spacing(6)
        .width(Length::Fixed(300.0))
        .height(Length::Fill);

        let editor: Element<Message> = if settings.selected_hook().is_some() {
            self.hook_editor_view()
        } else {
            column![
                text("Select a hook, or add one.")
                    .size(12)
                    .color(theme::TEXT_MUTED),
            ]
            .into()
        };

        let runs: Vec<Element<Message>> = settings
            .hook_runs
            .iter()
            .map(|run| {
                let color = if run.timed_out || !run.succeeded {
                    theme::ERROR
                } else {
                    theme::PRIMARY
                };
                let mut entry = column![text(run.summary()).size(11).color(color)].spacing(1);
                if let Some(detail) = run.detail() {
                    entry = entry.push(
                        text(detail.lines().next().unwrap_or_default().to_owned())
                            .size(10)
                            .color(theme::TEXT_MUTED),
                    );
                }
                entry.into()
            })
            .collect();
        let runs_section = column![
            row![
                text("Recent runs").size(12).font(theme::semibold()),
                horizontal_space(),
                text(format!("{} pending", settings.hook_pending))
                    .size(10)
                    .color(theme::TEXT_MUTED),
                container(action_button(
                    "Refresh",
                    Message::HooksRefresh,
                    !settings.loading,
                ))
                .width(Length::Fixed(96.0)),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            container(
                scrollable(WidgetColumn::with_children(runs).spacing(3)).height(Length::Fill)
            )
            .height(Length::Fixed(170.0)),
        ]
        .spacing(4);

        row![
            list,
            container(
                column![scrollable(editor).height(Length::Fill), runs_section,]
                    .spacing(8)
                    .height(Length::Fill)
            )
            .padding([0, 12])
            .width(Length::Fill)
            .height(Length::Fill),
        ]
        .height(Length::Fill)
        .into()
    }

    fn hook_editor_view(&self) -> Element<'_, Message> {
        let settings = &self.settings;
        let Some(row) = settings.selected_hook() else {
            return text("").size(1).into();
        };
        column![
            hook_field_view(row, HookField::Command, "/usr/local/bin/notify"),
            hook_field_view(row, HookField::Args, "{id} {name} {path} {infohash} {label}"),
            column![
                text("Event").size(11).color(theme::TEXT_MUTED),
                row![
                    hook_event_toggle(HookEvent::Added, row.event),
                    hook_event_toggle(HookEvent::Finished, row.event),
                    hook_event_toggle(HookEvent::Removed, row.event),
                    hook_event_toggle(HookEvent::Error, row.event),
                    hook_event_toggle(HookEvent::DiskLow, row.event),
                    hook_event_toggle(HookEvent::DiskOk, row.event),
                ]
                .spacing(6),
            ]
            .spacing(3),
            row![
                hook_field_view(row, HookField::Timeout, "60"),
                column![
                    text("Options").size(11).color(theme::TEXT_MUTED),
                    row![
                        checkbox("Run through shell", row.shell)
                            .on_toggle(Message::HooksShell)
                            .size(14)
                            .text_size(12),
                        checkbox("Enabled", row.enabled)
                            .on_toggle(Message::HooksEnabled)
                            .size(14)
                            .text_size(12),
                    ]
                    .spacing(12),
                ]
                .spacing(3)
                .width(Length::Fill),
            ]
            .spacing(8),
            text("Hooks are at-least-once: a crash replays the event, so make the command idempotent. The event is acknowledged after every matching hook finished.")
                .size(10)
                .color(theme::TEXT_MUTED),
        ]
        .spacing(10)
        .max_width(560.0)
        .into()
    }

    fn settings_scheduler_view(&self) -> Element<'_, Message> {
        let form = &self.settings.scheduler;
        let days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        let mut rows: Vec<Element<Message>> = Vec::with_capacity(7);
        for (day, label) in days.iter().enumerate() {
            let mut cells: Vec<Element<Message>> = Vec::with_capacity(24);
            for (hour, level) in form.grid[day].iter().enumerate() {
                cells.push(scheduler_cell(day, hour, *level));
            }
            rows.push(
                row![
                    text(*label)
                        .size(10)
                        .color(theme::TEXT_MUTED)
                        .width(Length::Fixed(32.0)),
                    WidgetRow::with_children(cells).spacing(1),
                ]
                .spacing(4)
                .align_y(iced::Alignment::Center)
                .into(),
            );
        }
        let header = row![
            text("").width(Length::Fixed(36.0)),
            text("00").size(9).color(theme::TEXT_MUTED),
            horizontal_space(),
            text("06").size(9).color(theme::TEXT_MUTED),
            horizontal_space(),
            text("12").size(9).color(theme::TEXT_MUTED),
            horizontal_space(),
            text("18").size(9).color(theme::TEXT_MUTED),
            horizontal_space(),
            text("23").size(9).color(theme::TEXT_MUTED),
        ];

        let field = |field: SchedulerField, placeholder: &'static str| {
            let value = match field {
                SchedulerField::Timezone => form.timezone.clone(),
                SchedulerField::DownloadLimit => form.download_rate_limit.clone(),
                SchedulerField::UploadLimit => form.upload_rate_limit.clone(),
            };
            column![
                text(match field {
                    SchedulerField::Timezone => "Timezone",
                    SchedulerField::DownloadLimit => "Yellow download limit (B/s)",
                    SchedulerField::UploadLimit => "Yellow upload limit (B/s)",
                })
                .size(11)
                .color(theme::TEXT_MUTED),
                text_input(placeholder, &value)
                    .style(muted_input_style)
                    .padding([4, 8])
                    .size(12)
                    .width(Length::Fill)
                    .on_input(move |text| Message::SchedulerField(field, text)),
            ]
            .spacing(3)
        };

        column![
            row![
                checkbox("Enabled", form.enabled)
                    .on_toggle(Message::SchedulerEnabled)
                    .size(14)
                    .text_size(12),
                checkbox("Red pauses seeding too", form.pause_seeding)
                    .on_toggle(Message::SchedulerPauseSeeding)
                    .size(14)
                    .text_size(12),
            ]
            .spacing(16),
            row![
                field(SchedulerField::Timezone, "Europe/Budapest"),
                field(SchedulerField::DownloadLimit, "0"),
                field(SchedulerField::UploadLimit, "0"),
            ]
            .spacing(8),
            text("Monday first. Click a cell to cycle green → yellow → red.")
                .size(10)
                .color(theme::TEXT_MUTED),
            header,
            WidgetColumn::with_children(rows).spacing(1),
            text(format!(
                "{} non-green hour(s) · green = session defaults, yellow = limits, red = scheduler pause",
                form.marked_cells()
            ))
            .size(10)
            .color(theme::TEXT_MUTED),
        ]
        .spacing(8)
        .into()
    }

    fn settings_notifications_view(&self) -> Element<'_, Message> {
        let settings = &self.settings;
        let notify = &settings.notify;

        let webhook_rows: Vec<Element<Message>> = notify
            .webhooks
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let selected = settings.notify_selected == Some(index);
                let host = row
                    .url
                    .split_once("://")
                    .map_or(row.url.as_str(), |(_, rest)| rest)
                    .split('/')
                    .next()
                    .unwrap_or("")
                    .to_owned();
                let display = if host.is_empty() {
                    "(no URL)".to_owned()
                } else {
                    host
                };
                let detail = if row.events.trim().is_empty() {
                    "all events".to_owned()
                } else {
                    row.events.clone()
                };
                notification_row(
                    display,
                    if row.enabled {
                        detail
                    } else {
                        format!("{detail} · disabled")
                    },
                    selected,
                    Message::NotifySelected(index),
                )
            })
            .collect();

        let telegram_rows: Vec<Element<Message>> = notify
            .telegram
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let selected = settings.notify_telegram == Some(index);
                let chat = if row.chat_id.trim().is_empty() {
                    "(no chat id)".to_owned()
                } else {
                    row.chat_id.trim().to_owned()
                };
                let detail = if row.events.trim().is_empty() {
                    "all events".to_owned()
                } else {
                    row.events.clone()
                };
                notification_row(
                    chat,
                    if row.enabled {
                        detail
                    } else {
                        format!("{detail} · disabled")
                    },
                    selected,
                    Message::NotifyTelegramSelected(index),
                )
            })
            .collect();

        let list = column![
            row![
                text("Webhooks").size(12).font(theme::semibold()),
                horizontal_space(),
                action_button("Add", Message::NotifyAdd, !settings.loading),
                action_button(
                    "Remove",
                    Message::NotifyRemove,
                    settings.notify_selected.is_some() && !settings.loading,
                ),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            container(
                scrollable(WidgetColumn::with_children(webhook_rows).spacing(2))
                    .height(Length::Fill)
            )
            .height(Length::FillPortion(1)),
            row![
                text("Telegram").size(12).font(theme::semibold()),
                horizontal_space(),
                action_button("Add", Message::NotifyTelegramAdd, !settings.loading),
                action_button(
                    "Remove",
                    Message::NotifyTelegramRemove,
                    settings.notify_telegram.is_some() && !settings.loading,
                ),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            container(
                scrollable(WidgetColumn::with_children(telegram_rows).spacing(2))
                    .height(Length::Fill)
            )
            .height(Length::FillPortion(1)),
        ]
        .spacing(6)
        .width(Length::Fixed(320.0))
        .height(Length::Fill);

        let webhook_editor: Element<Message> =
            if let Some(row) = notify.selected(settings.notify_selected) {
                let field = |field: WebhookField, placeholder: &'static str| {
                    column![
                        text(field.label()).size(11).color(theme::TEXT_MUTED),
                        text_input(placeholder, row.text(field))
                            .style(muted_input_style)
                            .padding([4, 8])
                            .size(12)
                            .width(Length::Fill)
                            .on_input(move |value| Message::NotifyField(field, value)),
                    ]
                    .spacing(3)
                };
                column![
                    field(WebhookField::Url, "https://example.test/ferrent"),
                    field(WebhookField::Secret, "optional HMAC secret"),
                    row![
                        field(WebhookField::Events, "finished, error (empty = all)"),
                        field(WebhookField::Timeout, "5000"),
                    ]
                    .spacing(8),
                    checkbox("Channel enabled", row.enabled)
                        .on_toggle(Message::NotifyWebhookEnabled)
                        .size(14)
                        .text_size(12),
                ]
                .spacing(8)
                .into()
            } else if let Some(row) = notify.selected_telegram(settings.notify_telegram) {
                let field = |field: settings::TelegramField, placeholder: &'static str| {
                    column![
                        text(field.label()).size(11).color(theme::TEXT_MUTED),
                        text_input(placeholder, row.text(field))
                            .style(muted_input_style)
                            .padding([4, 8])
                            .size(12)
                            .width(Length::Fill)
                            .on_input(move |value| Message::NotifyTelegramField(field, value)),
                    ]
                    .spacing(3)
                };
                column![
                    field(settings::TelegramField::Token, "123456:ABC-DEF"),
                    row![
                        field(settings::TelegramField::ChatId, "-1001234567890"),
                        field(settings::TelegramField::ThreadId, "optional topic id"),
                    ]
                    .spacing(8),
                    field(
                        settings::TelegramField::Events,
                        "finished, error (empty = all)"
                    ),
                    row![
                        field(settings::TelegramField::Label, "only this label"),
                        field(settings::TelegramField::Timeout, "5000"),
                    ]
                    .spacing(8),
                    field(settings::TelegramField::ApiBase, TELEGRAM_API_BASE),
                    checkbox("Channel enabled", row.enabled)
                        .on_toggle(Message::NotifyTelegramEnabled)
                        .size(14)
                        .text_size(12),
                ]
                .spacing(8)
                .into()
            } else {
                column![
                    text("Select a webhook or a Telegram chat, or add one.")
                        .size(12)
                        .color(theme::TEXT_MUTED)
                ]
                .into()
            };

        let digest = column![
            text("Digest and quiet hours (Telegram only)")
                .size(11)
                .color(theme::TEXT_MUTED),
            row![
                digest_field(DigestField::Events, "10", notify),
                digest_field(DigestField::Window, "30", notify),
            ]
            .spacing(8),
            row![
                digest_field(DigestField::QuietStart, "23:00", notify),
                digest_field(DigestField::QuietEnd, "07:00", notify),
                digest_field(DigestField::Timezone, "Europe/Budapest", notify),
            ]
            .spacing(8),
            digest_field(DigestField::LabelFilter, "only this label", notify),
        ]
        .spacing(6);

        let status = settings.notify_status.map_or_else(
            || "outbox: -".to_owned(),
            |(pending, dead, channels)| {
                format!("outbox: {pending} pending · {dead} dead · {channels} channel(s)")
            },
        );

        row![
            list,
            container(
                column![
                    checkbox("Notifications enabled", notify.enabled)
                        .on_toggle(Message::NotifyEnabled)
                        .size(14)
                        .text_size(12),
                    scrollable(
                        column![webhook_editor, digest,]
                            .spacing(10)
                            .max_width(560.0)
                    )
                    .height(Length::Fill),
                    row![
                        text(status)
                            .size(10)
                            .color(theme::TEXT_MUTED)
                            .width(Length::Fill),
                        container(action_button(
                            "Test",
                            Message::NotifyTest,
                            !settings.loading
                        ))
                        .width(Length::Fixed(80.0)),
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
                ]
                .spacing(8)
                .height(Length::Fill)
            )
            .padding([0, 12])
            .width(Length::Fill)
            .height(Length::Fill),
        ]
        .height(Length::Fill)
        .into()
    }

    fn settings_storage_view(&self) -> Element<'_, Message> {
        let settings = &self.settings;
        let storage = &settings.storage;

        let mut disks = column![row![
            text("MOUNT")
                .size(10)
                .color(theme::TEXT_MUTED)
                .width(Length::FillPortion(4)),
            text("FS")
                .size(10)
                .color(theme::TEXT_MUTED)
                .width(Length::FillPortion(2)),
            text("FREE / TOTAL")
                .size(10)
                .color(theme::TEXT_MUTED)
                .width(Length::FillPortion(3)),
            text("TORRENTS")
                .size(10)
                .color(theme::TEXT_MUTED)
                .width(Length::FillPortion(2)),
            text("STATUS")
                .size(10)
                .color(theme::TEXT_MUTED)
                .width(Length::FillPortion(2)),
            text("ZONES")
                .size(10)
                .color(theme::TEXT_MUTED)
                .width(Length::FillPortion(2)),
        ]]
        .spacing(4);
        if settings.storage_disks.is_empty() {
            disks = disks.push(
                text("Connect and open this tab to see the filesystems.")
                    .size(11)
                    .color(theme::TEXT_MUTED),
            );
        }
        for disk in &settings.storage_disks {
            disks = disks.push(
                row![
                    text(disk.mount.clone())
                        .size(11)
                        .width(Length::FillPortion(4)),
                    text(disk.fs.clone())
                        .size(11)
                        .color(theme::TEXT_MUTED)
                        .width(Length::FillPortion(2)),
                    text(format!(
                        "{} / {}",
                        format_size(disk.free),
                        format_size(disk.total)
                    ))
                    .size(11)
                    .width(Length::FillPortion(3)),
                    text(disk.torrents.to_string())
                        .size(11)
                        .width(Length::FillPortion(2)),
                    text(if disk.low { "LOW" } else { "ok" })
                        .size(11)
                        .color(if disk.low {
                            theme::ERROR
                        } else {
                            theme::TEXT_MUTED
                        })
                        .width(Length::FillPortion(2)),
                    text(disk.zones.len().to_string())
                        .size(11)
                        .width(Length::FillPortion(2)),
                ]
                .spacing(4),
            );
        }

        let flags = row![
            checkbox("Guard enabled", storage.enabled)
                .on_toggle(Message::StorageEnabled)
                .size(14)
                .text_size(12),
            checkbox("Allow low add", storage.on_low_notify)
                .on_toggle(Message::StorageOnLow)
                .size(14)
                .text_size(12),
            checkbox("Require room for the payload", storage.require_space)
                .on_toggle(Message::StorageRequireSpace)
                .size(14)
                .text_size(12),
        ]
        .spacing(14)
        .wrap();

        let number = |caption: &'static str, field: StorageField, width: f32| {
            column![
                text(caption).size(10).color(theme::TEXT_MUTED),
                text_input("0", storage.text(field))
                    .style(muted_input_style)
                    .padding([4, 8])
                    .size(12)
                    .on_input(move |value| Message::StorageNumber(field, value))
                    .width(Length::Fixed(width)),
            ]
            .spacing(2)
        };
        let numbers = row![
            number("Minimum free (bytes)", StorageField::MinFree, 150.0),
            number("Recovery margin (bytes)", StorageField::Margin, 150.0),
            number("Poll (s)", StorageField::Poll, 80.0),
        ]
        .spacing(10);

        let mut zones = column![row![
            text("PATH")
                .size(10)
                .color(theme::TEXT_MUTED)
                .width(Length::FillPortion(6)),
            text("LABEL")
                .size(10)
                .color(theme::TEXT_MUTED)
                .width(Length::FillPortion(3)),
            text("STAGING")
                .size(10)
                .color(theme::TEXT_MUTED)
                .width(Length::FillPortion(2)),
            text("ON")
                .size(10)
                .color(theme::TEXT_MUTED)
                .width(Length::FillPortion(1)),
            text("").width(Length::FillPortion(1)),
        ]]
        .spacing(4);
        if storage.zones.is_empty() {
            zones = zones.push(
                text("No zones yet: add the label folders and one staging folder per disk.")
                    .size(11)
                    .color(theme::TEXT_MUTED),
            );
        }
        for (index, zone) in storage.zones.iter().enumerate() {
            let selected = settings.zone_selected == Some(index);
            let path = text_input("/mnt/usbhdd2/Filmek", &zone.path)
                .style(muted_input_style)
                .padding([4, 8])
                .size(12)
                .on_input(move |value| Message::ZonePath(index, value))
                .width(Length::FillPortion(6));
            let label: Element<Message> = if zone.staging {
                text("staging")
                    .size(11)
                    .color(theme::TEXT_MUTED)
                    .width(Length::FillPortion(3))
                    .into()
            } else {
                text_input("Filmek", &zone.label)
                    .style(muted_input_style)
                    .padding([4, 8])
                    .size(12)
                    .on_input(move |value| Message::ZoneLabel(index, value))
                    .width(Length::FillPortion(3))
                    .into()
            };
            zones = zones.push(
                row![
                    path,
                    label,
                    checkbox("", zone.staging)
                        .on_toggle(move |value| Message::ZoneStaging(index, value))
                        .size(14)
                        .width(Length::FillPortion(2)),
                    checkbox("", zone.enabled)
                        .on_toggle(move |value| Message::ZoneEnabled(index, value))
                        .size(14)
                        .width(Length::FillPortion(1)),
                    button(text("×").size(14))
                        .on_press(Message::ZoneRemove(index))
                        .padding([2, 8])
                        .style(if selected {
                            iced::widget::button::primary
                        } else {
                            iced::widget::button::text
                        }),
                ]
                .spacing(6)
                .align_y(iced::Alignment::Center),
            );
        }

        let body = column![
            text("Filesystems").size(12).font(theme::medium()),
            disks,
            text("Guard").size(12).font(theme::medium()),
            flags,
            numbers,
            row![
                text("Minimum free and margin apply per filesystem; the guard pauses writers only, never seeds.")
                    .size(10)
                    .color(theme::TEXT_MUTED),
            ],
            text("Zones").size(12).font(theme::medium()),
            zones,
            row![action_button("Add zone", Message::ZoneAdd, true)].spacing(8),
        ]
        .spacing(10);

        scrollable(body).height(Length::Fill).into()
    }

    fn settings_labels_view(&self) -> Element<'_, Message> {
        let settings = &self.settings;
        let rows: Vec<Element<Message>> = settings
            .labels
            .iter()
            .map(|row| {
                let selected = settings.label_selected.as_deref() == Some(row.label.name.as_str());
                let dot = container(text("").size(1))
                    .width(Length::Fixed(8.0))
                    .height(Length::Fixed(8.0))
                    .style(move |_theme: &iced::Theme| container::Style {
                        background: Some(
                            settings::color_from_hex(&row.label.color)
                                .unwrap_or(theme::TEXT_MUTED)
                                .into(),
                        ),
                        border: iced::Border {
                            color: theme::BORDER,
                            width: 0.0,
                            radius: 4.0.into(),
                        },
                        ..container::Style::default()
                    });
                button(
                    row![
                        dot,
                        text(row.label.name.clone()).size(12),
                        horizontal_space(),
                        text(row.torrent_count.to_string())
                            .size(11)
                            .color(theme::TEXT_MUTED),
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
                )
                .on_press(Message::LabelSelected(row.label.name.clone()))
                .width(Length::Fill)
                .padding([5, 8])
                .style(move |_theme: &iced::Theme, _status| button::Style {
                    background: Some(
                        if selected {
                            theme::SURFACE
                        } else {
                            theme::BG_SECONDARY
                        }
                        .into(),
                    ),
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
                        radius: 4.0.into(),
                    },
                    ..button::Style::default()
                })
                .into()
            })
            .collect();

        let list = column![
            row![
                text("Labels").size(12).font(theme::semibold()),
                horizontal_space(),
                action_button("New", Message::LabelNew, !settings.loading),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            container(
                scrollable(WidgetColumn::with_children(rows).spacing(2)).height(Length::Fill)
            )
            .height(Length::Fill),
        ]
        .spacing(6)
        .width(Length::Fixed(272.0))
        .height(Length::Fill);

        let editor: Element<Message> = if settings.label_new || settings.label_selected.is_some() {
            self.label_editor_view()
        } else {
            column![
                text("Select a label to edit, or create a new one.")
                    .size(12)
                    .color(theme::TEXT_MUTED),
            ]
            .spacing(6)
            .into()
        };

        row![list, container(editor).padding([0, 12]).width(Length::Fill)]
            .spacing(4)
            .height(Length::Fill)
            .into()
    }

    fn label_editor_view(&self) -> Element<'_, Message> {
        let settings = &self.settings;
        let form = &settings.label_form;
        let name: Element<Message> = if settings.label_new {
            label_field_view(form, LabelField::Name, "movies", false)
        } else {
            column![
                row![
                    text(LabelField::Name.label())
                        .size(11)
                        .color(theme::TEXT_MUTED),
                    text("fixed").size(10).color(theme::TEXT_MUTED),
                ]
                .spacing(6),
                container(text(form.name.clone()).size(13))
                    .padding([6, 8])
                    .width(Length::Fill)
                    .style(|_theme: &iced::Theme| container::Style {
                        background: Some(theme::BG_SECONDARY.into()),
                        border: iced::Border {
                            color: theme::BORDER,
                            width: 1.0,
                            radius: 4.0.into(),
                        },
                        ..container::Style::default()
                    }),
            ]
            .spacing(3)
            .into()
        };
        // Preset colours: the theme's accent family plus a few neutrals.
        let palette = [
            "#e5484d", "#f76b15", "#ffb224", "#46a758", "#12a594", "#0090ff", "#8e4ec6", "#6e7681",
        ];
        let mut swatches = row![].spacing(4);
        for hex in palette {
            let color = settings::color_from_hex(hex).unwrap_or(theme::TEXT_MUTED);
            swatches = swatches.push(
                button("")
                    .on_press(Message::LabelField(LabelField::Color, hex.to_owned()))
                    .width(Length::Fixed(18.0))
                    .height(Length::Fixed(18.0))
                    .padding(0)
                    .style(move |_, _| iced::widget::button::Style {
                        background: Some(color.into()),
                        border: iced::border::rounded(4),
                        ..iced::widget::button::Style::default()
                    }),
            );
        }
        let editor = column![
            row![
                name,
                label_field_view(form, LabelField::Color, "#RRGGBB", false),
            ]
            .spacing(8),
            swatches,
            label_field_view(form, LabelField::MovePath, "/mnt/media/movies", false),
            row![
                label_field_view(form, LabelField::DownloadRateLimit, "0", false),
                label_field_view(form, LabelField::UploadRateLimit, "0", false),
            ]
            .spacing(8),
            row![
                label_field_view(form, LabelField::ConnectionsLimit, "0", false),
                label_field_view(form, LabelField::UploadSlotsLimit, "0", false),
            ]
            .spacing(8),
            label_field_view(
                form,
                LabelField::MatchTrackers,
                "tracker.example, other.example",
                false
            ),
            label_field_view(form, LabelField::MatchName, ".*\\.mkv", false),
        ]
        .spacing(8);

        let delete: Element<Message> = if settings.label_new {
            Element::from(text(""))
        } else if settings.confirming_delete {
            let count = settings.current_label().map_or(0, |row| row.torrent_count);
            row![
                text(format!("Delete and clear {count} assignment(s)?"))
                    .size(11)
                    .color(theme::QUEUED)
                    .width(Length::Fill),
                action_button("Cancel", Message::LabelDeleteCancelled, true),
                danger_button("Delete", Message::LabelDeleteConfirmed, !settings.loading),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center)
            .into()
        } else {
            row![
                horizontal_space(),
                danger_button("Delete", Message::LabelDeleteRequested, !settings.loading),
            ]
            .into()
        };

        column![scrollable(column![editor, delete].spacing(12)).height(Length::Fill),]
            .spacing(10)
            .height(Length::Fill)
            .into()
    }

    fn set_label_view(&self) -> Element<'_, Message> {
        let current = self
            .selected_torrents
            .values()
            .next()
            .map(|torrent| torrent.label.clone())
            .unwrap_or_default();
        let mut rows: Vec<Element<Message>> = vec![label_choice(
            "None".to_owned(),
            current.is_empty(),
            Message::ChooseLabel(None),
        )];
        rows.extend(self.settings.labels.iter().map(|row| {
            label_choice(
                row.label.name.clone(),
                current == row.label.name,
                Message::ChooseLabel(Some(row.label.name.clone())),
            )
        }));
        let body = column![
            text(format!("{} torrent(s) selected", self.selected.len()))
                .size(12)
                .color(theme::TEXT_SECONDARY),
            if self.settings.labels.is_empty() {
                Element::from(text("No labels yet; create them under Settings → Labels.").size(12))
            } else {
                Element::from(
                    scrollable(WidgetColumn::with_children(rows).spacing(2)).height(Length::Fill),
                )
            },
            row![
                horizontal_space(),
                action_button("Cancel", Message::CloseModal, true),
            ],
        ]
        .spacing(10)
        .height(Length::Fill);
        modal_panel(Modal::SetLabel, body.into())
    }

    fn add_torrent_view(&self) -> Element<'_, Message> {
        let mut labels: Vec<Element<Message>> = vec![label_choice(
            "None".to_owned(),
            self.add_label.is_empty(),
            Message::AddLabelChanged(String::new()),
        )];
        labels.extend(self.settings.labels.iter().map(|row| {
            label_choice(
                row.label.name.clone(),
                self.add_label == row.label.name,
                Message::AddLabelChanged(row.label.name.clone()),
            )
        }));
        let warning: Element<Message> = if self.add_label.trim().is_empty() {
            text("No label: the payload goes to the global download directory.")
                .size(10)
                .color(theme::QUEUED)
                .into()
        } else if self.staging_for(&self.add_label).is_some() {
            text(format!(
                "Staging folder of the label's filesystem: {}",
                self.staging_for(&self.add_label).unwrap_or_default()
            ))
            .size(10)
            .color(theme::TEXT_MUTED)
            .into()
        } else {
            text("This label has no storage zone yet; the path stays editable.")
                .size(10)
                .color(theme::QUEUED)
                .into()
        };
        let body = column![
            detail_row("File", self.add_path.clone(), theme::TEXT_MUTED),
            text("Name").size(11).color(theme::TEXT_MUTED),
            text_input("Torrent name", &self.add_name)
                .on_input(Message::AddNameChanged)
                .width(Length::Fill),
            text("Label").size(11).color(theme::TEXT_MUTED),
            scrollable(column(labels).spacing(2).width(Length::Fill))
                .width(Length::Fill)
                .height(Length::Fixed(120.0)),
            warning,
            text("Save path").size(11).color(theme::TEXT_MUTED),
            text_input("Save path (daemon default)", &self.add_save_path)
                .on_input(Message::AddSavePathChanged)
                .width(Length::Fill),
            row![
                horizontal_space(),
                action_button("Cancel", Message::CloseModal, true),
                action_button("Add torrent", Message::AddConfirmed, !self.add_in_flight),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(8);
        modal_panel(Modal::AddTorrent, body.into())
    }

    fn remove_view(&self) -> Element<'_, Message> {
        let count = self.selected.len();
        let bytes: u64 = self
            .selected_torrents
            .values()
            .map(|torrent| torrent.total_bytes)
            .sum();
        let body = column![
            text(format!(
                "Remove {count} torrent{}?",
                if count == 1 { "" } else { "s" }
            ))
            .size(13),
            text(format!(
                "{} of data on the daemon host.",
                format_size(bytes)
            ))
            .size(11)
            .color(theme::TEXT_MUTED),
            checkbox("Include downloaded files", self.remove_delete_files)
                .on_toggle(Message::ToggleRemoveFiles),
            row![
                horizontal_space(),
                action_button("Cancel", Message::CloseModal, true),
                action_button("Remove", Message::RemoveConfirmed, true),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(10);
        modal_panel(Modal::Remove, body.into())
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
            Some(expected) => column![
                text("Pinned fingerprint").size(12),
                text(fingerprint_display(expected))
                    .size(12)
                    .font(iced::Font::MONOSPACE),
            ]
            .spacing(4)
            .into(),
            None => column![].into(),
        };

        container(
            column![
                text(title).size(22),
                text(warning),
                text("Server").size(12),
                text(&pending.config.server),
                text("Presented SHA-256 fingerprint").size(12),
                text(fingerprint_display(&pending.fingerprint))
                    .size(12)
                    .font(iced::Font::MONOSPACE),
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
        // The label chip colour comes from the daemon's label definitions;
        // an unknown label falls back to the dim text colour.
        let label_colors: BTreeMap<String, iced::Color> = self
            .settings
            .labels
            .iter()
            .filter_map(|row| {
                settings::color_from_hex(&row.label.color)
                    .map(|color| (row.label.name.clone(), color))
            })
            .collect();
        let table: Element<TableMessage> = iced::widget::canvas::Canvas::new(Table {
            rows: &self.page.torrents,
            page_offset: self.page_offset.unwrap_or(0),
            total: self.page.total as usize,
            scroll: self.scroll,
            selected: &self.selected,
            sort: self.sort,
            descending: self.descending,
            context: self.context,
            layout: &self.columns,
            column_menu: self.column_menu,
            resizing: self.column_resize.map(|(column, _, _)| column),
            label_colors,
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .into();
        table.map(Message::Table)
    }

    fn sidebar_view(&self) -> Element<'_, Message> {
        let status = self.server_status.clone().unwrap_or_default();
        // A label filter replaces the state category; no state row stays
        // selected, including "All torrents".
        let active = if self.label_filter.is_empty() {
            self.state_filter
        } else {
            Some(i32::MIN)
        };
        let expanded = !self.sidebar_collapsed;

        let mark = iced::widget::image(iced::widget::image::Handle::from_bytes(theme::MARK_BYTES))
            .width(Length::Fixed(24.0))
            .height(Length::Fixed(24.0));
        let logo = iced::widget::image(iced::widget::image::Handle::from_bytes(theme::LOGO_BYTES))
            .width(Length::Fixed(113.0))
            .height(Length::Fixed(30.0));
        let toggle = icon_button(
            if expanded {
                glyph::CHEVRON_LEFT
            } else {
                glyph::CHEVRON_RIGHT
            },
            Message::ToggleSidebar,
        );

        let header: Element<Message> = if expanded {
            row![logo, horizontal_space(), toggle]
                .spacing(8)
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
        let moving = TorrentState::Moving as i32;

        // Only the disks that carry zones matter to the guard, so the sidebar
        // counts those; the Storage tab still offers every filesystem.
        let (zoned_count, zoned_free, zoned_low) = zoned_disks(&self.disks);
        let disks_line: Element<Message> = if expanded && zoned_count > 0 {
            button(
                column![
                    text(if zoned_count == 1 {
                        "1 disk".to_owned()
                    } else {
                        format!("{zoned_count} disks")
                    })
                    .size(10)
                    .color(theme::TEXT_MUTED),
                    text(format!("{} free", format_size(zoned_free)))
                        .size(10)
                        .color(if zoned_low {
                            theme::ERROR
                        } else {
                            theme::TEXT_MUTED
                        }),
                ]
                .spacing(1),
            )
            .on_press(Message::OpenStorage)
            .width(Length::Fill)
            .padding([4, 12])
            .style(iced::widget::button::text)
            .into()
        } else {
            text("").size(1).into()
        };
        let bottom: Element<Message> = WidgetColumn::with_children(vec![
            disks_line,
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
                glyph::DOWNLOAD,
                table::state_color(moving),
                "Moving",
                status.moving,
                Some(moving),
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
            if expanded && !self.settings.labels.is_empty() {
                Element::from(
                    column![
                        container(
                            text("LABELS")
                                .size(10)
                                .color(theme::TEXT_MUTED)
                                .font(theme::semibold())
                        )
                        .padding([8, 12]),
                        container(
                            scrollable(
                                WidgetColumn::with_children(
                                    self.settings
                                        .labels
                                        .iter()
                                        .map(|row| {
                                            label_nav(row, self.label_filter == row.label.name)
                                        })
                                        .collect::<Vec<Element<Message>>>(),
                                )
                                .spacing(1),
                            )
                            .height(Length::Fill),
                        )
                        .height(Length::Fixed(168.0)),
                    ]
                    .spacing(0),
                )
            } else {
                Element::from(text("").size(1))
            },
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
        let unfinished = self
            .selected_torrents
            .values()
            .any(|torrent| torrent.state != TorrentState::Seeding as i32);
        let icons = row![
            toolbar_icon(glyph::PLUS, "Add torrent", Message::PickTorrent, true),
            toolbar_icon(
                glyph::TRASH,
                "Remove selected",
                Message::OpenRemove,
                selected > 0
            ),
            toolbar_icon(glyph::SEARCH, "Search", Message::ToggleSearch, true),
            toolbar_icon(
                glyph::PAUSE,
                "Pause selected",
                Message::Action(TorrentAction::Pause),
                selected > 0
            ),
            toolbar_icon(
                glyph::PLAY,
                "Resume selected",
                Message::Action(TorrentAction::Resume),
                selected > 0
            ),
            toolbar_icon(
                glyph::ARROW_UP,
                "Move up in queue",
                Message::QueueMove(true),
                unfinished
            ),
            toolbar_icon(
                glyph::ARROW_DOWN,
                "Move down in queue",
                Message::QueueMove(false),
                unfinished
            ),
            Element::<Message>::from(horizontal_space()),
            toolbar_icon(glyph::REFRESH, "Refresh", Message::Refresh, true),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center);

        let search: Element<'_, Message> = if self.search_visible {
            row![
                toolbar_icon(glyph::CLOSE, "Close search", Message::ToggleSearch, true),
                text_input("Search torrents…", &self.filter)
                    .id(self.filter_id.clone())
                    .on_input(Message::FilterChanged)
                    .width(Length::Fill),
            ]
            .spacing(6)
            .align_y(iced::Alignment::Center)
            .into()
        } else {
            iced::widget::Space::with_height(0).into()
        };

        container(column![icons, search].spacing(6))
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
                async move {
                    connection::files(&client, id)
                        .await
                        .map(DetailsPayload::Files)
                },
                |result| match result {
                    Ok(payload) => Message::DetailsLoaded(Box::new(payload)),
                    Err(error) => Message::DetailsFailed(error),
                },
            ),
            DetailsTab::Peers => Task::perform(
                async move {
                    connection::peers(&client, id)
                        .await
                        .map(DetailsPayload::Peers)
                },
                |result| match result {
                    Ok(payload) => Message::DetailsLoaded(Box::new(payload)),
                    Err(error) => Message::DetailsFailed(error),
                },
            ),
            DetailsTab::Trackers => Task::perform(
                async move {
                    connection::trackers(&client, id)
                        .await
                        .map(DetailsPayload::Trackers)
                },
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
            // The activity log is session-wide, so it is worth showing even
            // when no row is selected.
            if self.details_tab == DetailsTab::Activity {
                return self.activity_body();
            }
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
                    text(&torrent.name)
                        .size(13)
                        .font(theme::medium())
                        .width(Length::Fill),
                    text(table::state_label(torrent.state))
                        .size(11)
                        .color(table::state_color(torrent.state)),
                ]
                .spacing(12),
                row![
                    detail_row("Size", format_size(torrent.total_bytes), dim),
                    detail_row("Progress", format!("{:.1}%", torrent.progress * 100.0), dim),
                    detail_row("Down", table::format_rate(torrent.download_rate), dim),
                    detail_row("Up", table::format_rate(torrent.upload_rate), dim),
                ]
                .spacing(24),
                row![
                    detail_row("Seeds", torrent.seeds.to_string(), dim),
                    detail_row("Peers", torrent.peers.to_string(), dim),
                    detail_row("Torrent ID", torrent.id.to_string(), dim),
                    detail_row(
                        "Label",
                        if torrent.label.is_empty() {
                            "—".to_owned()
                        } else {
                            torrent.label.clone()
                        },
                        dim,
                    ),
                ]
                .spacing(24),
                row![detail_row(
                    "Save",
                    if torrent.save_path.is_empty() {
                        "-".to_owned()
                    } else {
                        torrent.save_path.clone()
                    },
                    dim,
                )],
                if torrent.pause_reasons == 0 {
                    Element::<Message>::from(text(""))
                } else {
                    Element::<Message>::from(
                        text(format!(
                            "Paused by: {}",
                            pause_reason_label(torrent.pause_reasons)
                        ))
                        .size(11)
                        .color(theme::TEXT_SECONDARY),
                    )
                },
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
                            text(format!("{:.0}%", file.progress * 100.0))
                                .size(11)
                                .color(dim),
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
                            text(format!("{:.0}%", peer.progress * 100.0))
                                .size(11)
                                .color(dim),
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
                let header = row![
                    text("TRACKER")
                        .size(10)
                        .color(theme::TEXT_MUTED)
                        .width(Length::Fill),
                    text("SEEDS / LEECHERS")
                        .size(10)
                        .color(theme::TEXT_MUTED)
                        .width(Length::Fixed(100.0)),
                    text("STATUS")
                        .size(10)
                        .color(theme::TEXT_MUTED)
                        .width(Length::Fixed(96.0)),
                ]
                .spacing(16);
                let rows = trackers
                    .iter()
                    .map(|tracker: &TorrentTracker| {
                        let counts = if tracker.seeds == 0 && tracker.leechers == 0 {
                            "-".to_owned()
                        } else {
                            format!("{} / {}", tracker.seeds, tracker.leechers)
                        };
                        let (label, detail) = tracker::status_text(
                            &tracker.status,
                            &tracker.message,
                            tracker.next_announce,
                            tracker.fails,
                        );
                        let status_color = if tracker.status == "error" {
                            theme::ERROR
                        } else if tracker.status == "working" {
                            theme::SUCCESS
                        } else {
                            dim
                        };
                        // The reason sits on its own line so a long error text
                        // reads without wrapping into the next tracker.
                        WidgetColumn::with_children(vec![
                            row![
                                text(&tracker.url).size(11).width(Length::Fill),
                                text(counts).size(11).color(dim).width(Length::Fixed(100.0)),
                                text(label)
                                    .size(11)
                                    .color(status_color)
                                    .width(Length::Fixed(96.0)),
                            ]
                            .spacing(16)
                            .into(),
                            text(detail).size(10).color(theme::TEXT_MUTED).into(),
                        ])
                        .spacing(2)
                        .into()
                    })
                    .collect::<Vec<Element<Message>>>();
                let mut children: Vec<Element<Message>> = Vec::with_capacity(rows.len() + 1);
                children.push(header.into());
                children.extend(rows);
                scrollable(WidgetColumn::with_children(children).spacing(8))
                    .height(Length::Fill)
                    .into()
            }
            Some(_) => text("No trackers reported.").size(12).color(dim).into(),
            None => self.pending_details(dim),
        }
    }

    fn activity_body(&self) -> Element<'_, Message> {
        let dim = theme::TEXT_SECONDARY;
        // With a torrent selected the tab is its history; without one it is
        // the whole session, newest first.
        let only = self.active;
        if self.activity.is_empty(only) {
            let message = if only.is_some() {
                "No events for this torrent yet."
            } else {
                "No events observed yet."
            };
            return text(message).size(12).color(dim).into();
        }
        let rows = self
            .activity
            .iter(only)
            .map(|event| self.activity_row(event))
            .collect::<Vec<Element<Message>>>();
        scrollable(WidgetColumn::with_children(rows).spacing(2))
            .height(Length::Fill)
            .into()
    }

    fn activity_row(&self, event: &ActivityEvent) -> Element<'_, Message> {
        let dim = theme::TEXT_SECONDARY;
        let (change, color) = match &event.change {
            Change::State(state) => {
                let label = table::state_label(*state);
                let color = if *state == TorrentState::Error as i32 {
                    theme::ERROR
                } else {
                    dim
                };
                (label.to_owned(), color)
            }
            Change::Removed => ("removed".to_owned(), theme::ERROR),
        };
        let from = event.from.map_or_else(
            || "—".to_owned(),
            |state| table::state_label(state).to_owned(),
        );
        let source = match event.source {
            ActivitySource::Client => "client",
            ActivitySource::Daemon => "daemon",
        };
        let when = chrono::DateTime::<chrono::Local>::from(event.at)
            .format("%H:%M:%S")
            .to_string();
        row![
            text(when).size(11).color(dim).width(Length::Fixed(64.0)),
            text(from).size(11).color(dim).width(Length::Fixed(92.0)),
            text(change)
                .size(11)
                .color(color)
                .width(Length::Fixed(92.0)),
            text(source).size(11).color(dim).width(Length::Fixed(56.0)),
            text(event.name.clone()).size(11).width(Length::Fill),
            text(format!("#{}", event.torrent_id))
                .size(11)
                .color(dim)
                .width(Length::Fixed(56.0)),
        ]
        .spacing(8)
        .into()
    }

    fn pending_details(&self, dim: Color) -> Element<'_, Message> {
        if let Some(error) = &self.details_error {
            text(format!("Load failed: {error}"))
                .size(12)
                .color(theme::ERROR)
                .into()
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
        let (down, up, active, total) = status_numbers(
            self.client.is_some(),
            self.server_status.as_ref(),
            self.page.total,
        );
        container(
            row![
                text("●").size(9).color(color),
                text(label).size(11).color(color),
                text(server).size(11).color(theme::TEXT_SECONDARY),
                horizontal_space(),
                text(format!("↓ {}", table::format_rate(down))).size(11),
                text(format!("↑ {}", table::format_rate(up))).size(11),
                text(format!("{active} active"))
                    .size(11)
                    .color(theme::TEXT_SECONDARY),
                text(format!("{total} torrents"))
                    .size(11)
                    .color(theme::TEXT_SECONDARY),
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

fn watch_field_view<'a>(
    row: &'a settings::WatchRow,
    field: WatchField,
    placeholder: &'a str,
) -> Element<'a, Message> {
    column![
        text(field.label()).size(11).color(theme::TEXT_MUTED),
        text_input(placeholder, row.text(field))
            .style(muted_input_style)
            .padding([4, 8])
            .size(12)
            .width(Length::Fill)
            .on_input(move |value| Message::AutoAddField(field, value)),
    ]
    .spacing(3)
    .into()
}

fn hook_field_view<'a>(
    row: &'a settings::HookRow,
    field: HookField,
    placeholder: &'a str,
) -> Element<'a, Message> {
    column![
        text(field.label()).size(11).color(theme::TEXT_MUTED),
        text_input(placeholder, row.text(field))
            .style(muted_input_style)
            .padding([4, 8])
            .size(12)
            .width(Length::Fill)
            .on_input(move |value| Message::HooksField(field, value)),
    ]
    .spacing(3)
    .into()
}

fn hook_event_toggle(event: HookEvent, current: HookEvent) -> Element<'static, Message> {
    toggle_button(event.label(), event == current, Message::HooksEvent(event))
}

fn toggle_button(
    label: &'static str,
    selected: bool,
    message: Message,
) -> Element<'static, Message> {
    button(text(label).size(11))
        .on_press(message)
        .padding([5, 12])
        .style(move |_theme: &iced::Theme, _status| button::Style {
            background: Some(
                if selected {
                    theme::SURFACE
                } else {
                    theme::BG_SECONDARY
                }
                .into(),
            ),
            text_color: if selected {
                theme::BRIGHT
            } else {
                theme::TEXT_SECONDARY
            },
            border: iced::Border {
                color: if selected {
                    theme::PRIMARY
                } else {
                    theme::BORDER
                },
                width: if selected { 1.0 } else { 0.0 },
                radius: 4.0.into(),
            },
            ..button::Style::default()
        })
        .into()
}

fn watch_action(action: SuccessAction, current: SuccessAction) -> Element<'static, Message> {
    toggle_button(
        action.label(),
        action == current,
        Message::AutoAddAction(action),
    )
}

/// Placeholder for the Telegram API base field.
const TELEGRAM_API_BASE: &str = "https://api.telegram.org";

/// One selectable notification channel row.
fn notification_row(
    title: String,
    detail: String,
    selected: bool,
    message: Message,
) -> Element<'static, Message> {
    button(
        column![
            text(title).size(12),
            text(detail).size(10).color(theme::TEXT_MUTED),
        ]
        .spacing(2),
    )
    .on_press(message)
    .width(Length::Fill)
    .padding([5, 8])
    .style(move |_theme: &iced::Theme, _status| button::Style {
        background: Some(
            if selected {
                theme::SURFACE
            } else {
                theme::BG_SECONDARY
            }
            .into(),
        ),
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
            radius: 4.0.into(),
        },
        ..button::Style::default()
    })
    .into()
}

/// One digest/quiet-hours input.
fn digest_field<'a>(
    field: DigestField,
    placeholder: &'a str,
    notify: &'a settings::NotifyForm,
) -> Element<'a, Message> {
    column![
        text(field.label()).size(11).color(theme::TEXT_MUTED),
        text_input(placeholder, notify.digest_text(field))
            .style(muted_input_style)
            .padding([4, 8])
            .size(12)
            .width(Length::Fill)
            .on_input(move |value| Message::DigestChanged(field, value)),
    ]
    .spacing(3)
    .into()
}

fn settings_tab(tab: Tab, active: bool) -> Element<'static, Message> {
    button(text(tab.label()).size(12).font(theme::medium()))
        .on_press(Message::SettingsTab(tab))
        .width(Length::Fill)
        .padding([6, 10])
        .style(move |_theme: &iced::Theme, _status| button::Style {
            background: Some(
                if active {
                    theme::SURFACE
                } else {
                    theme::BG_SECONDARY
                }
                .into(),
            ),
            text_color: if active {
                theme::BRIGHT
            } else {
                theme::TEXT_SECONDARY
            },
            border: iced::Border {
                color: if active {
                    theme::PRIMARY
                } else {
                    theme::BG_SECONDARY
                },
                width: 0.0,
                radius: 4.0.into(),
            },
            ..button::Style::default()
        })
        .into()
}

fn settings_field<'a>(
    settings: &'a Settings,
    form: &'a settings::ConfigForm,
    field: ConfigField,
    placeholder: &'static str,
) -> Element<'a, Message> {
    let locked = settings.locked(field);
    let hint: Element<Message> = if locked {
        text("env-locked").size(10).color(theme::QUEUED).into()
    } else {
        text("").size(1).into()
    };
    column![
        row![text(field.label()).size(11).color(theme::TEXT_MUTED), hint,].spacing(6),
        text_input(placeholder, form.text(field))
            .style(muted_input_style)
            .padding([4, 8])
            .size(12)
            .on_input(move |value| Message::SettingsField(field, value))
            .width(Length::Fill),
    ]
    .spacing(3)
    .into()
}

/// One scheduler grid cell.
fn scheduler_cell(day: usize, hour: usize, level: SchedulerLevel) -> Element<'static, Message> {
    let color = match level {
        SchedulerLevel::Green => Color::from_rgb8(0x21, 0xc5, 0x5d),
        SchedulerLevel::Yellow => Color::from_rgb8(0xf2, 0xcc, 0x15),
        SchedulerLevel::Red => Color::from_rgb8(0xef, 0x44, 0x44),
    };
    button(text("").size(1))
        .on_press(Message::SchedulerCell(day, hour))
        .width(Length::Fixed(15.0))
        .height(Length::Fixed(18.0))
        .padding(0)
        .style(move |_theme: &iced::Theme, status| button::Style {
            background: Some(
                match status {
                    button::Status::Hovered => color.scale_alpha(0.8),
                    _ => color,
                }
                .into(),
            ),
            text_color: theme::BG,
            border: iced::Border {
                color: theme::BORDER,
                width: 0.0,
                radius: 2.0.into(),
            },
            ..button::Style::default()
        })
        .into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SchedulerField {
    Timezone,
    DownloadLimit,
    UploadLimit,
}

/// Splits a fingerprint into two lines so the last group is never wrapped
/// alone; a 32-group digest fits the dialog at 12 pt monospace.
fn fingerprint_display(fingerprint: &str) -> String {
    let groups: Vec<&str> = fingerprint.split(':').collect();
    if groups.len() % 2 == 0 && groups.len() >= 2 {
        let half = groups.len() / 2;
        format!("{}\n{}", groups[..half].join(":"), groups[half..].join(":"))
    } else {
        fingerprint.to_owned()
    }
}

fn muted_input_style(theme_: &iced::Theme, status: text_input::Status) -> text_input::Style {
    let mut style = text_input::default(theme_, status);
    style.placeholder = theme::TEXT_MUTED;
    style
}

fn label_field_view<'a>(
    form: &'a settings::LabelForm,
    field: LabelField,
    placeholder: &'a str,
    readonly: bool,
) -> Element<'a, Message> {
    let input = text_input(placeholder, form.text(field))
        .style(muted_input_style)
        .padding([4, 8])
        .size(12)
        .width(Length::Fill);
    let input = if readonly {
        input
    } else {
        input.on_input(move |value| Message::LabelField(field, value))
    };
    column![text(field.label()).size(11).color(theme::TEXT_MUTED), input,]
        .spacing(3)
        .into()
}

fn label_choice(name: String, selected: bool, message: Message) -> Element<'static, Message> {
    button(
        row![
            text(if selected { glyph::CHECK } else { ' ' })
                .font(icons::solid())
                .size(10.0)
                .color(theme::PRIMARY)
                .width(Length::Fixed(14.0)),
            text(name).size(12),
            horizontal_space(),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center),
    )
    .on_press(message)
    .width(Length::Fill)
    .padding([5, 8])
    .style(move |_theme: &iced::Theme, _status| button::Style {
        background: Some(
            if selected {
                theme::SURFACE
            } else {
                theme::BG_SECONDARY
            }
            .into(),
        ),
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
            radius: 4.0.into(),
        },
        ..button::Style::default()
    })
    .into()
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

fn label_nav(row: &settings::LabelRow, selected: bool) -> Element<'static, Message> {
    let color = settings::color_from_hex(&row.label.color).unwrap_or(theme::TEXT_MUTED);
    let name = row.label.name.clone();
    button(
        row![
            text(glyph::TAG)
                .font(icons::solid())
                .size(11.0)
                .color(color),
            text(name.clone()).size(12),
            horizontal_space(),
            text(row.torrent_count.to_string())
                .size(11)
                .color(theme::TEXT_MUTED),
        ]
        .spacing(9)
        .align_y(iced::Alignment::Center),
    )
    .on_press(Message::SelectLabelFilter(name))
    .width(Length::Fill)
    .padding([6, 12])
    .style(move |_theme: &iced::Theme, _status| button::Style {
        background: Some(
            if selected {
                theme::SURFACE
            } else {
                theme::BG_SECONDARY
            }
            .into(),
        ),
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
            text(icon).font(icons::solid()).size(13.0).color(icon_color),
            text(label).size(13),
            horizontal_space(),
            text(count.to_string()).size(11).color(theme::TEXT_MUTED),
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center)
        .into()
    } else {
        container(text(icon).font(icons::solid()).size(14.0).color(icon_color))
            .width(Length::Fill)
            .center_x(Length::Fill)
            .into()
    };
    button(content)
        .on_press(Message::SelectCategory(filter))
        .width(Length::Fill)
        .padding(nav_padding(expanded))
        .style(move |_theme: &iced::Theme, _status| button::Style {
            background: Some(
                if selected {
                    theme::SURFACE
                } else {
                    theme::BG_SECONDARY
                }
                .into(),
            ),
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
            text(icon).font(icons::solid()).size(13.0).color(icon_color),
            text(label).size(13).color(theme::TEXT_SECONDARY),
            horizontal_space(),
            text(count.to_string()).size(11).color(theme::TEXT_MUTED),
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center)
        .into()
    } else {
        container(text(icon).font(icons::solid()).size(14.0).color(icon_color))
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

fn toolbar_icon(
    icon: char,
    label: &'static str,
    message: Message,
    enabled: bool,
) -> Element<'static, Message> {
    let content = text(icon)
        .font(icons::solid())
        .size(13.0)
        .color(if enabled {
            theme::TEXT_SECONDARY
        } else {
            theme::TEXT_MUTED
        });
    let labeled = tooltip(
        content,
        text(label).size(11),
        iced::widget::tooltip::Position::Top,
    )
    .gap(6);
    let mut control = button(labeled)
        .padding([7, 10])
        .style(move |_theme, _status| button::Style {
            background: Some(
                if enabled {
                    theme::BG_SECONDARY
                } else {
                    theme::BG
                }
                .into(),
            ),
            border: iced::Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..button::Style::default()
        });
    if enabled {
        control = control.on_press(message);
    }
    control.into()
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
            text(icon).font(icons::solid()).size(13.0).color(icon_color),
            text(label).size(13).color(theme::TEXT_SECONDARY),
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center)
        .into()
    } else {
        container(text(icon).font(icons::solid()).size(14.0).color(icon_color))
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

/// The status bar numbers; a missing client never shows a stale poll.
fn status_numbers(
    connected: bool,
    status: Option<&ServerStatus>,
    fallback_total: u32,
) -> (u64, u64, u32, u32) {
    match status.filter(|_| connected) {
        Some(status) => (
            status.download_rate,
            status.upload_rate,
            status.active,
            status.torrent_count,
        ),
        None => (0, 0, 0, fallback_total),
    }
}

fn state_style(state: ConnectionState) -> (&'static str, Color) {
    match state {
        ConnectionState::Idle => ("idle", theme::TEXT_MUTED),
        ConnectionState::Connecting => ("connecting", theme::CONNECTING),
        ConnectionState::Connected => ("connected", theme::DOWNLOADING),
        ConnectionState::Reconnecting => ("reconnecting", theme::CONNECTING),
        ConnectionState::Offline => ("offline", theme::TEXT_MUTED),
    }
}

fn detail_row<'a>(label: &'a str, value: String, dim: Color) -> Element<'a, Message> {
    row![
        text(label).size(11).width(Length::Fixed(84.0)).color(dim),
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

/// A destructive action: muted red, compact, visibly different from Save.
fn danger_button<'a>(label: &'a str, message: Message, enabled: bool) -> Element<'a, Message> {
    let widget =
        button(text(label))
            .padding([4, 12])
            .style(|_, status| iced::widget::button::Style {
                background: Some(match status {
                    iced::widget::button::Status::Hovered => {
                        iced::Color::from_rgb(0.30, 0.13, 0.14).into()
                    }
                    iced::widget::button::Status::Pressed => {
                        iced::Color::from_rgb(0.36, 0.15, 0.16).into()
                    }
                    _ => iced::Color::from_rgb(0.20, 0.11, 0.12).into(),
                }),
                text_color: iced::Color::from_rgb(0.95, 0.48, 0.48),
                border: iced::border::rounded(6),
                ..iced::widget::button::Style::default()
            });
    if enabled {
        widget.on_press(message).into()
    } else {
        widget.into()
    }
}

fn pause_reason_label(bits: u32) -> String {
    let mut names = Vec::new();
    if bits & 1 != 0 {
        names.push("user");
    }
    if bits & 2 != 0 {
        names.push("scheduler");
    }
    if bits & 4 != 0 {
        names.push("disk guard");
    }
    if bits & 8 != 0 {
        names.push("ratio");
    }
    if names.is_empty() {
        "-".to_owned()
    } else {
        names.join(", ")
    }
}

/// Count, free space and low state of the disks that carry zones.
fn zoned_disks(disks: &[DiskRow]) -> (usize, u64, bool) {
    let zoned: Vec<&DiskRow> = disks.iter().filter(|disk| !disk.zones.is_empty()).collect();
    let free = zoned.iter().map(|disk| disk.free).sum();
    let low = zoned.iter().any(|disk| disk.low);
    (zoned.len(), free, low)
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
        .window(iced::window::Settings {
            size: Size::new(1440.0, 900.0),
            icon: iced::window::icon::from_file_data(theme::MARK_BYTES, None).ok(),
            ..iced::window::Settings::default()
        });

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

#[cfg(test)]
mod status_bar_tests {
    use super::*;

    #[test]
    fn only_zoned_disks_reach_the_sidebar() {
        let disk = |zones: usize, free: u64, low: bool| DiskRow {
            mount: format!("/mnt/{zones}"),
            free,
            low,
            zones: vec![ferrent_gui::settings::ZoneView::default(); zones],
            ..DiskRow::default()
        };
        assert_eq!(
            zoned_disks(&[disk(0, 10, true), disk(0, 20, true)]),
            (0, 0, false),
            "system disks alone show nothing"
        );
        assert_eq!(
            zoned_disks(&[disk(0, 10, true), disk(2, 100, false)]),
            (1, 100, false)
        );
        assert_eq!(
            zoned_disks(&[disk(1, 50, true), disk(1, 60, false)]),
            (2, 110, true)
        );
    }

    fn status() -> ServerStatus {
        ServerStatus {
            torrent_count: 9,
            download_rate: 100,
            upload_rate: 50,
            active: 3,
            ..ServerStatus::default()
        }
    }

    #[test]
    fn a_disconnected_client_never_shows_a_stale_poll() {
        let status = status();
        assert_eq!(status_numbers(true, Some(&status), 0), (100, 50, 3, 9));
        assert_eq!(
            status_numbers(false, Some(&status), 7),
            (0, 0, 0, 7),
            "the last poll must not look live while offline"
        );
        assert_eq!(status_numbers(true, None, 7), (0, 0, 0, 7));
    }
}
