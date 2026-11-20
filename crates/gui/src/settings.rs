//! Settings workbench: the daemon configuration form and the label editor.
//!
//! The numeric fields are kept as strings so a partially typed value is never
//! lost. `ConfigForm::render` writes the whole document back through
//! `toml_edit`, which preserves comments and the keys the form does not touch.

use ferrent_protocol::v1::{Label as ProtoLabel, LabelEntry};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};

/// Fixed size of the settings modal, per the M4-T5 design.
pub const MODAL_WIDTH: f32 = 900.0;
pub const MODAL_HEIGHT: f32 = 640.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Connection,
    Session,
    AutoAdd,
    Hooks,
    Labels,
}

impl Tab {
    pub const ALL: [Self; 5] = [
        Self::Connection,
        Self::Session,
        Self::AutoAdd,
        Self::Hooks,
        Self::Labels,
    ];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Connection => "Connection",
            Self::Session => "Session",
            Self::AutoAdd => "AutoAdd",
            Self::Hooks => "Hooks",
            Self::Labels => "Labels",
        }
    }

    #[must_use]
    pub fn hint(self) -> &'static str {
        match self {
            Self::Connection => "Daemon paths and bind address; changes apply on restart.",
            Self::Session => "Engine limits; changes apply as soon as they are saved.",
            Self::AutoAdd => {
                "Watch directories; a 1 s scan picks up new .torrent and .magnet files."
            }
            Self::Hooks => "Programs run on added, finished, removed and error events.",
            Self::Labels => "Definitions, limits and auto-assignment rules.",
        }
    }
}

/// One editable configuration field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigField {
    DataDir,
    DownloadDir,
    Bind,
    TokenFile,
    DownloadRateLimit,
    UploadRateLimit,
    ActiveDownloads,
    ActiveSeeds,
    ActiveLimit,
}

impl ConfigField {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::DataDir => "Data directory",
            Self::DownloadDir => "Download directory",
            Self::Bind => "Bind address",
            Self::TokenFile => "Token file",
            Self::DownloadRateLimit => "Download limit (B/s)",
            Self::UploadRateLimit => "Upload limit (B/s)",
            Self::ActiveDownloads => "Active downloads",
            Self::ActiveSeeds => "Active seeds",
            Self::ActiveLimit => "Active torrents",
        }
    }

    /// Key used by `GetConfig.env_locked`.
    #[must_use]
    pub fn locked_key(self) -> &'static str {
        match self {
            Self::DataDir => "data_dir",
            Self::DownloadDir => "download_dir",
            Self::Bind => "bind",
            Self::TokenFile => "token_file",
            Self::DownloadRateLimit
            | Self::UploadRateLimit
            | Self::ActiveDownloads
            | Self::ActiveSeeds
            | Self::ActiveLimit => "",
        }
    }
}

/// The editable view of the daemon configuration document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigForm {
    pub data_dir: String,
    pub download_dir: String,
    pub bind: String,
    pub token_file: String,
    pub download_rate_limit: String,
    pub upload_rate_limit: String,
    pub active_downloads: String,
    pub active_seeds: String,
    pub active_limit: String,
    pub seed_mode: bool,
    pub queue_enabled: bool,
}

impl Default for ConfigForm {
    fn default() -> Self {
        Self {
            data_dir: String::new(),
            download_dir: String::new(),
            bind: String::new(),
            token_file: String::new(),
            download_rate_limit: "0".to_owned(),
            upload_rate_limit: "0".to_owned(),
            active_downloads: "0".to_owned(),
            active_seeds: "0".to_owned(),
            active_limit: "0".to_owned(),
            seed_mode: false,
            queue_enabled: true,
        }
    }
}

