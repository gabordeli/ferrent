//! Ferrent palette: a dark and a light theme, plus the bundled IBM Plex Sans
//! faces.
//!
//! The palette is process-wide and swappable at runtime: the widgets ask for
//! colours through the accessors (`theme::bg()`, `theme::text_muted()`), so a
//! theme switch only needs `install()` and a redraw.

use iced::font::{Family, Weight};
use iced::{Color, Font, Theme};

/// One complete colour set.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub bg: Color,
    pub bg_secondary: Color,
    pub surface: Color,
    pub surface_hover: Color,
    pub border: Color,
    pub text: Color,
    pub text_secondary: Color,
    pub text_muted: Color,
    pub primary: Color,
    pub bright: Color,
    pub cyan: Color,
    pub deep: Color,
    pub downloading: Color,
    pub seeding: Color,
    pub queued: Color,
    pub paused: Color,
    pub error: Color,
    pub checking: Color,
    pub moving: Color,
    pub connecting: Color,
}

impl Palette {
    /// The original dark palette; a colour change here must stay in sync with
    /// the screenshots in `docs/measurements/`.
    #[must_use]
    pub const fn dark() -> Self {
        Self {
            bg: Color::from_rgb(0.043, 0.071, 0.125),
            bg_secondary: Color::from_rgb(0.067, 0.110, 0.180),
            surface: Color::from_rgb(0.118, 0.161, 0.231),
            surface_hover: Color::from_rgb(0.157, 0.204, 0.278),
            border: Color::from_rgb(0.204, 0.243, 0.314),
            text: Color::from_rgb(0.973, 0.980, 0.988),
            text_secondary: Color::from_rgb(0.580, 0.639, 0.722),
            text_muted: Color::from_rgb(0.392, 0.455, 0.545),
            primary: Color::from_rgb(0.086, 0.467, 0.949),
            bright: Color::from_rgb(0.141, 0.659, 1.0),
            cyan: Color::from_rgb(0.133, 0.827, 0.933),
            deep: Color::from_rgb(0.043, 0.227, 0.459),
            downloading: Color::from_rgb(0.133, 0.773, 0.369),
            seeding: Color::from_rgb(0.086, 0.467, 0.949),
            queued: Color::from_rgb(0.980, 0.800, 0.082),
            paused: Color::from_rgb(0.580, 0.639, 0.722),
            error: Color::from_rgb(0.937, 0.267, 0.267),
            checking: Color::from_rgb(0.655, 0.545, 0.980),
            moving: Color::from_rgb(0.298, 0.651, 0.949),
            connecting: Color::from_rgb(0.133, 0.827, 0.933),
        }
    }

    /// Light palette; the text colours keep a WCAG contrast of at least 4.5:1
    /// on `bg` (checked in the tests).
    #[must_use]
    pub const fn light() -> Self {
        Self {
            bg: Color::from_rgb(0.957, 0.965, 0.973),
            bg_secondary: Color::from_rgb(1.0, 1.0, 1.0),
            surface: Color::from_rgb(0.914, 0.929, 0.949),
            surface_hover: Color::from_rgb(0.867, 0.890, 0.918),
            border: Color::from_rgb(0.800, 0.827, 0.863),
            text: Color::from_rgb(0.067, 0.094, 0.122),
            text_secondary: Color::from_rgb(0.235, 0.275, 0.322),
            text_muted: Color::from_rgb(0.361, 0.400, 0.451),
            primary: Color::from_rgb(0.043, 0.384, 0.839),
            bright: Color::from_rgb(0.039, 0.337, 0.741),
            cyan: Color::from_rgb(0.043, 0.447, 0.522),
            deep: Color::from_rgb(0.812, 0.882, 0.969),
            downloading: Color::from_rgb(0.102, 0.498, 0.216),
            seeding: Color::from_rgb(0.043, 0.384, 0.839),
            queued: Color::from_rgb(0.518, 0.345, 0.0),
            paused: Color::from_rgb(0.235, 0.275, 0.322),
            error: Color::from_rgb(0.776, 0.157, 0.157),
            checking: Color::from_rgb(0.435, 0.259, 0.757),
            moving: Color::from_rgb(0.110, 0.392, 0.710),
            connecting: Color::from_rgb(0.043, 0.447, 0.522),
        }
    }

    /// Replaces the accent colour used by the chrome. The torrent state
    /// colours stay put, so a grey accent cannot grey out the table.
    #[must_use]
    pub fn with_accent(mut self, accent: Color) -> Self {
        self.primary = accent;
        self
    }
}

