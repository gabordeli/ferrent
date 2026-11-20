//! Scheduler: a 24×7 grid of green/yellow/red hours in a timezone.
//!
//! Green restores the session settings, yellow applies the scheduler limits,
//! and red pauses the running torrents with the [`PauseReason::Scheduler`]
//! reason only, so manual and guard pauses stay untouched. The task wakes at
//! the next hour boundary (absolute time, which is DST-safe) and re-evaluates
//! after a suspend instead of trusting a stale sleeping deadline.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Datelike, Timelike, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::config::ResolvedConfig;
use crate::control::PauseReason;
use ferrent_engine::TorrentState;

pub const DAYS: usize = 7;
pub const HOURS: usize = 24;

/// One scheduling level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Session defaults; scheduler pauses cleared.
    #[default]
    Green,
    /// Scheduler rate limits.
    Yellow,
    /// Pause the running torrents with the scheduler reason.
    Red,
}

impl Level {
    pub const ALL: [Self; 3] = [Self::Green, Self::Yellow, Self::Red];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Green => "green",
            Self::Yellow => "yellow",
            Self::Red => "red",
        }
    }

    #[must_use]
    pub fn letter(self) -> char {
        match self {
            Self::Green => 'g',
            Self::Yellow => 'y',
            Self::Red => 'r',
        }
    }

    #[must_use]
    pub fn from_letter(letter: char) -> Option<Self> {
        match letter {
            'g' => Some(Self::Green),
            'y' => Some(Self::Yellow),
            'r' => Some(Self::Red),
            _ => None,
        }
    }

    /// Next level when a grid cell is clicked.
    #[must_use]
    pub fn cycle(self) -> Self {
        match self {
            Self::Green => Self::Yellow,
            Self::Yellow => Self::Red,
            Self::Red => Self::Green,
        }
    }
}

impl std::fmt::Display for Level {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.label())
    }
}

/// The week grid: seven rows of 24 levels, Monday first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    days: Vec<[Level; HOURS]>,
}

impl Default for Grid {
    fn default() -> Self {
        Self {
            days: vec![[Level::Green; HOURS]; DAYS],
        }
    }
}

impl Grid {
    /// Parses seven rows of 24 `g`/`y`/`r` characters.
    pub fn parse(week: &[String]) -> Result<Self, String> {
        if week.len() != DAYS {
            return Err(format!("scheduler week must have {DAYS} rows"));
        }
        let mut days = Vec::with_capacity(DAYS);
        for (index, row) in week.iter().enumerate() {
            let letters: Vec<char> = row.chars().collect();
            if letters.len() != HOURS {
                return Err(format!(
                    "scheduler week row {} must have {HOURS} characters",
                    index + 1
                ));
            }
            let mut levels = [Level::Green; HOURS];
            for (hour, letter) in letters.into_iter().enumerate() {
                levels[hour] = Level::from_letter(letter).ok_or_else(|| {
                    format!(
                        "scheduler week row {} hour {}: expected g, y or r",
                        index + 1,
                        hour
                    )
                })?;
            }
            days.push(levels);
        }
        Ok(Self { days })
    }

    /// Renders the grid back into the config representation.
    #[must_use]
    pub fn render(&self) -> Vec<String> {
        self.days
            .iter()
            .map(|row| row.iter().map(|level| level.letter()).collect())
            .collect()
    }

    /// Level for a weekday (`0` = Monday) and an hour.
    #[must_use]
    pub fn level(&self, weekday: u32, hour: u32) -> Level {
        let day = usize::try_from(weekday).unwrap_or(0).min(DAYS - 1);
        let hour = usize::try_from(hour).unwrap_or(0).min(HOURS - 1);
        self.days[day][hour]
    }

    /// Sets one cell.
    pub fn set(&mut self, day: usize, hour: usize, level: Level) {
        if let Some(row) = self.days.get_mut(day) {
            if let Some(cell) = row.get_mut(hour) {
                *cell = level;
            }
        }
    }
}