impl ConfigForm {
    /// Reads the known keys out of a configuration document.
    ///
    /// A missing key means "use the daemon default", which the form shows as
    /// an empty field.
    pub fn from_toml(raw: &str) -> Result<Self, String> {
        let doc: DocumentMut = raw
            .parse()
            .map_err(|error| format!("invalid TOML: {error}"))?;
        let text = |key: &str| {
            doc.get(key)
                .and_then(Item::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let number = |section: &str, key: &str| {
            doc.get(section)
                .and_then(|item| item.get(key))
                .and_then(Item::as_integer)
                .unwrap_or(0)
                .to_string()
        };
        let flag = |section: &str, key: &str, default: bool| {
            doc.get(section)
                .and_then(|item| item.get(key))
                .and_then(Item::as_bool)
                .unwrap_or(default)
        };

        Ok(Self {
            data_dir: text("data_dir"),
            download_dir: text("download_dir"),
            bind: text("bind"),
            token_file: text("token_file"),
            download_rate_limit: number("session", "download_rate_limit"),
            upload_rate_limit: number("session", "upload_rate_limit"),
            active_downloads: number("session", "active_downloads"),
            active_seeds: number("session", "active_seeds"),
            active_limit: number("session", "active_limit"),
            seed_mode: flag("session", "seed_mode", false),
            queue_enabled: flag("queue", "enabled", true),
        })
    }

    /// Returns the field value.
    #[must_use]
    pub fn text(&self, field: ConfigField) -> &str {
        match field {
            ConfigField::DataDir => &self.data_dir,
            ConfigField::DownloadDir => &self.download_dir,
            ConfigField::Bind => &self.bind,
            ConfigField::TokenFile => &self.token_file,
            ConfigField::DownloadRateLimit => &self.download_rate_limit,
            ConfigField::UploadRateLimit => &self.upload_rate_limit,
            ConfigField::ActiveDownloads => &self.active_downloads,
            ConfigField::ActiveSeeds => &self.active_seeds,
            ConfigField::ActiveLimit => &self.active_limit,
        }
    }

    /// Replaces one text field.
    pub fn set_text(&mut self, field: ConfigField, text: String) {
        match field {
            ConfigField::DataDir => self.data_dir = text,
            ConfigField::DownloadDir => self.download_dir = text,
            ConfigField::Bind => self.bind = text,
            ConfigField::TokenFile => self.token_file = text,
            ConfigField::DownloadRateLimit => self.download_rate_limit = text,
            ConfigField::UploadRateLimit => self.upload_rate_limit = text,
            ConfigField::ActiveDownloads => self.active_downloads = text,
            ConfigField::ActiveSeeds => self.active_seeds = text,
            ConfigField::ActiveLimit => self.active_limit = text,
        }
    }

    /// Checks the values the daemon would reject anyway, with a clearer message.
    pub fn validate(&self) -> Result<(), String> {
        let bind = self.bind.trim();
        if !bind.is_empty() {
            let Some((host, port)) = bind.rsplit_once(':') else {
                return Err("Bind address must be host:port".to_owned());
            };
            if host.trim().is_empty() {
                return Err("Bind address must have a host".to_owned());
            }
            let port: u16 = port
                .parse()
                .map_err(|_| "Bind address has an invalid port".to_owned())?;
            if port == 0 {
                return Err("Bind address must not use port 0".to_owned());
            }
        }
        for (name, text) in [
            ("Download limit", &self.download_rate_limit),
            ("Upload limit", &self.upload_rate_limit),
        ] {
            parse_number(text, name, u64::MAX)?;
        }
        for (name, text) in [
            ("Active downloads", &self.active_downloads),
            ("Active seeds", &self.active_seeds),
            ("Active torrents", &self.active_limit),
        ] {
            parse_number(text, name, u64::from(u32::MAX))?;
        }
        for (name, text) in [
            ("Data directory", &self.data_dir),
            ("Download directory", &self.download_dir),
            ("Token file", &self.token_file),
        ] {
            if text.trim().is_empty() {
                continue;
            }
            if text.trim() != text {
                return Err(format!("{name} must not start or end with whitespace"));
            }
        }
        Ok(())
    }

    /// Writes the form back into `raw`, keeping everything else intact.
    pub fn render(&self, raw: &str) -> Result<String, String> {
        self.validate()?;
        let mut doc: DocumentMut = raw
            .parse()
            .map_err(|error| format!("invalid TOML: {error}"))?;

        set_text(&mut doc, "data_dir", &self.data_dir);
        set_text(&mut doc, "download_dir", &self.download_dir);
        set_text(&mut doc, "bind", &self.bind);
        set_text(&mut doc, "token_file", &self.token_file);

        set_number(
            &mut doc,
            "session",
            "download_rate_limit",
            parse_number(&self.download_rate_limit, "Download limit", u64::MAX)?,
        )?;
        set_number(
            &mut doc,
            "session",
            "upload_rate_limit",
            parse_number(&self.upload_rate_limit, "Upload limit", u64::MAX)?,
        )?;
        for (key, name, text) in [
            (
                "active_downloads",
                "Active downloads",
                &self.active_downloads,
            ),
            ("active_seeds", "Active seeds", &self.active_seeds),
            ("active_limit", "Active torrents", &self.active_limit),
        ] {
            set_number(
                &mut doc,
                "session",
                key,
                parse_number(text, name, u64::from(u32::MAX))?,
            )?;
        }
        set_bool(&mut doc, "session", "seed_mode", self.seed_mode);
        set_bool(&mut doc, "queue", "enabled", self.queue_enabled);

        Ok(doc.to_string())
    }
}

fn parse_number(text: &str, name: &str, max: u64) -> Result<u64, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(0);
    }
    let number: u64 = text
        .parse()
        .map_err(|_| format!("{name} must be a whole number"))?;
    if number > max {
        return Err(format!("{name} is too large"));
    }
    Ok(number)
}

fn set_text(doc: &mut DocumentMut, key: &str, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        doc.remove(key);
    } else {
        doc[key] = value(text);
    }
}

fn set_number(doc: &mut DocumentMut, section: &str, key: &str, number: u64) -> Result<(), String> {
    let number = i64::try_from(number)
        .map_err(|_| format!("{section}.{key} is larger than TOML can store"))?;
    table(doc, section)[key] = value(number);
    Ok(())
}

fn set_bool(doc: &mut DocumentMut, section: &str, key: &str, flag: bool) {
    table(doc, section)[key] = value(flag);
}

fn table<'a>(doc: &'a mut DocumentMut, key: &str) -> &'a mut Table {
    if !doc.get(key).is_some_and(Item::is_table) {
        doc[key] = Item::Table(Table::new());
    }
    doc[key]
        .as_table_mut()
        .expect("the section was just created as a table")
}

/// Parses `#RRGGBB` (or `#RRGGBBAA`) into a colour; alpha is ignored.
#[must_use]
pub fn color_from_hex(hex: &str) -> Option<iced::Color> {
    let hex = hex.strip_prefix('#')?;
    if hex.len() != 6 && hex.len() != 8 {
        return None;
    }
    let channel = |range: std::ops::Range<usize>| u8::from_str_radix(hex.get(range)?, 16).ok();
    Some(iced::Color::from_rgb8(
        channel(0..2)?,
        channel(2..4)?,
        channel(4..6)?,
    ))
}

/// What happens to a file after the engine accepted it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SuccessAction {
    #[default]
    Delete,
    Keep,
    Move,
}

impl SuccessAction {
    pub const ALL: [Self; 3] = [Self::Delete, Self::Keep, Self::Move];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Delete => "Delete",
            Self::Keep => "Keep",
            Self::Move => "Move",
        }
    }

    #[must_use]
    pub fn as_toml(self) -> &'static str {
        match self {
            Self::Delete => "delete",
            Self::Keep => "keep",
            Self::Move => "move",
        }
    }

    #[must_use]
    pub fn from_toml(text: &str) -> Self {
        match text {
            "keep" => Self::Keep,
            "move" => Self::Move,
            _ => Self::Delete,
        }
    }
}

/// One editable AutoAdd watch directory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchRow {
    pub path: String,
    pub enabled: bool,
    pub label: String,
    pub save_path: String,
    pub add_paused: bool,
    pub on_success: SuccessAction,
    pub move_to: String,
    pub min_free_bytes: String,
}

impl WatchRow {
    /// The row as configured (always enabled) for a new entry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: true,
            ..Self::default()
        }
    }

    /// Replaces one text field.
    pub fn set(&mut self, field: WatchField, text: String) {
        match field {
            WatchField::Path => self.path = text,
            WatchField::Label => self.label = text,
            WatchField::SavePath => self.save_path = text,
            WatchField::MoveTo => self.move_to = text,
            WatchField::MinFree => self.min_free_bytes = text,
        }
    }

    /// Returns the text of one field.
    #[must_use]
    pub fn text(&self, field: WatchField) -> &str {
        match field {
            WatchField::Path => &self.path,
            WatchField::Label => &self.label,
            WatchField::SavePath => &self.save_path,
            WatchField::MoveTo => &self.move_to,
            WatchField::MinFree => &self.min_free_bytes,
        }
    }
}

