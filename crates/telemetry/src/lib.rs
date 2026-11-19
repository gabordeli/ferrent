//! Shared Bugsink/Sentry initialization for every Ferrent binary.

use std::env;
use std::error::Error;
use std::time::Duration;

/// Bugsink project 5 DSN; can be overridden with `FERRENT_BUGSINK_DSN`.
pub const DEFAULT_DSN: &str = "https://13faf88a2a1e46458c7a3fbf35eca231@cirice-bugsink.ddns.net/5";

/// Initializes the client and tags every event with the binary name.
///
/// Keep the returned guard alive for the lifetime of the process.
pub fn init(service: &str) -> sentry::ClientInitGuard {
    let dsn = env::var("FERRENT_BUGSINK_DSN").ok();
    if dsn.as_deref().is_some_and(|value| value.trim().is_empty()) {
        return sentry::init(());
    }
    let dsn = dsn.unwrap_or_else(|| DEFAULT_DSN.to_owned());
    let environment = env::var("FERRENT_ENV").unwrap_or_else(|_| "production".to_owned());
    let mut options = sentry::ClientOptions::default();
    options.release = sentry::release_name!();
    options.environment = Some(environment.into());
    options.attach_stacktrace = true;
    options.send_default_pii = false;
    let guard = sentry::init((dsn, options));
    sentry::configure_scope(|scope| {
        scope.set_tag("service", service.to_owned());
    });
    guard
}

/// Captures a fatal or otherwise actionable error.
pub fn capture_error(error: &dyn Error) {
    sentry::capture_error(error);
}

/// Captures an informational event (useful for startup/control-flow markers).
pub fn capture_message(message: &str) {
    sentry::capture_message(message, sentry::Level::Info);
}

/// Flushes pending events; call before an explicit `process::exit`.
pub fn flush() {
    if let Some(client) = sentry::Hub::main().client() {
        let _ = client.flush(Some(Duration::from_secs(2)));
    }
}
