//! Virtualized torrent table drawn on an iced canvas.
//!
//! Only the visible rows are shaped and painted; the rows themselves come
//! from a server-side page around the viewport.

use std::collections::BTreeSet;
use std::ops::Range;

use ferrent_protocol::v1::{SortField, Torrent, TorrentState};
use iced::widget::canvas::{self, Frame, Text};
use iced::widget::text::{LineHeight, Shaping};
use iced::{Color, Font, Pixels, Point, Rectangle, Size, Theme, alignment, mouse};

pub const TABLE_WIDTH: f32 = 746.0;
pub const TABLE_HEIGHT: f32 = 460.0;
pub const HEADER_HEIGHT: f32 = 30.0;
pub const ROW_HEIGHT: f32 = 24.0;
pub const VISIBLE_ROWS: usize = ((TABLE_HEIGHT - HEADER_HEIGHT) / ROW_HEIGHT) as usize + 3;

const MENU_WIDTH: f32 = 160.0;
const MENU_ITEM_HEIGHT: f32 = 26.0;
const MENU_PADDING: f32 = 4.0;
const MENU_HEIGHT: f32 = MENU_PADDING * 2.0 + MENU_ITEM_HEIGHT * 2.0;

const BG_HEADER: Color = Color::from_rgb(0.90, 0.91, 0.94);
const BG_ROW: Color = Color::from_rgb(1.0, 1.0, 1.0);
const BG_ROW_ALT: Color = Color::from_rgb(0.965, 0.97, 0.98);
const BG_SELECTED: Color = Color::from_rgb(0.84, 0.89, 1.0);
const BG_HOVER: Color = Color::from_rgb(0.93, 0.95, 1.0);
const TEXT: Color = Color::from_rgb(0.12, 0.14, 0.18);
const TEXT_DIM: Color = Color::from_rgb(0.42, 0.46, 0.52);
const TEXT_HEADER: Color = Color::from_rgb(0.28, 0.32, 0.38);
const TEXT_ERROR: Color = Color::from_rgb(0.65, 0.12, 0.12);
const BAR_TRACK: Color = Color::from_rgb(0.88, 0.90, 0.93);
const TRACK: Color = Color::from_rgb(0.87, 0.89, 0.92);
const THUMB: Color = Color::from_rgb(0.58, 0.62, 0.69);
const MENU_BG: Color = Color::from_rgb(0.98, 0.99, 1.0);
const MENU_BORDER: Color = Color::from_rgb(0.68, 0.72, 0.78);

const COL_ID_RIGHT: f32 = 50.0;
const COL_NAME_X: f32 = 60.0;
const COL_STATE_DOT_X: f32 = 338.0;
const COL_STATE_X: f32 = 348.0;
const COL_PROGRESS_X: f32 = 432.0;
const BAR_WIDTH: f32 = 64.0;
const BAR_HEIGHT: f32 = 8.0;
const COL_PCT_X: f32 = 502.0;
const COL_DOWN_RIGHT: f32 = 618.0;
const COL_UP_RIGHT: f32 = 706.0;

/// Accent colour per torrent state; used for the state dot and the bar.
#[must_use]
pub fn state_color(state: i32) -> Color {
    match TorrentState::try_from(state).unwrap_or(TorrentState::Unspecified) {
        TorrentState::Seeding => Color::from_rgb(0.13, 0.55, 0.28),
        TorrentState::Downloading => Color::from_rgb(0.13, 0.42, 0.78),
        TorrentState::Checking => Color::from_rgb(0.80, 0.52, 0.06),
        TorrentState::Queued => Color::from_rgb(0.45, 0.36, 0.66),
        TorrentState::Paused => Color::from_rgb(0.48, 0.52, 0.58),
        TorrentState::Error => TEXT_ERROR,
        TorrentState::Unspecified => TEXT_DIM,
    }
}

/// One table column. `State` is displayed but not server-sortable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Id,
    Name,
    State,
    Progress,
    Download,
    Upload,
}

impl Column {
    #[must_use]
    pub fn sort_field(self) -> Option<SortField> {
        match self {
            Self::Id => Some(SortField::Id),
            Self::Name => Some(SortField::Name),
            Self::Progress => Some(SortField::Progress),
            Self::Download => Some(SortField::DownloadRate),
            Self::Upload => Some(SortField::UploadRate),
            Self::State => None,
        }
    }
}

/// Returns the column under a header x coordinate.
#[must_use]
pub fn column_at(x: f32) -> Option<Column> {
    if (10.0..55.0).contains(&x) {
        Some(Column::Id)
    } else if (60.0..330.0).contains(&x) {
        Some(Column::Name)
    } else if (330.0..430.0).contains(&x) {
        Some(Column::State)
    } else if (430.0..525.0).contains(&x) {
        Some(Column::Progress)
    } else if (525.0..625.0).contains(&x) {
        Some(Column::Download)
    } else if (625.0..746.0).contains(&x) {
        Some(Column::Upload)
    } else {
        None
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
        1 => Some(MenuAction::Remove),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub enum TableMessage {
    Scroll(f32),
    Pressed(Point),
    RightPressed(Point),
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
        match event {
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let dy = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => y * ROW_HEIGHT * 3.0,
                    mouse::ScrollDelta::Pixels { y, .. } => y,
                };
                (
                    canvas::event::Status::Captured,
                    Some(TableMessage::Scroll(dy)),
                )
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(button)) => {
                let Some(point) = cursor.position_in(bounds) else {
                    return (canvas::event::Status::Ignored, None);
                };
                match button {
                    mouse::Button::Left => (
                        canvas::event::Status::Captured,
                        Some(TableMessage::Pressed(point)),
                    ),
                    mouse::Button::Right => (
                        canvas::event::Status::Captured,
                        Some(TableMessage::RightPressed(point)),
                    ),
                    _ => (canvas::event::Status::Ignored, None),
                }
            }
            _ => (canvas::event::Status::Ignored, None),
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
        self.draw_header(&mut frame);

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
                Some(torrent) => draw_row(&mut frame, torrent, y),
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

        vec![frame.into_geometry()]
    }
}