/// One editable text of a watch row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchField {
    Path,
    Label,
    SavePath,
    MoveTo,
    MinFree,
}

impl WatchField {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Path => "Watch path",
            Self::Label => "Label",
            Self::SavePath => "Save path",
            Self::MoveTo => "Move to",
            Self::MinFree => "Defer below free bytes (0 = off)",
        }
    }
}

/// The watch directory table of the configuration document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AutoAddForm {
    pub rows: Vec<WatchRow>,
}

impl AutoAddForm {
    /// Reads the `[[autoadd]]` array of tables.
    pub fn from_toml(raw: &str) -> Result<Self, String> {
        let doc: DocumentMut = raw
            .parse()
            .map_err(|error| format!("invalid TOML: {error}"))?;
        let Some(item) = doc.get("autoadd") else {
            return Ok(Self::default());
        };
        let Item::ArrayOfTables(array) = item else {
            return Err("autoadd must be an array of tables".to_owned());
        };
        let mut rows = Vec::with_capacity(array.len());
        for table in array.iter() {
            let text = |key: &str| {
                table
                    .get(key)
                    .and_then(Item::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            let flag = |key: &str, default: bool| {
                table.get(key).and_then(Item::as_bool).unwrap_or(default)
            };
            rows.push(WatchRow {
                path: text("path"),
                enabled: flag("enabled", true),
                label: text("label"),
                save_path: text("save_path"),
                add_paused: flag("add_paused", false),
                on_success: SuccessAction::from_toml(&text("on_success")),
                move_to: text("move_to"),
                min_free_bytes: table
                    .get("min_free_bytes")
                    .and_then(Item::as_integer)
                    .filter(|value| *value > 0)
                    .map_or_else(String::new, |value| value.to_string()),
            });
        }
        Ok(Self { rows })
    }

    /// Checks the rows before they are written to the document.
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = std::collections::BTreeSet::new();
        for (index, row) in self.rows.iter().enumerate() {
            let path = row.path.trim();
            if path.is_empty() {
                return Err(format!(
                    "AutoAdd row {}: watch path must not be empty",
                    index + 1
                ));
            }
            if !seen.insert(path.to_owned()) {
                return Err(format!("AutoAdd row {}: duplicate watch path", index + 1));
            }
            let threshold = row.min_free_bytes.trim();
            if !threshold.is_empty() {
                let free: i64 = threshold.parse().map_err(|_| {
                    format!(
                        "AutoAdd row {}: free-space threshold must be a whole number",
                        index + 1
                    )
                })?;
                if free < 0 {
                    return Err(format!(
                        "AutoAdd row {}: free-space threshold must not be negative",
                        index + 1
                    ));
                }
            }
            if row.on_success == SuccessAction::Move {
                let target = row.move_to.trim();
                if target.is_empty() {
                    return Err(format!(
                        "AutoAdd row {}: Move needs a destination",
                        index + 1
                    ));
                }
                if target == path {
                    return Err(format!(
                        "AutoAdd row {}: Move destination must differ from the watch path",
                        index + 1
                    ));
                }
            }
        }
        Ok(())
    }

    /// Replaces the `autoadd` array in an already rendered document.
    pub fn render_into(&self, doc: &mut DocumentMut) -> Result<(), String> {
        self.validate()?;
        if self.rows.is_empty() {
            doc.remove("autoadd");
            return Ok(());
        }
        let mut array = ArrayOfTables::new();
        for row in &self.rows {
            let mut table = Table::new();
            table["path"] = value(row.path.trim());
            table["enabled"] = value(row.enabled);
            if !row.label.trim().is_empty() {
                table["label"] = value(row.label.trim());
            }
            if !row.save_path.trim().is_empty() {
                table["save_path"] = value(row.save_path.trim());
            }
            if row.add_paused {
                table["add_paused"] = value(true);
            }
            table["on_success"] = value(row.on_success.as_toml());
            if row.on_success == SuccessAction::Move {
                table["move_to"] = value(row.move_to.trim());
            }
            if let Ok(free) = row.min_free_bytes.trim().parse::<i64>() {
                if free > 0 {
                    table["min_free_bytes"] = value(free);
                }
            }
            array.push(table);
        }
        doc["autoadd"] = Item::ArrayOfTables(array);
        Ok(())
    }

    /// Adds an empty row and returns its index.
    pub fn add(&mut self) -> usize {
        self.rows.push(WatchRow::new());
        self.rows.len() - 1
    }

    /// Removes one row.
    pub fn remove(&mut self, index: usize) {
        if index < self.rows.len() {
            self.rows.remove(index);
        }
    }
}

/// Runtime counters for one watch directory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AutoAddStatusRow {
    pub path: String,
    pub enabled: bool,
    pub label: String,
    pub move_to: String,
    pub pending: u32,
    pub added: u32,
    pub duplicates: u32,
    pub invalid: u32,
    pub deferred: u32,
    pub last_error: String,
    pub last_scan_unix: i64,
}

impl AutoAddStatusRow {
    /// One-line summary for the table.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.added > 0 {
            parts.push(format!("{} added", self.added));
        }
        if self.duplicates > 0 {
            parts.push(format!("{} duplicate", self.duplicates));
        }
        if self.pending > 0 {
            parts.push(format!("{} pending", self.pending));
        }
        if self.invalid > 0 {
            parts.push(format!("{} invalid", self.invalid));
        }
        if self.deferred > 0 {
            parts.push(format!("{} deferred", self.deferred));
        }
        if parts.is_empty() {
            "idle".to_owned()
        } else {
            parts.join(" · ")
        }
    }
}

/// The journal event a hook listens to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HookEvent {
    #[default]
    Added,
    Finished,
    Removed,
    Error,
}

