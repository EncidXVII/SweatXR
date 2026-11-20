// Copyright (C) 2026 EncidXVII
// SPDX-License-Identifier: GPL-3.0-only

//! QA: drive the real views with pointer events, the way the VR pointer and the mouse do, and
//! check every control reaches its callback. Also renders edge cases (imperial units, huge
//! numbers, long names, scanning) into snapshots so layout regressions show up in review.

use std::cell::RefCell;
use std::rc::Rc;

use slint::ComponentHandle;
use sweatxr_core::profile::{Profile, Units};
use sweatxr_core::storage::Store;
use sweatxr_core::tracker::Tracker;
use sweatxr_hr::{Device, Status};
use sweatxr_ui::offscreen::{Pointer, Surface, install};
use sweatxr_ui::view::{Live, Nav, refresh};
use sweatxr_ui::{AppState, Feedback, Mode, PanelView, WatchView};

const NOW: i64 = 20_396 * 86_400 + 64_800;

#[derive(Debug, PartialEq)]
enum Got {
    Weight(f32),
    Goal(i32),
    Female(bool),
    Resting(i32),
    Delete,
    Imperial(bool),
    Theme(i32),
    AutoStart(bool),
    FirstRunDone,
    Close,
    Scan,
    Pick(String),
    Hover,
    Press,
}

fn click<C: ComponentHandle>(s: &mut Surface<C>, x: f32, y: f32) {
    s.pointer(x, y, Pointer::Move);
    s.pointer(x, y, Pointer::Down);
    s.pointer(x, y, Pointer::Up);
    s.pointer(x, y, Pointer::Leave);
    s.render();
}

fn record(panel: &Surface<PanelView>) -> Rc<RefCell<Vec<Got>>> {
    let log: Rc<RefCell<Vec<Got>>> = Rc::default();
    let s = panel.ui.global::<AppState>();
    let l = log.clone();
    s.on_profile_changed(move |d| {
        let mut l = l.borrow_mut();
        l.push(Got::Weight(d.weight));
        l.push(Got::Goal(d.goal));
        l.push(Got::Female(d.female));
        l.push(Got::Imperial(d.imperial));
        l.push(Got::Resting(d.resting_hr));
        l.push(Got::Theme(d.theme));
        l.push(Got::AutoStart(d.auto_start));
    });
    let l = log.clone();
    s.on_finish_first_run(move || l.borrow_mut().push(Got::FirstRunDone));
    let l = log.clone();
    s.on_delete_history(move || l.borrow_mut().push(Got::Delete));
    let l = log.clone();
    s.on_close_panel(move || l.borrow_mut().push(Got::Close));
    let l = log.clone();
    s.on_hr_scan(move || l.borrow_mut().push(Got::Scan));
    let l = log.clone();
    s.on_hr_pick(move |id| l.borrow_mut().push(Got::Pick(id.into())));
    let f = panel.ui.global::<Feedback>();
    let l = log.clone();
    f.on_hover(move || l.borrow_mut().push(Got::Hover));
    let l = log.clone();
    f.on_press(move || l.borrow_mut().push(Got::Press));
    log
}

fn has(log: &Rc<RefCell<Vec<Got>>>, g: &Got) -> bool {
    log.borrow().contains(g)
}

fn store_with(profile: &Profile, name: &str, secs: i64) -> Store {
    let mut store = Store::in_memory(|_| 0).expect("db");
    store.set_app_name("g", name).expect("name");
    let mut t = Tracker::new();
    for ts in NOW - secs..NOW {
        store
            .apply(&t.tick(ts, 1.4, Some(165), "g", profile))
            .expect("apply");
    }
    store
}

fn png<C: ComponentHandle>(s: &mut Surface<C>) -> Vec<u8> {
    s.render();
    let (w, h) = (s.width, s.height);
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut wr| wr.write_image_data(s.frame()))
        .expect("png");
    out
}

