// Copyright (C) 2026 EncidXVII
// SPDX-License-Identifier: GPL-3.0-only

//! The in-VR process: a watch on the wrist and a panel opened from it. Runs once per compositor
//! frame, ticks the tracker at 1 Hz, redraws the watch at most 10 Hz and only when something changed.
//! On SteamVR, or on Linux on an OpenXR runtime that shows overlays (Monado, WiVRn): see `connect`.

use std::cell::Cell;
use std::error::Error;
use std::io::Write;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use slint::ComponentHandle;
use sweatxr_core::intensity::IntensityMeter;
use sweatxr_core::profile::Hand;
use sweatxr_core::replay;
use sweatxr_core::time::now_unix;
use sweatxr_core::tracker::{Event, Tracker};
use sweatxr_hr::{BleHr, FakeHr};
use sweatxr_ui::offscreen::{Pointer, Surface, install};
use sweatxr_ui::view::progress;
use sweatxr_ui::{AppState, Feedback, Mode, PanelView, WatchView};
use sweatxr_vr::{
    AppKind, Apps, DOT_PX, DOT_WIDTH_M, ERR_NO_SERVER, EVENT_QUIT, EVENT_SCENE_CHANGED, Glance, Gpu, HMD,
    Handle, LASER_PX, Mat, Neighbour, PANEL_WIDTH_M, ROLE_LEFT, ROLE_RIGHT, Server, Vr, WATCH_WIDTH_M,
    at_default, beside, carried, compose, distance, dot_pose, dot_texture, grip, gripped_on_wrist, hit,
    inverse, laser, laser_texture, near_face, panel_in_front, position, ray, smooth, valid, watch_offset,
};

use crate::Opts;
use crate::app::{App, Hr, Queue, wire};
use crate::setup::{self, ACTION_GRAB, ACTION_HAPTIC, ACTION_POINTER, ACTION_SELECT, ACTION_SET, Dirs};

/// Longest wait for the compositor's next frame (and the pace when there is none).
const POLL_MS: u32 = 20;
/// Shortest loop. A frame wait that returns at once must never turn the loop into thousands of SteamVR calls a
/// second: that starves the game and the compositor until the headset freezes (as WristXR's GPU wait once did).
/// Well under any headset's frame time, so it never slows a healthy loop.
const MIN_LOOP: Duration = Duration::from_millis(2);
/// Once SteamVR's server is found gone, the loop gets this long to save and leave before the process is ended.
const SERVER_GONE_GRACE: Duration = Duration::from_secs(5);
const WATCH_MIN_FRAME: Duration = Duration::from_millis(100);
/// How long the watch shows the lime GOAL face.
const CELEBRATE: Duration = Duration::from_secs(4);
/// Watch brightness when not looked at: still there at a glance, never in the way.
const ALPHA_AWAY: f32 = 0.35;
/// The watch brightens over this when looked at and dims over this after (linear, as WristXR);
/// the panel fades in over `PANEL_FADE_S`.
const WATCH_SHOW_S: f32 = 0.08;
const WATCH_HIDE_S: f32 = 0.18;
const PANEL_FADE_S: f32 = 0.16;
const HOME_KEY: &str = "steamvr.home";
const HOME_NAME: &str = "SteamVR Home";
/// OpenXR does not say which game is in front, so every game there counts under this one name.
const XR_GAME_KEY: &str = "openxr.game";
const XR_GAME_NAME: &str = "VR game";
/// The watch card at its tallest (232 px plus a strip line); a shorter card leaves the rest clear.
const WATCH_PX: (u32, u32) = (560, 324);
const WATCH_CARD_PX: f32 = 232.0;
const PANEL_PX: (u32, u32) = (800, 600);
/// A tracked pointing controller whose trigger has not reached us this long gets the actions loaded again (once).
const UNBOUND_RELOAD: Duration = Duration::from_secs(3);

#[derive(Clone, Copy)]
enum Buzz {
    Hover,
    Press,
}

struct Input {
    set: u64,
    select: u64,
    grab: u64,
    pointer: u64,
    haptic: u64,
    hands: [u64; 2],
}

/// The watch's pose on its wrist as the profile keeps it, and back.
fn rows(m: &Mat) -> [f32; 12] {
    let mut out = [0.0; 12];
    for (i, v) in m.m.iter().flatten().enumerate() {
        out[i] = *v;
    }
    out
}

fn from_rows(r: &[f32; 12]) -> Mat {
    let mut m = Mat::default();
    for (i, v) in r.iter().enumerate() {
        m.m[i / 4][i % 4] = *v;
    }
    m
}