static CURRENT: std::sync::RwLock<Palette> = std::sync::RwLock::new(Palette::dark());

/// Installs a palette for every widget; takes effect on the next frame.
pub fn install(palette: Palette) {
    *CURRENT.write().expect("palette lock poisoned") = palette;
}

/// The palette in use.
#[must_use]
pub fn current() -> Palette {
    *CURRENT.read().expect("palette lock poisoned")
}

macro_rules! accessors {
    ($($name:ident),* $(,)?) => {
        $(
            /// Current theme value.
            #[must_use]
            pub fn $name() -> Color {
                current().$name
            }
        )*
    };
}

accessors!(
    bg,
    bg_secondary,
    surface,
    surface_hover,
    border,
    text,
    text_secondary,
    text_muted,
    primary,
    bright,
    cyan,
    deep,
    downloading,
    seeding,
    queued,
    paused,
    error,
    checking,
    moving,
    connecting,
);

/// Linear blend, used for the subtle row striping.
#[must_use]
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    let blend = |x: f32, y: f32| x + (y - x) * t;
    Color {
        r: blend(a.r, b.r),
        g: blend(a.g, b.g),
        b: blend(a.b, b.b),
        a: blend(a.a, b.a),
    }
}

/// True when the installed palette has a light background (the wordmark PNG
/// is white, so the sidebar falls back to text).
#[must_use]
pub fn is_light() -> bool {
    let palette = current();
    (palette.bg.r + palette.bg.g + palette.bg.b) / 3.0 > 0.5
}

/// The success colour is the same green as "downloading".
#[must_use]
pub fn success() -> Color {
    current().downloading
}

const FAMILY: &str = "IBM Plex Sans";

pub const REGULAR_BYTES: &[u8] = include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf");
pub const MEDIUM_BYTES: &[u8] = include_bytes!("../assets/fonts/IBMPlexSans-Medium.ttf");
pub const SEMIBOLD_BYTES: &[u8] = include_bytes!("../assets/fonts/IBMPlexSans-SemiBold.ttf");

/// Rendered at 3× (338×90) so the sidebar lockup stays crisp; the widget
/// scales it down to 113×30.
/// The application icon: the hexagon mark, used for the window and the
/// desktop entry. `icon-256.png` is a Lanczos upscale of `icon.png`.
pub const ICON_BYTES: &[u8] = include_bytes!("../assets/icon.png");

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
    let palette = current();
    Theme::custom(
        "Ferrent".to_owned(),
        iced::theme::Palette {
            background: palette.bg,
            text: palette.text,
            primary: palette.primary,
            success: palette.downloading,
            danger: palette.error,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(color: Color) -> f64 {
        let channel = |value: f32| {
            let value = f64::from(value);
            if value <= 0.039_28 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b)
    }

    fn contrast(a: Color, b: Color) -> f64 {
        let (lighter, darker) = {
            let (a, b) = (luminance(a), luminance(b));
            if a > b { (a, b) } else { (b, a) }
        };
        (lighter + 0.05) / (darker + 0.05)
    }

    #[test]
    fn text_stays_readable_in_both_themes() {
        // The light theme is held to WCAG AA (4.5:1). The dark palette keeps
        // its established muted tone (3.9:1), which is the documented
        // trade-off for de-emphasised hints.
        for (name, palette, minimum) in [
            ("dark", Palette::dark(), 3.8),
            ("light", Palette::light(), 4.5),
        ] {
            for (role, color) in [
                ("text", palette.text),
                ("text_secondary", palette.text_secondary),
                ("text_muted", palette.text_muted),
                ("error", palette.error),
                ("queued", palette.queued),
            ] {
                let ratio = contrast(color, palette.bg);
                assert!(
                    ratio >= minimum,
                    "{name}/{role} contrast {ratio:.2} is below {minimum}:1"
                );
            }
        }
    }

    #[test]
    fn the_accent_leaves_the_state_colours_alone() {
        let accent = Color::from_rgb(0.8, 0.2, 0.4);
        let palette = Palette::dark().with_accent(accent);
        assert_eq!(palette.primary, accent);
        assert_eq!(palette.seeding, Palette::dark().seeding);
        assert_eq!(palette.downloading, Palette::dark().downloading);
    }

    #[test]
    fn installing_a_palette_changes_the_accessors() {
        install(Palette::light());
        assert_eq!(bg(), Palette::light().bg);
        install(Palette::dark());
        assert_eq!(bg(), Palette::dark().bg);
    }
}
