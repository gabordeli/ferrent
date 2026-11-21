//! Virtualized torrent table drawn on an iced canvas.
//!
//! Only the visible rows are shaped and painted; the rows themselves come
//! from a server-side page around the viewport.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use ferrent_protocol::v1::{SortField, Torrent, TorrentState};
use iced::widget::canvas::{self, Frame, Text};
use iced::widget::text::{LineHeight, Shaping};
use iced::{Color, Font, Pixels, Point, Rectangle, Size, Theme, alignment, mouse};

pub const TABLE_WIDTH: f32 = 746.0;
pub const TABLE_HEIGHT: f32 = 460.0;
pub const HEADER_HEIGHT: f32 = 30.0;
pub const ROW_HEIGHT: f32 = 24.0;
pub const VISIBLE_ROWS: usize = 64;

const MENU_WIDTH: f32 = 160.0;
const MENU_ITEM_HEIGHT: f32 = 26.0;
const MENU_PADDING: f32 = 4.0;
const MENU_HEIGHT: f32 = MENU_PADDING * 2.0 + MENU_ITEM_HEIGHT * 3.0;

const BG_HEADER: Color = crate::theme::BG_SECONDARY;
const BG_ROW: Color = crate::theme::BG;
const BG_ROW_ALT: Color = Color::from_rgb(0.055, 0.090, 0.153);
const BG_SELECTED: Color = crate::theme::DEEP;
const BG_HOVER: Color = crate::theme::SURFACE;
const TEXT: Color = crate::theme::TEXT;
const TEXT_DIM: Color = crate::theme::TEXT_SECONDARY;
const TEXT_HEADER: Color = crate::theme::TEXT_MUTED;
const BAR_TRACK: Color = crate::theme::SURFACE;
const TRACK: Color = crate::theme::SURFACE;
const THUMB: Color = crate::theme::BORDER;
const MENU_BG: Color = crate::theme::SURFACE;
const MENU_BORDER: Color = crate::theme::BORDER;

const BAR_HEIGHT: f32 = 8.0;

/// One table column. `State` is displayed but not server-sortable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Column {
    Id,
    Name,
    Label,
    Size,
    SavePath,
    State,
    Progress,
    Download,
    Upload,
    Seeds,
    Peers,
    Eta,
}

impl Column {
    /// Every column, in display order.
    pub const ALL: [Self; 12] = [
        Self::Id,
        Self::Name,
        Self::Label,
        Self::Size,
        Self::SavePath,
        Self::State,
        Self::Progress,
        Self::Download,
        Self::Upload,
        Self::Seeds,
        Self::Peers,
        Self::Eta,
    ];

    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Id => "ID",
            Self::Name => "NAME",
            Self::Label => "LABEL",
            Self::Size => "SIZE",
            Self::SavePath => "SAVE PATH",
            Self::State => "STATUS",
            Self::Progress => "PROGRESS",
            Self::Download => "DOWN",
            Self::Upload => "UP",
            Self::Seeds => "SEEDS",
            Self::Peers => "PEERS",
            Self::Eta => "ETA",
        }
    }

    #[must_use]
    pub fn sort_field(self) -> Option<SortField> {
        match self {
            Self::Id => Some(SortField::Id),
            Self::Name => Some(SortField::Name),
            Self::Label => Some(SortField::Label),
            Self::Progress => Some(SortField::Progress),
            Self::Download => Some(SortField::DownloadRate),
            Self::Upload => Some(SortField::UploadRate),
            _ => None,
        }
    }

    fn default_width(self) -> f32 {
        match self {
            Self::Id => 42.0,
            Self::Name => 260.0,
            Self::Label => 88.0,
            Self::Size => 74.0,
            Self::SavePath => 168.0,
            Self::State => 96.0,
            Self::Progress => 128.0,
            Self::Download | Self::Upload => 86.0,
            Self::Seeds | Self::Peers => 52.0,
            Self::Eta => 52.0,
        }
    }

    const fn min_width(self) -> f32 {
        match self {
            Self::Id => 28.0,
            Self::Name => 120.0,
            Self::Progress => 92.0,
            Self::SavePath => 90.0,
            _ => 44.0,
        }
    }

    /// The name column soaks up the leftover width; everything else is a
    /// fixed, user-resizable width.
    #[must_use]
    pub fn flexible(self) -> bool {
        self == Self::Name
    }

    #[must_use]
    pub fn right_aligned(self) -> bool {
        matches!(
            self,
            Self::Id
                | Self::Size
                | Self::Download
                | Self::Upload
                | Self::Seeds
                | Self::Peers
                | Self::Eta
        )
    }
}