impl Input {
    fn new(vr: &Vr, dirs: &Dirs) -> Result<Self, Box<dyn Error>> {
        vr.set_action_manifest(&setup::write_action_manifest(dirs)?)?;
        Ok(Self {
            set: vr.handle(Handle::ActionSet, ACTION_SET)?,
            select: vr.handle(Handle::Action, ACTION_SELECT)?,
            grab: vr.handle(Handle::Action, ACTION_GRAB)?,
            pointer: vr.handle(Handle::Action, ACTION_POINTER)?,
            haptic: vr.handle(Handle::Action, ACTION_HAPTIC)?,
            hands: [
                vr.handle(Handle::Source, "/user/hand/left")?,
                vr.handle(Handle::Source, "/user/hand/right")?,
            ],
        })
    }

    fn hand(&self, h: Hand) -> u64 {
        self.hands[usize::from(h == Hand::Right)]
    }
}

fn other(h: Hand) -> Hand {
    if h == Hand::Left { Hand::Right } else { Hand::Left }
}

fn role(h: Hand) -> u32 {
    if h == Hand::Left { ROLE_LEFT } else { ROLE_RIGHT }
}

/// Watch placement, glance and pointer state carried between frames.
#[derive(Default)]
struct Look {
    attached_to: Option<u32>,
    /// Tip pose in the device's frame the watch was attached with.
    tip_offset: Option<Mat>,
    /// Hand the watch was attached for (its text runs the other way on the other hand).
    hand: Option<Hand>,
    /// Its spot on the wrist it was attached with (the default one, or where the player put it).
    spot: Option<Mat>,
    alpha: f32,
    shown_alpha: f32,
    visible: bool,
    glance: Glance,
}

/// The controller's tip pose (same on every controller model), else its raw pose.
fn tip(vr: &Vr, input: Option<&Input>, h: Hand, raw: Option<&Mat>) -> Option<Mat> {
    input
        .and_then(|i| vr.pose(i.pointer, i.hand(h)))
        .or_else(|| raw.copied())
}

/// SteamVR normally says goodbye with a quit event. When its server dies instead (a crash, a kill) nothing does,
/// and the overlay would stay behind holding the strap and the database while the next SteamVR starts a second
/// one. The flag turns true once the server process has ended; if the loop then cannot leave by itself (a SteamVR
/// call that never returns), the process is ended anyway, which also gives back a trigger it held.
fn watch_server() -> Arc<AtomicBool> {
    let gone = Arc::new(AtomicBool::new(false));
    let Some(server) = Server::find() else {
        tracing::warn!("SteamVR's server process not found; only its quit event ends the overlay");
        return gone;
    };
    let flag = gone.clone();
    let watch = std::thread::Builder::new()
        .name("steamvr-watch".into())
        .spawn(move || {
            while server.alive() {
                std::thread::sleep(Duration::from_secs(1));
            }
            flag.store(true, Ordering::Relaxed);
            std::thread::sleep(SERVER_GONE_GRACE);
            tracing::warn!("SteamVR is gone and the overlay did not leave in time; ending it");
            sweatxr_vr::terminate();
        });
    if let Err(e) = watch {
        tracing::warn!("SteamVR watch: {e}");
    }
    gone
}

/// SteamVR starts the overlay with itself. Started any other way while SteamVR is off (Windows
/// restoring apps at sign-in, a stale shortcut), it must not start SteamVR or wait in the
/// background: it leaves (None). On Linux an OpenXR runtime that shows overlays (Monado, WiVRn)
/// takes SteamVR's place when SteamVR is not running, or straight away with `SWEATXR_OPENXR=1`
/// (for a SteamVR install that is there but not in use).
fn connect() -> Result<Option<Vr>, Box<dyn Error>> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("SWEATXR_OPENXR").is_some_and(|v| v == "1") {
        tracing::info!("SteamVR skipped (SWEATXR_OPENXR=1)");
        return Ok(Some(Vr::openxr()?));
    }
    let steamvr = Apps::init(AppKind::Background).map(drop);
    #[cfg(target_os = "linux")]
    if let Err(e) = &steamvr {
        match Vr::openxr() {
            Ok(vr) => {
                tracing::info!("SteamVR unavailable ({e}); using OpenXR");
                return Ok(Some(vr));
            }
            Err(x) => tracing::info!("no OpenXR overlay runtime (Monado, WiVRn): {x}"),
        }
    }
    if let Err(e) = steamvr
        && e.code == ERR_NO_SERVER
    {
        tracing::info!("SteamVR is not running; the overlay starts with it");
        return Ok(None);
    }
    Ok(Some(Vr::init(AppKind::Overlay)?))
}

