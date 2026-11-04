//! Store + live readings -> what every view shows. One function so the watch, panel, desktop
//! and snapshot tests can never disagree on a number or a label.

use fitvr_core::profile::{Hand, Profile, Sex, Units};
use fitvr_core::storage::{GameTotal, Result, Store};
use fitvr_core::time::{Day, civil, weekday};
use fitvr_hr::{Device, Status};
use slint::{Model, ModelRc, SharedString, VecModel};

use crate::{AppState, Bar, GameRow, HrDevice, ProfileData};

const TODAY_GAMES: usize = 5;
const HISTORY_GAMES: usize = 3;
/// Device settings already hold 4 rows; 2 straps keep the view at the 6-item limit.
const SHOWN_DEVICES: usize = 2;

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
}

pub struct Live<'a> {
    /// A session is open right now (overlay only; the desktop cannot know).
    pub tracking: bool,
    pub hr: Option<u16>,
    pub hr_status: Status,
    pub devices: &'a [Device],
}

pub fn refresh(s: &AppState, store: &Store, p: &Profile, now: i64, range: Range, live: &Live) -> Result<()> {
    let today = store.day(now);
    let games = store.games(today, today)?;
    // max(0.0): an empty float sum is -0.0, which formats as "-0".
    let kcal = games.iter().map(|g| g.kcal).sum::<f32>().max(0.0);
    let active: u32 = games.iter().map(|g| g.active_min).sum();
    let progress = kcal / p.goal_kcal.max(1) as f32;
    s.set_progress(progress);
    // Past the goal a percentage stops answering "am I done?".
    s.set_percent_text(if progress >= 1.0 {
        "DONE".into()
    } else {
        format!("{:.0}%", progress * 100.0).into()
    });
    s.set_tracking(live.tracking);
    s.set_hr_shown(live.hr_status != Status::Off);
    s.set_kcal_text(format!("{kcal:.0}").into());
    s.set_goal_text(format!("OF {} KCAL", p.goal_kcal).into());
    let bpm = live.hr.map_or_else(|| "--".to_owned(), |b| b.to_string());
    s.set_active_text(format!("{active} MIN").into());
    s.set_active_value(active.to_string().into());
    s.set_hr_text(format!("{bpm} BPM").into());
    s.set_hr_value(bpm.into());
    s.set_hr_live(live.hr.is_some());
    let games_rows = rows(&games, TODAY_GAMES);
    if changed(&s.get_games(), &games_rows) {
        s.set_games(games_rows);
    }

    let (from, bars) = bars(store, today, range, p.goal_kcal as f32)?;
    let range_games = if from == today {
        games
    } else {
        store.games(from, today)?
    };
    let (rk, ra) = range_games
        .iter()
        .fold((0.0, 0), |(k, a), g| (k + g.kcal, a + g.active_min));
    let bars = ModelRc::new(VecModel::from(bars));
    if changed(&s.get_bars(), &bars) {
        s.set_bars(bars);
    }
    s.set_history_total(format!("{rk:.0} kcal, {ra} active min").into());
    let streak = store.streak(today, p.goal_kcal)?;
    s.set_streak_text(if streak > 0 {
        format!("{streak} DAY STREAK").into()
    } else {
        SharedString::new()
    });
    let history_rows = rows(&range_games, HISTORY_GAMES);
    if changed(&s.get_history_games(), &history_rows) {
        s.set_history_games(history_rows);
    }
    s.set_history_range(range as i32);

    s.set_profile(profile_data(p));
    s.set_first_run(!p.onboarded);
    s.set_hr_status(hr_status(live).into());
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
    let devices = ModelRc::new(VecModel::from(devices));
    if changed(&s.get_hr_devices(), &devices) {
        s.set_hr_devices(devices);
    }
    Ok(())
}

/// Replacing a model always repaints; only do it when rows differ, so an idle watch costs no
/// redraws and no texture uploads.
fn changed<T: Clone + PartialEq + 'static>(old: &ModelRc<T>, new: &ModelRc<T>) -> bool {
    old.row_count() != new.row_count() || old.iter().zip(new.iter()).any(|(a, b)| a != b)
}

fn hr_status(live: &Live) -> String {
    match (live.hr_status, live.hr) {
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

/// First day of the range and its bars.
fn bars(store: &Store, today: Day, range: Range, goal: f32) -> Result<(Day, Vec<Bar>)> {
    const DAYS: [&str; 7] = ["M", "T", "W", "T", "F", "S", "S"];
    let (from, values): (Day, Vec<(String, f32, bool)>) = match range {
        Range::Day => {
            let hours = store.hours(today)?;
            let labels = (0..24).map(|h| if h % 6 == 0 { h.to_string() } else { String::new() });
            (today, labels.zip(hours).map(|(l, k)| (l, k, false)).collect())
        }
        Range::Week | Range::Month => {
            let n = if range == Range::Week { 7 } else { 30 };
            let days = store.days(today - (n - 1), today)?;
            let label = |d: Day| match range {
                Range::Week => DAYS[weekday(d) as usize].to_owned(),
                _ if (today - d) % 5 == 0 => civil(d).2.to_string(),
                _ => String::new(),
            };
            (
                today - (n - 1),
                days.iter()
                    .map(|d| (label(d.day), d.kcal, d.kcal >= goal))
                    .collect(),
            )
        }
    };
    let max = values
        .iter()
        .map(|v| v.1)
        .fold(if range == Range::Day { 1.0 } else { goal }, f32::max);
    let show_text = range == Range::Week;
    let bars = values
        .into_iter()
        .map(|(label, kcal, met)| Bar {
            label: label.into(),
            text: if show_text {
                format!("{kcal:.0}").into()
            } else {
                SharedString::new()
            },
            value: kcal / max,
            met,
        })
        .collect();
    Ok((from, bars))
}

pub fn profile_data(p: &Profile) -> ProfileData {
    ProfileData {
        weight: p.weight_kg,
        height: p.height_cm,
        age: p.age as i32,
        female: p.sex == Sex::Female,
        goal: p.goal_kcal as i32,
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