const GAP: f32 = 10.0;
/// How close to a boundary counts as a resize grab.
pub const RESIZE_GRAB: f32 = 4.0;

/// Where a visible column starts and how wide it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    pub column: Column,
    pub x: f32,
    pub width: f32,
}

/// Which columns are visible and how wide they are. Held by the app so the
/// layout survives a reconnect and the view stays a pure function of state.
#[derive(Debug, Clone, Default)]
pub struct ColumnLayout {
    hidden: BTreeSet<Column>,
    widths: BTreeMap<Column, f32>,
}

impl ColumnLayout {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn is_visible(&self, column: Column) -> bool {
        !self.hidden.contains(&column)
    }

    #[must_use]
    pub fn width_of(&self, column: Column) -> f32 {
        self.widths
            .get(&column)
            .copied()
            .unwrap_or_else(|| column.default_width())
    }

    /// The name column is never hidden: the table would lose its identity.
    pub fn toggle(&mut self, column: Column) {
        if column == Column::Name {
            return;
        }
        if self.hidden.remove(&column) {
            // shown again
        } else {
            self.hidden.insert(column);
        }
    }

    pub fn set_width(&mut self, column: Column, width: f32) {
        if column.flexible() {
            return;
        }
        self.widths
            .insert(column, width.clamp(column.min_width(), 480.0));
    }

    /// Visible columns in display order, at their positions for this width.
    #[must_use]
    pub fn slots(&self, width: f32) -> Vec<Slot> {
        let visible: Vec<Column> = Column::ALL
            .iter()
            .copied()
            .filter(|column| self.is_visible(*column))
            .collect();
        let fixed: f32 = visible
            .iter()
            .filter(|column| !column.flexible())
            .map(|column| self.width_of(*column))
            .sum();
        let gaps = GAP * visible.len().saturating_sub(1) as f32;
        let name_width = if visible.contains(&Column::Name) {
            (width - fixed - gaps).max(Column::Name.min_width())
        } else {
            0.0
        };
        let mut x = 0.0;
        let mut slots = Vec::with_capacity(visible.len());
        for column in visible {
            let slot_width = if column.flexible() {
                name_width
            } else {
                self.width_of(column)
            };
            slots.push(Slot {
                column,
                x,
                width: slot_width,
            });
            x += slot_width + GAP;
        }
        slots
    }

    /// The column under a header x coordinate.
    #[must_use]
    pub fn column_at(&self, x: f32, width: f32) -> Option<Column> {
        self.slots(width)
            .into_iter()
            .find(|slot| x >= slot.x && x < slot.x + slot.width)
            .map(|slot| slot.column)
    }

    /// The column whose right edge is within grabbing distance of `x`, so a
    /// drag can start on a boundary even when the pointer sits in the gap.
    #[must_use]
    pub fn resize_handle(&self, x: f32, width: f32) -> Option<Column> {
        self.slots(width)
            .into_iter()
            .filter(|slot| !slot.column.flexible())
            .find(|slot| (x - (slot.x + slot.width)).abs() <= RESIZE_GRAB)
            .map(|slot| slot.column)
    }
}

/// Column menu geometry: a fixed-size panel anchored under the header.
const COLUMN_MENU_WIDTH: f32 = 184.0;
const COLUMN_MENU_ROW: f32 = 21.0;
const COLUMN_MENU_PAD: f32 = 6.0;

