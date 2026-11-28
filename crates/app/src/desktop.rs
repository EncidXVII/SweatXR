// Copyright (C) 2026 EncidXVII
// SPDX-License-Identifier: GPL-3.0-only

//! Desktop window: history, export, settings and strap pairing. Reads the database the overlay
//! writes, so it can stay open during a session and follow along.

use std::cell::RefCell;
use std::error::Error;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, Timer, TimerMode};
use sweatxr_core::time::now_unix;
use sweatxr_hr::BleHr;
use sweatxr_ui::{AppState, DesktopWindow, Mode};

use crate::app::{App, Hr, Queue, wire};
use crate::setup::Dirs;

const TICK: Duration = Duration::from_millis(100);
/// Ticks between database refreshes.
const REFRESH_EVERY: u32 = 20;
#[cfg(not(target_os = "linux"))]
const NO_STEAMVR: &str = "SteamVR is not installed. Install it from Steam to get the watch in VR.";
/// Monado and WiVRn do not start the overlay, so the way to start it is part of the notice.
#[cfg(target_os = "linux")]
const NO_STEAMVR: &str = "SteamVR is not installed. Install it from Steam to get the watch in VR, or with \
                          Monado or WiVRn running, start it with sweatxr --overlay (experimental).";

pub fn run(dirs: &Dirs) -> Result<(), Box<dyn Error>> {
    // Wayland docks match the window to sweatxr.desktop by this, for its name and icon.
    if let Err(e) = slint::set_xdg_app_id("sweatxr") {
        tracing::warn!("app id: {e}");
    }
    let app = Rc::new(RefCell::new(App::open(dirs, |_| {
        Hr::PairOnly(BleHr::start(None))
    })?));
    if sweatxr_vr::runtime_dir().is_none() {
        app.borrow_mut().notice = Some(NO_STEAMVR.into());
    }
    let ui = DesktopWindow::new()?;
    // Desk distance: denser type and controls than the VR panel, from the same tokens.
    ui.global::<Mode>().set_desktop(true);
    ui.global::<Mode>()
        .set_reduce_motion(!crate::motion::animations_enabled());
    let queue = Queue::default();
    wire(&ui.global::<AppState>(), &queue);
    app.borrow().show(&ui.global::<AppState>(), now_unix());

    // Opening SweatXR while SteamVR runs starts the watch, also with "Start with SteamVR" off.
    register(true);
    let mut applied_auto_start = app.borrow().profile.auto_start;

    // Closing the window quits: tracking runs in the VR overlay, not here, so nothing is lost, and
    // no SweatXR is left running unseen (the event loop would otherwise outlive the window).
    {
        ui.window().on_close_requested(|| {
            let _ = slint::quit_event_loop();
            slint::CloseRequestResponse::HideWindow
        });
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
        // "Start with SteamVR" changed here or in VR: tell SteamVR now (and start the watch if
        // SteamVR is running), not the next time the overlay happens to run.
        if app.profile.auto_start != applied_auto_start {
            applied_auto_start = app.profile.auto_start;
            register(false);
        }
    });

    ui.show()?;
    fit_to_screen(&ui);
    if cfg!(windows) {
        slint::run_event_loop_until_quit()?;
    } else {
        slint::run_event_loop()?;
    }
    Ok(())
}

/// Registers the overlay with SteamVR and applies "Start with SteamVR" (`sweatxr --register`, in its
/// own process: SteamVR's API lets one connection per process). With `start`, a running SteamVR gets the
/// watch now whatever that setting says. Harmless when SteamVR is off.
fn register(start: bool) {
    let Ok(exe) = std::env::current_exe() else { return };
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--register");
    if start {
        cmd.arg("--start");
    }
    match cmd.spawn() {
        // Waited for off the UI thread, so the finished child is not left behind as a zombie on Linux.
        Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
        Err(e) => tracing::warn!("register: {e}"),
    }
}

/// The preferred size suits 1440p and up; on a 1080p laptop at 150% it would run past the
/// taskbar. Shrinks and centres the window when it does not fit (runs once the window exists).
fn fit_to_screen(ui: &DesktopWindow) {
    let weak = ui.as_weak();
    Timer::single_shot(Duration::from_millis(50), move || {
        if let Some(ui) = weak.upgrade() {
            fit_now(&ui);
        }
    });
}

fn fit_now(ui: &DesktopWindow) {
    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::dpi::{PhysicalPosition, PhysicalSize};
    ui.window().with_winit_window(|w| {
        let Some(monitor) = w.current_monitor() else {
            return;
        };
        let (screen, origin) = (monitor.size(), monitor.position());
        let (outer, inner) = (w.outer_size(), w.inner_size());
        // Leave room for the taskbar and a margin.
        let max = PhysicalSize::new(screen.width * 92 / 100, screen.height * 88 / 100);
        if outer.width <= max.width && outer.height <= max.height {
            return;
        }
        let fit = PhysicalSize::new(outer.width.min(max.width), outer.height.min(max.height));
        let frame = PhysicalSize::new(
            outer.width.saturating_sub(inner.width),
            outer.height.saturating_sub(inner.height),
        );
        let _ = w.request_inner_size(PhysicalSize::new(
            fit.width - frame.width,
            fit.height - frame.height,
        ));
        w.set_outer_position(PhysicalPosition::new(
            origin.x + (screen.width - fit.width) as i32 / 2,
            origin.y + (screen.height - fit.height) as i32 / 2,
        ));
    });
}
