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

pub const MENU_WIDTH: f32 = 160.0;
pub const MENU_ITEM_HEIGHT: f32 = 26.0;
pub const MENU_PADDING: f32 = 4.0;
pub const MENU_HEIGHT: f32 = MENU_PADDING * 2.0 + MENU_ITEM_HEIGHT * 8.0;

fn bg_header() -> Color {
    crate::theme::bg_secondary()
}
fn bg_row() -> Color {
    crate::theme::bg()
}
fn bg_row_alt() -> Color {
    let palette = crate::theme::current();
    crate::theme::mix(palette.bg, palette.surface, 0.35)
}
fn bg_selected() -> Color {
    crate::theme::deep()
}
fn bg_hover() -> Color {
    crate::theme::surface()
}
fn text_main() -> Color {
    crate::theme::text()
}
fn text_dim() -> Color {
    crate::theme::text_secondary()
}
fn text_header() -> Color {
    crate::theme::text_muted()
}
fn bar_track() -> Color {
    crate::theme::surface()
}
fn track() -> Color {
    crate::theme::surface()
}
fn thumb() -> Color {
    crate::theme::border()
}

const BAR_HEIGHT: f32 = 8.0;

/// One table column. `State` is displayed but not server-sortable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Column {
    /// Queue position while the torrent is still downloading; a dash once it
    /// seeds, because the number means nothing there.
    Position,
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
        Self::Position,
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
            Self::Position => "#",
            Self::Name => crate::i18n::t("NAME"),
            Self::Label => crate::i18n::t("LABEL"),
            Self::Size => crate::i18n::t("SIZE"),
            Self::SavePath => crate::i18n::t("SAVE PATH"),
            Self::State => crate::i18n::t("STATUS"),
            Self::Progress => crate::i18n::t("PROGRESS"),
            Self::Download => crate::i18n::t("DOWN"),
            Self::Upload => crate::i18n::t("UP"),
            Self::Seeds => crate::i18n::t("SEEDS"),
            Self::Peers => crate::i18n::t("PEERS"),
            Self::Eta => crate::i18n::t("ETA"),
        }
    }

    #[must_use]
    pub fn sort_field(self) -> Option<SortField> {
        match self {
            Self::Position => Some(SortField::Queue),
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
            Self::Position => 42.0,
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
            Self::Position => 28.0,
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
            Self::Position
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

    /// Narrowest sensible table: the visible fixed columns plus the name
    /// column at its minimum. A narrower viewport scrolls sideways.
    #[must_use]
    pub fn min_total(&self) -> f32 {
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
        fixed + gaps + Column::Name.min_width()
    }

    /// How far a table of the given usable width can scroll sideways.
    #[must_use]
    pub fn max_hscroll(&self, usable_width: f32) -> f32 {
        (self.min_total() - usable_width.max(1.0)).max(0.0)
    }

    /// Slots for a table that can be wider than the viewport: the columns are
    /// laid out at least at their minimum total, then shifted by `hscroll`.
    #[must_use]
    pub fn slots_scrolled(&self, width: f32, hscroll: f32) -> Vec<Slot> {
        let mut slots = self.slots(width.max(self.min_total()));
        for slot in &mut slots {
            slot.x -= hscroll;
        }
        slots
    }

    /// The column under a header x coordinate.
    #[must_use]
    pub fn column_at(&self, x: f32, width: f32, hscroll: f32) -> Option<Column> {
        self.slots_scrolled(width, hscroll)
            .into_iter()
            .find(|slot| x >= slot.x && x < slot.x + slot.width)
            .map(|slot| slot.column)
    }

    /// The column whose right edge is within grabbing distance of `x`, so a
    /// drag can start on a boundary even when the pointer sits in the gap.
    #[must_use]
    pub fn resize_handle(&self, x: f32, width: f32, hscroll: f32) -> Option<Column> {
        self.slots_scrolled(width, hscroll)
            .into_iter()
            .filter(|slot| !slot.column.flexible())
            .find(|slot| (x - (slot.x + slot.width)).abs() <= RESIZE_GRAB)
            .map(|slot| slot.column)
    }
}

/// Column menu geometry: a fixed-size panel anchored under the header.
const COLUMN_MENU_WIDTH: f32 = 184.0;
pub const COLUMN_MENU_ROW: f32 = 21.0;
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
        TorrentState::Downloading => theme::downloading(),
        TorrentState::Seeding => theme::seeding(),
        TorrentState::Queued => theme::queued(),
        TorrentState::Checking => theme::checking(),
        TorrentState::Paused => theme::paused(),
        TorrentState::Moving => theme::moving(),
        TorrentState::Error => theme::error(),
        TorrentState::Unspecified => theme::text_muted(),
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
    Open,
    Pause,
    Resume,
    Label,
    Recheck,
    Rename,
    ToggleSequential,
    ToggleSuperSeeding,
    Remove,
}

