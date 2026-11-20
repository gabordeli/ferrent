//! Settings workbench: the daemon configuration form and the label editor.
//!
//! The numeric fields are kept as strings so a partially typed value is never
//! lost. `ConfigForm::render` writes the whole document back through
//! `toml_edit`, which preserves comments and the keys the form does not touch.

use ferrent_protocol::v1::{Label as ProtoLabel, LabelEntry};
use toml_edit::{DocumentMut, Item, Table, value};

/// Fixed size of the settings modal, per the M4-T5 design.
pub const MODAL_WIDTH: f32 = 900.0;
pub const MODAL_HEIGHT: f32 = 640.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Connection,
    Session,
    Labels,
}

impl Tab {
    pub const ALL: [Self; 3] = [Self::Connection, Self::Session, Self::Labels];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Connection => "Connection",
            Self::Session => "Session",
            Self::Labels => "Labels",
        }
    }

    #[must_use]
    pub fn hint(self) -> &'static str {
        match self {
            Self::Connection => "Daemon paths and bind address; changes apply on restart.",
            Self::Session => "Engine limits; changes apply as soon as they are saved.",
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
        self.raw = toml.to_owned();
        self.revision = revision;
        self.env_locked = env_locked;
        self.baseline = form.clone();
        self.form = form;
        Ok(())
    }

    #[must_use]
    pub fn dirty(&self) -> bool {
        self.form != self.baseline
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
