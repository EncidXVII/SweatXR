//! State and UI commands shared by the overlay and the desktop window.

use std::cell::RefCell;
use std::error::Error;
use std::rc::Rc;

use fitvr_core::energy::fit_calibration;
use fitvr_core::export::{csv, tcx};
use fitvr_core::profile::Profile;
use fitvr_core::storage::{Session, Store};
use fitvr_core::time::{Day, civil, now_unix};
use fitvr_hr::{BleHr, Device, FakeHr, HrSource, Status};
use fitvr_ui::view::{Live, Nav, Range, apply_profile, refresh};
use fitvr_ui::{AppState, ProfileData};

use crate::setup::Dirs;

pub enum UiCmd {
    Profile(ProfileData),
    FinishFirstRun,
    Range(i32),
    HistoryStep(i32),
    SessionsStep(i32),
    HrScan,
    HrPick(String),
    ExportSession(i64),
    ExportDay,
    ExportCsv,
    DeleteHistory,
    ClosePanel,
}

pub type Queue = Rc<RefCell<Vec<UiCmd>>>;

/// Callbacks only queue commands: they fire inside Slint event dispatch, where the app state
/// may already be borrowed by the caller.
pub fn wire(s: &AppState, q: &Queue) {
    let push = |q: &Queue| {
        let q = q.clone();
        move |c: UiCmd| q.borrow_mut().push(c)
    };
    let p = push(q);
    s.on_profile_changed(move |d| p(UiCmd::Profile(d)));
    let p = push(q);
    s.on_finish_first_run(move || p(UiCmd::FinishFirstRun));
    let p = push(q);
    s.on_history_range_changed(move |i| p(UiCmd::Range(i)));
    let p = push(q);
    s.on_history_step(move |d| p(UiCmd::HistoryStep(d)));
    let p = push(q);
    s.on_sessions_step(move |d| p(UiCmd::SessionsStep(d)));
    let p = push(q);
    s.on_hr_scan(move || p(UiCmd::HrScan));
    let p = push(q);
    s.on_hr_pick(move |id| p(UiCmd::HrPick(id.into())));
    let p = push(q);
    s.on_export_session(move |id| p(UiCmd::ExportSession(i64::from(id))));
    let p = push(q);
    s.on_export_day(move || p(UiCmd::ExportDay));
    let p = push(q);
    s.on_export_csv(move || p(UiCmd::ExportCsv));
    let p = push(q);
    s.on_delete_history(move || p(UiCmd::DeleteHistory));
    let p = push(q);
    s.on_close_panel(move || p(UiCmd::ClosePanel));
}

pub enum Hr {
    /// Connects to the saved strap and keeps reconnecting (overlay).
    Live(BleHr),
    /// Scans and saves a choice but never connects, so it cannot fight the overlay (desktop).
    PairOnly(BleHr),
    Fake(FakeHr),
}

impl Hr {
    pub fn bpm(&self) -> Option<u16> {
        match self {
            Self::Live(b) => b.bpm(),
            Self::PairOnly(_) => None,
            Self::Fake(f) => f.bpm(),
        }
    }

    fn status(&self) -> Status {
        match self {
            Self::Live(b) => b.status(),
            // The desktop never connects, so "Connecting" there would never resolve.
            Self::PairOnly(b) if matches!(b.status(), Status::Scanning | Status::NoAdapter) => b.status(),
            Self::PairOnly(_) => Status::Off,
            Self::Fake(f) => f.status(),
        }
    }

    fn devices(&self) -> Vec<Device> {
        match self {
            Self::Live(b) | Self::PairOnly(b) => b.devices(),
            Self::Fake(_) => vec![Device {
                id: "fake".into(),
                name: "Fake strap".into(),
            }],
        }
    }
}

pub struct App {
    pub store: Store,
    pub profile: Profile,
    pub hr: Hr,
    /// Set by the overlay each tick; always false on the desktop.
    pub tracking: bool,
    /// Replaces the empty-state hint, e.g. when SteamVR is missing.
    pub notice: Option<String>,
    nav: Nav,
    export_status: String,
    data_status: String,
    export_dir: std::path::PathBuf,
    db_path: String,
}

impl App {
    pub fn open(dirs: &Dirs, hr: impl FnOnce(&Profile) -> Hr) -> Result<Self, Box<dyn Error>> {
        let store = Store::open(&dirs.db())?;
        let profile = store.profile()?;
        Ok(Self {
            hr: hr(&profile),
            tracking: false,
            notice: None,
            store,
            profile,
            nav: Nav::default(),
            export_status: String::new(),
            data_status: String::new(),
            export_dir: dirs.export.clone(),
            db_path: dirs.db().display().to_string(),
        })
    }

