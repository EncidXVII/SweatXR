// Copyright (C) 2026 EncidXVII
// SPDX-License-Identifier: GPL-3.0-only

//! AUTO GATE 3: every view rendered offscreen matches its approved PNG snapshot. The data comes
//! from the real tracker and store, fed by synthetic movement and a fake heart rate strap.
//! Review changes with `cargo insta review`.

use std::time::Instant;

use slint::ComponentHandle;
use sweatxr_core::profile::Profile;
use sweatxr_core::storage::Store;
use sweatxr_core::tracker::Tracker;
use sweatxr_hr::{Device, FakeHr, Status};
use sweatxr_ui::offscreen::{Surface, install};
use sweatxr_ui::view::{Live, Nav, Range, refresh};
use sweatxr_ui::{AppState, DesktopWindow, Mode, PanelView, WatchView};

const DAY: i64 = 86_400;
/// 2025-11-04 18:00 UTC, a Tuesday.
const NOW: i64 = 20_396 * DAY + 64_800;

fn png(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut w| w.write_image_data(rgba))
        .expect("png");
    out
}

fn store() -> (Store, Profile) {
    let mut store = Store::in_memory(|_| 0).expect("db");
    let p = Profile {
        onboarded: true,
        panel_seen: true,
        hr_device: Some("strap-1".into()),
        ..Profile::default()
    };
    store.set_app_name("app.rhythm", "Rhythm Blocks").expect("name");
    store
        .set_app_name("app.boxing", "Shadow Boxing Club")
        .expect("name");
    store.set_app_name("app.sim", "Table Tennis Sim").expect("name");
    let strap = FakeHr::workout();
    let mut tracker = Tracker::new();
    let mut play = |start: i64, secs: i64, intensity: f32, app: &str, hr: bool| {
        for ts in start..start + secs {
            let bpm = if hr {
                strap.value_at((ts - start) as u64)
            } else {
                None
            };
            let ev = tracker.tick(ts, intensity, bpm, app, &p);
            store.apply(&ev).expect("apply");
        }
        store.apply(&tracker.finish()).expect("finish");
    };
    for back in 1..=6 {
        let day = NOW - back * DAY;
        play(
            day,
            600 + back * 120,
            0.5 + back as f32 * 0.08,
            "app.rhythm",
            false,
        );
    }
    // The weeks before: most days played, a rest day now and then, so a month of history looks lived
    // in. No strap on those days, so calibration still shows 19 of 20 minutes.
    for back in 7..=29 {
        if back % 5 == 2 || back == 16 {
            continue;
        }
        let app = if back % 3 == 0 { "app.boxing" } else { "app.rhythm" };
        play(
            NOW - back * DAY,
            420 + (back * 7 % 11) * 90,
            0.45 + (back * 5 % 7) as f32 * 0.06,
            app,
            false,
        );
    }
    play(NOW - 3 * 3600, 1200, 0.9, "app.boxing", true);
    play(NOW - 3600, 900, 0.6, "app.rhythm", false);
    play(NOW - 1800, 300, 0.35, "app.sim", false);
    (store, p)
}

fn check<C: ComponentHandle>(name: &str, s: &mut Surface<C>) {
    let (w, h) = (s.width, s.height);
    let rgba = if s.render() {
        s.frame().to_vec()
    } else {
        Vec::new()
    };
    assert_eq!(rgba.len(), (w * h * 4) as usize, "{name}: nothing rendered");
    insta::assert_binary_snapshot!(&format!("{name}.png"), png(&rgba, w, h));
}

