//! Human-readable tracker announce state.

/// Turns the engine's announce state into a short label plus a description.
///
/// The engine reports `working`, `updating`, `unverified` or `error` together
/// with the tracker's own message, the seconds until the next announce and the
/// number of consecutive failures. The label is what the tracker actually did;
/// the detail explains when or why.
pub fn status_text(
    status: &str,
    message: &str,
    next_announce: u32,
    fails: u32,
) -> (&'static str, String) {
    let message = message.trim();
    match status {
        "working" => {
            let detail = if next_announce > 0 {
                format!("answered, next announce in {}", duration(next_announce))
            } else if !message.is_empty() {
                format!("answered: {message}")
            } else {
                "answered".to_owned()
            };
            ("Working", detail)
        }
        "updating" => {
            let detail = if message.is_empty() {
                "announce in flight".to_owned()
            } else {
                message.to_owned()
            };
            ("Announcing", detail)
        }
        "unverified" => (
            "Not announced yet",
            "waiting for the first announce".to_owned(),
        ),
        "error" => {
            let mut detail = if message.is_empty() {
                "announce failed".to_owned()
            } else {
                message.to_owned()
            };
            if fails > 1 {
                detail.push_str(&format!(" ({fails} failed attempts)"));
            }
            ("Error", detail)
        }
        other => ("Unknown", format!("state {other:?}")),
    }
}

/// Renders a second count as a compact, human duration.
fn duration(seconds: u32) -> String {
    match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3599 => format!("{} min", seconds / 60),
        _ => format!("{}h {:02}m", seconds / 3600, (seconds % 3600) / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_says_when_the_next_announce_is_due() {
        assert_eq!(
            status_text("working", "", 125, 0),
            ("Working", "answered, next announce in 2 min".to_owned())
        );
    }

    #[test]
    fn an_error_carries_the_reason_and_the_attempt_count() {
        assert_eq!(
            status_text("error", "Host not found", 0, 4),
            ("Error", "Host not found (4 failed attempts)".to_owned())
        );
    }

    #[test]
    fn an_error_without_a_reason_still_reads_as_a_sentence() {
        assert_eq!(
            status_text("error", "", 0, 1),
            ("Error", "announce failed".to_owned())
        );
    }

    #[test]
    fn an_unannounced_tracker_is_not_an_error() {
        assert_eq!(
            status_text("unverified", "", 0, 0),
            (
                "Not announced yet",
                "waiting for the first announce".to_owned()
            )
        );
    }

    #[test]
    fn durations_stay_short() {
        assert_eq!(duration(45), "45s");
        assert_eq!(duration(600), "10 min");
        assert_eq!(duration(5400), "1h 30m");
    }
}