    /// Picks up edits the other process saved. Returns the previous profile if anything changed.
    pub fn reload_profile(&mut self) -> Option<Profile> {
        let fresh = self
            .store
            .profile()
            .map_err(|e| tracing::warn!("profile: {e}"))
            .ok()?;
        (fresh != self.profile).then(|| std::mem::replace(&mut self.profile, fresh))
    }

    fn save(&mut self, p: Profile) {
        if let Err(e) = self.store.save_profile(&p) {
            tracing::warn!("save profile: {e}");
        }
        self.profile = p;
    }

    /// Applies one UI command. Returns true when the panel should close.
    pub fn handle(&mut self, cmd: UiCmd) -> bool {
        match cmd {
            UiCmd::Profile(d) => self.save(apply_profile(&d, &self.profile)),
            UiCmd::FinishFirstRun => self.save(Profile {
                onboarded: true,
                ..self.profile.clone()
            }),
            UiCmd::Range(i) => {
                self.nav = Nav {
                    range: Range::from_index(i),
                    history_offset: 0,
                    ..self.nav
                }
            }
            UiCmd::HistoryStep(d) => self.nav.history_offset = (self.nav.history_offset + d).min(0),
            UiCmd::SessionsStep(d) => self.nav.sessions_offset = (self.nav.sessions_offset + d).min(0),
            UiCmd::HrScan => match &self.hr {
                Hr::Live(b) | Hr::PairOnly(b) => b.scan(),
                Hr::Fake(_) => {}
            },
            UiCmd::HrPick(id) => {
                let chosen = (self.profile.hr_device.as_deref() != Some(id.as_str())).then_some(id);
                if let Hr::Live(b) = &self.hr {
                    b.use_device(chosen.clone());
                }
                self.save(Profile {
                    hr_device: chosen,
                    ..self.profile.clone()
                });
            }
            UiCmd::ExportSession(id) => self.export_status = self.report(self.export_tcx(Some(id))),
            UiCmd::ExportDay => self.export_status = self.report(self.export_tcx(None)),
            UiCmd::ExportCsv => self.export_status = self.report(self.export_csv()),
            UiCmd::DeleteHistory => {
                self.data_status = match self.store.delete_history() {
                    Ok(()) => "History deleted. Your profile is kept.".into(),
                    Err(e) => format!("Delete failed: {e}"),
                }
            }
            UiCmd::ClosePanel => return true,
        }
        false
    }

    fn report(&self, r: Result<String, Box<dyn Error>>) -> String {
        r.unwrap_or_else(|e| format!("Export failed: {e}"))
    }

    fn sessions_day(&self) -> Day {
        self.store.day(now_unix()) + self.nav.sessions_offset
    }

    /// One session, or every session of the day shown on the Sessions page.
    fn export_tcx(&self, only: Option<i64>) -> Result<String, Box<dyn Error>> {
        let day = self.sessions_day();
        let start = self.store.day_start(day);
        let sessions: Vec<Session> = self
            .store
            .sessions(start, self.store.day_start(day + 1))?
            .into_iter()
            .filter(|s| only.is_none_or(|id| s.id == id))
            .collect();
        if sessions.is_empty() {
            return Ok("No sessions on this day.".into());
        }
        let with_samples = sessions
            .into_iter()
            .map(|s| self.store.samples(s.id).map(|x| (s, x)))
            .collect::<Result<Vec<_>, _>>()?;
        let (y, m, d) = civil(day);
        let name = match only {
            Some(id) => format!("fitvr-{y:04}-{m:02}-{d:02}-session-{id}.tcx"),
            None => format!("fitvr-{y:04}-{m:02}-{d:02}.tcx"),
        };
        self.write(&name, &tcx(&with_samples))
    }

    fn export_csv(&self) -> Result<String, Box<dyn Error>> {
        let (y, m, d) = civil(self.store.day(now_unix()));
        self.write(
            &format!("fitvr-all-{y:04}-{m:02}-{d:02}.csv"),
            &csv(&self.store.samples_since(0)?),
        )
    }

    fn write(&self, name: &str, body: &str) -> Result<String, Box<dyn Error>> {
        std::fs::create_dir_all(&self.export_dir)?;
        let path = self.export_dir.join(name);
        std::fs::write(&path, body)?;
        Ok(format!("Saved {}", path.display()))
    }

