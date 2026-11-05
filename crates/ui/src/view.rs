//! Store + live readings -> what every view shows. One function so the watch, panel, desktop
//! and snapshot tests can never disagree on a number or a label.

use fitvr_core::energy::MIN_CALIBRATION_SAMPLES;
use fitvr_core::profile::{Hand, Profile, Sex, Units};
use fitvr_core::storage::{GameTotal, Result, Store};
use fitvr_core::time::{Day, civil, weekday};
use fitvr_hr::{Device, Status};
use slint::{Model, ModelRc, SharedString, VecModel};

use crate::{AppState, Bar, GameRow, HrDevice, ProfileData, SessionRow};

const TODAY_GAMES: usize = 5;
const HISTORY_GAMES: usize = 3;
const DAY_SESSIONS: usize = 5;
/// The strap section holds a status row and the calibration line; 2 straps keep it short.
const SHOWN_DEVICES: usize = 2;
const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];
const WEEKDAYS: [&str; 7] = ["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Range {
    Day,
    Week,
    Month,
}

impl Range {
    pub fn from_index(i: i32) -> Self {
        match i {
            0 => Self::Day,
            2 => Self::Month,
            _ => Self::Week,
        }
    }

    fn days(self) -> i32 {
        match self {
            Self::Day => 1,
            Self::Week => 7,
            Self::Month => 30,
        }
    }
}

/// Which period the History and Sessions pages show. Offsets count back from today: 0 is the
/// current period, -1 the one before, never positive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Nav {
    pub range: Range,
    pub history_offset: i32,
    pub sessions_offset: i32,
}

impl Default for Nav {
    fn default() -> Self {
        Self {
            range: Range::Week,
            history_offset: 0,
            sessions_offset: 0,
        }
    }
}

pub struct Live<'a> {
    /// A session is open right now (overlay only; the desktop cannot know).
    pub tracking: bool,
    pub hr: Option<u16>,
    pub hr_status: Status,
    pub devices: &'a [Device],
}

pub fn refresh(s: &AppState, store: &Store, p: &Profile, now: i64, nav: &Nav, live: &Live) -> Result<()> {
    let today = store.day(now);
    today_page(s, store, p, today, live)?;
    history_page(s, store, p, today, nav)?;
    sessions_page(s, store, today, nav.sessions_offset)?;
    settings_page(s, store, p, live)?;
    Ok(())
}

/// kcal so far today over the goal; above 1 once the goal is met.
pub fn progress(store: &Store, p: &Profile, now: i64) -> Result<f32> {
    let today = store.day(now);
    let kcal: f32 = store.games(today, today)?.iter().map(|g| g.kcal).sum();
    Ok(kcal / p.goal_kcal.max(1) as f32)
}

fn today_page(s: &AppState, store: &Store, p: &Profile, today: Day, live: &Live) -> Result<()> {
    let games = store.games(today, today)?;
    // max(0.0): an empty float sum is -0.0, which formats as "-0".
    let kcal = games.iter().map(|g| g.kcal).sum::<f32>().max(0.0);
    let active: u32 = games.iter().map(|g| g.active_min).sum();
    let progress = kcal / p.goal_kcal.max(1) as f32;
    s.set_progress(progress);
    s.set_goal_met(progress >= 1.0);
    s.set_kcal_text(format!("{kcal:.0}").into());
    s.set_goal_text(if progress >= 1.0 {
        "GOAL MET".into()
    } else {
        format!("OF {} KCAL", p.goal_kcal).into()
    });
    s.set_tracking(live.tracking);
    s.set_panel_seen(p.panel_seen);
    s.set_active_text(format!("{active} MIN").into());
    s.set_active_value(active.to_string().into());

    let bpm = live.hr.map_or_else(|| "--".to_owned(), |b| b.to_string());
    let zone = live.hr.and_then(|b| zone(b, p.age));
    s.set_hr_shown(live.hr_status != Status::Off);
    s.set_hr_text(format!("{bpm} BPM").into());
    s.set_hr_value(bpm.into());
    s.set_hr_live(live.hr.is_some());
    s.set_hr_zone(zone.map_or_else(SharedString::new, |z| format!("ZONE {z}").into()));
    s.set_hr_label(zone.map_or_else(|| "HEART RATE".into(), |z| format!("HEART RATE, ZONE {z}").into()));
    set_if_changed(&s.get_games(), rows(&games, TODAY_GAMES), |m| s.set_games(m));
    Ok(())
}

/// Heart rate zone 1-5 as a share of the age-predicted maximum (220 - age). Below 50% there is
/// no zone: the player is resting.
pub fn zone(bpm: u16, age: u32) -> Option<u8> {
    let share = f32::from(bpm) / (220.0 - age as f32).max(100.0);
    [0.5, 0.6, 0.7, 0.8, 0.9]
        .iter()
        .rposition(|&t| share >= t)
        .map(|i| i as u8 + 1)
}

