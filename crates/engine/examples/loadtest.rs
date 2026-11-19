//! M0-T4 load harness: add N single-file torrents in seed mode and measure
//! RSS, CPU, file descriptors, startup/load and shutdown time.
//!
//! Run with:
//! `cargo run -p ferrent-engine --example loadtest --features libtorrent -- \
//!      --torrents 4000 --mode seed`

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ferrent_engine::{
    AddTorrent, EngineSettings, LibtorrentEngine, TorrentEngine, TorrentSource, make_torrent,
};

const DEFAULT_TORRENTS: usize = 4_000;
const PAYLOAD_BYTES: usize = 256;
const SETTLE: Duration = Duration::from_secs(10);
const IDLE_WINDOW: Duration = Duration::from_secs(10);
const UPDATE_INTERVAL_MS: u64 = 2_000;

// Debian x86_64 uses 100 clock ticks per second.
const CLK_TCK: f64 = 100.0;

const RSS_BUDGET_MB: f64 = 1024.0;
const IDLE_CPU_BUDGET_PCT: f64 = 5.0;
const STARTUP_BUDGET_SECS: f64 = 30.0;

struct Args {
    torrents: usize,
    mode: String,
    out_dir: PathBuf,
}

impl Args {
    fn parse() -> Self {
        let mut args = Self {
            torrents: DEFAULT_TORRENTS,
            mode: "seed".to_owned(),
            out_dir: PathBuf::from("docs/measurements"),
        };
        let mut iter = env::args().skip(1);
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--torrents" => {
                    args.torrents = iter
                        .next()
                        .expect("--torrents requires a value")
                        .parse()
                        .expect("--torrents must be a number");
                }
                "--mode" => args.mode = iter.next().expect("--mode requires a value"),
                "--out-dir" => {
                    args.out_dir = PathBuf::from(iter.next().expect("--out-dir requires a value"));
                }
                "--help" | "-h" => {
                    println!("usage: loadtest [--torrents N] [--mode seed] [--out-dir DIR]");
                    std::process::exit(0);
                }
                other => panic!("unknown argument: {other}"),
            }
        }
        args
    }
}

struct Report {
    torrents: usize,
    mode: String,
    payload_bytes: usize,
    add_phase_secs: f64,
    rss_kb_after_add: u64,
    rss_kb_after_settle: u64,
    fd_after_add: usize,
    fd_after_settle: usize,
    cpu_active_pct: f64,
    cpu_idle_pct: f64,
    shutdown_secs: f64,
    events: usize,
    events_per_sec: f64,
    update_interval_ms: u64,
}

impl Report {
    fn rss_mb_after_settle(&self) -> f64 {
        self.rss_kb_after_settle as f64 / 1024.0
    }

    fn json(&self) -> String {
        let mut out = String::new();
        writeln!(out, "{{").unwrap();
        writeln!(out, "  \"torrents\": {},", self.torrents).unwrap();
        writeln!(out, "  \"mode\": \"{}\",", self.mode).unwrap();
        writeln!(out, "  \"payload_bytes\": {},", self.payload_bytes).unwrap();
        writeln!(out, "  \"add_phase_secs\": {:.3},", self.add_phase_secs).unwrap();
        writeln!(out, "  \"rss_kb_after_add\": {},", self.rss_kb_after_add).unwrap();
        writeln!(
            out,
            "  \"rss_kb_after_settle\": {},",
            self.rss_kb_after_settle
        )
        .unwrap();
        writeln!(
            out,
            "  \"rss_mb_after_settle\": {:.1},",
            self.rss_mb_after_settle()
        )
        .unwrap();
        writeln!(out, "  \"fd_after_add\": {},", self.fd_after_add).unwrap();
        writeln!(out, "  \"fd_after_settle\": {},", self.fd_after_settle).unwrap();
        writeln!(out, "  \"cpu_active_pct\": {:.2},", self.cpu_active_pct).unwrap();
        writeln!(out, "  \"cpu_idle_pct\": {:.2},", self.cpu_idle_pct).unwrap();
        writeln!(out, "  \"shutdown_secs\": {:.3},", self.shutdown_secs).unwrap();
        writeln!(out, "  \"events\": {},", self.events).unwrap();
        writeln!(out, "  \"events_per_sec\": {:.1},", self.events_per_sec).unwrap();
        writeln!(out, "  \"update_interval_ms\": {}", self.update_interval_ms).unwrap();
        writeln!(out, "}}").unwrap();
        out
    }