#[must_use]
pub fn column_menu_rect(origin: Point, bounds_width: f32) -> Rectangle {
    let count = Column::ALL.len() as f32;
    let x = origin
        .x
        .min((bounds_width - COLUMN_MENU_WIDTH - 4.0).max(0.0));
    Rectangle::new(
        Point::new(x, origin.y),
        Size::new(
            COLUMN_MENU_WIDTH,
            count * COLUMN_MENU_ROW + COLUMN_MENU_PAD * 2.0,
        ),
    )
}

/// Which column the pointer is over inside the open menu, if any.
#[must_use]
pub fn column_menu_item(origin: Point, bounds_width: f32, point: Point) -> Option<Column> {
    let rect = column_menu_rect(origin, bounds_width);
    if !rect.contains(point) {
        return None;
    }
    let row = ((point.y - rect.y - COLUMN_MENU_PAD) / COLUMN_MENU_ROW).floor();
    if row < 0.0 {
        return None;
    }
    Column::ALL.get(row as usize).copied()
}

/// Accent colour per torrent state; used for the state marker and the bar.
#[must_use]
pub fn state_color(state: i32) -> Color {
    use crate::theme;
    match TorrentState::try_from(state).unwrap_or(TorrentState::Unspecified) {
        TorrentState::Downloading => theme::DOWNLOADING,
        TorrentState::Seeding => theme::SEEDING,
        TorrentState::Queued => theme::QUEUED,
        TorrentState::Checking => theme::CHECKING,
        TorrentState::Paused => theme::PAUSED,
        TorrentState::Moving => theme::MOVING,
        TorrentState::Error => theme::ERROR,
        TorrentState::Unspecified => theme::TEXT_MUTED,
    }
}

#[must_use]
pub fn first_visible(scroll: f32) -> usize {
    (scroll.max(0.0) / ROW_HEIGHT).floor() as usize
}

#[must_use]
pub fn visible_range(scroll: f32, total: usize) -> Range<usize> {
    let first = first_visible(scroll).min(total);
    let end = (first + VISIBLE_ROWS).min(total);
    first..end
}

/// Maps a canvas point to a torrent row, excluding the header.
#[must_use]
pub fn row_at(point: Point, scroll: f32, total: usize) -> Option<usize> {
    if point.y < HEADER_HEIGHT {
        return None;
    }
    let y = point.y - HEADER_HEIGHT + scroll.max(0.0);
    if y < 0.0 {
        return None;
    }
    let index = (y / ROW_HEIGHT).floor() as usize;
    (index < total).then_some(index)
}

#[must_use]
pub fn max_scroll(total: usize) -> f32 {
    (total as f32 * ROW_HEIGHT - (TABLE_HEIGHT - HEADER_HEIGHT)).max(0.0)
}