fn history_page(s: &AppState, store: &Store, p: &Profile, today: Day, nav: &Nav) -> Result<()> {
    let end = today + nav.history_offset.min(0) * nav.range.days();
    let from = end - (nav.range.days() - 1);
    let bars = bars(store, from, end, nav.range, p.goal_kcal as f32)?;
    let games = store.games(from, end)?;
    let (kcal, active) = games
        .iter()
        .fold((0.0, 0), |(k, a), g| (k + g.kcal, a + g.active_min));
    set_if_changed(&s.get_bars(), ModelRc::new(VecModel::from(bars)), |m| {
        s.set_bars(m)
    });
    s.set_history_label(period_label(from, end, nav.range, nav.history_offset).into());
    s.set_history_has_next(nav.history_offset < 0);
    s.set_history_total(format!("{kcal:.0} kcal, {active} active min").into());
    let streak = store.streak(today, p.goal_kcal)?;
    s.set_streak_text(if streak > 0 {
        format!("{streak} DAY STREAK").into()
    } else {
        SharedString::new()
    });
    set_if_changed(&s.get_history_games(), rows(&games, HISTORY_GAMES), |m| {
        s.set_history_games(m)
    });
    s.set_history_range(nav.range as i32);
    Ok(())
}

fn date(d: Day) -> String {
    let (_, m, day) = civil(d);
    format!("{} {day}", MONTHS[(m - 1) as usize])
}

fn period_label(from: Day, end: Day, range: Range, offset: i32) -> String {
    match (range, offset) {
        (Range::Day, 0) => "TODAY".into(),
        (Range::Day, -1) => "YESTERDAY".into(),
        (Range::Week, 0) => "THIS WEEK".into(),
        (Range::Month, 0) => "LAST 30 DAYS".into(),
        (Range::Day, _) => format!("{} {}", WEEKDAYS[weekday(end) as usize], date(end)),
        _ => format!("{} - {}", date(from), date(end)),
    }
}

fn sessions_page(s: &AppState, store: &Store, today: Day, offset: i32) -> Result<()> {
    let day = today + offset.min(0);
    let start = store.day_start(day);
    let sessions = store.sessions(start, store.day_start(day + 1))?;
    let clock = |ts: i64| {
        let t = store.local_seconds(ts);
        format!("{:02}:{:02}", t / 3600, t / 60 % 60)
    };
    let rows: Vec<SessionRow> = sessions
        .iter()
        .take(DAY_SESSIONS)
        .map(|x| {
            let minutes = ((x.end - x.start + 30) / 60).max(1);
            let detail = match x.hr_avg {
                Some(hr) => format!("{minutes} min, avg {hr:.0} bpm"),
                None => format!("{minutes} min"),
            };
            SessionRow {
                id: i32::try_from(x.id).unwrap_or(i32::MAX),
                time: format!("{}-{}", clock(x.start), clock(x.end)).into(),
                name: x.app_name.as_str().into(),
                detail: detail.into(),
                kcal: format!("{:.0} kcal", x.kcal).into(),
            }
        })
        .collect();
    set_if_changed(&s.get_sessions(), ModelRc::new(VecModel::from(rows)), |m| {
        s.set_sessions(m)
    });
    let label = match offset {
        0 => "TODAY".into(),
        -1 => "YESTERDAY".into(),
        _ => format!("{} {}", WEEKDAYS[weekday(day) as usize], date(day)),
    };
    s.set_sessions_label(label.into());
    s.set_sessions_has_next(offset < 0);
    let more = sessions.len().saturating_sub(DAY_SESSIONS);
    s.set_sessions_more(if more > 0 {
        format!("{more} more in the day's TCX export").into()
    } else {
        SharedString::new()
    });
    Ok(())
}

fn settings_page(s: &AppState, store: &Store, p: &Profile, live: &Live) -> Result<()> {
    s.set_profile(profile_data(p));
    s.set_first_run(!p.onboarded);
    s.set_hr_status(hr_status(live, p).into());
    s.set_hr_scanning(live.hr_status == Status::Scanning);
    let devices: Vec<HrDevice> = live
        .devices
        .iter()
        .take(SHOWN_DEVICES)
        .map(|d| HrDevice {
            id: d.id.as_str().into(),
            name: d.name.as_str().into(),
            chosen: p.hr_device.as_deref() == Some(&d.id),
        })
        .collect();
    set_if_changed(&s.get_hr_devices(), ModelRc::new(VecModel::from(devices)), |m| {
        s.set_hr_devices(m)
    });
    let calibration = if p.calibration.is_some() {
        "Personal calibration on: without the strap, estimates use your own heart-rate data.".to_owned()
    } else {
        let have = store.hr_seconds()? / 60;
        let need = (MIN_CALIBRATION_SAMPLES / 60) as u32;
        format!(
            "Personal calibration starts after {need} min of play with a strap ({} so far).",
            have.min(need)
        )
    };
    s.set_calibration_text(calibration.into());
    Ok(())
}

