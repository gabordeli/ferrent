//! RSS/Atom watcher: parse a feed, pick the `.torrent` items, and let the
//! daemon add them once.
//!
//! The daemon orchestrates (it owns the engine and the store); this module is
//! the pure part: fetching, parsing and the item identity used for dedupe.

use std::sync::OnceLock;
use std::time::Duration;

use regex::Regex;

/// One feed entry that may carry a torrent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Item {
    pub title: String,
    /// Stable identity: the guid/id, falling back to the link.
    pub id: String,
    /// The item's page link (may itself be a .torrent or a magnet).
    pub link: String,
    /// `<enclosure url="…">`, the usual place for the .torrent file.
    pub enclosure: String,
}

impl Item {
    /// The URL to download: the enclosure wins, the link is the fallback.
    #[must_use]
    pub fn torrent_url(&self) -> &str {
        if self.enclosure.is_empty() {
            &self.link
        } else {
            &self.enclosure
        }
    }

    /// Dedupe key; feeds without guids fall back to the link, then the title.
    #[must_use]
    pub fn key(&self) -> String {
        if !self.id.is_empty() {
            self.id.clone()
        } else if !self.link.is_empty() {
            self.link.clone()
        } else {
            self.title.clone()
        }
    }
}

/// True for URLs the daemon can turn into a torrent on its own.
#[must_use]
pub fn looks_like_torrent(url: &str) -> bool {
    let url = url.trim();
    url.starts_with("magnet:")
        || url.starts_with("http://") && url.ends_with(".torrent")
        || url.starts_with("https://") && url.ends_with(".torrent")
}

/// Parses RSS 2.0 and Atom into items, ignoring everything else.
///
/// Feeds are small and machine-generated, so a tag-level regex is enough and
/// keeps the daemon free of an XML stack; CDATA and the five predefined
/// entities are handled.
pub fn parse(xml: &str) -> Result<Vec<Item>, String> {
    let mut items = Vec::new();
    for capture in item_regex().captures_iter(xml) {
        let block = &capture[1];
        let title = tag_text(block, "title").unwrap_or_default();
        let id = tag_text(block, "guid")
            .or_else(|| tag_text(block, "id"))
            .unwrap_or_default();
        let link = attribute(block, "link", "href")
            .or_else(|| tag_text(block, "link"))
            .unwrap_or_default();
        let enclosure = attribute(block, "enclosure", "url").unwrap_or_default();
        items.push(Item {
            title,
            id,
            link,
            enclosure,
        });
    }
    if items.is_empty() && !xml.contains("<item") && !xml.contains("<entry") {
        // A feed with no entries is fine; something that is not a feed is not.
        if !xml.contains("<rss") && !xml.contains("<feed") {
            return Err("not an RSS/Atom document".to_owned());
        }
    }
    Ok(items)
}

fn item_regex() -> &'static Regex {
    static ITEM: OnceLock<Regex> = OnceLock::new();
    ITEM.get_or_init(|| {
        Regex::new(r"(?s)<(?:item|entry)\b[^>]*>(.*?)</(?:item|entry)>").expect("item regex")
    })
}

/// Fetches the text of a simple `<tag>text</tag>` pair.
fn tag_text(block: &str, tag: &str) -> Option<String> {
    let pattern = format!(r"(?s)<{tag}\b[^>]*>(.*?)</{tag}>");
    let regex = Regex::new(&pattern).ok()?;
    let raw = regex.captures(block)?.get(1)?.as_str().trim();
    let raw = raw
        .strip_prefix("<![CDATA[")
        .and_then(|inner| inner.strip_suffix("]]>"))
        .unwrap_or(raw);
    Some(unescape(raw))
}

/// Fetches `attr="value"` (or single quotes) from the first `<tag …>`.
fn attribute(block: &str, tag: &str, attr: &str) -> Option<String> {
    let pattern = format!(r#"(?s)<{tag}\b[^>]*\b{attr}\s*=\s*["']([^"']*)["']"#);
    let regex = Regex::new(&pattern).ok()?;
    Some(unescape(regex.captures(block)?.get(1)?.as_str().trim()))
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
        .trim()
        .to_owned()
}

/// Result of one conditional GET.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub body: String,
    pub etag: Option<String>,
    pub modified: Option<String>,
    /// The server answered 304; there is no body to parse.
    pub not_modified: bool,
}

/// Downloads one feed, honouring `If-None-Match` / `If-Modified-Since`.
pub fn fetch(
    url: &str,
    etag: Option<&str>,
    modified: Option<&str>,
    timeout: Duration,
) -> Result<Fetched, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(
            timeout.clamp(Duration::from_secs(2), Duration::from_secs(60)),
        ))
        .http_status_as_error(false)
        .build()
        .into();
    let mut request = agent.get(url).header("User-Agent", "ferrent-rss");
    if let Some(etag) = etag {
        request = request.header("If-None-Match", etag);
    }
    if let Some(modified) = modified {
        request = request.header("If-Modified-Since", modified);
    }
    let response = request.call().map_err(|error| error.to_string())?;
    let status = response.status().as_u16();
    if status == 304 {
        return Ok(Fetched {
            body: String::new(),
            etag: etag.map(str::to_owned),
            modified: modified.map(str::to_owned),
            not_modified: true,
        });
    }
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status}"));
    }
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let etag = header("etag");
    let modified = header("last-modified");
    let body = response
        .into_body()
        .read_to_string()
        .map_err(|error| error.to_string())?;
    Ok(Fetched {
        body,
        etag,
        modified,
        not_modified: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rss_items_with_enclosures() {
        let xml = r#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>feed</title>
<item><title>First</title><guid>g-1</guid>
  <link>https://x.example/1</link>
  <enclosure url="https://x.example/1.torrent" type="application/x-bittorrent"/>
</item>
<item><title>Second</title><link>https://x.example/2.torrent</link></item>
</channel></rss>"#;
        let items = parse(xml).expect("parse");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "First");
        assert_eq!(items[0].key(), "g-1");
        assert_eq!(items[0].torrent_url(), "https://x.example/1.torrent");
        assert_eq!(items[1].title, "Second");
        assert_eq!(items[1].key(), "https://x.example/2.torrent");
    }

    #[test]
    fn parses_atom_entries_with_href_links() {
        let xml = r#"<?xml version="1.0"?>
<feed xmlns="http://www.w3.org/2005/Atom"><title>f</title>
<entry><title>A</title><id>urn:1</id><link href="https://x.example/a.torrent"/></entry>
</feed>"#;
        let items = parse(xml).expect("parse");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "urn:1");
        assert_eq!(items[0].link, "https://x.example/a.torrent");
        assert!(looks_like_torrent(items[0].torrent_url()));
    }

    #[test]
    fn only_torrent_urls_are_candidates() {
        assert!(looks_like_torrent("https://x/a.torrent"));
        assert!(looks_like_torrent("magnet:?xt=urn:btih:abc"));
        assert!(!looks_like_torrent("https://x/a.html"));
        assert!(!looks_like_torrent(""));
    }
}