/// Keeps a context menu inside the table.
#[must_use]
pub fn clamp_menu(x: f32, y: f32) -> (f32, f32) {
    (
        x.clamp(0.0, (TABLE_WIDTH - MENU_WIDTH - 4.0).max(0.0)),
        y.clamp(0.0, (TABLE_HEIGHT - MENU_HEIGHT - 4.0).max(0.0)),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    Pause,
    Resume,
    Label,
    Remove,
}

#[derive(Debug, Clone, Copy)]
pub struct ContextMenu {
    pub row: u64,
    pub x: f32,
    pub y: f32,
    pub paused: bool,
}

#[must_use]
pub fn menu_rect(menu: &ContextMenu) -> Rectangle {
    Rectangle::new(
        Point::new(menu.x, menu.y),
        Size::new(MENU_WIDTH, MENU_HEIGHT),
    )
}

#[must_use]
pub fn menu_action_at(menu: &ContextMenu, point: Point) -> Option<MenuAction> {
    if !menu_rect(menu).contains(point) {
        return None;
    }
    let index = ((point.y - menu.y - MENU_PADDING) / MENU_ITEM_HEIGHT).floor() as usize;
    match index {
        0 if menu.paused => Some(MenuAction::Resume),
        0 => Some(MenuAction::Pause),
        1 => Some(MenuAction::Label),
        2 => Some(MenuAction::Remove),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub enum TableMessage {
    Scroll(f32),
    Pressed(Point),
    RightPressed(Point),
    /// Show or hide a column (from the header's column chooser).
    ToggleColumn(Column),
    /// The column chooser was opened at this point, or closed with `None`.
    ColumnMenu(Option<Point>),
    /// Start dragging a column's right edge.
    ResizeStart(Column, f32),
    Resize(Column, f32),
    ResizeEnd,
}

pub struct Table<'a> {
    pub rows: &'a [Torrent],
    pub page_offset: usize,
    pub total: usize,
    pub scroll: f32,
    pub selected: &'a BTreeSet<u64>,
    pub sort: SortField,
    pub descending: bool,
    pub context: Option<ContextMenu>,
    pub layout: &'a ColumnLayout,
    /// Open column chooser, anchored at this point.
    pub column_menu: Option<Point>,
    /// Column currently being dragged wider/narrower.
    pub resizing: Option<Column>,
    /// label name → chip colour, built by the app for this frame.
    pub label_colors: BTreeMap<String, Color>,
}

impl canvas::Program<TableMessage> for Table<'_> {
    type State = ();

    fn update(
        &self,
        _state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<TableMessage>) {
        use canvas::event::Status;
        match event {
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let dy = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => y * ROW_HEIGHT * 3.0,
                    mouse::ScrollDelta::Pixels { y, .. } => y,
                };
                (Status::Captured, Some(TableMessage::Scroll(dy)))
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(button)) => {
                let Some(point) = cursor.position_in(bounds) else {
                    return (Status::Ignored, None);
                };
                match button {
                    mouse::Button::Left => {
                        if let Some(origin) = self.column_menu {
                            let action = column_menu_item(origin, bounds.width, point);
                            return (
                                Status::Captured,
                                Some(match action {
                                    Some(column) => TableMessage::ToggleColumn(column),
                                    None => TableMessage::ColumnMenu(None),
                                }),
                            );
                        }
                        if point.y < HEADER_HEIGHT {
                            if let Some(column) = self.layout.resize_handle(point.x, bounds.width) {
                                return (
                                    Status::Captured,
                                    Some(TableMessage::ResizeStart(column, point.x)),
                                );
                            }
                        }
                        (Status::Captured, Some(TableMessage::Pressed(point)))
                    }
                    mouse::Button::Right => {
                        if point.y < HEADER_HEIGHT {
                            (
                                Status::Captured,
                                Some(TableMessage::ColumnMenu(Some(point))),
                            )
                        } else {
                            (Status::Captured, Some(TableMessage::RightPressed(point)))
                        }
                    }
                    _ => (Status::Ignored, None),
                }
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let (Some(column), Some(point)) = (self.resizing, cursor.position_in(bounds)) {
                    return (
                        Status::Captured,
                        Some(TableMessage::Resize(column, point.x)),
                    );
                }
                (Status::Ignored, None)
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if self.resizing.is_some() {
                    return (Status::Captured, Some(TableMessage::ResizeEnd));
                }
                (Status::Ignored, None)
            }
            _ => (Status::Ignored, None),
        }
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let hovered = cursor
            .position_in(bounds)
            .and_then(|point| row_at(point, self.scroll, self.total));

        frame.fill_rectangle(
            Point::new(0.0, 0.0),
            Size::new(bounds.width, HEADER_HEIGHT),
            BG_HEADER,
        );
        // Leave room for the scrollbar: with a column layout the last
        // column would otherwise sit under it.
        let content_width = bounds.width - if self.total > 0 { 14.0 } else { 0.0 };
        self.draw_header(&mut frame, content_width);

        let range = visible_range(self.scroll, self.total);
        let page_end = self.page_offset + self.rows.len();
        for index in range {
            let y = HEADER_HEIGHT + index as f32 * ROW_HEIGHT - self.scroll;
            let torrent = (index >= self.page_offset && index < page_end)
                .then(|| &self.rows[index - self.page_offset]);
            let selected = torrent.is_some_and(|torrent| self.selected.contains(&torrent.id));
            let background = if selected {
                BG_SELECTED
            } else if hovered == Some(index) {
                BG_HOVER
            } else if index % 2 == 0 {
                BG_ROW
            } else {
                BG_ROW_ALT
            };
            frame.fill_rectangle(
                Point::new(0.0, y),
                Size::new(bounds.width, ROW_HEIGHT),
                background,
            );

            match torrent {
                Some(torrent) => draw_row(
                    &mut frame,
                    self.layout,
                    &self.label_colors,
                    torrent,
                    y,
                    content_width,
                ),
                None => {
                    frame.fill_text(Text {
                        content: "loading…".to_owned(),
                        position: Point::new(64.0, y + 4.0),
                        color: TEXT_DIM,
                        size: Pixels(13.0),
                        line_height: LineHeight::default(),
                        font: Font::MONOSPACE,
                        horizontal_alignment: alignment::Horizontal::Left,
                        vertical_alignment: alignment::Vertical::Top,
                        shaping: Shaping::Basic,
                    });
                }
            }
        }

        if self.total == 0 {
            frame.fill_text(Text {
                content: "No torrents yet; add a .torrent from the toolbar".to_owned(),
                position: Point::new(bounds.width / 2.0, 80.0),
                color: TEXT_DIM,
                size: Pixels(14.0),
                line_height: LineHeight::default(),
                font: Font::default(),
                horizontal_alignment: alignment::Horizontal::Center,
                vertical_alignment: alignment::Vertical::Top,
                shaping: Shaping::Basic,
            });
        }

        self.draw_scrollbar(&mut frame, bounds);
        if let Some(menu) = self.context {
            self.draw_menu(&mut frame, menu);
        }
        if let Some(origin) = self.column_menu {
            self.draw_column_menu(&mut frame, origin, bounds);
        }

        vec![frame.into_geometry()]
    }
}