impl HookEvent {
    pub const ALL: [Self; 4] = [Self::Added, Self::Finished, Self::Removed, Self::Error];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Finished => "finished",
            Self::Removed => "removed",
            Self::Error => "error",
        }
    }

    #[must_use]
    pub fn as_toml(self) -> &'static str {
        self.label()
    }

    #[must_use]
    pub fn from_toml(text: &str) -> Self {
        match text.trim() {
            "finished" | "complete" => Self::Finished,
            "removed" => Self::Removed,
            "error" => Self::Error,
            _ => Self::Added,
        }
    }
}

/// One editable hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRow {
    pub event: HookEvent,
    pub command: String,
    /// Whitespace separated argv template.
    pub args: String,
    pub shell: bool,
    pub timeout_secs: String,
    pub enabled: bool,
}

impl Default for HookRow {
    fn default() -> Self {
        Self {
            event: HookEvent::Added,
            command: String::new(),
            args: "{id} {name} {path} {infohash} {label}".to_owned(),
            shell: false,
            timeout_secs: "60".to_owned(),
            enabled: true,
        }
    }
}

impl HookRow {
    /// Replaces one text field.
    pub fn set(&mut self, field: HookField, text: String) {
        match field {
            HookField::Command => self.command = text,
            HookField::Args => self.args = text,
            HookField::Timeout => self.timeout_secs = text,
        }
    }

    /// Returns the text of one field.
    #[must_use]
    pub fn text(&self, field: HookField) -> &str {
        match field {
            HookField::Command => &self.command,
            HookField::Args => &self.args,
            HookField::Timeout => &self.timeout_secs,
        }
    }
}

/// One editable text of a hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookField {
    Command,
    Args,
    Timeout,
}

impl HookField {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Command => "Command",
            Self::Args => "Arguments (whitespace separated)",
            Self::Timeout => "Timeout (seconds)",
        }
    }
}

/// The `[[hook]]` table of the configuration document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookForm {
    pub rows: Vec<HookRow>,
}

impl HookForm {
    /// Reads the `[[hook]]` array of tables.
    pub fn from_toml(raw: &str) -> Result<Self, String> {
        let doc: DocumentMut = raw
            .parse()
            .map_err(|error| format!("invalid TOML: {error}"))?;
        let Some(item) = doc.get("hook") else {
            return Ok(Self::default());
        };
        let Item::ArrayOfTables(array) = item else {
            return Err("hook must be an array of tables".to_owned());
        };
        let mut rows = Vec::with_capacity(array.len());
        for table in array.iter() {
            let text = |key: &str| {
                table
                    .get(key)
                    .and_then(Item::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            let flag = |key: &str, default: bool| {
                table.get(key).and_then(Item::as_bool).unwrap_or(default)
            };
            let args = table
                .get("args")
                .and_then(Item::as_array)
                .map(|array| {
                    array
                        .iter()
                        .filter_map(|item| item.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_else(|| HookRow::default().args);
            rows.push(HookRow {
                event: HookEvent::from_toml(&text("event")),
                command: text("command"),
                args,
                shell: flag("shell", false),
                timeout_secs: table
                    .get("timeout_secs")
                    .and_then(Item::as_integer)
                    .map_or_else(|| "60".to_owned(), |value| value.to_string()),
                enabled: flag("enabled", true),
            });
        }
        Ok(Self { rows })
    }

    /// Checks the rows before they are written to the document.
    pub fn validate(&self) -> Result<(), String> {
        for (index, row) in self.rows.iter().enumerate() {
            if row.command.trim().is_empty() {
                return Err(format!("Hook {}: command must not be empty", index + 1));
            }
            let timeout: u64 = row
                .timeout_secs
                .trim()
                .parse()
                .map_err(|_| format!("Hook {}: timeout must be a whole number", index + 1))?;
            if timeout == 0 || timeout > 3600 {
                return Err(format!(
                    "Hook {}: timeout must be between 1 and 3600 seconds",
                    index + 1
                ));
            }
        }
        Ok(())
    }

    /// Replaces the `hook` array in an already rendered document.
    pub fn render_into(&self, doc: &mut DocumentMut) -> Result<(), String> {
        self.validate()?;
        if self.rows.is_empty() {
            doc.remove("hook");
            return Ok(());
        }
        let mut array = ArrayOfTables::new();
        for row in &self.rows {
            let mut table = Table::new();
            table["event"] = value(row.event.as_toml());
            table["command"] = value(row.command.trim());
            let mut args = toml_edit::Array::new();
            for argument in row.args.split_whitespace() {
                args.push(argument);
            }
            if !args.is_empty() {
                table["args"] = value(args);
            }
            if row.shell {
                table["shell"] = value(true);
            }
            if let Ok(timeout) = row.timeout_secs.trim().parse::<i64>() {
                table["timeout_secs"] = value(timeout);
            }
            if !row.enabled {
                table["enabled"] = value(false);
            }
            array.push(table);
        }
        doc["hook"] = Item::ArrayOfTables(array);
        Ok(())
    }

    /// Adds an empty row and returns its index.
    pub fn add(&mut self) -> usize {
        self.rows.push(HookRow::default());
        self.rows.len() - 1
    }

    /// Removes one row.
    pub fn remove(&mut self, index: usize) {
        if index < self.rows.len() {
            self.rows.remove(index);
        }
    }
}

/// One finished hook run, as shown in the Hooks tab.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookRunRow {
    pub event: String,
    pub torrent_name: String,
    pub command: String,
    pub succeeded: bool,
    pub exit_code: i32,
    pub timed_out: bool,
    pub duration_ms: u64,
    pub stdout: String,
    pub stderr: String,
    pub at_unix: i64,
}

impl HookRunRow {
    /// One-line table summary.
    #[must_use]
    pub fn summary(&self) -> String {
        let status = if self.timed_out {
            "timeout".to_owned()
        } else if self.succeeded {
            "ok".to_owned()
        } else {
            format!("exit {}", self.exit_code)
        };
        format!(
            "{} · {} · {status} · {} ms",
            self.event, self.torrent_name, self.duration_ms
        )
    }

    /// The error text to show under the row, when any.
    #[must_use]
    pub fn detail(&self) -> Option<&str> {
        if !self.stderr.is_empty() {
            Some(&self.stderr)
        } else if !self.stdout.is_empty() {
            Some(&self.stdout)
        } else {
            None
        }
    }
}

/// One label row plus its assignment count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelRow {
    pub label: ProtoLabel,
    pub torrent_count: u32,
}

impl From<LabelEntry> for LabelRow {
    fn from(entry: LabelEntry) -> Self {
        Self {
            label: entry.label.unwrap_or_default(),
            torrent_count: entry.torrent_count,
        }
    }
}

/// One editable field of the label editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelField {
    Name,
    Color,
    MovePath,
    DownloadRateLimit,
    UploadRateLimit,
    ConnectionsLimit,
    UploadSlotsLimit,
    MatchTrackers,
    MatchName,
}

impl LabelField {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Color => "Colour",
            Self::MovePath => "Move on complete",
            Self::DownloadRateLimit => "Download limit (B/s)",
            Self::UploadRateLimit => "Upload limit (B/s)",
            Self::ConnectionsLimit => "Connections",
            Self::UploadSlotsLimit => "Upload slots",
            Self::MatchTrackers => "Match trackers",
            Self::MatchName => "Match name regex",
        }
    }
}