pub fn run(dirs: &Dirs, opts: &Opts) -> Result<(), Box<dyn Error>> {
    let Some(vr) = connect()? else {
        return Ok(());
    };
    tracing::info!("overlay {} started", env!("CARGO_PKG_VERSION"));
    // Only SteamVR has a server to watch, a game name to give, and a list to be registered on.
    let steamvr = vr.apps();
    let server_gone = if steamvr.is_some() {
        watch_server()
    } else {
        Arc::default()
    };
    let mut app = App::open(dirs, |p| {
        if opts.fake_hr {
            Hr::Fake(FakeHr::workout())
        } else {
            Hr::Live(BleHr::start(p.hr_device.clone()))
        }
    })?;
    if let Some(apps) = steamvr {
        if let Err(e) = setup::register(apps, dirs, app.profile.auto_start) {
            tracing::warn!("register with SteamVR: {e}");
        }
        // Before the actions, so even an overlay SteamVR did not start gets its buttons on the first launch.
        if let Err(e) = apps.identify(setup::APP_KEY) {
            tracing::warn!("{e}");
        }
    }
    let input = Input::new(&vr, dirs)
        .map_err(|e| tracing::warn!("input actions: {e}; pointer disabled"))
        .ok();
    let mut record = match &opts.record {
        Some(path) => {
            let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
            writeln!(f, "{}", replay::CSV_HEADER)?;
            Some(f)
        }
        None => None,
    };

    install();
    let motion = crate::motion::animations_enabled();
    let mut watch = Surface::new(WatchView::new, WATCH_PX.0, WATCH_PX.1)?;
    let mut panel = Surface::new(PanelView::new, PANEL_PX.0, PANEL_PX.1)?;
    for mode in [watch.ui.global::<Mode>(), panel.ui.global::<Mode>()] {
        mode.set_reduce_motion(!motion);
    }
    let queue = Queue::default();
    let buzz: Rc<Cell<Option<Buzz>>> = Rc::default();
    for state in [watch.ui.global::<AppState>(), panel.ui.global::<AppState>()] {
        wire(&state, &queue);
    }
    let fb = panel.ui.global::<Feedback>();
    let b = buzz.clone();
    fb.on_hover(move || b.set(Some(Buzz::Hover)));
    let b = buzz.clone();
    fb.on_press(move || b.set(Some(Buzz::Press)));

    let ov_watch = vr.create_overlay("sweatxr.watch", "SweatXR watch")?;
    let ov_panel = vr.create_overlay("sweatxr.panel", "SweatXR")?;
    vr.set_width(ov_watch, WATCH_WIDTH_M)?;
    vr.set_width(ov_panel, PANEL_WIDTH_M)?;
    let ov_laser = vr.create_overlay("sweatxr.laser", "SweatXR pointer")?;
    let ov_dot = vr.create_overlay("sweatxr.dot", "SweatXR pointer dot")?;
    vr.set_width(ov_dot, DOT_WIDTH_M)?;
    // Over the panel and the watch, which sit at the dot's depth.
    for (ov, order) in [(ov_laser, 1), (ov_dot, 2)] {
        if let Err(e) = vr.set_sort_order(ov, order) {
            tracing::warn!("{e}");
        }
    }
    let mut tex = Textures::new(&vr);
    let mut accent = None;
    let (mut laser_on, mut dot_on) = (false, false);
    // None until the first tick, so a goal already met at launch does not celebrate again.
    let mut goal_met: Option<bool> = None;
    let mut celebrate_until: Option<Instant> = None;

    let started = Instant::now();
    let mut meter = IntensityMeter::new(app.profile.height_cm);
    let mut tracker = Tracker::new();
    let home = if steamvr.is_some() {
        (HOME_KEY, HOME_NAME)
    } else {
        (XR_GAME_KEY, XR_GAME_NAME)
    };
    let mut scene = (home.0.to_owned(), home.1.to_owned());
    let (mut last_sec, mut last_watch_frame) = (0i64, Instant::now() - WATCH_MIN_FRAME);
    let mut look = Look {
        shown_alpha: -1.0,
        ..Look::default()
    };
    let (mut last_frame, mut panel_alpha, mut on_watch) = (Instant::now(), 1.0f32, false);
    let (mut panel_open, mut select_down, mut scene_dirty, mut watch_stale) = (false, false, true, false);
    // The pointing hand's trigger is the overlay's (not the game's) while it points at the watch or the panel;
    // it changes hands only while let go, so the game never sees a held trigger freeze.
    let mut trigger_taken = false;
    // The watch gripped by the pointing hand: how that hand's ray held it, where that puts it on the wrist now, and
    // whether the grip was already down last frame.
    let (mut held, mut held_spot, mut grip_down): (Option<Mat>, Option<Mat>, bool) = (None, None, false);
    // The pointing hand's ray: its tip as it will be when the next frame shows, smoothed, as on WristXR. The beam,
    // the dot and the control under them all come from this one ray.
    let mut aim: Option<Mat> = None;
    // The watch pinned in front of the head (tests), and the panel: where it is, and how far along the ray and
    // where on it the grip holds it.
    let (mut pinned, mut panel_pose): (Option<Mat>, Option<Mat>) = (None, None);
    let mut panel_held: Option<(f32, [f32; 2])> = None;
    let panel_size = (
        PANEL_WIDTH_M,
        PANEL_WIDTH_M * PANEL_PX.1 as f32 / PANEL_PX.0 as f32,
    );
    // Safety net: since when the pointing hand's trigger has not reached us (unbound), for one manifest reload.
    let (mut unbound_since, mut reloaded): (Option<Instant>, bool) = (None, false);
    let (mut loops, mut redraws) = (0u64, 0u64);
    // What SteamVR and the strap were last told, so a change from either window reaches them.
    let mut applied = Applied::from(&app.profile);
    // WristXR's watch, while WristXR runs with it on our wrist: ours then starts beside it, not on top of it.
    let mut neighbour: Option<Neighbour> = None;

    'main: loop {
        let loop_start = Instant::now();
        let dt = loop_start.duration_since(last_frame).as_secs_f32();
        last_frame = loop_start;
        while let Some(ev) = vr.poll_event() {
            match ev.event_type {
                EVENT_QUIT => break 'main,
                EVENT_SCENE_CHANGED => scene_dirty = true,
                _ => {}
            }
        }
        if server_gone.load(Ordering::Relaxed) {
            tracing::info!("SteamVR's server is gone; leaving");
            break 'main;
        }
        let hand = app.profile.watch_hand;
        // The watch's spot on this wrist: where the grip holds it, else where the player left it, else the default
        // (beside WristXR's watch when that is on this wrist too).
        let aspect = WATCH_PX.1 as f32 / WATCH_PX.0 as f32;
        let default_spot = neighbour
            .as_ref()
            .map_or_else(|| watch_offset(hand, aspect), |n| beside(n, aspect));
        let placed = app
            .profile
            .watch_pose
            .filter(|(h, _)| *h == hand)
            .map(|(_, m)| from_rows(&m));
        let spot = held_spot.or(placed).unwrap_or(default_spot);
        // Beam and dot in the theme's accent, redrawn when the theme changes.
        if accent != Some(app.profile.theme) {
            accent = Some(app.profile.theme);
            let rgb = sweatxr_ui::accent_rgb(app.profile.theme);
            tex.upload(&vr, ov_laser, 2, &mut laser_texture(rgb), LASER_PX);
            tex.upload(&vr, ov_dot, 3, &mut dot_texture(rgb), (DOT_PX, DOT_PX));
        }
        let poses = vr.poses();
        let (left, right) = (vr.controller(ROLE_LEFT), vr.controller(ROLE_RIGHT));
        let frame = sweatxr_vr::frame(started.elapsed().as_secs_f64(), &poses[..], left, right);
        if let Some(f) = record.as_mut() {
            writeln!(f, "{}", replay::csv_row(&frame))?;
        }
        let intensity = meter.update(&frame);

        let now = now_unix();
        if now != last_sec {
            last_sec = now;
            // Every two seconds: WristXR's watch as its settings have it, counted only while its overlay is there
            // (WristXR runs) and on the wrist ours is on. It follows within two seconds when that watch is moved.
            if now % 2 == 0 {
                let theirs = crate::wristxr::load()
                    .filter(|w| w.hand == hand && vr.overlay_exists(&w.key))
                    .map(|w| w.neighbour);
                if theirs.is_some() != neighbour.is_some() {
                    tracing::info!(
                        "WristXR's watch is {} this wrist; ours is {}",
                        if theirs.is_some() { "on" } else { "no longer on" },
                        if theirs.is_some() {
                            "beside it unless it was placed by hand"
                        } else {
                            "on its own spot"
                        }
                    );
                }
                neighbour = theirs;
            }
            if scene_dirty || now % 5 == 0 {
                scene = steamvr
                    .and_then(Apps::scene_app)
                    .unwrap_or_else(|| (home.0.to_owned(), home.1.to_owned()));
                scene_dirty = false;
                app.game.clone_from(&scene.1);
                if let Err(e) = app.store.set_app_name(&scene.0, &scene.1) {
                    tracing::warn!("app name: {e}");
                }
            }
            let events = tracker.tick(now, intensity, app.hr.bpm(), &scene.0, &app.profile);
            app.tracking = tracker.in_session();
            if let Err(e) = app.store.apply(&events) {
                tracing::warn!("store: {e}");
            }
            if events.iter().any(|e| matches!(e, Event::End { .. })) {
                app.recalibrate();
            }
            // Picks up the desktop window's edits.
            app.reload_profile();
            meter.set_height(app.profile.height_cm);
            app.show(&watch.ui.global::<AppState>(), now);
            if panel_open {
                app.show(&panel.ui.global::<AppState>(), now);
            }
            let met = progress(&app.store, &app.profile, now).is_ok_and(|p| p >= 1.0);
            if goal_met == Some(false) && met {
                celebrate_until = Some(Instant::now() + CELEBRATE);
                watch.ui.global::<AppState>().set_celebrate(true);
                if let Some(inp) = &input {
                    for h in [Hand::Left, Hand::Right] {
                        vr.haptic(inp.haptic, inp.hand(h), 0.3, 0.8);
                    }
                }
            }
            goal_met = Some(met);
        }
        if celebrate_until.is_some_and(|t| Instant::now() >= t) {
            celebrate_until = None;
            watch.ui.global::<AppState>().set_celebrate(false);
        }

        let head = poses.get(HMD as usize).and_then(valid);
        let wrist = vr.controller(role(hand));
        let wrist_pose = wrist.and_then(|i| poses.get(i as usize)).and_then(valid);
        let wrist_tip = tip(&vr, input.as_ref(), hand, wrist_pose);
        match (opts.pin_watch, head) {
            (true, Some(h)) if !look.visible => {
                let m = panel_in_front(h);
                look.visible = vr
                    .place(ov_watch, &m)
                    .and_then(|()| vr.set_visible(ov_watch, true))
                    .is_ok();
                pinned = Some(m);
            }
            (true, _) => {}
            (false, _) => place_watch(
                &vr,
                ov_watch,
                &mut look,
                Wrist {
                    hand,
                    device: wrist,
                    raw: wrist_pose,
                    tip: wrist_tip.as_ref(),
                    spot: &spot,
                },
                head,
                // Celebrating, pointed at or held: full brightness wherever the wrist points.
                celebrate_until.is_some() || on_watch || held.is_some(),
                if motion { dt } else { f32::INFINITY },
            ),
        }

        let pointing = other(hand);
        let pointer_raw = vr
            .controller(role(pointing))
            .and_then(|i| poses.get(i as usize))
            .and_then(valid);
        let (pressed, changed) = match &input {
            Some(inp) => {
                let take = [Hand::Left, Hand::Right].map(|h| h == pointing && trigger_taken);
                let _ = vr.update_actions(inp.set, inp.hands, take);
                let select = vr.digital(inp.select, inp.hand(pointing));
                let unbound = select.is_none() && pointer_raw.is_some() && !vr.dashboard_visible();
                unbound_since = if unbound {
                    unbound_since.or(Some(loop_start))
                } else {
                    None
                };
                if !reloaded && unbound_since.is_some_and(|t| t.elapsed() >= UNBOUND_RELOAD) {
                    reloaded = true;
                    tracing::warn!("trigger not bound; loading the action manifest again");
                    match setup::write_action_manifest(dirs) {
                        Ok(m) => {
                            if let Err(e) = vr.set_action_manifest(&m) {
                                tracing::warn!("{e}");
                            }
                        }
                        Err(e) => tracing::warn!("action manifest: {e}"),
                    }
                }
                select.unwrap_or_default()
            }
            None => (false, false),
        };
        let (down, up) = (pressed && changed && !select_down, !pressed && select_down);
        select_down = pressed;

        // After the actions were updated: both hands' tips as they will be when the next frame shows (else as
        // they are now). The ray is the pointing hand's, smoothed; the watch is where its wrist will be then.
        let next = |h: Hand, raw: Option<&Mat>| {
            input
                .as_ref()
                .and_then(|i| vr.pose_next(i.pointer, i.hand(h)))
                .or_else(|| tip(&vr, input.as_ref(), h, raw))
        };
        aim = next(pointing, pointer_raw).map(|raw| aim.map_or(raw, |prev| smooth(&prev, &raw)));
        let pointer = aim.as_ref();
        let wrist_next = next(hand, wrist_pose);
        let watch_pose = if opts.pin_watch {
            pinned
        } else {
            wrist_next.map(|t| compose(&t, &spot))
        };
        // The wrist's device pose at that moment, for what rides on it (the dot on the watch).
        let wrist_ride = match (wrist_next, look.tip_offset) {
            (Some(t), Some(off)) => Some(compose(&t, &inverse(&off))),
            (t, _) => t,
        };

        let mut on_panel = false;
        // Where the beam ends, if the pointer is on the watch or the panel.
        let mut beam = None;
        // The panel under the ray: how far along the ray, and where on it (metres right and up of its middle).
        let mut panel_at = None;
        if let (Some(ptr), Some(pp), true) = (pointer, panel_pose, panel_open) {
            let (o, d) = ray(ptr);
            if let Some((uv, at)) = hit(&pp, panel_size.0, panel_size.1, o, d) {
                on_panel = true;
                beam = Some((o, at));
                panel_at = Some((
                    distance(o, at),
                    [(uv[0] - 0.5) * panel_size.0, (0.5 - uv[1]) * panel_size.1],
                ));
                let (x, y) = (uv[0] * PANEL_PX.0 as f32, uv[1] * PANEL_PX.1 as f32);
                panel.pointer(x, y, Pointer::Move);
                panel.ui.global::<AppState>().set_cursor_visible(true);
                if down {
                    panel.pointer(x, y, Pointer::Down);
                }
                if up {
                    panel.pointer(x, y, Pointer::Up);
                }
            }
        }
        if !on_panel && panel_open {
            panel.pointer(0.0, 0.0, Pointer::Leave);
            panel.ui.global::<AppState>().set_cursor_visible(false);
        }
        if !on_panel
            && look.visible
            && let (Some(p), Some(wp)) = (pointer, watch_pose.as_ref())
        {
            let (o, d) = ray(p);
            // Only the card counts, not the clear part of the texture under a short card. Once on
            // it, the grown card counts too, so the strip it shows can be pointed at.
            let s = watch.ui.global::<AppState>();
            let tall = on_watch || s.get_hr_lost() || !s.get_panel_seen();
            let card = if tall { WATCH_PX.1 as f32 } else { WATCH_CARD_PX };
            beam = hit(wp, WATCH_WIDTH_M, WATCH_WIDTH_M * aspect, o, d)
                .filter(|(uv, _)| uv[1] * (WATCH_PX.1 as f32) < card)
                .map(|(_, at)| (o, at));
        }
        on_watch = !on_panel && beam.is_some();
        // Grip on the pointing hand while its beam is on the watch: the watch goes with that hand (slide it along
        // the wrist, turn the hand to tilt it) and stays where the grip lets go, as on WristXR. Let go on its
        // default spot, it is back on it.
        let gripping = input
            .as_ref()
            .and_then(|inp| vr.digital(inp.grab, inp.hand(pointing)))
            .is_some_and(|(pressed, _)| pressed);
        match (held, pointer, wrist_next.as_ref()) {
            (None, Some(ptr), Some(tip)) if gripping && !grip_down && on_watch => {
                held = Some(grip(ptr, &compose(tip, &spot)));
                buzz.set(Some(Buzz::Press));
            }
            (Some(h), Some(ptr), Some(tip)) if gripping => held_spot = Some(gripped_on_wrist(ptr, &h, tip)),
            (Some(_), ..) => {
                held = None;
                if let Some(s) = held_spot.take() {
                    app.set_watch_pose((!at_default(&s, &default_spot)).then(|| (hand, rows(&s))));
                    buzz.set(Some(Buzz::Press));
                }
            }
            _ => {}
        }
        // Grip on the panel: it keeps the gripped spot under the beam and turns to face you, level, as a held panel
        // does on WristXR, and stays where the grip lets go.
        match (panel_held, pointer, panel_pose, head) {
            (None, ..) if gripping && !grip_down && panel_at.is_some() => {
                panel_held = panel_at;
                buzz.set(Some(Buzz::Press));
            }
            (Some((reach, local)), Some(ptr), Some(was), Some(h)) if gripping && panel_open => {
                let (o, d) = ray(ptr);
                let m = carried(&was, local, o, d, reach, position(h));
                if let Err(e) = vr.place(ov_panel, &m) {
                    tracing::warn!("{e}");
                }
                panel_pose = Some(m);
            }
            (Some(_), ..) => panel_held = None,
            _ => {}
        }
        grip_down = gripping;
        let wants = on_watch || on_panel || held.is_some() || panel_held.is_some();
        trigger_taken = if trigger_taken {
            wants || pressed
        } else {
            wants && !pressed
        };
        watch.ui.global::<AppState>().set_cursor_visible(on_watch);
        if on_watch && down {
            panel_open = !panel_open;
            if let (true, Some(h)) = (panel_open, head) {
                app.mark_panel_seen();
                app.show(&watch.ui.global::<AppState>(), now);
                app.show(&panel.ui.global::<AppState>(), now);
                let m = panel_in_front(h);
                if let Err(e) = vr.place(ov_panel, &m) {
                    tracing::warn!("{e}");
                }
                panel_pose = Some(m);
                // Fades in rather than flashing into view.
                panel_alpha = if motion { 0.0 } else { 1.0 };
                let _ = vr.set_alpha(ov_panel, panel_alpha);
            }
            let _ = vr.set_visible(ov_panel, panel_open);
            buzz.set(Some(Buzz::Press));
        }

        match (beam, head) {
            (Some((from, to)), Some(h)) => {
                let eye = position(h);
                let (m, len) = laser(from, to, eye);
                // The beam runs from the ray's start to the very point the dot sits on: one ray, one answer, so
                // the two cannot part (a beam riding the controller pointed where the hand is now, with a dot
                // where the ray was a frame or two ago). On the watch the dot rides the wrist, so it stays on the
                // spot of the watch it was measured on between our frames.
                let placed = vr.place(ov_laser, &m).and_then(|()| vr.set_width(ov_laser, len));
                let carrier = if on_panel || opts.pin_watch {
                    None
                } else {
                    wrist.zip(wrist_ride.as_ref())
                };
                let dotted = ride(&vr, ov_dot, &dot_pose(to, eye), carrier);
                if placed.is_ok() && !laser_on {
                    laser_on = vr.set_visible(ov_laser, true).is_ok();
                }
                if dotted.is_ok() && !dot_on {
                    dot_on = vr.set_visible(ov_dot, true).is_ok();
                }
            }
            _ => {
                if laser_on {
                    laser_on = vr.set_visible(ov_laser, false).is_err();
                }
                if dot_on {
                    dot_on = vr.set_visible(ov_dot, false).is_err();
                }
            }
        }

        if panel_open && panel_alpha < 1.0 {
            panel_alpha = (panel_alpha + dt / PANEL_FADE_S).min(1.0);
            let _ = vr.set_alpha(ov_panel, panel_alpha);
        }

        if let (Some(inp), Some(b)) = (&input, buzz.take()) {
            let (secs, amp) = match b {
                Buzz::Hover => (0.01, 0.25),
                Buzz::Press => (0.02, 0.6),
            };
            vr.haptic(inp.haptic, inp.hand(other(hand)), secs, amp);
        }

        let cmds: Vec<_> = queue.borrow_mut().drain(..).collect();
        for c in cmds {
            if app.handle(c) {
                panel_open = false;
                let _ = vr.set_visible(ov_panel, false);
            }
            app.show(&panel.ui.global::<AppState>(), now);
            app.show(&watch.ui.global::<AppState>(), now);
        }
        applied.sync(&vr, &app);

        if last_watch_frame.elapsed() >= WATCH_MIN_FRAME && watch.render() {
            last_watch_frame = Instant::now();
            redraws += 1;
            watch_stale = true;
        }
        // Upload only to a visible watch: SteamVR does not release raw uploads to an overlay it
        // never draws (soak test: RequestFailed after ~240 uploads while hidden).
        if watch_stale && look.visible {
            watch_stale = false;
            tex.upload(&vr, ov_watch, 0, watch.frame(), WATCH_PX);
        }
        if panel_open && panel.render() {
            redraws += 1;
            tex.upload(&vr, ov_panel, 1, panel.frame(), PANEL_PX);
        }

        loops += 1;
        if opts.exit_after.is_some_and(|s| started.elapsed().as_secs() >= s) {
            break;
        }
        vr.wait_frame(POLL_MS);
        if let Some(rest) = MIN_LOOP.checked_sub(loop_start.elapsed()) {
            std::thread::sleep(rest);
        }
    }

    if let Err(e) = app.store.apply(&tracker.finish()) {
        tracing::warn!("store: {e}");
    }
    vr.destroy_overlay(ov_watch);
    vr.destroy_overlay(ov_panel);
    vr.destroy_overlay(ov_laser);
    vr.destroy_overlay(ov_dot);
    // SteamVR keeps a handle to our Vulkan images until it shuts down; destroying the device
    // first crashed on exit. So: OpenVR down, then the GPU objects.
    drop(vr);
    drop(tex);
    tracing::info!(
        "overlay stopped after {:.0} s: {loops} loops, {redraws} redraws",
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

/// GPU textures when Vulkan works (slot 0 watch, 1 panel, 2 laser, 3 dot), else SetOverlayRaw, which leaks
/// inside SteamVR and is only a fallback.
struct Textures(Option<Gpu>);

impl Textures {
    fn new(vr: &Vr) -> Self {
        // OpenXR: the session has textures of its own, filled through `set_raw`.
        #[cfg(target_os = "linux")]
        if vr.is_openxr() {
            return Self(None);
        }
        let gpu = vr.gpu().and_then(|mut g| {
            g.add_texture(WATCH_PX.0, WATCH_PX.1)?;
            g.add_texture(PANEL_PX.0, PANEL_PX.1)?;
            g.add_texture(LASER_PX.0, LASER_PX.1)?;
            g.add_texture(DOT_PX, DOT_PX)?;
            Ok(g)
        });
        match gpu {
            Ok(g) => Self(Some(g)),
            Err(e) => {
                tracing::warn!("Vulkan textures unavailable, using SetOverlayRaw: {e}");
                Self(None)
            }
        }
    }

    fn upload(&mut self, vr: &Vr, overlay: u64, slot: usize, px: &mut [u8], (w, h): (u32, u32)) {
        let result = match &mut self.0 {
            Some(g) => vr.set_texture(overlay, g, slot, px),
            None => vr.set_raw(overlay, px, w, h),
        };
        if let Err(e) = result {
            tracing::warn!("{e}");
        }
    }
}

/// Places an overlay at world pose `m`, held by a tracked device (its index and pose this frame)
/// when there is one, so the compositor moves it with that device between our frames.
fn ride(vr: &Vr, ov: u64, m: &Mat, device: Option<(u32, &Mat)>) -> Result<(), sweatxr_vr::VrError> {
    match device {
        Some((i, pose)) => vr.attach(ov, i, &compose(&inverse(pose), m)),
        None => vr.place(ov, m),
    }
}

/// The watch's controller: which hand, its device and its raw and tip poses this frame.
struct Wrist<'a> {
    hand: Hand,
    device: Option<u32>,
    raw: Option<&'a Mat>,
    tip: Option<&'a Mat>,
    /// The watch relative to the tip pose: its default spot, or where the player put it.
    spot: &'a Mat,
}

