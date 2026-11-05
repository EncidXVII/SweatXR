//! Desktop window: history, export, settings and strap pairing. Reads the database the overlay
//! writes, so it can stay open during a session and follow along.

use std::cell::RefCell;
use std::error::Error;
use std::rc::Rc;
use std::time::Duration;

use fitvr_core::time::now_unix;
use fitvr_hr::BleHr;
use fitvr_ui::{AppState, DesktopWindow, Mode};
use slint::{ComponentHandle, Timer, TimerMode};

use crate::app::{App, Hr, Queue, wire};
use crate::setup::Dirs;

const TICK: Duration = Duration::from_millis(100);
/// Ticks between database refreshes.
const REFRESH_EVERY: u32 = 20;

pub fn run(dirs: &Dirs) -> Result<(), Box<dyn Error>> {
    let app = Rc::new(RefCell::new(App::open(dirs, |_| {
        Hr::PairOnly(BleHr::start(None))
    })?));
    if fitvr_vr::runtime_dir().is_none() {
        app.borrow_mut().notice =
            Some("SteamVR is not installed. Install it from Steam to get the watch in VR.".into());
    }
    let ui = DesktopWindow::new()?;
    // Desk distance: denser type and controls than the VR panel, from the same tokens.
    ui.global::<Mode>().set_desktop(true);
    let queue = Queue::default();
    wire(&ui.global::<AppState>(), &queue);
    app.borrow().show(&ui.global::<AppState>(), now_unix());

    // Registers the overlay with SteamVR if it is running; harmless if not.
    if let Ok(exe) = std::env::current_exe()
        && let Err(e) = std::process::Command::new(exe).arg("--register").spawn()
    {
        tracing::warn!("register: {e}");
    }

    #[cfg(windows)]
    let tray = crate::tray::Tray::new()
        .map_err(|e| tracing::warn!("tray: {e}"))
        .ok();
    #[cfg(windows)]
    if tray.is_some() {
        // With a tray icon, closing the window keeps tracking history reachable from the tray.
        ui.window()
            .on_close_requested(|| slint::CloseRequestResponse::HideWindow);
    }

    let timer = Timer::default();
    let weak = ui.as_weak();
    let mut ticks = 0u32;
    timer.start(TimerMode::Repeated, TICK, move || {
        let Some(ui) = weak.upgrade() else { return };
        let cmds: Vec<_> = queue.borrow_mut().drain(..).collect();
        let changed = !cmds.is_empty();
        let mut app = app.borrow_mut();
        for c in cmds {
            app.handle(c);
        }
        ticks += 1;
        if changed || ticks.is_multiple_of(REFRESH_EVERY) {
            app.reload_profile();
            app.show(&ui.global::<AppState>(), now_unix());
        }
        #[cfg(windows)]
        if let Some(t) = &tray {
            t.poll(&ui);
        }
    });

    ui.show()?;
    if cfg!(windows) {
        slint::run_event_loop_until_quit()?;
    } else {
        slint::run_event_loop()?;
    }
    Ok(())
}
