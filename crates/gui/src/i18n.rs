//! Message catalogue.
//!
//! The English text doubles as the key: `t("All torrents")` returns the
//! Hungarian translation when the UI language is Hungarian, and the English
//! text itself otherwise. A missing translation is not an error — it falls
//! back to English, so a new string never blocks a release.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

/// The languages the GUI ships.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    Hu,
    En,
}

impl Lang {
    /// Parses `hu`/`en` (case-insensitive); anything else is `hu`.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "en" | "en-us" | "en-gb" => Self::En,
            _ => Self::Hu,
        }
    }

    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Hu => "hu",
            Self::En => "en",
        }
    }
}

static LANG: RwLock<Lang> = RwLock::new(Lang::Hu);

static CATALOGUE: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    let raw = include_str!("../locales/hu.toml");
    // A broken catalogue must not take the GUI down; it falls back to English.
    let table: HashMap<String, String> = toml::from_str(raw).unwrap_or_default();
    table
        .into_iter()
        .map(|(key, value)| {
            let key: &'static str = Box::leak(key.into_boxed_str());
            let value: &'static str = Box::leak(value.into_boxed_str());
            (key, value)
        })
        .collect()
});

/// Selects the UI language; takes effect on the next frame.
pub fn install(lang: Lang) {
    *LANG.write().expect("language lock poisoned") = lang;
}

/// The language in use.
#[must_use]
pub fn lang() -> Lang {
    *LANG.read().expect("language lock poisoned")
}

/// Translates one message; unknown keys stay English.
#[must_use]
pub fn t(english: &'static str) -> &'static str {
    match lang() {
        Lang::En => english,
        Lang::Hu => CATALOGUE.get(english).copied().unwrap_or(english),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hungarian_is_the_default_and_english_falls_back() {
        install(Lang::Hu);
        assert_eq!(t("All torrents"), "Összes torrent");
        // Unknown keys keep the English text.
        assert_eq!(t("Some brand new label"), "Some brand new label");

        install(Lang::En);
        assert_eq!(t("All torrents"), "All torrents");
        install(Lang::Hu);
    }

    #[test]
    fn language_codes_round_trip() {
        assert_eq!(Lang::parse("en"), Lang::En);
        assert_eq!(Lang::parse("EN-GB"), Lang::En);
        assert_eq!(Lang::parse("hu"), Lang::Hu);
        assert_eq!(Lang::parse("de"), Lang::Hu);
    }

    #[test]
    fn the_catalogue_is_not_empty() {
        assert!(CATALOGUE.len() > 40, "{} entries", CATALOGUE.len());
    }
}
