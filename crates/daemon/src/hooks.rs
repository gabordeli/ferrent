//! Execute hooks: the config contract and the runner for journal events.
//!
//! Hooks are plain argv executions (shell is opt-in), capped by a timeout and
//! four parallel runs. Every run is recorded in the journal; the event is
//! acknowledged only after all matching hooks finished, so a crash replays it.

use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::events::{EventKind, HookEvent, HookRun, NewEvent};
use crate::store::StateStore;

/// Default per-hook timeout.
pub const DEFAULT_TIMEOUT_SECS: u64 = 60;
/// Maximum number of hooks running at once.
pub const MAX_CONCURRENT: usize = 4;
/// Upper bound for the configured timeout.
pub const MAX_TIMEOUT_SECS: u64 = 3600;
/// Captured stdout/stderr tail per run.
const OUTPUT_LIMIT: usize = 4096;

/// One configured hook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct HookSpec {
    pub event: EventKind,
    pub command: String,
    /// argv template; the default is the five-field contract.
    pub args: Vec<String>,
    /// Run the command through `/bin/sh -c` instead of exec.
    pub shell: bool,
    pub timeout_secs: u64,
    pub enabled: bool,
}

impl Default for HookSpec {
    fn default() -> Self {
        Self {
            event: EventKind::Added,
            command: String::new(),
            args: default_args(),
            shell: false,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            enabled: true,
        }
    }
}

/// The default argv after the command.
#[must_use]
pub fn default_args() -> Vec<String> {
    ["{id}", "{name}", "{path}", "{infohash}", "{label}"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

impl HookSpec {
    /// Checks a hook definition before it is saved or executed.
    pub fn validate(&self) -> Result<(), String> {
        if self.command.trim().is_empty() {
            return Err("hook command must not be empty".to_owned());
        }
        if self.timeout_secs == 0 || self.timeout_secs > MAX_TIMEOUT_SECS {
            return Err(format!(
                "hook timeout must be between 1 and {MAX_TIMEOUT_SECS} seconds"
            ));
        }
        Ok(())
    }

    #[must_use]
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs.clamp(1, MAX_TIMEOUT_SECS))
    }
}

/// Renders one argv template for an event.
#[must_use]
pub fn render_args(template: &[String], event: &HookEvent, label: &str) -> Vec<String> {
    template
        .iter()
        .map(|arg| {
            arg.replace("{id}", &event.torrent_id.to_string())
                .replace("{name}", &event.name)
                .replace("{path}", &event.save_path)
                .replace("{infohash}", &event.infohash)
                .replace("{label}", label)
                .replace("{event}", event.kind.as_str())
                .replace("{error}", event.error.as_deref().unwrap_or(""))
        })
        .collect()
}

/// The environment variables handed to every hook.
#[must_use]
pub fn hook_env(event: &HookEvent, label: &str) -> Vec<(String, String)> {
    vec![
        ("FERRENT_EVENT".to_owned(), event.kind.as_str().to_owned()),
        ("FERRENT_ID".to_owned(), event.torrent_id.to_string()),
        ("FERRENT_NAME".to_owned(), event.name.clone()),
        ("FERRENT_PATH".to_owned(), event.save_path.clone()),
        ("FERRENT_INFOHASH".to_owned(), event.infohash.clone()),
        ("FERRENT_LABEL".to_owned(), label.to_owned()),
        (
            "FERRENT_ERROR".to_owned(),
            event.error.clone().unwrap_or_default(),
        ),
    ]
}

/// Quotes one argument for `/bin/sh -c`.
#[must_use]
pub fn shell_quote(argument: &str) -> String {
    if !argument.is_empty()
        && argument
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-/:=@,+".contains(character))
    {
        return argument.to_owned();
    }
    format!("'{}'", argument.replace('\'', "'\\''"))
}

/// Builds the `/bin/sh -c` line for a shell hook.
#[must_use]
pub fn shell_line(spec: &HookSpec, rendered: &[String]) -> String {
    let mut line = spec.command.clone();
    for argument in rendered {
        line.push(' ');
        line.push_str(&shell_quote(argument));
    }
    line
}