fn place_watch(vr: &Vr, ov: u64, look: &mut Look, wrist: Wrist, head: Option<&Mat>, full: bool, dt: f32) {
    // The overlay rides on the device (no lag); the watch is placed from the tip pose, so the
    // offset is the tip in the device's frame. It is the same every frame for one controller model;
    // re-attach when SteamVR hands the role to another device or the model changes.
    let offset = match (wrist.raw, wrist.tip) {
        (Some(raw), Some(tip)) => Some(compose(&inverse(raw), tip)),
        _ => None,
    };
    let moved = match (offset, look.tip_offset) {
        (Some(a), Some(b)) => distance(position(&a), position(&b)) > 0.005,
        (a, b) => a.is_some() != b.is_some(),
    };
    if wrist.device != look.attached_to
        || look.hand != Some(wrist.hand)
        || look.spot != Some(*wrist.spot)
        || moved
    {
        look.attached_to = wrist.device;
        look.hand = Some(wrist.hand);
        look.spot = Some(*wrist.spot);
        look.tip_offset = offset;
        let identity = Mat {
            m: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
        };
        let m = compose(&offset.unwrap_or(identity), wrist.spot);
        if let Some(i) = wrist.device
            && let Err(e) = vr.attach(ov, i, &m)
        {
            tracing::warn!("{e}");
        }
    }
    // As on WristXR: the watch fades with the turn of the wrist and the look of the eyes, and
    // hides when the controller is at the face.
    let target = match (wrist.tip, head) {
        _ if full => 1.0,
        (Some(t), Some(h)) if near_face(t, h) => 0.0,
        (Some(t), Some(h)) => {
            let w = compose(t, wrist.spot);
            if look.glance.update(h, &w, dt) {
                1.0
            } else {
                ALPHA_AWAY
            }
        }
        _ => 0.0,
    };
    look.alpha = if target > look.alpha {
        (look.alpha + dt / WATCH_SHOW_S).min(target)
    } else {
        (look.alpha - dt / WATCH_HIDE_S).max(target)
    };
    let visible = look.alpha > 0.0;
    if visible != look.visible {
        look.visible = visible;
        let _ = vr.set_visible(ov, visible);
    }
    if (look.alpha - look.shown_alpha).abs() > 0.004 {
        look.shown_alpha = look.alpha;
        let _ = vr.set_alpha(ov, look.alpha);
    }
}

/// The settings that live outside the profile too: SteamVR's auto-launch flag and the strap.
struct Applied {
    auto_start: bool,
    hr_device: Option<String>,
}

impl From<&sweatxr_core::profile::Profile> for Applied {
    fn from(p: &sweatxr_core::profile::Profile) -> Self {
        Self {
            auto_start: p.auto_start,
            hr_device: p.hr_device.clone(),
        }
    }
}

impl Applied {
    /// Passes on whatever changed since last time, whichever window changed it.
    fn sync(&mut self, vr: &Vr, app: &App) {
        let p = &app.profile;
        if self.auto_start != p.auto_start {
            self.auto_start = p.auto_start;
            if let Some(apps) = vr.apps() {
                match apps.set_auto_launch(setup::APP_KEY, p.auto_start) {
                    Ok(()) => tracing::info!("start with SteamVR: {}", p.auto_start),
                    Err(e) => tracing::warn!("{e}"),
                }
            }
        }
        if self.hr_device != p.hr_device {
            self.hr_device.clone_from(&p.hr_device);
            if let Hr::Live(b) = &app.hr {
                b.use_device(p.hr_device.clone());
            }
        }
    }
}
