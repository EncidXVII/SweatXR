//! One SQLite file. Every tick is its own transaction, so killing the process loses at most the
//! current second. WAL lets the desktop window read while the overlay writes.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use crate::profile::{Calibration, Hand, Profile, Sex, Units};
use crate::time::{Day, day_of, local_offset};
use crate::tracker::{Event, Sample};

pub use rusqlite::Result;

/// Each entry moves the schema one version up. Never edit a shipped entry; append a new one.
const MIGRATIONS: &[&str] = &[
    "CREATE TABLE profile (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        weight_kg REAL NOT NULL, height_cm REAL NOT NULL, age INTEGER NOT NULL, sex INTEGER NOT NULL,
        goal_kcal INTEGER NOT NULL, resting_hr INTEGER NOT NULL, hand INTEGER NOT NULL,
        units INTEGER NOT NULL, auto_start INTEGER NOT NULL, hr_device TEXT,
        cal_slope REAL, cal_intercept REAL, onboarded INTEGER NOT NULL);
     CREATE TABLE apps (key TEXT PRIMARY KEY, name TEXT NOT NULL);
     CREATE TABLE sessions (
        id INTEGER PRIMARY KEY, app_key TEXT NOT NULL, start INTEGER NOT NULL, end INTEGER NOT NULL,
        kcal REAL NOT NULL DEFAULT 0, active_min INTEGER NOT NULL DEFAULT 0,
        hr_sum INTEGER NOT NULL DEFAULT 0, hr_count INTEGER NOT NULL DEFAULT 0, hr_max INTEGER);
     CREATE TABLE samples (
        ts INTEGER PRIMARY KEY, session_id INTEGER, app_key TEXT NOT NULL,
        intensity REAL NOT NULL, met REAL NOT NULL, hr INTEGER, kcal REAL NOT NULL);
     CREATE TABLE daily (
        day INTEGER NOT NULL, app_key TEXT NOT NULL, kcal REAL NOT NULL DEFAULT 0,
        active_min INTEGER NOT NULL DEFAULT 0, seconds INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (day, app_key));",
    "CREATE INDEX samples_session ON samples (session_id);
     CREATE INDEX sessions_start ON sessions (start);",
];