    /// The overlay calls this when the panel opens, so the watch stops teaching the gesture.
    pub fn mark_panel_seen(&mut self) {
        if !self.profile.panel_seen {
            self.save(Profile {
                panel_seen: true,
                ..self.profile.clone()
            });
        }
    }

    /// Refit the motion model from recent HR-backed seconds; called when a session ends.
    pub fn recalibrate(&mut self) {
        match self.store.calibration_points(4 * 3600) {
            Ok(points) => {
                if let Some(c) = fit_calibration(&points) {
                    tracing::info!(
                        "calibrated: {:.2} kcal/min per unit + {:.2}",
                        c.slope,
                        c.intercept
                    );
                    self.save(Profile {
                        calibration: Some(c),
                        ..self.profile.clone()
                    });
                }
            }
            Err(e) => tracing::warn!("calibration data: {e}"),
        }
    }

    pub fn show(&self, s: &AppState, now: i64) {
        let devices = self.hr.devices();
        let live = Live {
            tracking: self.tracking,
            hr: self.hr.bpm(),
            hr_status: self.hr.status(),
            devices: &devices,
        };
        if let Err(e) = refresh(s, &self.store, &self.profile, now, &self.nav, &live) {
            tracing::warn!("refresh: {e}");
        }
        s.set_export_status(self.export_status.as_str().into());
        s.set_data_status(self.data_status.as_str().into());
        s.set_data_path(self.db_path.as_str().into());
        if let Some(n) = &self.notice {
            s.set_empty_text(n.as_str().into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fitvr_core::tracker::Tracker;

    fn app(name: &str) -> (App, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("fitvr-app-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("dir");
        let dirs = Dirs {
            data: root.clone(),
            export: root.join("export"),
        };
        (
            App::open(&dirs, |_| Hr::Fake(FakeHr::new(vec![Some(120)]))).expect("open"),
            root,
        )
    }

    fn profile_edit(app: &App, f: impl FnOnce(&mut ProfileData)) -> UiCmd {
        let mut d = fitvr_ui::view::profile_data(&app.profile);
        f(&mut d);
        UiCmd::Profile(d)
    }

    #[test]
    fn edits_are_clamped_saved_and_survive_reopen() {
        let (mut a, root) = app("edits");
        let cmd = profile_edit(&a, |d| {
            d.weight = 5.0;
            d.goal = 40_000;
            d.female = true;
        });
        assert!(!a.handle(cmd));
        a.handle(UiCmd::FinishFirstRun);
        let saved = a.store.profile().expect("profile");
        assert_eq!(
            (saved.weight_kg, saved.goal_kcal, saved.onboarded),
            (30.0, 3000, true)
        );
        assert!(
            a.handle(UiCmd::ClosePanel),
            "close asks the overlay to hide the panel"
        );
        drop(a);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn picking_a_strap_twice_forgets_it() {
        let (mut a, root) = app("pick");
        a.handle(UiCmd::HrPick("strap".into()));
        assert_eq!(a.profile.hr_device.as_deref(), Some("strap"));
        a.handle(UiCmd::HrPick("strap".into()));
        assert_eq!(a.profile.hr_device, None);
        drop(a);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn exports_write_files_and_report() {
        let (mut a, root) = app("export");
        a.handle(UiCmd::ExportDay);
        assert_eq!(a.export_status, "No sessions on this day.");
        let now = now_unix();
        let mut t = Tracker::new();
        // Clamped to today's start so the test also holds just after midnight.
        let start = (now - 300).max(a.store.day_start(a.store.day(now)));
        for ts in start..now {
            let ev = t.tick(ts, 1.0, Some(140), "g", &a.profile);
            a.store.apply(&ev).expect("apply");
        }
        a.handle(UiCmd::ExportDay);
        let id = a.store.sessions(0, i64::MAX).expect("sessions")[0].id;
        a.handle(UiCmd::ExportSession(id));
        a.handle(UiCmd::ExportCsv);
        let files: Vec<_> = std::fs::read_dir(root.join("export"))
            .expect("dir")
            .flatten()
            .collect();
        assert_eq!(files.len(), 3, "{}", a.export_status);
        a.handle(UiCmd::DeleteHistory);
        assert!(a.store.sessions(0, i64::MAX).expect("sessions").is_empty());
        assert!(a.export_status.starts_with("Saved "));
        drop(a);
        let _ = std::fs::remove_dir_all(root);
    }
}