#[derive(Debug, Clone, Copy)]
pub struct ContextMenu {
    pub row: u64,
    pub x: f32,
    pub y: f32,
    pub paused: bool,
    pub sequential: bool,
    pub super_seeding: bool,
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
        0 => Some(MenuAction::Open),
        1 if menu.paused => Some(MenuAction::Resume),
        1 => Some(MenuAction::Pause),
        2 => Some(MenuAction::Label),
        3 => Some(MenuAction::Recheck),
        4 => Some(MenuAction::Rename),
        5 => Some(MenuAction::ToggleSequential),
        6 => Some(MenuAction::ToggleSuperSeeding),
        7 => Some(MenuAction::Remove),
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
    /// Horizontal offset request, in pixels.
    HScroll(f32),
}

/// Height of the horizontal scrollbar strip at the bottom of the table.
pub const HSCROLL_HEIGHT: f32 = 10.0;

/// Geometry of the horizontal scrollbar for one frame.
#[derive(Debug, Clone, Copy)]
pub struct HBar {
    pub thumb_x: f32,
    pub thumb_w: f32,
    track_w: f32,
    max: f32,
}

impl HBar {
    /// The horizontal offset that puts the thumb's left edge at `x`.
    #[must_use]
    pub fn offset_for(&self, x: f32) -> f32 {
        let span = (self.track_w - self.thumb_w).max(1.0);
        (x / span).clamp(0.0, 1.0) * self.max
    }
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
    /// Horizontal offset, for a viewport narrower than the columns.
    pub hscroll: f32,
    /// label name → chip colour, built by the app for this frame.
    pub label_colors: BTreeMap<String, Color>,
}

/// Canvas state: whether the horizontal scrollbar thumb is being dragged,
/// with the grab offset inside the thumb.
#[derive(Debug, Default)]
pub struct TableState {
    hdrag: Option<f32>,
}