impl Table<'_> {
    fn draw_header(&self, frame: &mut Frame, width: f32) {
        let marker = |field: SortField| {
            if self.sort == field {
                if self.descending { " v" } else { " ^" }
            } else {
                ""
            }
        };
        for slot in self.layout.slots(width) {
            let content = match slot.column.sort_field() {
                Some(field) => format!("{}{}", slot.column.title(), marker(field)),
                None => slot.column.title().to_owned(),
            };
            let (x, horizontal) = if slot.column.right_aligned() {
                (slot.x + slot.width, alignment::Horizontal::Right)
            } else {
                (slot.x, alignment::Horizontal::Left)
            };
            frame.fill_text(Text {
                content,
                position: Point::new(x, 8.0),
                color: TEXT_HEADER,
                size: Pixels(11.0),
                line_height: LineHeight::default(),
                font: crate::theme::semibold(),
                horizontal_alignment: horizontal,
                vertical_alignment: alignment::Vertical::Top,
                shaping: Shaping::Basic,
            });
        }
    }

    fn draw_column_menu(&self, frame: &mut Frame, origin: Point, bounds: Rectangle) {
        let rect = column_menu_rect(origin, bounds.width);
        frame.fill_rectangle(rect.position(), rect.size(), MENU_BG);
        frame.fill_rectangle(rect.position(), Size::new(1.0, rect.height), MENU_BORDER);
        for (index, column) in Column::ALL.iter().enumerate() {
            let y = rect.y + COLUMN_MENU_PAD + index as f32 * COLUMN_MENU_ROW;
            let check = if self.layout.is_visible(*column) {
                "x"
            } else {
                " "
            };
            frame.fill_text(Text {
                content: format!("[{check}] {}", column.title()),
                position: Point::new(rect.x + 10.0, y + 3.0),
                color: if self.layout.is_visible(*column) {
                    TEXT
                } else {
                    TEXT_HEADER
                },
                size: Pixels(11.0),
                line_height: LineHeight::default(),
                font: crate::theme::regular(),
                horizontal_alignment: alignment::Horizontal::Left,
                vertical_alignment: alignment::Vertical::Top,
                shaping: Shaping::Basic,
            });
        }
    }

    fn draw_scrollbar(&self, frame: &mut Frame, bounds: Rectangle) {
        if self.total == 0 {
            return;
        }
        let track_height = bounds.height - HEADER_HEIGHT;
        if track_height <= 0.0 {
            return;
        }
        frame.fill_rectangle(
            Point::new(bounds.width - 9.0, HEADER_HEIGHT),
            Size::new(6.0, track_height),
            TRACK,
        );
        let visible = VISIBLE_ROWS.min(self.total) as f32;
        let thumb_height = (track_height * visible / self.total as f32)
            .max(24.0)
            .min(track_height);
        let max = max_scroll(self.total);
        let fraction = if max > 0.0 {
            (self.scroll / max).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let thumb_y = HEADER_HEIGHT + fraction * (track_height - thumb_height);
        frame.fill_rectangle(
            Point::new(bounds.width - 9.0, thumb_y),
            Size::new(6.0, thumb_height),
            THUMB,
        );
    }

    fn draw_menu(&self, frame: &mut Frame, menu: ContextMenu) {
        let rect = menu_rect(&menu);
        frame.fill_rectangle(rect.position(), rect.size(), MENU_BG);
        frame.stroke_rectangle(
            rect.position(),
            rect.size(),
            canvas::Stroke::default()
                .with_color(MENU_BORDER)
                .with_width(1.0),
        );
        let labels = if menu.paused {
            ["Resume", "Set label…", "Remove"]
        } else {
            ["Pause", "Set label…", "Remove"]
        };
        for (index, label) in labels.iter().enumerate() {
            frame.fill_text(Text {
                content: (*label).to_owned(),
                position: Point::new(
                    menu.x + 10.0,
                    menu.y + MENU_PADDING + index as f32 * MENU_ITEM_HEIGHT + 5.0,
                ),
                color: TEXT,
                size: Pixels(13.0),
                line_height: LineHeight::default(),
                font: Font::default(),
                horizontal_alignment: alignment::Horizontal::Left,
                vertical_alignment: alignment::Vertical::Top,
                shaping: Shaping::Basic,
            });
        }
    }
}