/// Executes one hook and returns its journal record.
pub async fn run_hook(spec: &HookSpec, event: &HookEvent, label: &str) -> HookRun {
    let rendered = render_args(&spec.args, event, label);
    let started = Instant::now();
    let mut command = if spec.shell {
        let mut command = tokio::process::Command::new("/bin/sh");
        command.arg("-c").arg(shell_line(spec, &rendered));
        command
    } else {
        let mut command = tokio::process::Command::new(spec.command.trim());
        command.args(&rendered);
        command
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .envs(hook_env(event, label))
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        command.process_group(0);
    }

    let mut run = HookRun {
        seq: 0,
        event_seq: event.seq,
        event: event.kind,
        torrent_id: event.torrent_id,
        torrent_name: event.name.clone(),
        command: spec.command.clone(),
        exit_code: None,
        timed_out: false,
        duration_ms: 0,
        stdout: String::new(),
        stderr: String::new(),
        at_unix: 0,
    };

    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            run.stderr = format!("spawn failed: {error}");
            run.duration_ms = elapsed_millis(started);
            return run;
        }
    };
    let pid = child.id();

    match tokio::time::timeout(spec.timeout(), child.wait_with_output()).await {
        Ok(Ok(output)) => {
            run.exit_code = output.status.code();
            run.stdout = tail(&output.stdout);
            run.stderr = tail(&output.stderr);
        }
        Ok(Err(error)) => {
            run.stderr = format!("wait failed: {error}");
        }
        Err(_) => {
            run.timed_out = true;
            if let Some(pid) = pid {
                kill_tree(pid);
            }
        }
    }
    run.duration_ms = elapsed_millis(started);
    run
}

/// Kills a timed-out hook together with anything it started.
///
/// Unix gives the hook its own process group (`process_group(0)`), so one
/// group signal reaches the whole tree. Windows has no equivalent on a
/// console child, so `taskkill /T` walks the tree by parent pid instead.
fn kill_tree(pid: u32) {
    #[cfg(unix)]
    {
        use rustix::process::{Pid, Signal, kill_process_group};
        if let Some(pid) = Pid::from_raw(i32::try_from(pid).unwrap_or(i32::MAX)) {
            let _ = kill_process_group(pid, Signal::KILL);
        }
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    #[cfg(not(any(unix, windows)))]
    let _ = pid;
}

fn elapsed_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn tail(bytes: &[u8]) -> String {
    let start = bytes.len().saturating_sub(OUTPUT_LIMIT);
    String::from_utf8_lossy(&bytes[start..])
        .trim_end()
        .to_owned()
}

/// Picks the enabled hooks for one event.
#[must_use]
pub fn matching<'a>(hooks: &'a [HookSpec], event: &HookEvent) -> Vec<&'a HookSpec> {
    hooks
        .iter()
        .filter(|hook| hook.enabled && hook.event == event.kind)
        .collect()
}

/// Reads the pending events from the store.
pub async fn pending_events(
    store: Arc<std::sync::Mutex<Box<dyn StateStore>>>,
) -> Result<Vec<HookEvent>, String> {
    tokio::task::spawn_blocking(move || {
        let store = store.lock().expect("state store mutex poisoned");
        crate::events::pending(&**store).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Records one run and acknowledges the event when `ack` is set.
pub async fn store_run(
    store: Arc<std::sync::Mutex<Box<dyn StateStore>>>,
    run: HookRun,
    ack: Option<u64>,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let mut store = store.lock().expect("state store mutex poisoned");
        crate::events::record_run(&mut **store, run).map_err(|error| error.to_string())?;
        if let Some(seq) = ack {
            crate::events::ack(&mut **store, seq).map_err(|error| error.to_string())?;
        }
        store.flush().map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Acknowledges one event without running hooks.
pub async fn ack_event(
    store: Arc<std::sync::Mutex<Box<dyn StateStore>>>,
    seq: u64,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let mut store = store.lock().expect("state store mutex poisoned");
        crate::events::ack(&mut **store, seq).map_err(|error| error.to_string())?;
        store.flush().map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Appends one event to the journal.
pub async fn record_event(
    store: Arc<std::sync::Mutex<Box<dyn StateStore>>>,
    event: NewEvent,
) -> Result<HookEvent, String> {
    tokio::task::spawn_blocking(move || {
        let mut store = store.lock().expect("state store mutex poisoned");
        let event =
            crate::events::append(&mut **store, event).map_err(|error| error.to_string())?;
        store.flush().map_err(|error| error.to_string())?;
        Ok(event)
    })
    .await
    .map_err(|error| error.to_string())?
}

impl crate::rpc::AppState {
    /// Runs every hook that matches the pending journal events.
    ///
    /// Events without a matching hook are acknowledged immediately; an event
    /// is acknowledged only after all of its runs were recorded.
    pub async fn run_pending_hooks(&self) -> Result<usize, String> {
        let config = self.config.resolved().map_err(|error| error.to_string())?;
        let hooks = config.hook;
        let events = pending_events(Arc::clone(&self.store)).await?;
        if events.is_empty() {
            return Ok(0);
        }
        let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT));
        let mut executed = 0;
        for event in events {
            let selected: Vec<HookSpec> = hooks
                .iter()
                .filter(|hook| hook.enabled && hook.event == event.kind)
                .cloned()
                .collect();
            if selected.is_empty() {
                ack_event(Arc::clone(&self.store), event.seq).await?;
                continue;
            }
            let label = self
                .labels
                .lock()
                .expect("label state poisoned")
                .label(event.torrent_id)
                .unwrap_or_default()
                .to_owned();
            let mut handles = Vec::with_capacity(selected.len());
            for spec in selected {
                let permit = Arc::clone(&semaphore)
                    .acquire_owned()
                    .await
                    .map_err(|error| error.to_string())?;
                let event = event.clone();
                let label = label.clone();
                handles.push(tokio::spawn(async move {
                    let run = run_hook(&spec, &event, &label).await;
                    drop(permit);
                    run
                }));
            }
            for handle in handles {
                let run = handle.await.map_err(|error| error.to_string())?;
                store_run(Arc::clone(&self.store), run, None).await?;
                executed += 1;
            }
            ack_event(Arc::clone(&self.store), event.seq).await?;
        }
        Ok(executed)
    }
}

impl crate::rpc::AppState {
    /// Appends one hook event to the journal.
    pub async fn append_hook_event(&self, event: NewEvent) -> Result<HookEvent, String> {
        record_event(Arc::clone(&self.store), event).await
    }
}

/// Starts the hook runner loop.
pub fn spawn_hook_runner(state: Arc<crate::rpc::AppState>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(error) = crate::rpc::AppState::run_pending_hooks(&state).await {
                eprintln!("hook runner: {error}");
            }
        }
    });
}