#[derive(Clone, Debug, PartialEq)]
pub struct GameTotal {
    pub app_key: String,
    pub name: String,
    pub kcal: f32,
    pub active_min: u32,
    pub seconds: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DayTotal {
    pub day: Day,
    pub kcal: f32,
    pub active_min: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub id: i64,
    pub app_key: String,
    pub app_name: String,
    pub start: i64,
    pub end: i64,
    pub kcal: f32,
    pub active_min: u32,
    pub hr_avg: Option<f32>,
    pub hr_max: Option<u16>,
}

pub struct Store {
    conn: Connection,
    open_session: Option<i64>,
    tz: fn(i64) -> i32,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(2))?;
        Self::with(conn, local_offset)
    }

    /// For tests: in-memory and with a fixed time zone.
    pub fn in_memory(tz: fn(i64) -> i32) -> Result<Self> {
        Self::with(Connection::open_in_memory()?, tz)
    }

    fn with(mut conn: Connection, tz: fn(i64) -> i32) -> Result<Self> {
        migrate(&mut conn)?;
        Ok(Self {
            conn,
            open_session: None,
            tz,
        })
    }

    pub fn day(&self, ts: i64) -> Day {
        day_of(ts, (self.tz)(ts))
    }

    /// First second of a local day. Uses the offset at noon so night-time DST changes are handled.
    pub fn day_start(&self, day: Day) -> i64 {
        let midnight_utc = i64::from(day) * 86_400;
        midnight_utc - i64::from((self.tz)(midnight_utc + 43_200))
    }

    pub fn profile(&self) -> Result<Profile> {
        let row = self
            .conn
            .query_row("SELECT weight_kg, height_cm, age, sex, goal_kcal, resting_hr, hand, units, auto_start, hr_device, cal_slope, cal_intercept, onboarded FROM profile WHERE id = 1", [], |r| {
                let cal = match (r.get::<_, Option<f32>>(10)?, r.get::<_, Option<f32>>(11)?) {
                    (Some(slope), Some(intercept)) => Some(Calibration { slope, intercept }),
                    _ => None,
                };
                Ok(Profile {
                    weight_kg: r.get(0)?,
                    height_cm: r.get(1)?,
                    age: r.get(2)?,
                    sex: if r.get::<_, i32>(3)? == 1 { Sex::Female } else { Sex::Male },
                    goal_kcal: r.get(4)?,
                    resting_hr: r.get(5)?,
                    watch_hand: if r.get::<_, i32>(6)? == 1 { Hand::Right } else { Hand::Left },
                    units: if r.get::<_, i32>(7)? == 1 { Units::Imperial } else { Units::Metric },
                    auto_start: r.get(8)?,
                    hr_device: r.get(9)?,
                    calibration: cal,
                    onboarded: r.get(12)?,
                })
            })
            .optional()?;
        Ok(row.unwrap_or_default())
    }

    pub fn save_profile(&self, p: &Profile) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO profile VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                p.weight_kg,
                p.height_cm,
                p.age,
                i32::from(p.sex == Sex::Female),
                p.goal_kcal,
                p.resting_hr,
                i32::from(p.watch_hand == Hand::Right),
                i32::from(p.units == Units::Imperial),
                p.auto_start,
                p.hr_device,
                p.calibration.map(|c| c.slope),
                p.calibration.map(|c| c.intercept),
                p.onboarded,
            ],
        )?;
        Ok(())
    }

    pub fn set_app_name(&self, key: &str, name: &str) -> Result<()> {
        self.conn
            .execute("INSERT OR REPLACE INTO apps VALUES (?1, ?2)", params![key, name])?;
        Ok(())
    }

    pub fn apply(&mut self, events: &[Event]) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        let tx = self.conn.transaction()?;
        for ev in events {
            match ev {
                Event::Start { ts, app_key } => {
                    tx.execute(
                        "INSERT INTO sessions (app_key, start, end) VALUES (?1, ?2, ?2)",
                        params![app_key, ts],
                    )?;
                    self.open_session = Some(tx.last_insert_rowid());
                }
                Event::Sample(s) => {
                    tx.execute(
                        "INSERT OR REPLACE INTO samples VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![
                            s.ts,
                            self.open_session,
                            s.app_key,
                            s.intensity,
                            s.met,
                            s.hr,
                            s.kcal
                        ],
                    )?;
                    if let Some(id) = self.open_session {
                        tx.execute(
                            "UPDATE sessions SET end = ?2, kcal = kcal + ?3,
                                hr_sum = hr_sum + COALESCE(?4, 0), hr_count = hr_count + (?4 IS NOT NULL),
                                hr_max = MAX(COALESCE(hr_max, 0), COALESCE(?4, 0)) WHERE id = ?1",
                            params![id, s.ts, s.kcal, s.hr],
                        )?;
                    }
                    tx.execute(
                        "INSERT INTO daily (day, app_key, kcal, seconds) VALUES (?1, ?2, ?3, 1)
                         ON CONFLICT (day, app_key) DO UPDATE SET kcal = kcal + excluded.kcal, seconds = seconds + 1",
                        params![day_of(s.ts, (self.tz)(s.ts)), s.app_key, s.kcal],
                    )?;
                }
                Event::ActiveMinute { ts, app_key } => {
                    tx.execute(
                        "INSERT INTO daily (day, app_key, active_min) VALUES (?1, ?2, 1)
                         ON CONFLICT (day, app_key) DO UPDATE SET active_min = active_min + 1",
                        params![day_of(*ts, (self.tz)(*ts)), app_key],
                    )?;
                    if let Some(id) = self.open_session {
                        tx.execute(
                            "UPDATE sessions SET active_min = active_min + 1 WHERE id = ?1",
                            [id],
                        )?;
                    }
                }
                Event::End { .. } => self.open_session = None,
            }
        }
        tx.commit()
    }

    /// Per-game totals for days `from..=to`, biggest first.
    pub fn games(&self, from: Day, to: Day) -> Result<Vec<GameTotal>> {
        let mut st = self.conn.prepare_cached(
            "SELECT d.app_key, COALESCE(a.name, d.app_key), SUM(d.kcal), SUM(d.active_min), SUM(d.seconds)
             FROM daily d LEFT JOIN apps a ON a.key = d.app_key
             WHERE d.day BETWEEN ?1 AND ?2 GROUP BY d.app_key ORDER BY SUM(d.kcal) DESC",
        )?;
        st.query_map([from, to], |r| {
            Ok(GameTotal {
                app_key: r.get(0)?,
                name: r.get(1)?,
                kcal: r.get(2)?,
                active_min: r.get(3)?,
                seconds: r.get(4)?,
            })
        })?
        .collect()
    }

    /// One entry per day in `from..=to`, zero-filled.
    pub fn days(&self, from: Day, to: Day) -> Result<Vec<DayTotal>> {
        let mut st = self.conn.prepare_cached(
            "SELECT day, SUM(kcal), SUM(active_min) FROM daily WHERE day BETWEEN ?1 AND ?2 GROUP BY day",
        )?;
        let rows: Vec<DayTotal> = st
            .query_map([from, to], |r| {
                Ok(DayTotal {
                    day: r.get(0)?,
                    kcal: r.get(1)?,
                    active_min: r.get(2)?,
                })
            })?
            .collect::<Result<_>>()?;
        Ok((from..=to)
            .map(|day| {
                rows.iter().find(|t| t.day == day).copied().unwrap_or(DayTotal {
                    day,
                    kcal: 0.0,
                    active_min: 0,
                })
            })
            .collect())
    }

    /// kcal per local hour of `day`.
    pub fn hours(&self, day: Day) -> Result<[f32; 24]> {
        let start = self.day_start(day);
        let mut st = self
            .conn
            .prepare_cached("SELECT ts, kcal FROM samples WHERE ts >= ?1 AND ts < ?2")?;
        let mut out = [0.0; 24];
        for row in st.query_map([start, start + 86_400 + 3_600], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, f32>(1)?))
        })? {
            let (ts, kcal) = row?;
            if self.day(ts) == day {
                let local = ts + i64::from((self.tz)(ts));
                out[(local.rem_euclid(86_400) / 3_600) as usize] += kcal;
            }
        }
        Ok(out)
    }

    /// Consecutive days meeting the goal, ending today (or yesterday if today is not done yet).
    pub fn streak(&self, today: Day, goal_kcal: u32) -> Result<u32> {
        let days = self.days(today - 400, today)?;
        let met = |d: &DayTotal| d.kcal >= goal_kcal as f32;
        let mut it = days.iter().rev().peekable();
        if it.peek().is_some_and(|d| !met(d)) {
            it.next();
        }
        Ok(it.take_while(|d| met(d)).count() as u32)
    }

    pub fn sessions(&self, from_ts: i64, to_ts: i64) -> Result<Vec<Session>> {
        let mut st = self.conn.prepare_cached(
            "SELECT s.id, s.app_key, COALESCE(a.name, s.app_key), s.start, s.end, s.kcal, s.active_min,
                    s.hr_sum, s.hr_count, s.hr_max
             FROM sessions s LEFT JOIN apps a ON a.key = s.app_key
             WHERE s.start >= ?1 AND s.start < ?2 ORDER BY s.start",
        )?;
        st.query_map([from_ts, to_ts], |r| {
            let (sum, count): (i64, i64) = (r.get(7)?, r.get(8)?);
            Ok(Session {
                id: r.get(0)?,
                app_key: r.get(1)?,
                app_name: r.get(2)?,
                start: r.get(3)?,
                end: r.get(4)?,
                kcal: r.get(5)?,
                active_min: r.get(6)?,
                hr_avg: (count > 0).then(|| sum as f32 / count as f32),
                hr_max: r.get::<_, Option<u16>>(9)?.filter(|&m| m > 0),
            })
        })?
        .collect()
    }

    pub fn samples(&self, session_id: i64) -> Result<Vec<Sample>> {
        self.sample_query(
            "SELECT ts, app_key, intensity, met, hr, kcal FROM samples WHERE session_id = ?1 ORDER BY ts",
            session_id,
        )
    }

    pub fn samples_since(&self, from_ts: i64) -> Result<Vec<Sample>> {
        self.sample_query(
            "SELECT ts, app_key, intensity, met, hr, kcal FROM samples WHERE ts >= ?1 ORDER BY ts",
            from_ts,
        )
    }

    fn sample_query(&self, sql: &str, arg: i64) -> Result<Vec<Sample>> {
        let mut st = self.conn.prepare_cached(sql)?;
        st.query_map([arg], |r| {
            Ok(Sample {
                ts: r.get(0)?,
                app_key: r.get(1)?,
                intensity: r.get(2)?,
                met: r.get(3)?,
                hr: r.get(4)?,
                kcal: r.get(5)?,
            })
        })?
        .collect()
    }

    /// (intensity, kcal/min) pairs from seconds with a heart rate, newest first.
    pub fn calibration_points(&self, limit: u32) -> Result<Vec<(f32, f32)>> {
        let mut st = self.conn.prepare_cached(
            "SELECT intensity, kcal * 60 FROM samples WHERE hr IS NOT NULL ORDER BY ts DESC LIMIT ?1",
        )?;
        st.query_map([limit], |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
    }
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (i, sql) in (0i64..).zip(MIGRATIONS).skip(version.max(0) as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i + 1)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::Tracker;

    fn utc(_: i64) -> i32 {
        0
    }

    fn run(store: &mut Store, tracker: &mut Tracker, from: i64, secs: i64, intensity: f32, app: &str) {
        let p = Profile::default();
        for ts in from..from + secs {
            let ev = tracker.tick(ts, intensity, None, app, &p);
            store.apply(&ev).expect("apply");
        }
    }

    #[test]
    fn totals_survive_a_kill_mid_session() {
        let dir = std::env::temp_dir().join(format!("fitvr-kill-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("t.db");
        let _ = std::fs::remove_file(&path);
        let t0 = 1_800_000_000;
        let before = {
            let mut store = Store::open(&path).expect("open");
            let mut tracker = Tracker::new();
            run(&mut store, &mut tracker, t0, 600, 0.8, "game.a");
            let day = store.day(t0);
            store.games(day, day).expect("games")
            // Dropped without Tracker::finish: what a crash looks like.
        };
        let store = Store::open(&path).expect("reopen");
        let day = store.day(t0);
        assert_eq!(store.games(day, day).expect("games"), before);
        assert!(before[0].kcal > 30.0 && before[0].active_min >= 9, "{before:?}");
        assert_eq!(store.sessions(0, i64::MAX).expect("sessions").len(), 1);
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrates_v1_database_keeping_data() {
        let conn = Connection::open_in_memory().expect("db");
        conn.execute_batch(MIGRATIONS[0]).expect("v1");
        conn.pragma_update(None, "user_version", 1).expect("version");
        conn.execute("INSERT INTO daily VALUES (100, 'g', 42.0, 3, 600)", [])
            .expect("row");
        let store = Store::with(conn, utc).expect("migrate");
        let v: i64 = store
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .expect("v");
        assert_eq!(v, MIGRATIONS.len() as i64);
        assert_eq!(store.days(100, 100).expect("days")[0].kcal, 42.0);
    }

    #[test]
    fn profile_round_trips() {
        let store = Store::in_memory(utc).expect("db");
        assert_eq!(store.profile().expect("default"), Profile::default());
        let p = Profile {
            sex: Sex::Female,
            hr_device: Some("AA:BB".into()),
            calibration: Some(Calibration {
                slope: 8.0,
                intercept: 1.0,
            }),
            onboarded: true,
            ..Profile::default()
        };
        store.save_profile(&p).expect("save");
        assert_eq!(store.profile().expect("load"), p);
    }

    #[test]
    fn streak_counts_back_from_today() {
        let mut store = Store::in_memory(utc).expect("db");
        let mut tracker = Tracker::new();
        // 20 min of hard work on days 10, 11 and 12; day 13 (today) not done yet.
        for day in 10..=12 {
            run(&mut store, &mut tracker, day * 86_400 + 36_000, 1_200, 1.5, "g");
        }
        assert_eq!(store.streak(13, 150).expect("streak"), 3);
        assert_eq!(store.streak(14, 150).expect("streak"), 0);
    }

    #[test]
    fn day_boundary_uses_local_time() {
        fn plus_two(_: i64) -> i32 {
            7_200
        }
        let mut store = Store::in_memory(plus_two).expect("db");
        let mut tracker = Tracker::new();
        // 22:50 to 23:10 UTC = 00:50 to 01:10 local: all on the next local day.
        run(&mut store, &mut tracker, 100 * 86_400 + 82_200, 1_200, 1.0, "g");
        assert_eq!(store.days(100, 100).expect("d")[0].kcal, 0.0);
        assert!(store.days(101, 101).expect("d")[0].kcal > 0.0);
        let hours = store.hours(101).expect("hours");
        assert!(hours[0] > 0.0 && hours[1] > 0.0 && hours[2] == 0.0);
    }
}