    fn markdown(&self) -> String {
        let rss = self.rss_mb_after_settle();
        let mut out = String::new();
        writeln!(
            out,
            "# M0 load test – {} torrents ({})",
            self.torrents, self.mode
        )
        .unwrap();
        writeln!(out).unwrap();
        writeln!(out, "| Metrika | Érték | Budget | Eredmény |").unwrap();
        writeln!(out, "|---|---|---|---|").unwrap();
        writeln!(
            out,
            "| Torrentek | {} | ≥ 4 000 | {} |",
            self.torrents,
            verdict(self.torrents >= 4_000)
        )
        .unwrap();
        writeln!(
            out,
            "| Payload / torrent | {} B | – | – |",
            self.payload_bytes
        )
        .unwrap();
        writeln!(
            out,
            "| Add/load idő | {:.2} s | ≤ {:.0} s | {} |",
            self.add_phase_secs,
            STARTUP_BUDGET_SECS,
            verdict(self.add_phase_secs <= STARTUP_BUDGET_SECS)
        )
        .unwrap();
        writeln!(
            out,
            "| RSS (add után) | {:.1} MB | – | – |",
            self.rss_kb_after_add as f64 / 1024.0
        )
        .unwrap();
        writeln!(
            out,
            "| RSS (settle után) | {:.1} MB | ≤ {:.0} MB | {} |",
            rss,
            RSS_BUDGET_MB,
            verdict(rss <= RSS_BUDGET_MB)
        )
        .unwrap();
        writeln!(
            out,
            "| Nyitott fd (settle) | {} | – | – |",
            self.fd_after_settle
        )
        .unwrap();
        writeln!(
            out,
            "| Aktív CPU (add alatt) | {:.2}% | – | – |",
            self.cpu_active_pct
        )
        .unwrap();
        writeln!(
            out,
            "| Idle CPU ({} s) | {:.2}% | ≤ {:.0}% | {} |",
            IDLE_WINDOW.as_secs(),
            self.cpu_idle_pct,
            IDLE_CPU_BUDGET_PCT,
            verdict(self.cpu_idle_pct <= IDLE_CPU_BUDGET_PCT)
        )
        .unwrap();
        writeln!(out, "| Leállás | {:.2} s | – | – |", self.shutdown_secs).unwrap();
        writeln!(out, "| Események | {} | – | – |", self.events).unwrap();
        writeln!(out, "| Esemény/sec | {:.1} | – | – |", self.events_per_sec).unwrap();
        writeln!(
            out,
            "| Progress update | {} ms | – | – |",
            self.update_interval_ms
        )
        .unwrap();
        writeln!(out).unwrap();
        writeln!(
            out,
            "A budget a daemonra vonatkozik; ez a harness a teljes tesztfolyamat RSS-ét méri, tehát felső becslés."
        )
        .unwrap();
        writeln!(
            out,
            "A progress update {} ms-onként fut; a state-változások alertből azonnal érkeznek.",
            self.update_interval_ms
        )
        .unwrap();
        out
    }

    fn write(&self, out_dir: &Path) -> io::Result<()> {
        fs::create_dir_all(out_dir)?;
        fs::write(out_dir.join("m0-loadtest.json"), self.json())?;
        fs::write(out_dir.join("m0-loadtest.md"), self.markdown())?;
        Ok(())
    }
}

fn verdict(ok: bool) -> &'static str {
    if ok { "PASS" } else { "FAIL" }
}

