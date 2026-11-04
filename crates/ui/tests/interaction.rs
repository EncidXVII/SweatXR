//! QA: drive the real views with pointer events, the way the VR pointer and the mouse do, and
//! check every control reaches its callback. Also renders edge cases (imperial units, huge
//! numbers, long names, scanning) into snapshots so layout regressions show up in review.

use std::cell::RefCell;
use std::rc::Rc;

use fitvr_core::profile::{Profile, Units};
use fitvr_core::storage::Store;
use fitvr_core::tracker::Tracker;
use fitvr_hr::{Device, Status};
use fitvr_ui::offscreen::{Pointer, Surface, install};
use fitvr_ui::view::{Live, Range, refresh};
use fitvr_ui::{AppState, Feedback, PanelView, WatchView};
use slint::ComponentHandle;

const NOW: i64 = 20_396 * 86_400 + 64_800;

#[derive(Debug, PartialEq)]
enum Got {
    Weight(f32),
    Goal(i32),
    Female(bool),
    Imperial(bool),
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
    });
    let l = log.clone();
    s.on_finish_first_run(move || l.borrow_mut().push(Got::FirstRunDone));
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
        ..Profile::default()
    };
    let store = Store::in_memory(|_| 0).expect("db");
    let devices = [Device {
        id: "strap-1".into(),
        name: "Chest Strap".into(),
    }];
    let live = Live {
        tracking: false,
        hr: None,
        hr_status: Status::Off,
        devices: &devices,
    };
    let mut panel = Surface::new(PanelView::new, 1200, 800).expect("panel");
    refresh(
        &panel.ui.global::<AppState>(),
        &store,
        &p,
        NOW,
        Range::Week,
        &live,
    )
    .expect("refresh");
    let log = record(&panel);
    panel.render();

    // Tabs (y = 80): TODAY, WEEK, SETTINGS.
    click(&mut panel, 215.0, 80.0);
    assert_eq!(panel.ui.get_tab(), 1, "WEEK tab");
    click(&mut panel, 358.0, 80.0);
    assert_eq!(panel.ui.get_tab(), 2, "SETTINGS tab");
    assert!(
        has(&log, &Got::Hover) && has(&log, &Got::Press),
        "haptic feedback hooks"
    );

    // Profile section: weight +, goal -, sex FEMALE.
    click(&mut panel, 1119.0, 264.0);
    assert!(has(&log, &Got::Weight(71.0)), "weight +: {:?}", log.borrow());
    click(&mut panel, 831.0, 568.0);
    assert!(has(&log, &Got::Goal(275)), "goal -");
    click(&mut panel, 1078.0, 492.0);
    assert!(has(&log, &Got::Female(true)), "sex");

    // Device section: units, scan, pick a strap.
    click(&mut panel, 251.0, 176.0);
    panel.render();
    let png_device = png(&mut panel);
    insta::assert_binary_snapshot!("panel_settings_device.png", png_device);
    log.borrow_mut().clear();
    click(&mut panel, 1060.0, 340.0);
    assert!(has(&log, &Got::Imperial(true)), "units: {:?}", log.borrow());
    click(&mut panel, 1090.0, 492.0);
    assert!(has(&log, &Got::Scan), "scan: {:?}", log.borrow());
    click(&mut panel, 600.0, 568.0);
    assert!(
        has(&log, &Got::Pick("strap-1".into())),
        "pick: {:?}",
        log.borrow()
    );

    click(&mut panel, 1086.0, 80.0);
    assert!(has(&log, &Got::Close), "close");

    panel.ui.global::<AppState>().set_first_run(true);
    panel.render();
    click(&mut panel, 1089.0, 580.0);
    assert!(has(&log, &Got::FirstRunDone), "first run START");
}

#[test]
fn edge_cases_render() {
    install();
    // Imperial, heavy session, very long game name, strap connected.
    let p = Profile {
        onboarded: true,
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
        hr: Some(178),
        hr_status: Status::Connected,
        devices: &[],
    };

    let mut watch = Surface::new(WatchView::new, 512, 320).expect("watch");
    refresh(
        &watch.ui.global::<AppState>(),
        &store,
        &p,
        NOW,
        Range::Week,
        &live,
    )
    .expect("refresh");
    insta::assert_binary_snapshot!("watch_big_numbers.png", png(&mut watch));

    let mut panel = Surface::new(PanelView::new, 1200, 800).expect("panel");
    refresh(
        &panel.ui.global::<AppState>(),
        &store,
        &p,
        NOW,
        Range::Week,
        &live,
    )
    .expect("refresh");
    insta::assert_binary_snapshot!("panel_today_long_name.png", png(&mut panel));
    panel.ui.set_tab(2);
    insta::assert_binary_snapshot!("panel_settings_imperial.png", png(&mut panel));

    // Scanning with several straps found.
    let devices: Vec<Device> = (1..=4)
        .map(|i| Device {
            id: format!("s{i}"),
            name: format!("Heart Strap {i}"),
        })
        .collect();
    let scanning = Live {
        tracking: false,
        hr: None,
        hr_status: Status::Scanning,
        devices: &devices,
    };
    refresh(
        &panel.ui.global::<AppState>(),
        &store,
        &p,
        NOW,
        Range::Week,
        &scanning,
    )
    .expect("refresh");
    click(&mut panel, 251.0, 176.0);
    insta::assert_binary_snapshot!("panel_settings_scanning.png", png(&mut panel));
}