fn draw_row(
    frame: &mut Frame,
    layout: &ColumnLayout,
    label_colors: &BTreeMap<String, Color>,
    torrent: &Torrent,
    y: f32,
    width: f32,
) {
    let accent = state_color(torrent.state);
    let text_y = y + 5.0;
    let regular = crate::theme::regular();
    let mono = Font::MONOSPACE;

    for slot in layout.slots(width) {
        let (content, color, size, font) = match slot.column {
            Column::Id => (torrent.id.to_string(), TEXT_DIM, 11.0, mono),
            Column::Name => (short_name(&torrent.name, 42), TEXT, 12.0, regular),
            Column::Label => {
                if torrent.label.is_empty() {
                    ("—".to_owned(), TEXT_DIM, 11.0, regular)
                } else {
                    (
                        short_name(&torrent.label, 14),
                        label_colors
                            .get(&torrent.label)
                            .copied()
                            .unwrap_or(TEXT_DIM),
                        11.0,
                        regular,
                    )
                }
            }
            Column::Size => (format_size(torrent.total_bytes), TEXT_DIM, 11.0, mono),
            Column::SavePath => (
                short_name(&folder_label(&torrent.save_path), 30),
                TEXT_DIM,
                11.0,
                regular,
            ),
            Column::State => {
                frame.fill_rectangle(Point::new(slot.x, y + 9.0), Size::new(6.0, 6.0), accent);
                (state_label(torrent.state).to_owned(), accent, 11.0, regular)
            }
            Column::Progress => {
                let bar = (slot.width - 44.0).clamp(24.0, 120.0);
                let progress = torrent.progress.clamp(0.0, 1.0);
                let bar_y = y + (ROW_HEIGHT - BAR_HEIGHT) / 2.0;
                frame.fill_rectangle(
                    Point::new(slot.x, bar_y),
                    Size::new(bar, BAR_HEIGHT),
                    BAR_TRACK,
                );
                let filled = (bar * progress).round();
                if filled > 0.0 {
                    frame.fill_rectangle(
                        Point::new(slot.x, bar_y),
                        Size::new(filled, BAR_HEIGHT),
                        accent,
                    );
                }
                cell(
                    frame,
                    format!("{}%", (progress * 100.0).round() as u32),
                    slot.x + bar + 6.0,
                    text_y,
                    TEXT_DIM,
                    11.0,
                    alignment::Horizontal::Left,
                    mono,
                );
                continue;
            }
            Column::Download => (format_rate(torrent.download_rate), TEXT, 11.0, mono),
            Column::Upload => (format_rate(torrent.upload_rate), TEXT, 11.0, mono),
            Column::Seeds => (torrent.seeds.to_string(), TEXT_DIM, 11.0, mono),
            Column::Peers => (torrent.peers.to_string(), TEXT_DIM, 11.0, mono),
            Column::Eta => (format_eta(torrent.eta_seconds), TEXT_DIM, 11.0, mono),
        };
        let (x, horizontal) = if slot.column.right_aligned() {
            (slot.x + slot.width, alignment::Horizontal::Right)
        } else if slot.column == Column::State {
            (slot.x + 10.0, alignment::Horizontal::Left)
        } else {
            (slot.x, alignment::Horizontal::Left)
        };
        cell(frame, content, x, text_y, color, size, horizontal, font);
    }
}

