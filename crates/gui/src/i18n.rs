//! Re-export of the catalogue shared with the CLI (`ferrent-i18n`).
//!
//! Keeping the module path stable means the GUI code and its tests read the
//! same as before while the entries live in one place.

pub use ferrent_i18n::{Lang, install, lang, t, t_fmt, translate};