#[test]
fn every_panel_control_reaches_its_callback() {
    install();
    let p = Profile {
        onboarded: true,
        panel_seen: true,
        ..Profile::default()
    };
    let store = Store::in_memory(|_| 0).expect("db");
    let devices = [Device {
        id: "strap-1".into(),
        name: "Chest Strap".into(),
    }];
    let live = Live {
        tracking: false,
        game: "",
        hr: None,
        hr_status: Status::Off,
        devices: &devices,
    };
    let mut panel = Surface::new(PanelView::new, 800, 600).expect("panel");
    panel.ui.global::<Mode>().set_reduce_motion(true);
    refresh(
        &panel.ui.global::<AppState>(),
        &store,
        &p,
        NOW,
        &Nav::default(),
        &live,
    )
    .expect("refresh");
    let log = record(&panel);
    panel.render();

    // Rail tabs (x = 60): Today, Week, You, Strap, Setup.
    click(&mut panel, 60.0, TAB_Y[1]);
    assert_eq!(panel.ui.get_tab(), 1, "Week tab");
    click(&mut panel, 60.0, TAB_Y[2]);
    assert_eq!(panel.ui.get_tab(), 2, "You tab");
    assert!(
        has(&log, &Got::Hover) && has(&log, &Got::Press),
        "haptic feedback hooks"
    );

    // You: weight +, sex Female, resting heart rate -.
    click(&mut panel, PLUS_X, row_y(0));
    assert!(has(&log, &Got::Weight(71.0)), "weight +: {:?}", log.borrow());
    click(&mut panel, SEG_X[1], row_y(3));
    assert!(has(&log, &Got::Female(true)), "sex");
    click(&mut panel, MINUS_X, row_y(4));
    assert!(has(&log, &Got::Resting(59)), "resting heart rate -");

    // Setup: goal -, units Imperial.
    click(&mut panel, 60.0, TAB_Y[4]);
    click(&mut panel, MINUS_X, row_y(0));
    assert!(has(&log, &Got::Goal(275)), "goal -");
    click(&mut panel, SEG_X[1], row_y(2));
    assert!(has(&log, &Got::Imperial(true)), "units: {:?}", log.borrow());
    // Start with SteamVR: one click turns it off, the next back on.
    let auto = panel.ui.global::<AppState>().get_profile().auto_start;
    click(&mut panel, 400.0, row_y(3));
    assert!(
        has(&log, &Got::AutoStart(!auto)),
        "auto-start: {:?}",
        log.borrow()
    );
    assert_eq!(panel.ui.global::<AppState>().get_profile().auto_start, !auto);
    click(&mut panel, 400.0, row_y(3));
    assert_eq!(panel.ui.global::<AppState>().get_profile().auto_start, auto);
    // Theme: the fourth dot is Sky.
    click(&mut panel, SKY_X, row_y(4));
    assert!(has(&log, &Got::Theme(3)), "theme: {:?}", log.borrow());
    click(&mut panel, LIME_X, row_y(4));

    // Strap: scan, pick the strap.
    click(&mut panel, 60.0, TAB_Y[3]);
    insta::assert_binary_snapshot!("panel_strap.png", png(&mut panel));
    click(&mut panel, SCAN.0, SCAN.1);
    assert!(has(&log, &Got::Scan), "scan: {:?}", log.borrow());
    click(&mut panel, 400.0, 179.0);
    assert!(
        has(&log, &Got::Pick("strap-1".into())),
        "pick: {:?}",
        log.borrow()
    );

    // Setup, your data: delete takes two clicks.
    click(&mut panel, 60.0, TAB_Y[4]);
    insta::assert_binary_snapshot!("panel_setup.png", png(&mut panel));
    click(&mut panel, DELETE.0, DELETE.1);
    assert!(!has(&log, &Got::Delete), "first click only arms");
    insta::assert_binary_snapshot!("panel_setup_delete_armed.png", png(&mut panel));
    click(&mut panel, DELETE.0, DELETE.1);
    assert!(has(&log, &Got::Delete), "second click deletes");

    click(&mut panel, 60.0, CLOSE_Y);
    assert!(has(&log, &Got::Close), "close");

    // First run: Next, then Got it on the second step.
    panel.ui.global::<AppState>().set_first_run(true);
    insta::assert_binary_snapshot!("panel_first_run.png", png(&mut panel));
    click(&mut panel, NEXT.0, NEXT.1);
    assert!(!has(&log, &Got::FirstRunDone), "Next only moves to step 2");
    insta::assert_binary_snapshot!("panel_first_run_step2.png", png(&mut panel));
    click(&mut panel, NEXT.0, NEXT.1);
    assert!(has(&log, &Got::FirstRunDone), "first run Got it");
}

/// Centres on the 800x600 panel: rail tabs, rows of a page's first group, the stepper and the
/// segment cells at the right end of a row, and single buttons.
const TAB_Y: [f32; 5] = [56.0, 148.0, 240.0, 332.0, 424.0];
const CLOSE_Y: f32 = 556.0;
const MINUS_X: f32 = 576.0;
const PLUS_X: f32 = 736.0;
const SEG_X: [f32; 2] = [582.0, 698.0];
const SCAN: (f32, f32) = (735.0, 110.0);
const DELETE: (f32, f32) = (700.0, 446.0);
const LIME_X: f32 = 345.0;
const SKY_X: f32 = 513.0;
const NEXT: (f32, f32) = (720.0, 540.0);

fn row_y(i: usize) -> f32 {
    115.0 + 62.0 * i as f32
}