/// The editable view of one label definition.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LabelForm {
    pub name: String,
    pub color: String,
    pub move_completed_path: String,
    pub download_rate_limit: String,
    pub upload_rate_limit: String,
    pub connections_limit: String,
    pub upload_slots_limit: String,
    pub match_trackers: String,
    pub match_name: String,
}

impl LabelForm {
    /// Replaces one text field.
    pub fn set(&mut self, field: LabelField, text: String) {
        match field {
            LabelField::Name => self.name = text,
            LabelField::Color => self.color = text,
            LabelField::MovePath => self.move_completed_path = text,
            LabelField::DownloadRateLimit => self.download_rate_limit = text,
            LabelField::UploadRateLimit => self.upload_rate_limit = text,
            LabelField::ConnectionsLimit => self.connections_limit = text,
            LabelField::UploadSlotsLimit => self.upload_slots_limit = text,
            LabelField::MatchTrackers => self.match_trackers = text,
            LabelField::MatchName => self.match_name = text,
        }
    }

    /// Returns the text of one field.
    #[must_use]
    pub fn text(&self, field: LabelField) -> &str {
        match field {
            LabelField::Name => &self.name,
            LabelField::Color => &self.color,
            LabelField::MovePath => &self.move_completed_path,
            LabelField::DownloadRateLimit => &self.download_rate_limit,
            LabelField::UploadRateLimit => &self.upload_rate_limit,
            LabelField::ConnectionsLimit => &self.connections_limit,
            LabelField::UploadSlotsLimit => &self.upload_slots_limit,
            LabelField::MatchTrackers => &self.match_trackers,
            LabelField::MatchName => &self.match_name,
        }
    }

    #[must_use]
    pub fn from_label(label: &ProtoLabel) -> Self {
        let numbers = |value: u64| {
            if value == 0 {
                String::new()
            } else {
                value.to_string()
            }
        };
        Self {
            name: label.name.clone(),
            color: label.color.clone(),
            move_completed_path: label.move_completed_path.clone(),
            download_rate_limit: numbers(label.download_rate_limit),
            upload_rate_limit: numbers(label.upload_rate_limit),
            connections_limit: numbers(u64::from(label.connections_limit)),
            upload_slots_limit: numbers(u64::from(label.upload_slots_limit)),
            match_trackers: label.match_trackers.join(", "),
            match_name: label.match_name.clone(),
        }
    }

    /// Converts the form into the RPC message, applying the same rules as the
    /// daemon so obvious mistakes are caught before the round-trip.
    pub fn to_label(&self) -> Result<ProtoLabel, String> {
        let name = self.name.trim().to_ascii_lowercase();
        if name.is_empty() || name.len() > 64 {
            return Err("Label name must be 1–64 characters".to_owned());
        }
        if !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
        {
            return Err("Label name may contain letters, digits, '.', '_' and '-'".to_owned());
        }
        let color = self.color.trim().to_owned();
        if !color.is_empty() {
            let hex = color.strip_prefix('#').unwrap_or("");
            if !(hex.len() == 6 || hex.len() == 8)
                || !hex.chars().all(|character| character.is_ascii_hexdigit())
            {
                return Err("Colour must be #RRGGBB or #RRGGBBAA".to_owned());
            }
        }
        let trackers: Vec<String> = self
            .match_trackers
            .split(',')
            .map(str::trim)
            .filter(|tracker| !tracker.is_empty())
            .map(str::to_owned)
            .collect();
        Ok(ProtoLabel {
            name,
            color,
            move_completed_path: self.move_completed_path.trim().to_owned(),
            download_rate_limit: parse_number(
                &self.download_rate_limit,
                "Download limit",
                u64::MAX,
            )?,
            upload_rate_limit: parse_number(&self.upload_rate_limit, "Upload limit", u64::MAX)?,
            connections_limit: parse_number(
                &self.connections_limit,
                "Connections limit",
                u64::from(u32::MAX),
            )? as u32,
            upload_slots_limit: parse_number(
                &self.upload_slots_limit,
                "Upload slots limit",
                u64::from(u32::MAX),
            )? as u32,
            match_trackers: trackers,
            match_name: self.match_name.trim().to_owned(),
        })
    }
}

/// Everything the settings modal shows.
#[derive(Debug, Clone)]
pub struct Settings {
    pub tab: Tab,
    pub loading: bool,
    pub message: String,
    pub error: Option<String>,
    pub revision: u64,
    pub env_locked: Vec<String>,
    pub raw: String,
    pub baseline: ConfigForm,
    pub form: ConfigForm,
    pub autoadd: AutoAddForm,
    pub autoadd_baseline: AutoAddForm,
    pub autoadd_selected: Option<usize>,
    pub autoadd_status: Vec<AutoAddStatusRow>,
    pub hooks: HookForm,
    pub hooks_baseline: HookForm,
    pub hooks_selected: Option<usize>,
    pub hook_runs: Vec<HookRunRow>,
    pub hook_pending: u32,
    pub labels: Vec<LabelRow>,
    pub label_selected: Option<String>,
    pub label_new: bool,
    pub label_form: LabelForm,
    pub label_baseline: LabelForm,
    pub confirming_delete: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            tab: Tab::Connection,
            loading: false,
            message: String::new(),
            error: None,
            revision: 0,
            env_locked: Vec::new(),
            raw: String::new(),
            baseline: ConfigForm::default(),
            form: ConfigForm::default(),
            autoadd: AutoAddForm::default(),
            autoadd_baseline: AutoAddForm::default(),
            autoadd_selected: None,
            autoadd_status: Vec::new(),
            hooks: HookForm::default(),
            hooks_baseline: HookForm::default(),
            hooks_selected: None,
            hook_runs: Vec::new(),
            hook_pending: 0,
            labels: Vec::new(),
            label_selected: None,
            label_new: false,
            label_form: LabelForm::default(),
            label_baseline: LabelForm::default(),
            confirming_delete: false,
        }
    }
}