/// The slot that applies now and the instant the next slot starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Boundary {
    pub level: Level,
    pub weekday: u32,
    pub hour: u32,
    /// Unix seconds of the next hour boundary (absolute time).
    pub next_unix: i64,
}

/// Evaluates the grid for `now` in `timezone`.
///
/// The next boundary is the next absolute hour: at a DST spring-forward the
/// local hour jumps, and at a fall-back it repeats; both are handled by
/// re-reading the local hour at the boundary instead of caching the slot.
#[must_use]
pub fn evaluate(grid: &Grid, timezone: Tz, now: DateTime<Utc>) -> Boundary {
    let local = now.with_timezone(&timezone);
    let weekday = local.weekday().num_days_from_monday();
    let hour = local.hour();
    Boundary {
        level: grid.level(weekday, hour),
        weekday,
        hour,
        next_unix: (now.timestamp() / 3600 + 1) * 3600,
    }
}

/// Runtime scheduler state, shared with `GetStatus`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SchedulerState {
    pub level: Option<Level>,
    pub next_unix: i64,
    pub applied_unix: i64,
    pub last_error: Option<String>,
}

impl crate::rpc::AppState {
    /// Applies one scheduler tick; returns the new level when it changed.
    pub async fn scheduler_tick(&self, now: DateTime<Utc>) -> Result<Option<Level>, String> {
        let config = self.config.resolved().map_err(|error| error.to_string())?;
        if !config.scheduler.enabled {
            let was_active = self
                .scheduler
                .lock()
                .expect("scheduler mutex poisoned")
                .level
                .is_some();
            {
                let mut state = self.scheduler.lock().expect("scheduler mutex poisoned");
                state.level = None;
                state.next_unix = 0;
            }
            if was_active {
                self.apply_scheduler_level(Level::Green, &config).await?;
            }
            return Ok(None);
        }

        let timezone: Tz =
            config.scheduler.timezone.parse().map_err(|_| {
                format!("unknown scheduler timezone {:?}", config.scheduler.timezone)
            })?;
        let grid = Grid::parse(&config.scheduler.week)?;
        let boundary = evaluate(&grid, timezone, now);
        {
            let mut state = self.scheduler.lock().expect("scheduler mutex poisoned");
            state.next_unix = boundary.next_unix;
        }
        if self
            .scheduler
            .lock()
            .expect("scheduler mutex poisoned")
            .level
            == Some(boundary.level)
        {
            return Ok(None);
        }
        match self.apply_scheduler_level(boundary.level, &config).await {
            Ok(()) => {
                let mut state = self.scheduler.lock().expect("scheduler mutex poisoned");
                state.level = Some(boundary.level);
                state.applied_unix = now.timestamp();
                Ok(Some(boundary.level))
            }
            Err(error) => Err(error),
        }
    }

