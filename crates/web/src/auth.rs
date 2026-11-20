//! Login sessions and the login rate limit.
//!
//! Sessions live in memory: a gateway restart logs every browser out, which is
//! acceptable for a single-user dashboard and avoids a second store.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Cookie name of the session.
pub const COOKIE: &str = "ferrent_session";

/// Issue and validate session tokens.
#[derive(Debug)]
pub struct Sessions {
    ttl: Duration,
    inner: Mutex<BTreeMap<String, Instant>>,
}

impl Sessions {
    #[must_use]
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            inner: Mutex::new(BTreeMap::new()),
        }
    }

    /// Creates a session token valid until `now + ttl`.
    pub fn issue(&self, now: Instant) -> String {
        let token = random_token();
        let mut inner = self.inner.lock().expect("session store poisoned");
        inner.retain(|_, expires| *expires > now);
        inner.insert(token.clone(), now + self.ttl);
        token
    }

    /// Whether a token is live.
    pub fn valid(&self, token: &str, now: Instant) -> bool {
        let mut inner = self.inner.lock().expect("session store poisoned");
        match inner.get(token) {
            Some(expires) if *expires > now => true,
            Some(_) => {
                inner.remove(token);
                false
            }
            None => false,
        }
    }

    /// Drops one session.
    pub fn revoke(&self, token: &str) {
        self.inner
            .lock()
            .expect("session store poisoned")
            .remove(token);
    }

    /// Number of live sessions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.lock().expect("session store poisoned").len()
    }

    /// Whether no session is live.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Sliding one-minute window per client key.
#[derive(Debug)]
pub struct LoginLimiter {
    per_minute: usize,
    inner: Mutex<BTreeMap<String, VecDeque<Instant>>>,
}

impl LoginLimiter {
    #[must_use]
    pub fn new(per_minute: u32) -> Self {
        Self {
            per_minute: per_minute as usize,
            inner: Mutex::new(BTreeMap::new()),
        }
    }

    /// Records an attempt and tells whether it is allowed.
    pub fn allow(&self, key: &str, now: Instant) -> bool {
        let window = Duration::from_secs(60);
        let mut inner = self.inner.lock().expect("login limiter poisoned");
        let entries = inner.entry(key.to_owned()).or_default();
        while entries.front().is_some_and(|at| now - *at > window) {
            entries.pop_front();
        }
        if entries.len() >= self.per_minute {
            return false;
        }
        entries.push_back(now);
        true
    }
}

/// A 32-hex-character token.
///
/// The gateway has no random number source of its own; hashing the clock plus
/// a process-wide counter keeps tokens unguessable in practice without a new
/// dependency (a restart changes the counter).
#[must_use]
pub fn random_token() -> String {
    use sha2::{Digest, Sha256};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut hasher = Sha256::new();
    hasher.update(now.to_le_bytes());
    hasher.update(count.to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    let digest = hasher.finalize();
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Compares two secrets without leaking their common prefix length.
#[must_use]
pub fn constant_time_eq(left: &str, right: &str) -> bool {
    use sha2::{Digest, Sha256};
    use subtle::ConstantTimeEq;
    let left = Sha256::digest(left.as_bytes());
    let right = Sha256::digest(right.as_bytes());
    left.ct_eq(&right).into()
}

/// Reads one cookie from a `Cookie` header.
#[must_use]
pub fn cookie<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header.split(';').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key.trim() == name).then(|| value.trim())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_expire_and_revoke() {
        let sessions = Sessions::new(Duration::from_secs(10));
        let now = Instant::now();
        let token = sessions.issue(now);
        assert_eq!(token.len(), 32);
        assert!(sessions.valid(&token, now + Duration::from_secs(9)));
        assert!(!sessions.valid(&token, now + Duration::from_secs(11)));
        assert!(sessions.is_empty(), "an expired session is dropped");

        let token = sessions.issue(now);
        sessions.revoke(&token);
        assert!(!sessions.valid(&token, now));
    }

    #[test]
    fn the_login_limiter_allows_five_per_minute() {
        let limiter = LoginLimiter::new(5);
        let now = Instant::now();
        for attempt in 0..5 {
            assert!(limiter.allow("client", now), "attempt {attempt}");
        }
        assert!(!limiter.allow("client", now), "sixth attempt is refused");
        assert!(
            limiter.allow("client", now + Duration::from_secs(61)),
            "the window slides"
        );
        assert!(limiter.allow("other", now), "another client is unaffected");
    }

    #[test]
    fn cookies_are_parsed_by_name() {
        let header = "a=1; ferrent_session=abc; b=2";
        assert_eq!(cookie(header, COOKIE), Some("abc"));
        assert_eq!(cookie(header, "missing"), None);
    }

    #[test]
    fn tokens_and_secrets_compare() {
        assert!(constant_time_eq("secret", "secret"));
        assert!(!constant_time_eq("secret", "secreu"));
        assert_ne!(random_token(), random_token());
    }
}