/// Replacing a model always repaints; only do it when rows differ, so an idle watch costs no
/// redraws and no texture uploads.
fn set_if_changed<T: Clone + PartialEq + 'static>(
    old: &ModelRc<T>,
    new: ModelRc<T>,
    set: impl FnOnce(ModelRc<T>),
) {
    if old.row_count() != new.row_count() || old.iter().zip(new.iter()).any(|(a, b)| a != b) {
        set(new);
    }
}

fn hr_status(live: &Live, p: &Profile) -> String {
    match (live.hr_status, live.hr) {
        (Status::Off, _) if p.hr_device.is_some() => "Saved, connects in VR".into(),
        (Status::Off, _) => "No strap chosen".into(),
        (Status::NoAdapter, _) => "No Bluetooth".into(),
        (Status::Scanning, _) if live.devices.len() > SHOWN_DEVICES => {
            format!("{SHOWN_DEVICES} of {} shown", live.devices.len())
        }
        (Status::Scanning, _) => format!("{} found", live.devices.len()),
        (Status::Connecting, _) => "Connecting...".into(),
        (Status::Connected, Some(b)) => format!("{b} bpm"),
        (Status::Connected, None) => "No contact".into(),
    }
}

fn rows(games: &[GameTotal], n: usize) -> ModelRc<GameRow> {
    let max = games.iter().map(|g| g.kcal).fold(1.0, f32::max);
    let rows: Vec<GameRow> = games
        .iter()
        .take(n)
        .map(|g| GameRow {
            name: g.name.as_str().into(),
            kcal: format!("{:.0} kcal", g.kcal).into(),
            minutes: format!("{} min", (g.seconds + 30) / 60).into(),
            share: g.kcal / max,
        })
        .collect();
    ModelRc::new(VecModel::from(rows))
}

fn bars(store: &Store, from: Day, end: Day, range: Range, goal: f32) -> Result<Vec<Bar>> {
    const DAYS: [&str; 7] = ["M", "T", "W", "T", "F", "S", "S"];
    let values: Vec<(String, f32, bool)> = match range {
        Range::Day => {
            let hours = store.hours(end)?;
            let labels = (0..24).map(|h| if h % 6 == 0 { h.to_string() } else { String::new() });
            labels.zip(hours).map(|(l, k)| (l, k, false)).collect()
        }
        Range::Week | Range::Month => {
            let label = |d: Day| match range {
                Range::Week => DAYS[weekday(d) as usize].to_owned(),
                _ if (end - d) % 5 == 0 => civil(d).2.to_string(),
                _ => String::new(),
            };
            store
                .days(from, end)?
                .iter()
                .map(|d| (label(d.day), d.kcal, d.kcal >= goal))
                .collect()
        }
    };
    let peak = values.iter().map(|v| v.1).fold(0.0, f32::max);
    let max = peak.max(if range == Range::Day { 1.0 } else { goal });
    Ok(values
        .into_iter()
        .map(|(label, kcal, met)| {
            // Week shows every value; denser charts only label their peak. Empty days stay blank.
            let labelled = kcal >= 0.5 && (range == Range::Week || kcal == peak);
            Bar {
                label: label.into(),
                text: if labelled {
                    format!("{kcal:.0}").into()
                } else {
                    SharedString::new()
                },
                value: kcal / max,
                met,
            }
        })
        .collect())
}

pub fn profile_data(p: &Profile) -> ProfileData {
    ProfileData {
        weight: p.weight_kg,
        height: p.height_cm,
        age: p.age as i32,
        female: p.sex == Sex::Female,
        goal: p.goal_kcal as i32,
        resting_hr: p.resting_hr as i32,
        right_hand: p.watch_hand == Hand::Right,
        imperial: p.units == Units::Imperial,
        auto_start: p.auto_start,
    }
}

/// UI edits applied over the stored profile, clamped to valid ranges.
pub fn apply_profile(d: &ProfileData, p: &Profile) -> Profile {
    Profile {
        weight_kg: d.weight,
        height_cm: d.height,
        age: d.age.max(0) as u32,
        sex: if d.female { Sex::Female } else { Sex::Male },
        goal_kcal: d.goal.max(0) as u32,
        resting_hr: d.resting_hr.max(0) as u32,
        watch_hand: if d.right_hand { Hand::Right } else { Hand::Left },
        units: if d.imperial {
            Units::Imperial
        } else {
            Units::Metric
        },
        auto_start: d.auto_start,
        ..p.clone()
    }
    .clamped()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zones_follow_age_predicted_max() {
        // Max 190 at age 30.
        assert_eq!(zone(90, 30), None);
        assert_eq!(zone(100, 30), Some(1));
        assert_eq!(zone(135, 30), Some(3));
        assert_eq!(zone(175, 30), Some(5));
    }

    #[test]
    fn period_labels() {
        let d = fitvr_core::time::days_from_civil(2026, 11, 4);
        assert_eq!(period_label(d - 6, d, Range::Week, 0), "THIS WEEK");
        assert_eq!(period_label(d - 6, d, Range::Week, -1), "OCT 29 - NOV 4");
        assert_eq!(period_label(d, d, Range::Day, -2), "WED NOV 4");
    }
}