    /// Applies one level to the engine and the pause authority.
    pub async fn apply_scheduler_level(
        &self,
        level: Level,
        config: &ResolvedConfig,
    ) -> Result<(), String> {
        let paused_by_scheduler: Vec<u64> = self
            .control
            .lock()
            .expect("pause control mutex poisoned")
            .scheduler_paused()
            .collect();
        if level != Level::Red && !paused_by_scheduler.is_empty() {
            let changed = self
                .control
                .lock()
                .expect("pause control mutex poisoned")
                .set_many(&paused_by_scheduler, PauseReason::Scheduler, false);
            if !changed.is_empty() {
                self.engine_call(move |engine| engine.resume_many(&changed))
                    .await?;
            }
        }

        match level {
            Level::Green => {
                let settings = session_settings(config, None);
                self.engine_call(move |engine| engine.set_settings(settings))
                    .await?;
                Ok(())
            }
            Level::Yellow => {
                let settings = session_settings(
                    config,
                    Some((
                        config.scheduler.download_rate_limit,
                        config.scheduler.upload_rate_limit,
                    )),
                );
                self.engine_call(move |engine| engine.set_settings(settings))
                    .await?;
                Ok(())
            }
            Level::Red => {
                let user_paused: std::collections::BTreeSet<u64> = self
                    .control
                    .lock()
                    .expect("pause control mutex poisoned")
                    .user_paused()
                    .collect();
                let candidates: Vec<u64> = self
                    .mirror
                    .all()
                    .into_iter()
                    .filter(|status| status.state != TorrentState::Paused)
                    .filter(|status| {
                        config.scheduler.pause_seeding || status.state != TorrentState::Seeding
                    })
                    .map(|status| status.id)
                    .filter(|id| !user_paused.contains(id))
                    .collect();
                let changed = self
                    .control
                    .lock()
                    .expect("pause control mutex poisoned")
                    .set_many(&candidates, PauseReason::Scheduler, true);
                if changed.is_empty() {
                    return Ok(());
                }
                self.engine_call(move |engine| engine.pause_many(&changed))
                    .await?;
                Ok(())
            }
        }
    }

    /// Current scheduler state for the status RPC.
    #[must_use]
    pub fn scheduler_status(&self) -> SchedulerState {
        self.scheduler
            .lock()
            .expect("scheduler mutex poisoned")
            .clone()
    }

    /// Runs an engine operation on the blocking pool.
    pub async fn engine_call<T, F>(&self, operation: F) -> Result<T, String>
    where
        F: FnOnce(&mut dyn ferrent_engine::TorrentEngine) -> T + Send + 'static,
        T: Send + 'static,
    {
        let engine = Arc::clone(&self.engine);
        tokio::task::spawn_blocking(move || {
            let mut engine = engine.lock().expect("engine mutex poisoned");
            operation(&mut **engine)
        })
        .await
        .map_err(|error| error.to_string())
    }
}

fn session_settings(
    config: &ResolvedConfig,
    limits: Option<(u64, u64)>,
) -> ferrent_engine::EngineSettings {
    let (download_rate_limit, upload_rate_limit) = limits.unwrap_or((
        config.session.download_rate_limit,
        config.session.upload_rate_limit,
    ));
    ferrent_engine::EngineSettings {
        download_rate_limit,
        upload_rate_limit,
        seed_mode: config.session.seed_mode,
        active_downloads: config.session.active_downloads,
        active_seeds: config.session.active_seeds,
        active_limit: config.session.active_limit,
    }
}