/// The tail of a save path: the torrent's own folder plus its parent, so a
/// download directory and a library directory stay distinguishable.
fn folder_label(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    let mut parts = trimmed.rsplit('/');
    match (parts.next(), parts.next()) {
        (Some(last), Some(parent)) if !parent.is_empty() => format!("{parent}/{last}"),
        (Some(last), _) => last.to_owned(),
        _ => trimmed.to_owned(),
    }
}

fn format_eta(seconds: u32) -> String {
    if seconds == 0 {
        "-".to_owned()
    } else if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h", seconds / 3600)
    } else {
        format!("{}d", seconds / 86_400)
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

#[allow(clippy::too_many_arguments)]
fn cell(
    frame: &mut Frame,
    content: String,
    x: f32,
    y: f32,
    color: Color,
    size: f32,
    horizontal: alignment::Horizontal,
    font: Font,
) {
    frame.fill_text(Text {
        content,
        position: Point::new(x, y),
        color,
        size: Pixels(size),
        line_height: LineHeight::default(),
        font,
        horizontal_alignment: horizontal,
        vertical_alignment: alignment::Vertical::Top,
        shaping: Shaping::Basic,
    });
}

fn short_name(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        name.to_owned()
    } else {
        let mut short: String = name.chars().take(max.saturating_sub(1)).collect();
        short.push('…');
        short
    }
}

#[must_use]
pub fn state_label(state: i32) -> &'static str {
    match TorrentState::try_from(state).unwrap_or(TorrentState::Unspecified) {
        TorrentState::Unspecified => "unknown",
        TorrentState::Queued => "queued",
        TorrentState::Checking => "checking",
        TorrentState::Downloading => "downloading",
        TorrentState::Seeding => "seeding",
        TorrentState::Paused => "paused",
        TorrentState::Moving => "moving",
        TorrentState::Error => "error",
    }
}