impl canvas::Program<TableMessage> for Table<'_> {
    type State = TableState;

    fn update(
        &self,
        state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<TableMessage>) {
        use canvas::event::Status;
        let bar = self.hbar(bounds);
        match event {
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let (dx, dy) = match delta {
                    mouse::ScrollDelta::Lines { x, y } => (x * 40.0, y * ROW_HEIGHT * 3.0),
                    mouse::ScrollDelta::Pixels { x, y } => (x, y),
                };
                if dx.abs() > 0.0 {
                    // A horizontal wheel or trackpad swipe moves the columns.
                    return (
                        Status::Captured,
                        Some(TableMessage::HScroll(self.hscroll + dx)),
                    );
                }
                (Status::Captured, Some(TableMessage::Scroll(dy)))
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(button)) => {
                let Some(point) = cursor.position_in(bounds) else {
                    return (Status::Ignored, None);
                };
                match button {
                    mouse::Button::Left => {
                        if let Some(bar) = bar {
                            if point.y >= bounds.height - HSCROLL_HEIGHT {
                                let grab = (point.x - bar.thumb_x).clamp(0.0, bar.thumb_w);
                                state.hdrag = Some(grab);
                                return (
                                    Status::Captured,
                                    Some(TableMessage::HScroll(bar.offset_for(point.x - grab))),
                                );
                            }
                        }
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
                            if let Some(column) =
                                self.layout
                                    .resize_handle(point.x, bounds.width, self.hscroll)
                            {
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
                if let (Some(grab), Some(point), Some(bar)) =
                    (state.hdrag, cursor.position_in(bounds), bar)
                {
                    return (
                        Status::Captured,
                        Some(TableMessage::HScroll(bar.offset_for(point.x - grab))),
                    );
                }
                if let (Some(column), Some(point)) = (self.resizing, cursor.position_in(bounds)) {
                    return (
                        Status::Captured,
                        Some(TableMessage::Resize(column, point.x)),
                    );
                }
                (Status::Ignored, None)
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.hdrag = None;
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
            bg_header(),
        );
        // Leave room for the vertical scrollbar: with a column layout the
        // last column would otherwise sit under it. When the columns still do
        // not fit, they keep their minimum width and scroll sideways.
        let content_width = self.content_width(bounds.width);
        self.draw_header(&mut frame, content_width, self.hscroll);

        let range = visible_range(self.scroll, self.total);
        let page_end = self.page_offset + self.rows.len();
        // A canvas layer grows to the painted primitives; without this bound
        // the rows below the canvas run into whatever sits under it (the
        // details panel) and show through.
        let max_y = bounds.height;
        for index in range {
            let y = HEADER_HEIGHT + index as f32 * ROW_HEIGHT - self.scroll;
            if y >= max_y {
                break;
            }
            let row_height = ROW_HEIGHT.min(max_y - y);
            let torrent = (index >= self.page_offset && index < page_end)
                .then(|| &self.rows[index - self.page_offset]);
            let selected = torrent.is_some_and(|torrent| self.selected.contains(&torrent.id));
            let background = if selected {
                bg_selected()
            } else if hovered == Some(index) {
                bg_hover()
            } else if index % 2 == 0 {
                bg_row()
            } else {
                bg_row_alt()
            };
            frame.fill_rectangle(
                Point::new(0.0, y),
                Size::new(bounds.width, row_height),
                background,
            );

            match torrent {
                Some(torrent) => draw_row(
                    &mut frame,
                    self.layout,
                    &self.label_colors,
                    torrent,
                    index + 1,
                    y,
                    content_width,
                    self.hscroll,
                ),
                None => {
                    frame.fill_text(Text {
                        content: "loading…".to_owned(),
                        position: Point::new(64.0, y + 4.0),
                        color: text_dim(),
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
                color: text_dim(),
                size: Pixels(14.0),
                line_height: LineHeight::default(),
                font: Font::default(),
                horizontal_alignment: alignment::Horizontal::Center,
                vertical_alignment: alignment::Vertical::Top,
                shaping: Shaping::Basic,
            });
        }

        self.draw_scrollbar(&mut frame, bounds);
        if let Some(bar) = self.hbar(bounds) {
            let y = bounds.height - HSCROLL_HEIGHT;
            frame.fill_rectangle(
                Point::new(0.0, y),
                Size::new(bounds.width, HSCROLL_HEIGHT),
                track(),
            );
            frame.fill_rectangle(
                Point::new(bar.thumb_x, y),
                Size::new(bar.thumb_w, HSCROLL_HEIGHT),
                thumb(),
            );
        }
        vec![frame.into_geometry()]
    }
}

impl Table<'_> {
    /// Space available to the columns; the vertical scrollbar keeps 14 px.
    fn usable_width(&self, width: f32) -> f32 {
        (width - if self.total > 0 { 14.0 } else { 0.0 }).max(1.0)
    }

    /// The width the columns are laid out at: the viewport, or their minimum
    /// total when the window is narrower — the rest scrolls sideways.
    fn content_width(&self, width: f32) -> f32 {
        self.usable_width(width).max(self.layout.min_total())
    }

    /// How far the table can scroll sideways at this width.
    #[must_use]
    pub fn max_hscroll(&self, width: f32) -> f32 {
        (self.content_width(width) - self.usable_width(width)).max(0.0)
    }

    fn hbar(&self, bounds: Rectangle) -> Option<HBar> {
        let max = self.max_hscroll(bounds.width);
        if max <= 0.0 {
            return None;
        }
        let track_w = bounds.width;
        let content = self.content_width(bounds.width);
        let thumb_w = (track_w * self.usable_width(bounds.width) / content)
            .max(40.0)
            .min(track_w);
        let fraction = (self.hscroll / max).clamp(0.0, 1.0);
        Some(HBar {
            thumb_x: fraction * (track_w - thumb_w),
            thumb_w,
            track_w,
            max,
        })
    }

    fn draw_header(&self, frame: &mut Frame, width: f32, hscroll: f32) {
        let marker = |field: SortField| {
            if self.sort == field {
                if self.descending { " v" } else { " ^" }
            } else {
                ""
            }
        };
        for slot in self.layout.slots_scrolled(width, hscroll) {
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
                color: text_header(),
                size: Pixels(11.0),
                line_height: LineHeight::default(),
                font: crate::theme::semibold(),
                horizontal_alignment: horizontal,
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
            track(),
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
            thumb(),
        );
    }
}

/// The row menu's entries, top to bottom: icon, label and action. Rendered
/// as a widget panel above the canvas, because a canvas paints its fills
/// before its texts and a menu drawn inside it cannot cover the row text.
#[must_use]
pub fn menu_items(menu: &ContextMenu) -> Vec<(char, String, MenuAction)> {
    use crate::icons::glyph;
    let sequential = if menu.sequential {
        crate::i18n::t("✓ Sequential download")
    } else {
        crate::i18n::t("Sequential download")
    };
    let super_seeding = if menu.super_seeding {
        crate::i18n::t("✓ Super-seeding")
    } else {
        crate::i18n::t("Super-seeding")
    };
    let first = if menu.paused {
        (glyph::PLAY, crate::i18n::t("Resume"), MenuAction::Resume)
    } else {
        (glyph::PAUSE, crate::i18n::t("Pause"), MenuAction::Pause)
    };
    vec![
        (
            glyph::FOLDER_OPEN,
            crate::i18n::t("Open folder").to_owned(),
            MenuAction::Open,
        ),
        (first.0, first.1.to_owned(), first.2),
        (
            glyph::TAG,
            crate::i18n::t("Set label…").to_owned(),
            MenuAction::Label,
        ),
        (
            glyph::REFRESH,
            crate::i18n::t("Force recheck").to_owned(),
            MenuAction::Recheck,
        ),
        (
            glyph::EDIT,
            crate::i18n::t("Rename file…").to_owned(),
            MenuAction::Rename,
        ),
        (
            glyph::LIST_OL,
            sequential.to_owned(),
            MenuAction::ToggleSequential,
        ),
        (
            glyph::ROCKET,
            super_seeding.to_owned(),
            MenuAction::ToggleSuperSeeding,
        ),
        (
            glyph::TRASH,
            crate::i18n::t("Remove").to_owned(),
            MenuAction::Remove,
        ),
    ]
}

/// The column chooser's entries: the column title and the column it toggles.
/// The check mark is drawn as an icon by the panel, not baked into the text.
#[must_use]
pub fn column_menu_items(_layout: &ColumnLayout) -> Vec<(String, Column)> {
    Column::ALL
        .iter()
        .map(|column| (column.title().to_owned(), *column))
        .collect()
}

// The row renderer takes the whole drawing context: splitting it into a struct
// would only move the same fields one level down (see the other renderers).
#[allow(clippy::too_many_arguments)]
fn draw_row(
    frame: &mut Frame,
    layout: &ColumnLayout,
    label_colors: &BTreeMap<String, Color>,
    torrent: &Torrent,
    row: usize,
    y: f32,
    width: f32,
    hscroll: f32,
) {
    let accent = state_color(torrent.state);
    let text_y = y + 5.0;
    let regular = crate::theme::regular();
    let mono = Font::MONOSPACE;

    for slot in layout.slots_scrolled(width, hscroll) {
        let (content, color, size, font) = match slot.column {
            Column::Position => (
                // Only a torrent that still has pieces to fetch carries a
                // number; a complete one (seeding or paused) leaves the cell
                // empty. Downloading sorts first, so the numbers are the
                // contiguous run at the top of the list.
                if torrent.progress < 1.0 {
                    row.to_string()
                } else {
                    String::new()
                },
                text_dim(),
                11.0,
                mono,
            ),
            Column::Name => (
                short_name(&torrent.name, chars_that_fit(slot.width, 12.0)),
                text_main(),
                12.0,
                regular,
            ),
            Column::Label => {
                if torrent.label.is_empty() {
                    ("—".to_owned(), text_dim(), 11.0, regular)
                } else {
                    (
                        short_name(&torrent.label, chars_that_fit(slot.width, 11.0)),
                        label_colors
                            .get(&torrent.label)
                            .copied()
                            .unwrap_or(text_dim()),
                        11.0,
                        regular,
                    )
                }
            }
            Column::Size => (format_size(torrent.total_bytes), text_dim(), 11.0, mono),
            Column::SavePath => (
                short_name(
                    &folder_label(&torrent.save_path),
                    chars_that_fit(slot.width, 11.0),
                ),
                text_dim(),
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
                    bar_track(),
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
                    text_dim(),
                    11.0,
                    alignment::Horizontal::Left,
                    mono,
                );
                continue;
            }
            Column::Download => (format_rate(torrent.download_rate), text_main(), 11.0, mono),
            Column::Upload => (format_rate(torrent.upload_rate), text_main(), 11.0, mono),
            Column::Seeds => (torrent.seeds.to_string(), text_dim(), 11.0, mono),
            Column::Peers => (torrent.peers.to_string(), text_dim(), 11.0, mono),
            Column::Eta => (format_eta(torrent.eta_seconds), text_dim(), 11.0, mono),
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

pub fn format_size(bytes: u64) -> String {
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

/// How many characters of `size` pixels fit in `width`. The canvas text is
/// truncated by character, not by measured advance, so the estimate keeps the
/// tail of a long name from spilling into the next column.
fn chars_that_fit(width: f32, size: f32) -> usize {
    ((width / (size * 0.58)).floor() as usize).max(4)
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
        TorrentState::Unspecified => crate::i18n::t("unknown"),
        TorrentState::Queued => crate::i18n::t("queued"),
        TorrentState::Checking => crate::i18n::t("checking"),
        TorrentState::Downloading => crate::i18n::t("downloading"),
        TorrentState::Seeding => crate::i18n::t("seeding"),
        TorrentState::Paused => crate::i18n::t("paused"),
        TorrentState::Moving => crate::i18n::t("moving"),
        TorrentState::Error => crate::i18n::t("error"),
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
        assert_eq!(layout.column_at(12.0, width, 0.0), Some(Column::Position));
        assert_eq!(
            layout.column_at(name.x + 2.0, width, 0.0),
            Some(Column::Name)
        );
        assert_eq!(
            layout
                .column_at(name.x + 2.0, width, 0.0)
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
            layout.resize_handle(size.x + size.width - 1.0, width, 0.0),
            Some(Column::Size)
        );
        assert_eq!(layout.resize_handle(size.x + 2.0, width, 0.0), None);
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
            Some(Column::Position)
        );
        assert!(layout.is_visible(Column::Position));
    }

    #[test]
    fn menu_hit_testing_matches_drawn_labels() {
        let menu = ContextMenu {
            row: 7,
            x: 100.0,
            y: 100.0,
            paused: false,
            sequential: false,
            super_seeding: false,
        };
        assert_eq!(
            menu_action_at(&menu, Point::new(110.0, 108.0)),
            Some(MenuAction::Open)
        );
        assert_eq!(
            menu_action_at(&menu, Point::new(110.0, 134.0)),
            Some(MenuAction::Pause)
        );
        assert_eq!(
            menu_action_at(&menu, Point::new(110.0, 160.0)),
            Some(MenuAction::Label)
        );
        assert_eq!(
            menu_action_at(&menu, Point::new(110.0, 186.0)),
            Some(MenuAction::Recheck)
        );
        assert_eq!(
            menu_action_at(&menu, Point::new(110.0, 212.0)),
            Some(MenuAction::Rename)
        );
        assert_eq!(
            menu_action_at(&menu, Point::new(110.0, 238.0)),
            Some(MenuAction::ToggleSequential)
        );
        assert_eq!(
            menu_action_at(&menu, Point::new(110.0, 264.0)),
            Some(MenuAction::ToggleSuperSeeding)
        );
        assert_eq!(
            menu_action_at(&menu, Point::new(110.0, 290.0)),
            Some(MenuAction::Remove)
        );
        assert_eq!(menu_action_at(&menu, Point::new(10.0, 10.0)), None);
    }

    /// The widget panel renders `menu_items`; a click on the panel padding
    /// resolves through the canvas slots. The two must not drift apart.
    #[test]
    fn menu_items_align_with_the_hit_testing_slots() {
        for paused in [false, true] {
            let menu = ContextMenu {
                row: 3,
                x: 10.0,
                y: 10.0,
                paused,
                sequential: false,
                super_seeding: false,
            };
            let items = menu_items(&menu);
            assert_eq!(items.len(), 8);
            assert_eq!(items[0].2, MenuAction::Open);
            assert_eq!(
                items[1].2,
                if paused {
                    MenuAction::Resume
                } else {
                    MenuAction::Pause
                }
            );
            for (index, (_icon, label, action)) in items.iter().enumerate() {
                assert!(!label.is_empty());
                let y = menu.y + MENU_PADDING + index as f32 * MENU_ITEM_HEIGHT + 1.0;
                assert_eq!(menu_action_at(&menu, Point::new(20.0, y)), Some(*action));
            }
        }
    }
}