impl Settings {
    /// Replaces the loaded configuration snapshot and resets the form.
    pub fn apply_snapshot(
        &mut self,
        toml: &str,
        revision: u64,
        env_locked: Vec<String>,
    ) -> Result<(), String> {
        let form = ConfigForm::from_toml(toml)?;
        let autoadd = AutoAddForm::from_toml(toml)?;
        let hooks = HookForm::from_toml(toml)?;
        self.raw = toml.to_owned();
        self.revision = revision;
        self.env_locked = env_locked;
        self.baseline = form.clone();
        self.form = form;
        self.autoadd_baseline = autoadd.clone();
        self.autoadd = autoadd;
        self.autoadd_selected = None;
        self.hooks_baseline = hooks.clone();
        self.hooks = hooks;
        self.hooks_selected = None;
        Ok(())
    }

    #[must_use]
    pub fn dirty(&self) -> bool {
        self.form != self.baseline
    }

    /// Whether the given tab has unsaved edits.
    #[must_use]
    pub fn dirty_tab(&self, tab: Tab) -> bool {
        match tab {
            Tab::AutoAdd => self.autoadd != self.autoadd_baseline,
            Tab::Hooks => self.hooks != self.hooks_baseline,
            Tab::Labels => self.label_dirty(),
            Tab::Connection | Tab::Session => self.dirty(),
        }
    }

    /// Renders the whole configuration document: the typed fields plus the
    /// watch directory table.
    pub fn render(&self) -> Result<String, String> {
        let mut doc: DocumentMut = self
            .form
            .render(&self.raw)?
            .parse()
            .map_err(|error| format!("invalid TOML: {error}"))?;
        self.autoadd.render_into(&mut doc)?;
        self.hooks.render_into(&mut doc)?;
        Ok(doc.to_string())
    }

    /// The selected watch row.
    #[must_use]
    pub fn selected_watch(&self) -> Option<&WatchRow> {
        let index = self.autoadd_selected?;
        self.autoadd.rows.get(index)
    }

    /// The selected watch row, mutably.
    pub fn selected_watch_mut(&mut self) -> Option<&mut WatchRow> {
        let index = self.autoadd_selected?;
        self.autoadd.rows.get_mut(index)
    }

    /// The selected hook row.
    #[must_use]
    pub fn selected_hook(&self) -> Option<&HookRow> {
        let index = self.hooks_selected?;
        self.hooks.rows.get(index)
    }

    /// The selected hook row, mutably.
    pub fn selected_hook_mut(&mut self) -> Option<&mut HookRow> {
        let index = self.hooks_selected?;
        self.hooks.rows.get_mut(index)
    }

    /// Adds a hook row and selects it.
    pub fn add_hook(&mut self) {
        self.hooks_selected = Some(self.hooks.add());
    }

    /// Removes the selected hook row.
    pub fn remove_hook(&mut self) {
        let Some(index) = self.hooks_selected else {
            return;
        };
        self.hooks.remove(index);
        self.hooks_selected = if self.hooks.rows.is_empty() {
            None
        } else {
            Some(index.min(self.hooks.rows.len() - 1))
        };
    }

    /// Adds a watch row and selects it.
    pub fn add_watch(&mut self) {
        self.autoadd_selected = Some(self.autoadd.add());
    }

    /// Removes the selected watch row.
    pub fn remove_watch(&mut self) {
        let Some(index) = self.autoadd_selected else {
            return;
        };
        self.autoadd.remove(index);
        self.autoadd_selected = if self.autoadd.rows.is_empty() {
            None
        } else {
            Some(index.min(self.autoadd.rows.len() - 1))
        };
    }

    #[must_use]
    pub fn label_dirty(&self) -> bool {
        self.label_form != self.label_baseline
    }

    #[must_use]
    pub fn locked(&self, field: ConfigField) -> bool {
        let key = field.locked_key();
        !key.is_empty() && self.env_locked.iter().any(|locked| locked == key)
    }

    #[must_use]
    pub fn current_label(&self) -> Option<&LabelRow> {
        let selected = self.label_selected.as_deref()?;
        self.labels.iter().find(|row| row.label.name == selected)
    }

    /// Loads one label into the editor.
    pub fn select_label(&mut self, name: &str) {
        let Some(row) = self.labels.iter().find(|row| row.label.name == name) else {
            return;
        };
        self.label_selected = Some(row.label.name.clone());
        self.label_new = false;
        self.label_form = LabelForm::from_label(&row.label);
        self.label_baseline = self.label_form.clone();
        self.confirming_delete = false;
        self.error = None;
    }

    /// Clears the editor for a new label.
    pub fn new_label(&mut self) {
        self.label_selected = None;
        self.label_new = true;
        self.label_form = LabelForm::default();
        self.label_baseline = LabelForm::default();
        self.confirming_delete = false;
        self.error = None;
    }

    /// Replaces the label list, keeping the editor untouched.
    ///
    /// A selected label that disappeared (deleted on another client) resets the
    /// editor so the next Save cannot resurrect it by accident.
    pub fn set_rows(&mut self, rows: Vec<LabelRow>) {
        self.labels = rows;
        if let Some(selected) = self.label_selected.clone() {
            if !self.labels.iter().any(|row| row.label.name == selected) {
                self.label_selected = None;
                self.label_new = false;
                self.label_form = LabelForm::default();
                self.label_baseline = LabelForm::default();
                self.confirming_delete = false;
            }
        }
    }