#[must_use]
pub fn format_rate(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B/s")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB/s", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MiB/s", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtualizes_only_the_visible_rows() {
        let range = visible_range(0.0, 4_000);
        assert!(range.len() <= VISIBLE_ROWS);
        assert_eq!(range.start, 0);

        let range = visible_range(max_scroll(4_000), 4_000);
        assert!(range.end == 4_000);
        assert!(range.len() <= VISIBLE_ROWS + 1);
    }

    #[test]
    fn maps_rows_and_headers() {
        assert_eq!(
            row_at(Point::new(20.0, HEADER_HEIGHT + 1.0), 0.0, 10),
            Some(0)
        );
        assert_eq!(
            row_at(Point::new(20.0, HEADER_HEIGHT + 1.0), ROW_HEIGHT * 3.0, 10),
            Some(3)
        );
        assert_eq!(row_at(Point::new(20.0, 2.0), 0.0, 10), None);
        let layout = ColumnLayout::new();
        let width = 1224.0;
        let name = layout
            .slots(width)
            .into_iter()
            .find(|slot| slot.column == Column::Name)
            .expect("name slot");
        assert_eq!(layout.column_at(12.0, width), Some(Column::Id));
        assert_eq!(layout.column_at(name.x + 2.0, width), Some(Column::Name));
        assert_eq!(
            layout
                .column_at(name.x + 2.0, width)
                .and_then(Column::sort_field),
            Some(SortField::Name)
        );
        // The name column soaks up the leftover width, so hiding a fixed
        // column moves the ones after it without ever hiding the name.
        let mut narrow = layout.clone();
        narrow.toggle(Column::SavePath);
        let slots = narrow.slots(width);
        assert!(slots.iter().all(|slot| slot.column != Column::SavePath));
        assert!(slots.iter().any(|slot| slot.column == Column::Name));
        assert!(slots.last().expect("slot").x + slots.last().unwrap().width <= width + 1.0);
    }

    #[test]
    fn columns_resize_with_a_floor() {
        let mut layout = ColumnLayout::new();
        let before = layout.width_of(Column::Size);
        layout.set_width(Column::Size, before + 30.0);
        assert_eq!(layout.width_of(Column::Size), before + 30.0);
        layout.set_width(Column::Size, 5.0);
        assert_eq!(layout.width_of(Column::Size), Column::Size.min_width());
        // The flexible column ignores explicit widths.
        layout.set_width(Column::Name, 999.0);
        assert_eq!(layout.width_of(Column::Name), Column::Name.default_width());
    }

    #[test]
    fn a_column_is_grabbed_by_its_right_edge() {
        let layout = ColumnLayout::new();
        let width = 1224.0;
        let size = layout
            .slots(width)
            .into_iter()
            .find(|slot| slot.column == Column::Size)
            .expect("size slot");
        assert_eq!(
            layout.resize_handle(size.x + size.width - 1.0, width),
            Some(Column::Size)
        );
        assert_eq!(layout.resize_handle(size.x + 2.0, width), None);
    }

    #[test]
    fn the_column_menu_marks_visible_columns() {
        let origin = Point::new(120.0, HEADER_HEIGHT);
        let layout = ColumnLayout::new();
        let rect = column_menu_rect(origin, 1224.0);
        assert!(rect.contains(Point::new(rect.x + 8.0, rect.y + 8.0)));
        // Rows are hit-tested in display order, so the first row is ID.
        assert_eq!(
            column_menu_item(origin, 1224.0, Point::new(rect.x + 12.0, rect.y + 8.0)),
            Some(Column::Id)
        );
        assert!(layout.is_visible(Column::Id));
    }

    #[test]
    fn menu_hit_testing_matches_drawn_labels() {
        let menu = ContextMenu {
            row: 7,
            x: 100.0,
            y: 100.0,
            paused: false,
        };
        assert_eq!(
            menu_action_at(&menu, Point::new(110.0, 108.0)),
            Some(MenuAction::Pause)
        );
        assert_eq!(
            menu_action_at(&menu, Point::new(110.0, 134.0)),
            Some(MenuAction::Label)
        );
        assert_eq!(
            menu_action_at(&menu, Point::new(110.0, 160.0)),
            Some(MenuAction::Remove)
        );
        assert_eq!(menu_action_at(&menu, Point::new(10.0, 10.0)), None);
    }
}