impl Table<'_> {
    fn draw_header(&self, frame: &mut Frame) {
        let marker = |field: SortField| {
            if self.sort == field {
                if self.descending { " v" } else { " ^" }
            } else {
                ""
            }
        };
        let cells = [
            (
                format!("ID{}", marker(SortField::Id)),
                COL_ID_RIGHT,
                alignment::Horizontal::Right,
            ),
            (
                format!("NAME{}", marker(SortField::Name)),
                COL_NAME_X,
                alignment::Horizontal::Left,
            ),
            (
                "STATE".to_owned(),
                COL_STATE_X,
                alignment::Horizontal::Left,
            ),
            (
                format!("PROGRESS{}", marker(SortField::Progress)),
                COL_PROGRESS_X,
                alignment::Horizontal::Left,
            ),
            (
                format!("DOWN{}", marker(SortField::DownloadRate)),
                COL_DOWN_RIGHT,
                alignment::Horizontal::Right,
            ),
            (
                format!("UP{}", marker(SortField::UploadRate)),
                COL_UP_RIGHT,
                alignment::Horizontal::Right,
            ),
        ];
        for (content, x, horizontal) in cells {
            frame.fill_text(Text {
                content,
                position: Point::new(x, 8.0),
                color: TEXT_HEADER,
                size: Pixels(12.0),
                line_height: LineHeight::default(),
                font: Font::MONOSPACE,
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
            ["Resume", "Remove"]
        } else {
            ["Pause", "Remove"]
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

fn draw_row(frame: &mut Frame, torrent: &Torrent, y: f32) {
    let accent = state_color(torrent.state);
    let text_y = y + 5.0;

    cell(
        frame,
        torrent.id.to_string(),
        COL_ID_RIGHT,
        text_y,
        TEXT_DIM,
        12.0,
        alignment::Horizontal::Right,
        Font::MONOSPACE,
    );
    cell(
        frame,
        short_name(&torrent.name),
        COL_NAME_X,
        text_y,
        TEXT,
        13.0,
        alignment::Horizontal::Left,
        Font::default(),
    );
    frame.fill_rectangle(
        Point::new(COL_STATE_DOT_X, y + 9.0),
        Size::new(6.0, 6.0),
        accent,
    );
    cell(
        frame,
        state_label(torrent.state).to_owned(),
        COL_STATE_X,
        text_y,
        accent,
        12.0,
        alignment::Horizontal::Left,
        Font::default(),
    );

    let progress = torrent.progress.clamp(0.0, 1.0);
    let bar_y = y + (ROW_HEIGHT - BAR_HEIGHT) / 2.0;
    frame.fill_rectangle(
        Point::new(COL_PROGRESS_X, bar_y),
        Size::new(BAR_WIDTH, BAR_HEIGHT),
        BAR_TRACK,
    );
    let filled = (BAR_WIDTH * progress).round();
    if filled > 0.0 {
        frame.fill_rectangle(
            Point::new(COL_PROGRESS_X, bar_y),
            Size::new(filled, BAR_HEIGHT),
            accent,
        );
    }
    cell(
        frame,
        format!("{}%", (progress * 100.0).round() as u32),
        COL_PCT_X,
        text_y,
        TEXT_DIM,
        12.0,
        alignment::Horizontal::Left,
        Font::MONOSPACE,
    );
    cell(
        frame,
        format_rate(torrent.download_rate),
        COL_DOWN_RIGHT,
        text_y,
        TEXT,
        12.0,
        alignment::Horizontal::Right,
        Font::MONOSPACE,
    );
    cell(
        frame,
        format_rate(torrent.upload_rate),
        COL_UP_RIGHT,
        text_y,
        TEXT,
        12.0,
        alignment::Horizontal::Right,
        Font::MONOSPACE,
    );
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

fn short_name(name: &str) -> String {
    if name.chars().count() <= 32 {
        name.to_owned()
    } else {
        let mut short: String = name.chars().take(31).collect();
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
        assert_eq!(column_at(12.0), Some(Column::Id));
        assert_eq!(column_at(100.0), Some(Column::Name));
        assert_eq!(column_at(700.0), Some(Column::Upload));
        assert_eq!(
            column_at(80.0).and_then(Column::sort_field),
            Some(SortField::Name)
        );
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
            Some(MenuAction::Remove)
        );
        assert_eq!(menu_action_at(&menu, Point::new(10.0, 10.0)), None);
    }
}
