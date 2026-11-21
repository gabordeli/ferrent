//! IP blocklist loading (eMule/PeerGuardian text and CIDR lines).
//!
//! Lines come in three shapes and anything else is a warning, not an error:
//!
//! ```text
//! # comment
//! Some label:1.2.3.4-1.2.3.255
//! 1.2.3.4 - 1.2.3.255 , label
//! 10.0.0.0/8
//! ```

use std::net::Ipv4Addr;

/// One inclusive IPv4 range, as numbers so the engine can hand it over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Range {
    pub start: u32,
    pub end: u32,
}

/// Parses a blocklist document. The second value holds the rejected lines.
#[must_use]
pub fn parse(text: &str) -> (Vec<Range>, Vec<String>) {
    let mut ranges = Vec::new();
    let mut warnings = Vec::new();
    for (number, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        match parse_line(line) {
            Some(range) if range.start <= range.end => ranges.push(range),
            Some(_) => warnings.push(format!("line {}: reversed range: {line}", number + 1)),
            None => warnings.push(format!("line {}: cannot parse {line:?}", number + 1)),
        }
    }
    ranges.sort_unstable();
    ranges.dedup();
    (ranges, warnings)
}

fn parse_line(line: &str) -> Option<Range> {
    // `label:start-end` keeps the label free of the range; the label itself
    // may contain anything but a colon.
    let body = match line.rsplit_once(':') {
        Some((_, tail)) if tail.contains('-') => tail.trim(),
        _ => line,
    };
    // A trailing `, label` is decoration.
    let body = body.split(',').next().unwrap_or(body).trim();

    if let Some((network, bits)) = body.split_once('/') {
        let start = network.trim().parse::<Ipv4Addr>().ok()?;
        let bits: u32 = bits.trim().parse().ok()?;
        if bits > 32 {
            return None;
        }
        let mask = if bits == 0 {
            0
        } else {
            u32::MAX << (32 - bits)
        };
        let start = u32::from(start) & mask;
        let end = start | !mask;
        return Some(Range { start, end });
    }

    let (left, right) = body.split_once('-')?;
    let start = left.trim().parse::<Ipv4Addr>().ok()?;
    let end = right.trim().parse::<Ipv4Addr>().ok()?;
    Some(Range {
        start: u32::from(start),
        end: u32::from(end),
    })
}

/// Renders the ranges as `a.b.c.d-a.b.c.d` lines for the engine bridge.
#[must_use]
pub fn to_bridge_text(ranges: &[Range]) -> String {
    let mut text = String::with_capacity(ranges.len() * 32);
    for range in ranges {
        text.push_str(&Ipv4Addr::from(range.start).to_string());
        text.push('-');
        text.push_str(&Ipv4Addr::from(range.end).to_string());
        text.push('\n');
    }
    text
}

/// Loads one source: a local path, or an http(s) URL.
pub async fn load_source(source: &str) -> Result<String, String> {
    let source = source.to_owned();
    tokio::task::spawn_blocking(move || {
        if source.starts_with("http://") || source.starts_with("https://") {
            let mut response = ureq::get(&source)
                .call()
                .map_err(|error| format!("{source}: {error}"))?;
            return response
                .body_mut()
                .read_to_string()
                .map_err(|error| format!("{source}: {error}"));
        }
        std::fs::read_to_string(&source).map_err(|error| format!("{source}: {error}"))
    })
    .await
    .map_err(|error| format!("blocklist task: {error}"))?
}

/// Reloads the blocklist on the configured interval (`0` disables the timer).
pub fn spawn_refresh(state: std::sync::Arc<crate::rpc::AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let hours = state
                .config
                .resolved()
                .map_or(24, |config| config.network.blocklist_refresh_hours);
            if hours == 0 {
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                continue;
            }
            tokio::time::sleep(std::time::Duration::from_secs(hours.saturating_mul(3600))).await;
            let _ = state.refresh_blocklist().await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: [u8; 4], end: [u8; 4]) -> Range {
        Range {
            start: u32::from(Ipv4Addr::from(start)),
            end: u32::from(Ipv4Addr::from(end)),
        }
    }

    #[test]
    fn parses_the_peer_guardian_and_cidr_shapes() {
        let (ranges, warnings) = parse(
            "# header\n\
             \n\
             Bad peers:1.2.3.4-1.2.3.255\n\
             10.0.0.0 - 10.0.0.255 , private\n\
             172.16.0.0/12\n\
             nonsense here\n",
        );
        assert_eq!(
            ranges,
            vec![
                range([1, 2, 3, 4], [1, 2, 3, 255]),
                range([10, 0, 0, 0], [10, 0, 0, 255]),
                range([172, 16, 0, 0], [172, 31, 255, 255]),
            ]
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("cannot parse"));
    }

    #[test]
    fn duplicates_collapse_and_reversed_ranges_warn() {
        let (ranges, warnings) = parse("1.1.1.0/24\n1.1.1.0-1.1.1.255\n2.2.2.9-2.2.2.1\n");
        assert_eq!(ranges, vec![range([1, 1, 1, 0], [1, 1, 1, 255])]);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }

    #[test]
    fn bridge_text_round_trips() {
        let ranges = vec![range([1, 2, 3, 4], [1, 2, 3, 9])];
        assert_eq!(to_bridge_text(&ranges), "1.2.3.4-1.2.3.9\n");
    }
}
