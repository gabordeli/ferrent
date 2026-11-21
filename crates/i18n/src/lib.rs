//! Message catalogue shared by the GUI and the CLI.
//!
//! The English text doubles as the key: `t("All torrents")` returns the
//! Hungarian translation when the UI language is Hungarian, and the English
//! text itself otherwise. A missing translation is not an error — it falls
//! back to English, so a new string never blocks a release.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

/// The languages Ferrent ships.
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

    /// Installs the language named by `FERRENT_LANG`, defaulting to Hungarian.
    pub fn from_env() {
        install(Self::parse(
            &std::env::var("FERRENT_LANG").unwrap_or_default(),
        ));
    }
}

static LANG: RwLock<Lang> = RwLock::new(Lang::Hu);

static CATALOGUE: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    let raw = include_str!("../locales/hu.toml");
    // A broken catalogue must not take a client down; it falls back to English.
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

/// Selects the language; takes effect on the next call to [`t`].
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

/// Translates a message whose English text is only known at run time
/// (clap's generated help, for example).
#[must_use]
pub fn translate(english: &str) -> String {
    match lang() {
        Lang::En => english.to_owned(),
        Lang::Hu => CATALOGUE
            .get(english)
            .map_or_else(|| english.to_owned(), |translated| (*translated).to_owned()),
    }
}

/// Translates a message with `{}` placeholders, filled left to right.
#[must_use]
pub fn t_fmt(english: &'static str, args: &[&str]) -> String {
    let mut text = t(english).to_owned();
    for value in args {
        if let Some(position) = text.find("{}") {
            text.replace_range(position..position + 2, value);
        }
    }
    text
}

/// Every catalogue key, for coverage tests.
pub fn keys() -> impl Iterator<Item = &'static str> {
    CATALOGUE.keys().copied()
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
    fn placeholders_are_filled() {
        install(Lang::Hu);
        assert_eq!(t_fmt("{} active", &["5"]), "5 aktív");
        assert_eq!(
            t_fmt(
                "{} torrents · ↓ {} · ↑ {} · {} free",
                &["13", "1.2 KiB/s", "8.7 KiB/s", "12.1 GiB"]
            ),
            "13 torrent · ↓ 1.2 KiB/s · ↑ 8.7 KiB/s · 12.1 GiB szabad"
        );
        install(Lang::En);
        assert_eq!(t_fmt("{} active", &["5"]), "5 active");
        install(Lang::Hu);
    }

    #[test]
    fn every_entry_translates_without_losing_placeholders() {
        let mut checked = 0;
        for key in keys() {
            let value = translate(key);
            assert_eq!(
                key.matches("{}").count(),
                value.matches("{}").count(),
                "placeholder mismatch for {key:?}"
            );
            checked += 1;
        }
        assert!(checked > 40, "{checked} entries");
    }
}