#[test]
fn every_view_matches_its_snapshot() {
    install();
    let (store, p) = store();
    let devices = [
        Device {
            id: "strap-1".into(),
            name: "Chest Strap 1".into(),
        },
        Device {
            id: "strap-2".into(),
            name: "Arm Band".into(),
        },
    ];
    let live = Live {
        tracking: true,
        game: "Shadow Boxing Club",
        hr: Some(142),
        hr_status: Status::Connected,
        devices: &devices,
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
    check("watch", &mut watch);
    // Pointed at with the other hand: the card grows a line saying what the trigger does.
    watch.ui.global::<AppState>().set_cursor_visible(true);
    check("watch_pointed", &mut watch);
    watch.ui.global::<AppState>().set_cursor_visible(false);

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
    for (tab, name) in [
        (0, "panel_today"),
        (1, "panel_week"),
        (2, "panel_you"),
        (3, "panel_strap"),
        (4, "panel_setup"),
    ] {
        panel.ui.set_tab(tab);
        check(name, &mut panel);
    }
    panel.ui.global::<AppState>().set_first_run(true);
    check("panel_first_run", &mut panel);

    let mut desk = Surface::new(DesktopWindow::new, 1200, 800).expect("desktop");
    desk.ui.global::<Mode>().set_reduce_motion(true);
    desk.ui.global::<Mode>().set_desktop(true);
    for (range, name) in [
        (Range::Day, "desktop_history_day"),
        (Range::Month, "desktop_history_month"),
    ] {
        refresh(
            &desk.ui.global::<AppState>(),
            &store,
            &p,
            NOW,
            &Nav {
                range,
                ..Nav::default()
            },
            &live,
        )
        .expect("refresh");
        desk.ui.set_tab(1);
        check(name, &mut desk);
    }
    for (tab, name) in [
        (0, "desktop_today"),
        (2, "desktop_export"),
        (3, "desktop_settings"),
    ] {
        desk.ui.set_tab(tab);
        check(name, &mut desk);
    }

    // First use: the watch teaches the gesture until the panel has been opened once.
    let first = Profile {
        panel_seen: false,
        ..p.clone()
    };
    refresh(
        &watch.ui.global::<AppState>(),
        &store,
        &first,
        NOW,
        &Nav::default(),
        &live,
    )
    .expect("refresh");
    check("watch_first_use", &mut watch);
    watch.ui.global::<AppState>().set_celebrate(true);
    check("watch_goal_celebration", &mut watch);
    watch.ui.global::<AppState>().set_celebrate(false);

    // The strap stops sending mid-session.
    let lost = Live { hr: None, ..live };
    refresh(
        &watch.ui.global::<AppState>(),
        &store,
        &p,
        NOW,
        &Nav::default(),
        &lost,
    )
    .expect("refresh");
    check("watch_strap_lost", &mut watch);

    // Empty state, no strap.
    let empty = Store::in_memory(|_| 0).expect("db");
    let none = Live {
        tracking: false,
        game: "",
        hr: None,
        hr_status: Status::Off,
        devices: &[],
    };
    refresh(
        &watch.ui.global::<AppState>(),
        &empty,
        &p,
        NOW,
        &Nav::default(),
        &none,
    )
    .expect("refresh");
    check("watch_empty", &mut watch);
}

/// UI spike numbers for DECISIONS.md: `cargo test --release -p sweatxr-ui -- --ignored --nocapture`.
#[test]
#[ignore]
fn render_cost() {
    install();
    let (store, p) = store();
    let live = Live {
        tracking: false,
        game: "",
        hr: Some(120),
        hr_status: Status::Connected,
        devices: &[],
    };
    let mut watch = Surface::new(WatchView::new, 560, 324).expect("watch");
    watch.ui.global::<Mode>().set_reduce_motion(true);
    let mut panel = Surface::new(PanelView::new, 800, 600).expect("panel");
    panel.ui.global::<Mode>().set_reduce_motion(true);
    // A macro, not a closure: the surface is borrowed both for its global and for rendering.
    macro_rules! bench {
        ($name:literal, $frames:literal, $surface:expr) => {{
            let (mut data, mut draw, mut drawn) = (0.0, 0.0, 0);
            for i in 0..$frames {
                let state = $surface.ui.global::<AppState>();
                let t = Instant::now();
                refresh(&state, &store, &p, NOW + i64::from(i), &Nav::default(), &live).expect("refresh");
                // A changing kcal value forces the same partial redraw a live tick causes.
                state.set_kcal_text(i.to_string().into());
                data += t.elapsed().as_secs_f64();
                let t = Instant::now();
                drawn += u32::from($surface.render());
                draw += t.elapsed().as_secs_f64();
            }
            let per = |s: f64| s * 1000.0 / f64::from($frames);
            let (name, frames) = ($name, $frames);
            println!(
                "{name}: refresh {:.2} ms + redraw {:.2} ms, {drawn}/{frames} drawn",
                per(data),
                per(draw)
            );
        }};
    }
    bench!("watch", 200u32, watch);
    bench!("panel", 50u32, panel);
}