#[test]
fn edge_cases_render() {
    install();
    // Imperial, heavy session, very long game name, strap connected.
    let p = Profile {
        onboarded: true,
        panel_seen: true,
        units: Units::Imperial,
        goal_kcal: 500,
        ..Profile::default()
    };
    let store = store_with(
        &p,
        "An Extremely Long Virtual Reality Game Title: Definitive Edition",
        3 * 3600,
    );
    let live = Live {
        tracking: false,
        game: "",
        hr: Some(178),
        hr_status: Status::Connected,
        devices: &[],
    };

    let mut watch = Surface::new(WatchView::new, 560, 324).expect("watch");
    watch.ui.global::<Mode>().set_reduce_motion(true);
    refresh(
        &watch.ui.global::<AppState>(),
        &store,
        &p,
        NOW,
        &Nav::default(),
        &live,
    )
    .expect("refresh");
    insta::assert_binary_snapshot!("watch_big_numbers.png", png(&mut watch));

    let mut panel = Surface::new(PanelView::new, 800, 600).expect("panel");
    panel.ui.global::<Mode>().set_reduce_motion(true);
    refresh(
        &panel.ui.global::<AppState>(),
        &store,
        &p,
        NOW,
        &Nav::default(),
        &live,
    )
    .expect("refresh");
    insta::assert_binary_snapshot!("panel_today_long_name.png", png(&mut panel));
    panel.ui.set_tab(2);
    insta::assert_binary_snapshot!("panel_you_imperial.png", png(&mut panel));

    // Scanning with several straps found.
    let devices: Vec<Device> = (1..=4)
        .map(|i| Device {
            id: format!("s{i}"),
            name: format!("Heart Strap {i}"),
        })
        .collect();
    let scanning = Live {
        tracking: false,
        game: "",
        hr: None,
        hr_status: Status::Scanning,
        devices: &devices,
    };
    refresh(
        &panel.ui.global::<AppState>(),
        &store,
        &p,
        NOW,
        &Nav::default(),
        &scanning,
    )
    .expect("refresh");
    panel.ui.set_tab(3);
    insta::assert_binary_snapshot!("panel_strap_scanning.png", png(&mut panel));
}

#[test]
fn desktop_states_render_at_minimum_size() {
    use sweatxr_ui::DesktopWindow;
    install();
    let p = Profile {
        onboarded: true,
        panel_seen: true,
        ..Profile::default()
    };
    let store = store_with(&p, "Rhythm Blocks", 1800);
    let devices = [Device {
        id: "strap-1".into(),
        name: "Chest Strap With A Rather Long Bluetooth Name 12345".into(),
    }];
    let live = Live {
        tracking: false,
        game: "",
        hr: None,
        hr_status: Status::Connecting,
        devices: &devices,
    };
    // The desktop window's minimum size: compact sidebar, one column, pages scroll.
    let mut desk = Surface::new(DesktopWindow::new, 760, 520).expect("desktop");
    desk.ui.global::<Mode>().set_reduce_motion(true);
    desk.ui.global::<Mode>().set_desktop(true);
    refresh(
        &desk.ui.global::<AppState>(),
        &store,
        &p,
        NOW,
        &Nav::default(),
        &live,
    )
    .expect("refresh");
    insta::assert_binary_snapshot!("desktop_min_today.png", png(&mut desk));
    desk.ui.set_tab(1);
    insta::assert_binary_snapshot!("desktop_min_history_week.png", png(&mut desk));
    desk.ui.set_tab(2);
    desk.ui.global::<AppState>().set_export_status(
        r"Saved C:\Users\someone with a long name\Documents\SweatXR\sweatxr-2026-11-04.tcx".into(),
    );
    insta::assert_binary_snapshot!("desktop_min_export_saved.png", png(&mut desk));
    desk.ui.set_tab(3);
    insta::assert_binary_snapshot!("desktop_min_settings_device.png", png(&mut desk));
    // Hover state: pointer resting on the History nav item (icon only at this width).
    desk.pointer(36.0, 154.0, Pointer::Move);
    insta::assert_binary_snapshot!("desktop_min_hover.png", png(&mut desk));
    // Scrolled to the bottom of Settings: the last controls are reachable.
    desk.pointer(400.0, 300.0, Pointer::Scroll(-2000));
    insta::assert_binary_snapshot!("desktop_min_settings_scrolled.png", png(&mut desk));

    // First run at the minimum size: the form and its button still fit.
    let mut first = Surface::new(DesktopWindow::new, 760, 520).expect("desktop");
    first.ui.global::<Mode>().set_reduce_motion(true);
    first.ui.global::<Mode>().set_desktop(true);
    let fresh = Profile::default();
    refresh(
        &first.ui.global::<AppState>(),
        &store,
        &fresh,
        NOW,
        &Nav::default(),
        &live,
    )
    .expect("refresh");
    insta::assert_binary_snapshot!("desktop_min_first_run.png", png(&mut first));

    // A maximized 1440p window: cards keep sensible sizes instead of stretching.
    let mut big = Surface::new(DesktopWindow::new, 1700, 890).expect("desktop");
    big.ui.global::<Mode>().set_reduce_motion(true);
    big.ui.global::<Mode>().set_desktop(true);
    refresh(
        &big.ui.global::<AppState>(),
        &store,
        &p,
        NOW,
        &Nav::default(),
        &live,
    )
    .expect("refresh");
    insta::assert_binary_snapshot!("desktop_large_today.png", png(&mut big));
    big.ui.set_tab(3);
    insta::assert_binary_snapshot!("desktop_large_settings.png", png(&mut big));
}
