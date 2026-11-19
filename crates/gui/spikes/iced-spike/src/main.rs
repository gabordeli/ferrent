//! iced GUI spike: virtualized 4 000-row table on a canvas.
//!
//! Run with `--auto` to auto-scroll for 5 seconds, print startup/RSS/FPS,
//! and exit. Without it the window stays open for manual scrolling.

use std::time::Instant;

use iced::mouse;
use iced::widget::canvas::{self, Canvas, Frame, Text};
use iced::widget::text::{LineHeight, Shaping};
use iced::widget::{column, container, row, text};
use iced::{
    Color, Element, Length, Pixels, Point, Rectangle, Size, Subscription, Task, Theme, alignment,
};

const ROWS: usize = 4_000;
const ROW_HEIGHT: f32 = 26.0;
const HEADER_HEIGHT: f32 = 48.0;
const WINDOW_HEIGHT: f32 = 700.0;
const MEASURE_SECS: f64 = 5.0;

fn max_offset() -> f32 {
    (ROWS as f32 * ROW_HEIGHT - (WINDOW_HEIGHT - HEADER_HEIGHT)).max(0.0)
}

#[derive(Debug, Clone, Copy)]
enum Message {
    Frame(Instant),
    Scroll(f32),
}

struct App {
    offset: f32,
    auto: bool,
    started: Instant,
    first_frame: Option<Instant>,
    frames: u64,
    reported: bool,
}

impl Default for App {
    fn default() -> Self {
        Self {
            offset: 0.0,
            auto: std::env::args().any(|arg| arg == "--auto"),
            started: Instant::now(),
            first_frame: None,
            frames: 0,
            reported: false,
        }
    }
}

impl App {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Frame(frame_time) => {
                if self.first_frame.is_none() {
                    self.first_frame = Some(frame_time);
                    println!(
                        "startup_ms={:.1}",
                        self.started.elapsed().as_secs_f64() * 1000.0
                    );
                }
                self.frames += 1;
                if self.auto {
                    self.offset = (self.offset + 18.0) % max_offset();
                }
                self.report_if_done();
            }
            Message::Scroll(delta) => {
                self.offset = (self.offset + delta).clamp(0.0, max_offset());
            }
        }
        Task::none()
    }

    fn report_if_done(&mut self) {
        if !self.auto || self.reported {
            return;
        }
        let Some(first_frame) = self.first_frame else {
            return;
        };
        let elapsed = first_frame.elapsed().as_secs_f64();
        if elapsed < MEASURE_SECS {
            return;
        }
        self.reported = true;
        println!("fps={:.1}", self.frames as f64 / elapsed);
        println!("rss_kb={}", read_rss_kb());
        println!("rows={ROWS}");
        println!(
            "visible_rows={:.0}",
            ((WINDOW_HEIGHT - HEADER_HEIGHT) / ROW_HEIGHT).ceil() + 2.0
        );
        println!("framework=iced");
        std::process::exit(0);
    }

    fn subscription(&self) -> Subscription<Message> {
        iced::window::frames().map(Message::Frame)
    }

    fn view(&self) -> Element<'_, Message> {
        let header = container(
            row![
                text("Ferrent iced spike").size(20),
                text(format!("{ROWS} rows")).size(14),
            ]
            .spacing(24),
        )
        .padding(10)
        .width(Length::Fill);

        let table = Canvas::new(Table {
            offset: self.offset,
        })
        .width(Length::Fill)
        .height(Length::Fill);

        column![header, table].into()
    }
}

struct Table {
    offset: f32,
}

impl canvas::Program<Message> for Table {
    type State = ();

    fn update(
        &self,
        _state: &mut Self::State,
        event: canvas::Event,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        match event {
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let dy = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => y * ROW_HEIGHT * 3.0,
                    mouse::ScrollDelta::Pixels { y, .. } => y,
                };
                (canvas::event::Status::Captured, Some(Message::Scroll(dy)))
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
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let first = (self.offset / ROW_HEIGHT).floor().max(0.0) as usize;
        let visible = (bounds.height / ROW_HEIGHT).ceil() as usize + 2;
        let end = (first + visible).min(ROWS);

        for index in first..end {
            let y = index as f32 * ROW_HEIGHT - self.offset;
            let background = if index % 2 == 0 {
                Color::from_rgb(0.10, 0.11, 0.13)
            } else {
                Color::from_rgb(0.125, 0.135, 0.155)
            };
            frame.fill_rectangle(
                Point::new(0.0, y),
                Size::new(bounds.width, ROW_HEIGHT),
                background,
            );

            let progress = (index * 37 % 100) as f32;
            let down = ((index as u64 * 131_071) % 5_000_000) as f64;
            let up = ((index as u64 * 97_003) % 900_000) as f64;
            let content = format!(
                "#{index:04}  torrent-{index:04}.mkv    {progress:>5.1}%    down {:>8}    up {:>8}",
                format_rate(down),
                format_rate(up)
            );

            frame.fill_text(Text {
                content,
                position: Point::new(14.0, y + 5.0),
                color: Color::from_rgb(0.86, 0.88, 0.91),
                size: Pixels(14.0),
                line_height: LineHeight::default(),
                font: iced::Font::default(),
                horizontal_alignment: alignment::Horizontal::Left,
                vertical_alignment: alignment::Vertical::Top,
                shaping: Shaping::Basic,
            });
        }

        let max = max_offset();
        if max > 0.0 {
            let fraction = (self.offset / max).clamp(0.0, 1.0);
            let bar_height = (bounds.height * (bounds.height / (ROWS as f32 * ROW_HEIGHT)))
                .max(24.0)
                .min(bounds.height);
            let bar_y = fraction * (bounds.height - bar_height);
            frame.fill_rectangle(
                Point::new(bounds.width - 9.0, bar_y),
                Size::new(6.0, bar_height),
                Color::from_rgb(0.38, 0.43, 0.5),
            );
        }

        vec![frame.into_geometry()]
    }
}

fn format_rate(bytes: f64) -> String {
    if bytes < 1024.0 {
        format!("{bytes:.0} B/s")
    } else if bytes < 1024.0 * 1024.0 {
        format!("{:.1} KiB/s", bytes / 1024.0)
    } else {
        format!("{:.1} MiB/s", bytes / (1024.0 * 1024.0))
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
    iced::application("Ferrent iced spike", App::update, App::view)
        .subscription(App::subscription)
        .window_size(Size::new(1100.0, WINDOW_HEIGHT))
        .run()
}