fn rss_kb() -> u64 {
    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
    for line in status.lines() {
        if let Some(value) = line.strip_prefix("VmRSS:") {
            return value
                .split_whitespace()
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
        }
    }
    0
}

fn fd_count() -> usize {
    fs::read_dir("/proc/self/fd")
        .map(|entries| entries.count())
        .unwrap_or(0)
}

fn cpu_ticks() -> u64 {
    let stat = fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let rest = stat.rsplit_once(')').map_or("", |(_, rest)| rest);
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let utime = fields
        .get(11)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let stime = fields
        .get(12)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    utime + stime
}

fn cpu_pct(start: u64, end: u64, elapsed: Duration) -> f64 {
    let ticks = end.saturating_sub(start) as f64;
    let seconds = elapsed.as_secs_f64();
    if seconds <= 0.0 {
        0.0
    } else {
        (ticks / CLK_TCK) / seconds * 100.0
    }
}

fn main() {
    let args = Args::parse();
    assert!(
        args.mode == "seed",
        "only --mode seed is supported (got {})",
        args.mode
    );

    let dir = env::temp_dir().join(format!("ferrent-loadtest-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create load test dir");

    let mut engine = LibtorrentEngine::new(EngineSettings::default());
    let mut events = engine.subscribe();
    let mut event_count = 0usize;
    let mut payload = vec![b'x'; PAYLOAD_BYTES];

    let add_start = Instant::now();
    let cpu_start = cpu_ticks();
    for index in 0..args.torrents {
        let name = format!("payload-{index:05}.bin");
        let tag = format!("ferrent-{index:05}");
        payload[..tag.len()].copy_from_slice(tag.as_bytes());
        payload[PAYLOAD_BYTES - 1] = (index % 251) as u8;

        fs::write(dir.join(&name), &payload).expect("write payload");
        let metainfo = make_torrent(&name, &payload);
        engine
            .add_torrent(AddTorrent {
                name,
                total_bytes: payload.len() as u64,
                source: TorrentSource::Metainfo(metainfo),
                save_path: dir.clone(),
                resume_data: None,
            })
            .expect("add torrent");

        while events.try_recv().is_ok() {
            event_count += 1;
        }
    }
    let add_phase = add_start.elapsed();
    let cpu_active = cpu_pct(cpu_start, cpu_ticks(), add_phase);
    let rss_after_add = rss_kb();
    let fd_after_add = fd_count();

    std::thread::sleep(SETTLE);
    while events.try_recv().is_ok() {
        event_count += 1;
    }
    let rss_after_settle = rss_kb();
    let fd_after_settle = fd_count();

    let idle_start = Instant::now();
    let idle_cpu_start = cpu_ticks();
    std::thread::sleep(IDLE_WINDOW);
    let idle_phase = idle_start.elapsed();
    let cpu_idle = cpu_pct(idle_cpu_start, cpu_ticks(), idle_phase);
    while events.try_recv().is_ok() {
        event_count += 1;
    }

    let shutdown_start = Instant::now();
    drop(engine);
    let shutdown = shutdown_start.elapsed();

    let report = Report {
        torrents: args.torrents,
        mode: args.mode,
        payload_bytes: PAYLOAD_BYTES,
        add_phase_secs: add_phase.as_secs_f64(),
        rss_kb_after_add: rss_after_add,
        rss_kb_after_settle: rss_after_settle,
        fd_after_add,
        fd_after_settle,
        cpu_active_pct: cpu_active,
        cpu_idle_pct: cpu_idle,
        shutdown_secs: shutdown.as_secs_f64(),
        events: event_count,
        events_per_sec: if add_phase.as_secs_f64() > 0.0 {
            event_count as f64 / add_phase.as_secs_f64()
        } else {
            0.0
        },
        update_interval_ms: UPDATE_INTERVAL_MS,
    };

    report.write(&args.out_dir).expect("write report");
    println!("{}", report.json());
    let _ = fs::remove_dir_all(&dir);
}
