//! Font Awesome 6 Free (Solid) glyphs used by the shell.
//!
//! The font is bundled under the SIL OFL 1.1; see
//! `assets/fonts/FONT-AWESOME-OFL.txt`. Codepoints mirror the upstream
//! `all.css` so a font upgrade only needs a check against it.

use iced::Font;
use iced::font::{Family, Weight};

const FAMILY: &str = "Font Awesome 6 Free";

pub const SOLID_BYTES: &[u8] = include_bytes!("../assets/fonts/fa-solid-900.ttf");

#[must_use]
pub fn solid() -> Font {
    Font {
        family: Family::Name(FAMILY),
        weight: Weight::Black,
        ..Font::default()
    }
}

pub mod glyph {
    // Torrent states, matching `theme` status colours.
    pub const LIST: char = '\u{f0c9}';
    pub const DOWNLOAD: char = '\u{f019}';
    pub const UPLOAD: char = '\u{f093}';
    pub const QUEUED: char = '\u{f017}';
    pub const CHECKING: char = '\u{f1ce}';
    pub const PAUSED: char = '\u{f28b}';
    pub const ERROR: char = '\u{f071}';
    pub const ACTIVE: char = '\u{f0e7}';
    pub const CHECK: char = '\u{f058}';
    pub const HOURGLASS: char = '\u{f252}';

    // Navigation and chrome.
    pub const CHEVRON_LEFT: char = '\u{f053}';
    pub const CHEVRON_RIGHT: char = '\u{f054}';
    pub const SERVER: char = '\u{f233}';
    pub const SETTINGS: char = '\u{f013}';
    pub const POWER: char = '\u{f011}';
    pub const CLOSE: char = '\u{f00d}';
    pub const PLUS: char = '\u{2b}';
    pub const TRASH: char = '\u{f2ed}';
    pub const EDIT: char = '\u{f304}';
    pub const REFRESH: char = '\u{f01e}';
    pub const PLAY: char = '\u{f04b}';
    pub const PAUSE: char = '\u{f04c}';
}