    /// Inserts or replaces one row after a successful save.
    pub fn upsert_row(&mut self, label: ProtoLabel, torrent_count: u32) {
        let name = label.name.clone();
        match self.labels.iter_mut().find(|row| row.label.name == name) {
            Some(row) => {
                row.label = label.clone();
                row.torrent_count = torrent_count;
            }
            None => self.labels.push(LabelRow {
                label: label.clone(),
                torrent_count,
            }),
        }
        self.labels
            .sort_by(|left, right| left.label.name.cmp(&right.label.name));
        self.label_selected = Some(name.clone());
        self.label_new = false;
        self.label_form = LabelForm::from_label(&label);
        self.label_baseline = self.label_form.clone();
        self.confirming_delete = false;
    }

    /// Drops one row after a successful delete.
    pub fn remove_row(&mut self, name: &str) {
        self.labels.retain(|row| row.label.name != name);
        if self.label_selected.as_deref() == Some(name) {
            self.label_selected = None;
            self.label_form = LabelForm::default();
            self.label_baseline = LabelForm::default();
        }
        self.confirming_delete = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_document_means_defaults() {
        let form = ConfigForm::from_toml("").expect("empty config parses");
        assert_eq!(form, ConfigForm::default());
        assert!(form.queue_enabled, "the daemon defaults the queue to on");
        assert!(!form.seed_mode);
    }

    #[test]
    fn render_preserves_comments_and_untouched_keys() {
        let raw = "# keep me\ndata_dir = \"/srv/ferrent\"\n\n[session]\n# limits\nupload_rate_limit = 10\n";
        let mut form = ConfigForm::from_toml(raw).expect("config");
        form.upload_rate_limit = "2048".to_owned();
        form.active_limit = "12".to_owned();
        let rendered = form.render(raw).expect("render");

        assert!(rendered.contains("# keep me"), "{rendered}");
        assert!(rendered.contains("# limits"), "{rendered}");
        assert!(
            rendered.contains("data_dir = \"/srv/ferrent\""),
            "{rendered}"
        );
        assert!(rendered.contains("upload_rate_limit = 2048"), "{rendered}");
        assert!(rendered.contains("active_limit = 12"), "{rendered}");

        let reloaded = ConfigForm::from_toml(&rendered).expect("reparse");
        assert_eq!(reloaded, form);
    }

    #[test]
    fn empty_text_removes_the_key() {
        let raw = "data_dir = \"/srv/ferrent\"\n";
        let mut form = ConfigForm::from_toml(raw).expect("config");
        form.data_dir.clear();
        let rendered = form.render(raw).expect("render");
        assert!(!rendered.contains("data_dir"), "{rendered}");
    }

    #[test]
    fn invalid_values_are_rejected_before_saving() {
        let mut form = ConfigForm {
            bind: "localhost".to_owned(),
            ..ConfigForm::default()
        };
        assert!(form.render("").is_err());
        form.bind = "127.0.0.1:0".to_owned();
        assert!(form.render("").is_err());
        form.bind = String::new();
        form.download_rate_limit = "fast".to_owned();
        assert!(form.render("").is_err());
        form.download_rate_limit = String::new();
        assert!(form.render("").is_ok());
    }

    #[test]
    fn env_locked_fields_are_reported() {
        let mut settings = Settings::default();
        settings
            .apply_snapshot("", 3, vec!["bind".to_owned(), "data_dir".to_owned()])
            .expect("snapshot");
        assert_eq!(settings.revision, 3);
        assert!(settings.locked(ConfigField::Bind));
        assert!(settings.locked(ConfigField::DataDir));
        assert!(!settings.locked(ConfigField::DownloadDir));
        assert!(!settings.locked(ConfigField::ActiveLimit));
        assert!(!settings.dirty());
    }

    #[test]
    fn label_form_round_trips_and_splits_trackers() {
        let form = LabelForm {
            name: " Movies ".to_owned(),
            color: "#112233".to_owned(),
            move_completed_path: "/mnt/media/movies".to_owned(),
            download_rate_limit: "1024".to_owned(),
            upload_rate_limit: String::new(),
            connections_limit: "40".to_owned(),
            upload_slots_limit: "4".to_owned(),
            match_trackers: " tracker.example , other.example ".to_owned(),
            match_name: ".*\\.mkv".to_owned(),
        };
        let label = form.to_label().expect("valid label");
        assert_eq!(label.name, "movies");
        assert_eq!(
            label.match_trackers,
            vec!["tracker.example".to_owned(), "other.example".to_owned()]
        );
        assert_eq!(label.connections_limit, 40);
        assert_eq!(label.upload_slots_limit, 4);
        assert_eq!(label.download_rate_limit, 1024);
        assert_eq!(label.upload_rate_limit, 0);

        let back = LabelForm::from_label(&label);
        assert_eq!(back.match_trackers, "tracker.example, other.example");
        assert_eq!(back.connections_limit, "40");
        assert_eq!(back.upload_slots_limit, "4");
        assert_eq!(back.download_rate_limit, "1024");
        assert!(back.upload_rate_limit.is_empty());
    }

    #[test]
    fn label_validation_matches_the_daemon() {
        let mut form = LabelForm::default();
        assert!(form.to_label().is_err(), "an empty name is invalid");

        form.name = "with space".to_owned();
        assert!(form.to_label().is_err());

        form.name = "movies".to_owned();
        form.color = "red".to_owned();
        assert!(form.to_label().is_err());

        form.color = "#a1b2c3".to_owned();
        assert!(form.to_label().is_ok());

        form.connections_limit = "-1".to_owned();
        assert!(form.to_label().is_err());
    }

    #[test]
    fn autoadd_rows_round_trip_through_the_document() {
        let raw = "# keep this comment\ndata_dir = \"/srv/ferrent\"\n\n[[autoadd]]\npath = \"/data/watch\"\nlabel = \"movies\"\non_success = \"move\"\nmove_to = \"/data/done\"\n\n[[autoadd]]\npath = \"/data/watch2\"\nenabled = false\nadd_paused = true\n";
        let form = ConfigForm::from_toml(raw).expect("config");
        let autoadd = AutoAddForm::from_toml(raw).expect("autoadd");
        assert_eq!(autoadd.rows.len(), 2);
        assert_eq!(autoadd.rows[0].label, "movies");
        assert_eq!(autoadd.rows[0].on_success, SuccessAction::Move);
        assert!(!autoadd.rows[1].enabled);
        assert!(autoadd.rows[1].add_paused);

        let mut settings = Settings {
            raw: raw.to_owned(),
            form,
            autoadd,
            ..Settings::default()
        };
        settings.autoadd.rows[0].label = "films".to_owned();
        settings.autoadd.rows[1].min_free_bytes = "1048576".to_owned();
        settings.autoadd.rows.push(WatchRow {
            path: "/data/watch3".to_owned(),
            enabled: true,
            on_success: SuccessAction::Keep,
            ..WatchRow::default()
        });
        let rendered = settings.render().expect("render");
        assert!(rendered.contains("# keep this comment"), "{rendered}");
        assert!(rendered.contains("label = \"films\""), "{rendered}");
        assert!(rendered.contains("path = \"/data/watch3\""), "{rendered}");
        assert_eq!(
            AutoAddForm::from_toml(&rendered).expect("reparse"),
            settings.autoadd
        );
    }

    #[test]
    fn hook_rows_round_trip_through_the_document() {
        let raw = "[[hook]]\nevent = \"finished\"\ncommand = \"/usr/local/bin/notify\"\ntimeout_secs = 5\n\n[[hook]]\nevent = \"added\"\ncommand = \"/bin/echo\"\nargs = [\"{id}\", \"{name}\"]\nshell = true\nenabled = false\n";
        let mut settings = Settings {
            raw: raw.to_owned(),
            ..Settings::default()
        };
        settings
            .apply_snapshot(raw, 1, Vec::new())
            .expect("snapshot");
        assert_eq!(settings.hooks.rows.len(), 2);
        assert_eq!(settings.hooks.rows[0].event, HookEvent::Finished);
        assert_eq!(settings.hooks.rows[0].timeout_secs, "5");
        assert_eq!(settings.hooks.rows[1].args, "{id} {name}");
        assert!(!settings.dirty_tab(Tab::Hooks));

        settings.hooks.rows[1].enabled = true;
        settings.hooks.rows[0].timeout_secs = "30".to_owned();
        assert!(settings.dirty_tab(Tab::Hooks));
        let rendered = settings.render().expect("render");
        assert!(rendered.contains("timeout_secs = 30"), "{rendered}");
        assert!(
            rendered.contains("args = [\"{id}\", \"{name}\"]"),
            "{rendered}"
        );
        assert_eq!(
            HookForm::from_toml(&rendered).expect("reparse"),
            settings.hooks
        );
    }

    #[test]
    fn hook_validation_reports_the_row() {
        let mut form = HookForm::default();
        form.add();
        assert!(form.validate().is_err(), "empty command");
        form.rows[0].command = "/bin/true".to_owned();
        assert!(form.validate().is_ok());
        form.rows[0].timeout_secs = "0".to_owned();
        assert!(form.validate().is_err());
        form.rows[0].timeout_secs = "nope".to_owned();
        assert!(form.validate().is_err());
        form.rows[0].timeout_secs = "60".to_owned();
        assert!(form.validate().is_ok());

        let mut settings = Settings::default();
        settings.add_hook();
        assert_eq!(settings.hooks_selected, Some(0));
        assert!(settings.dirty_tab(Tab::Hooks));
        settings.remove_hook();
        assert!(settings.hooks.rows.is_empty());
        assert!(!settings.dirty_tab(Tab::Hooks));
    }

    #[test]
    fn hook_run_summary_shows_the_outcome() {
        let run = HookRunRow {
            event: "finished".to_owned(),
            torrent_name: "movie".to_owned(),
            succeeded: true,
            duration_ms: 12,
            ..HookRunRow::default()
        };
        assert_eq!(run.summary(), "finished · movie · ok · 12 ms");
        let failed = HookRunRow {
            exit_code: 3,
            succeeded: false,
            ..run.clone()
        };
        assert_eq!(failed.summary(), "finished · movie · exit 3 · 12 ms");
        let timed_out = HookRunRow {
            timed_out: true,
            stderr: "killed".to_owned(),
            ..run
        };
        assert_eq!(timed_out.summary(), "finished · movie · timeout · 12 ms");
        assert_eq!(timed_out.detail(), Some("killed"));
    }

    #[test]
    fn autoadd_validation_reports_the_row() {
        let mut form = AutoAddForm::default();
        assert!(form.validate().is_ok(), "an empty table is valid");
        form.add();
        assert!(form.validate().is_err(), "empty path");
        form.rows[0].path = "/data/watch".to_owned();
        assert!(form.validate().is_ok());
        form.rows[0].on_success = SuccessAction::Move;
        assert!(form.validate().is_err(), "move without a destination");
        form.rows[0].move_to = "/data/watch".to_owned();
        assert!(form.validate().is_err(), "same destination");
        form.rows[0].move_to = "/data/done".to_owned();
        assert!(form.validate().is_ok());
        form.rows.push(WatchRow {
            path: "/data/watch".to_owned(),
            ..WatchRow::default()
        });
        assert!(form.validate().is_err(), "duplicate path");

        let mut settings = Settings::default();
        settings.add_watch();
        assert_eq!(settings.autoadd_selected, Some(0));
        assert!(settings.dirty_tab(Tab::AutoAdd));
        settings.remove_watch();
        assert!(settings.autoadd.rows.is_empty());
        assert_eq!(settings.autoadd_selected, None);
        assert!(!settings.dirty_tab(Tab::AutoAdd));
    }

    #[test]
    fn autoadd_status_summary_counts_the_runs() {
        let row = AutoAddStatusRow {
            added: 3,
            duplicates: 1,
            ..AutoAddStatusRow::default()
        };
        assert_eq!(row.summary(), "3 added · 1 duplicate");
        assert_eq!(AutoAddStatusRow::default().summary(), "idle");
    }

    #[test]
    fn label_rows_follow_the_saved_definition() {
        let mut settings = Settings::default();
        settings.upsert_row(
            ProtoLabel {
                name: "movies".to_owned(),
                color: "#112233".to_owned(),
                ..ProtoLabel::default()
            },
            3,
        );
        let row = settings.current_label().expect("selected row");
        assert_eq!(row.torrent_count, 3);
        assert!(!settings.label_dirty());

        settings.label_form.color = "#445566".to_owned();
        assert!(settings.label_dirty());
        settings.select_label("movies");
        assert!(!settings.label_dirty(), "reselecting resets the baseline");

        settings.remove_row("movies");
        assert!(settings.current_label().is_none());
        assert!(settings.labels.is_empty());
    }
}
