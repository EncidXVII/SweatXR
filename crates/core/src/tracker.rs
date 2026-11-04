//! Turns 1 Hz ticks into sessions, samples and active minutes.
//!
//! Session rules: motion at MET >= 3 starts one; 5 min below light motion, an app change or a
//! clock gap ends it. Idle seconds are held back and only kept if motion resumes, so the idle
//! tail before a session ends never counts.

use crate::energy::{estimate, met_from_intensity};
use crate::profile::Profile;

pub const ACTIVE_MET: f32 = 3.0;
/// Below this the player is standing in a menu, not playing.
pub const MOTION_MET: f32 = 2.0;
pub const IDLE_END_S: i64 = 300;
pub const ACTIVE_SECS_PER_MIN: u32 = 30;

#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub ts: i64,
    pub app_key: String,
    pub intensity: f32,
    pub met: f32,
    pub hr: Option<u16>,
    /// kcal burnt in this one second.
    pub kcal: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Start { ts: i64, app_key: String },
    Sample(Sample),
    ActiveMinute { ts: i64, app_key: String },
    End { ts: i64 },
}

struct Open {
    app_key: String,
    last_motion: i64,
    pending: Vec<Sample>,
}

#[derive(Default)]
pub struct Tracker {
    open: Option<Open>,
    last_ts: Option<i64>,
    minute: i64,
    active_secs: u32,
    counted: bool,
}

impl Tracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn in_session(&self) -> bool {
        self.open.is_some()
    }

    pub fn tick(
        &mut self,
        ts: i64,
        intensity: f32,
        hr: Option<u16>,
        app_key: &str,
        p: &Profile,
    ) -> Vec<Event> {
        let mut out = Vec::new();
        // Sleep, a debugger or a stalled loop: whatever happened in the gap was not tracked.
        let gap = self.last_ts.is_some_and(|last| ts - last > 10 || ts < last);
        self.last_ts = Some(ts);
        let app_changed = self.open.as_ref().is_some_and(|o| o.app_key != app_key);
        if gap || app_changed {
            self.end(&mut out);
        }

        let motion_met = met_from_intensity(intensity);
        let (kcal_min, met) = estimate(intensity, hr, p);
        if self.open.is_none() {
            if motion_met < ACTIVE_MET {
                return out;
            }
            out.push(Event::Start {
                ts,
                app_key: app_key.to_owned(),
            });
            self.open = Some(Open {
                app_key: app_key.to_owned(),
                last_motion: ts,
                pending: Vec::new(),
            });
        }
        let Some(open) = self.open.as_mut() else {
            return out;
        };

        let sample = Sample {
            ts,
            app_key: app_key.to_owned(),
            intensity,
            met,
            hr,
            kcal: kcal_min / 60.0,
        };
        if motion_met >= MOTION_MET {
            open.last_motion = ts;
            out.extend(open.pending.drain(..).map(Event::Sample));
            out.push(Event::Sample(sample));
        } else {
            open.pending.push(sample);
            if ts - open.last_motion >= IDLE_END_S {
                self.end(&mut out);
                return out;
            }
        }

        let minute = ts.div_euclid(60);
        if minute != self.minute {
            (self.minute, self.active_secs, self.counted) = (minute, 0, false);
        }
        if met >= ACTIVE_MET {
            self.active_secs += 1;
            if self.active_secs >= ACTIVE_SECS_PER_MIN && !self.counted {
                self.counted = true;
                out.push(Event::ActiveMinute {
                    ts,
                    app_key: app_key.to_owned(),
                });
            }
        }
        out
    }

    /// Close the open session, e.g. when SteamVR quits.
    pub fn finish(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        self.end(&mut out);
        out
    }

    fn end(&mut self, out: &mut Vec<Event>) {
        if let Some(open) = self.open.take() {
            out.push(Event::End { ts: open.last_motion });
        }
    }
}
