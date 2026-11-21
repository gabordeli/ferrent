//! Ferrent dark theme: palette, iced theme and the bundled IBM Plex Sans faces.

use iced::font::{Family, Weight};
use iced::{Color, Font, Theme};

pub const BG: Color = Color::from_rgb(0.043, 0.071, 0.125);
pub const BG_SECONDARY: Color = Color::from_rgb(0.067, 0.110, 0.180);
pub const SURFACE: Color = Color::from_rgb(0.118, 0.161, 0.231);
pub const SURFACE_HOVER: Color = Color::from_rgb(0.157, 0.204, 0.278);
pub const BORDER: Color = Color::from_rgb(0.204, 0.243, 0.314);

pub const TEXT: Color = Color::from_rgb(0.973, 0.980, 0.988);
pub const TEXT_SECONDARY: Color = Color::from_rgb(0.580, 0.639, 0.722);
pub const TEXT_MUTED: Color = Color::from_rgb(0.392, 0.455, 0.545);

pub const PRIMARY: Color = Color::from_rgb(0.086, 0.467, 0.949);
pub const BRIGHT: Color = Color::from_rgb(0.141, 0.659, 1.0);
pub const CYAN: Color = Color::from_rgb(0.133, 0.827, 0.933);
pub const DEEP: Color = Color::from_rgb(0.043, 0.227, 0.459);

pub const DOWNLOADING: Color = Color::from_rgb(0.133, 0.773, 0.369);
pub const SEEDING: Color = PRIMARY;
pub const QUEUED: Color = Color::from_rgb(0.980, 0.800, 0.082);
pub const PAUSED: Color = TEXT_SECONDARY;
pub const ERROR: Color = Color::from_rgb(0.937, 0.267, 0.267);
pub const CHECKING: Color = Color::from_rgb(0.655, 0.545, 0.980);
pub const MOVING: Color = Color::from_rgb(0.298, 0.651, 0.949);
pub const CONNECTING: Color = CYAN;

const FAMILY: &str = "IBM Plex Sans";

pub const REGULAR_BYTES: &[u8] = include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf");
pub const MEDIUM_BYTES: &[u8] = include_bytes!("../assets/fonts/IBMPlexSans-Medium.ttf");
pub const SEMIBOLD_BYTES: &[u8] = include_bytes!("../assets/fonts/IBMPlexSans-SemiBold.ttf");

/// Rendered at 3× (338×90) so the sidebar lockup stays crisp; the widget
/// scales it down to 113×30.
pub const LOGO_BYTES: &[u8] = include_bytes!("../assets/logo.png");
/// The square mark alone, for the collapsed sidebar and the window icon.
pub const MARK_BYTES: &[u8] = include_bytes!("../assets/mark.png");

#[must_use]
pub fn regular() -> Font {
    Font {
        family: Family::Name(FAMILY),
        weight: Weight::Normal,
        ..Font::default()
    }
}

#[must_use]
pub fn medium() -> Font {
    Font {
        family: Family::Name(FAMILY),
        weight: Weight::Medium,
        ..Font::default()
    }
}

#[must_use]
pub fn semibold() -> Font {
    Font {
        family: Family::Name(FAMILY),
        weight: Weight::Semibold,
        ..Font::default()
    }
}

#[must_use]
pub fn iced_theme() -> Theme {
    Theme::custom(
        "Ferrent Dark".to_owned(),
        iced::theme::Palette {
            background: BG,
            text: TEXT,
            primary: PRIMARY,
            success: DOWNLOADING,
            danger: ERROR,
        },
    )
}