/// Whether a command path looks executable; used by the config validator.
#[must_use]
pub fn is_executable(path: &str) -> bool {
    let path = Path::new(path.trim());
    path.is_absolute() && path.is_file()
}

/// Creates an event for a newly added torrent.
#[must_use]
pub fn added_event(id: u64, name: &str, save_path: &Path, infohash: &str) -> NewEvent {
    NewEvent::new(EventKind::Added, id, name)
        .with_path(save_path.display().to_string())
        .with_infohash(infohash)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event() -> HookEvent {
        HookEvent {
            seq: 9,
            kind: EventKind::Finished,
            torrent_id: 42,
            name: "movie.mkv".to_owned(),
            save_path: "/data/movies".to_owned(),
            infohash: "ab".repeat(20),
            error: None,
            at_unix: 1,
        }
    }

    #[test]
    fn argv_placeholders_render_from_the_event() {
        let rendered = render_args(&default_args(), &event(), "movies");
        assert_eq!(rendered[0], "42");
        assert_eq!(rendered[1], "movie.mkv");
        assert_eq!(rendered[2], "/data/movies");
        assert_eq!(rendered[3], "ab".repeat(20));
        assert_eq!(rendered[4], "movies");

        let custom = vec!["{event}:{id}".to_owned()];
        assert_eq!(render_args(&custom, &event(), ""), vec!["finished:42"]);
    }

    #[test]
    fn env_carries_the_same_contract() {
        let env = hook_env(&event(), "movies");
        let get = |key: &str| {
            env.iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.as_str())
                .unwrap_or_default()
        };
        assert_eq!(get("FERRENT_EVENT"), "finished");
        assert_eq!(get("FERRENT_ID"), "42");
        assert_eq!(get("FERRENT_NAME"), "movie.mkv");
        assert_eq!(get("FERRENT_PATH"), "/data/movies");
        assert_eq!(get("FERRENT_LABEL"), "movies");
        assert_eq!(get("FERRENT_ERROR"), "");
    }

    #[test]
    fn shell_quoting_keeps_simple_values_bare() {
        assert_eq!(shell_quote("/data/movie.mkv"), "/data/movie.mkv");
        assert_eq!(shell_quote("two words"), "'two words'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn shell_line_appends_quoted_arguments() {
        let spec = HookSpec {
            command: "/usr/bin/notify".to_owned(),
            shell: true,
            ..HookSpec::default()
        };
        let line = shell_line(&spec, &["a b".to_owned(), "42".to_owned()]);
        assert_eq!(line, "/usr/bin/notify 'a b' 42");
    }

    #[test]
    fn validation_rejects_empty_commands_and_bad_timeouts() {
        let mut spec = HookSpec::default();
        assert!(spec.validate().is_err());
        spec.command = "/bin/true".to_owned();
        assert!(spec.validate().is_ok());
        spec.timeout_secs = 0;
        assert!(spec.validate().is_err());
        spec.timeout_secs = MAX_TIMEOUT_SECS + 1;
        assert!(spec.validate().is_err());
        spec.timeout_secs = MAX_TIMEOUT_SECS;
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn matching_filters_by_event_and_enabled() {
        let hooks = vec![
            HookSpec {
                command: "/bin/a".to_owned(),
                event: EventKind::Finished,
                ..HookSpec::default()
            },
            HookSpec {
                command: "/bin/b".to_owned(),
                event: EventKind::Added,
                ..HookSpec::default()
            },
            HookSpec {
                command: "/bin/c".to_owned(),
                event: EventKind::Finished,
                enabled: false,
                ..HookSpec::default()
            },
        ];
        let matches = matching(&hooks, &event());
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].command, "/bin/a");
    }
}