/// Starts the scheduler loop; it re-evaluates at the next boundary and at
/// most every 30 s so config edits apply quickly.
pub fn spawn_scheduler(state: Arc<crate::rpc::AppState>) {
    tokio::spawn(async move {
        loop {
            if let Err(error) = state.scheduler_tick(Utc::now()).await {
                let mut scheduler = state.scheduler.lock().expect("scheduler mutex poisoned");
                scheduler.last_error = Some(error);
            }
            let next = state
                .scheduler
                .lock()
                .expect("scheduler mutex poisoned")
                .next_unix;
            let now = Utc::now().timestamp();
            let until = if next > now { next - now } else { 30 };
            let wait = u64::try_from(until.clamp(1, 30)).unwrap_or(30);
            tokio::time::sleep(Duration::from_secs(wait)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::Europe::Budapest;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("timestamp")
            .with_timezone(&Utc)
    }

    #[test]
    fn grid_round_trips_and_rejects_bad_rows() {
        let mut grid = Grid::default();
        grid.set(2, 14, Level::Red);
        grid.set(6, 23, Level::Yellow);
        let week = grid.render();
        assert_eq!(week.len(), 7);
        assert_eq!(week[2].chars().nth(14), Some('r'));
        assert_eq!(week[6].chars().nth(23), Some('y'));
        assert_eq!(Grid::parse(&week).expect("parse"), grid);

        assert!(Grid::parse(&week[..6]).is_err(), "six rows");
        let mut short = week.clone();
        short[0].pop();
        assert!(Grid::parse(&short).is_err(), "23 hours");
        let mut bad = week.clone();
        bad[0].replace_range(0..1, "x");
        assert!(Grid::parse(&bad).is_err(), "unknown level");
    }

    #[test]
    fn levels_cycle_and_map_to_letters() {
        assert_eq!(Level::Green.cycle(), Level::Yellow);
        assert_eq!(Level::Yellow.cycle(), Level::Red);
        assert_eq!(Level::Red.cycle(), Level::Green);
        for level in Level::ALL {
            assert_eq!(Level::from_letter(level.letter()), Some(level));
        }
        assert_eq!(Level::from_letter('x'), None);
    }

    #[test]
    fn evaluation_uses_the_local_weekday_and_hour() {
        let mut grid = Grid::default();
        // Wednesday 2026-11-18 12:30 UTC = 13:30 CET (Wednesday).
        grid.set(2, 13, Level::Red);
        let boundary = evaluate(&grid, Budapest, at("2026-11-18T12:30:00Z"));
        assert_eq!(boundary.level, Level::Red);
        assert_eq!(boundary.hour, 13);
        assert_eq!(boundary.next_unix, at("2026-11-18T13:00:00Z").timestamp());
    }

    #[test]
    fn dst_spring_forward_skips_the_missing_hour() {
        // 2026-03-29: 02:00 CET -> 03:00 CEST. At 01:30 local the next
        // absolute hour is 03:00 local, so the 03:00 level must apply.
        let mut grid = Grid::default();
        grid.set(6, 3, Level::Yellow);
        let boundary = evaluate(&grid, Budapest, at("2026-03-29T00:30:00Z"));
        assert_eq!(boundary.hour, 1);
        assert_eq!(boundary.level, Level::Green);
        let next = evaluate(&grid, Budapest, at("2026-03-29T01:00:00Z"));
        assert_eq!(next.hour, 3, "02:00 does not exist");
        assert_eq!(next.level, Level::Yellow);
    }

    #[test]
    fn dst_fall_back_repeats_the_hour_with_the_same_level() {
        // 2026-10-25: 03:00 CEST -> 02:00 CET. 02:30 happens twice.
        let mut grid = Grid::default();
        grid.set(6, 2, Level::Red);
        let first = evaluate(&grid, Budapest, at("2026-10-25T00:30:00Z"));
        assert_eq!((first.hour, first.level), (2, Level::Red));
        let second = evaluate(&grid, Budapest, at("2026-10-25T01:30:00Z"));
        assert_eq!((second.hour, second.level), (2, Level::Red));
        let after = evaluate(&grid, Budapest, at("2026-10-25T02:00:00Z"));
        assert_eq!(after.hour, 3);
        assert_eq!(after.level, Level::Green);
    }

    #[test]
    fn the_next_boundary_is_the_next_absolute_hour() {
        let boundary = evaluate(&Grid::default(), chrono_tz::UTC, at("2026-11-18T12:59:30Z"));
        assert_eq!(boundary.next_unix, at("2026-11-18T13:00:00Z").timestamp());
        let boundary = evaluate(&Grid::default(), chrono_tz::UTC, at("2026-11-18T12:00:00Z"));
        assert_eq!(boundary.next_unix, at("2026-11-18T13:00:00Z").timestamp());
    }

    #[test]
    fn timezone_conversion_uses_the_zone_rules() {
        // 2026-11-18 23:30 UTC is 00:30 CET on Thursday.
        let mut grid = Grid::default();
        grid.set(3, 0, Level::Yellow);
        let boundary = evaluate(&grid, Budapest, at("2026-11-18T23:30:00Z"));
        assert_eq!(boundary.weekday, 3, "Thursday");
        assert_eq!(boundary.hour, 0);
        assert_eq!(boundary.level, Level::Yellow);
    }
}
