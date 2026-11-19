//! The in-VR process: a watch on the wrist and a panel opened from it. Runs once per compositor
//! frame, ticks the tracker at 1 Hz, redraws the watch at most 10 Hz and only when something changed.

use std::cell::Cell;
use std::error::Error;
use std::io::Write;
use std::rc::Rc;
use std::time::{Duration, Instant};

use fitvr_core::intensity::IntensityMeter;
use fitvr_core::profile::Hand;
use fitvr_core::replay;
use fitvr_core::time::now_unix;
use fitvr_core::tracker::{Event, Tracker};
use fitvr_hr::{BleHr, FakeHr};
use fitvr_ui::offscreen::{Pointer, Surface, install};
use fitvr_ui::view::progress;
use fitvr_ui::{AppState, Feedback, Mode, PanelView, WatchView};
use fitvr_vr::{
    AppKind, Apps, DOT_PX, DOT_WIDTH_M, ERR_NO_SERVER, EVENT_QUIT, EVENT_SCENE_CHANGED, Glance, Gpu, HMD,
    Handle, LASER_PX, Mat, PANEL_WIDTH_M, ROLE_LEFT, ROLE_RIGHT, Vr, WATCH_WIDTH_M, compose, distance,
    dot_pose, dot_texture, inverse, laser, laser_texture, near_face, panel_in_front, position, ray, uv_to_px,
    valid, watch_offset,
};
use slint::ComponentHandle;

use crate::Opts;
use crate::app::{App, Hr, Queue, wire};
use crate::setup::{self, ACTION_HAPTIC, ACTION_POINTER, ACTION_SELECT, ACTION_SET, Dirs};

/// Longest wait for the compositor's next frame (and the pace when there is none).
const POLL_MS: u32 = 20;
const WATCH_MIN_FRAME: Duration = Duration::from_millis(100);
/// How long the watch shows the lime GOAL face.
const CELEBRATE: Duration = Duration::from_secs(4);
/// Watch brightness when not looked at: still there at a glance, never in the way.
const ALPHA_AWAY: f32 = 0.35;
/// The watch brightens over this when looked at and dims over this after (linear, as DesktopVR);
/// the panel fades in over `PANEL_FADE_S`.
const WATCH_SHOW_S: f32 = 0.08;
const WATCH_HIDE_S: f32 = 0.18;
const PANEL_FADE_S: f32 = 0.16;
const HOME_KEY: &str = "steamvr.home";
const HOME_NAME: &str = "SteamVR Home";
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
    pointer: u64,
    haptic: u64,
    hands: [u64; 2],
}

impl Input {
    fn new(vr: &Vr, dirs: &Dirs) -> Result<Self, Box<dyn Error>> {
        vr.set_action_manifest(&setup::write_action_manifest(dirs)?)?;
        Ok(Self {
            set: vr.handle(Handle::ActionSet, ACTION_SET)?,
            select: vr.handle(Handle::Action, ACTION_SELECT)?,
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

pub fn run(dirs: &Dirs, opts: &Opts) -> Result<(), Box<dyn Error>> {
    // SteamVR starts the overlay with itself. Started any other way while SteamVR is off (Windows
    // restoring apps at sign-in, a stale shortcut), it must not start SteamVR or wait in the
    // background: it leaves.
    if let Err(e) = Apps::init(AppKind::Background)
        && e.code == ERR_NO_SERVER
    {
        tracing::info!("SteamVR is not running; the overlay starts with it");
        return Ok(());
    }
    let vr = Vr::init(AppKind::Overlay)?;
    tracing::info!("overlay {} started", env!("CARGO_PKG_VERSION"));
    let mut app = App::open(dirs, |p| {
        if opts.fake_hr {
            Hr::Fake(FakeHr::workout())
        } else {
            Hr::Live(BleHr::start(p.hr_device.clone()))
        }
    })?;
    if let Err(e) = setup::register(vr.apps(), dirs, app.profile.auto_start) {
        tracing::warn!("register with SteamVR: {e}");
    }
    // Before the actions, so even an overlay SteamVR did not start gets its buttons on the first launch.
    if let Err(e) = vr.apps().identify(setup::APP_KEY) {
        tracing::warn!("{e}");
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

    let ov_watch = vr.create_overlay("fitnessvr.watch", "FitnessVR watch")?;
    let ov_panel = vr.create_overlay("fitnessvr.panel", "FitnessVR")?;
    vr.set_width(ov_watch, WATCH_WIDTH_M)?;
    vr.set_width(ov_panel, PANEL_WIDTH_M)?;
    let ov_laser = vr.create_overlay("fitnessvr.laser", "FitnessVR pointer")?;
    let ov_dot = vr.create_overlay("fitnessvr.dot", "FitnessVR pointer dot")?;
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
    let mut scene = (HOME_KEY.to_owned(), HOME_NAME.to_owned());
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
    // Safety net: since when the pointing hand's trigger has not reached us (unbound), for one manifest reload.
    let (mut unbound_since, mut reloaded): (Option<Instant>, bool) = (None, false);
    let (mut loops, mut redraws) = (0u64, 0u64);
    // What SteamVR and the strap were last told, so a change from either window reaches them.
    let mut applied = Applied::from(&app.profile);

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
        let hand = app.profile.watch_hand;
        // Beam and dot in the theme's accent, redrawn when the theme changes.
        if accent != Some(app.profile.theme) {
            accent = Some(app.profile.theme);
            let rgb = fitvr_ui::accent_rgb(app.profile.theme);
            tex.upload(&vr, ov_laser, 2, &mut laser_texture(rgb), LASER_PX);
            tex.upload(&vr, ov_dot, 3, &mut dot_texture(rgb), (DOT_PX, DOT_PX));
        }
        let poses = vr.poses();
        let (left, right) = (vr.controller(ROLE_LEFT), vr.controller(ROLE_RIGHT));
        let frame = fitvr_vr::frame(started.elapsed().as_secs_f64(), &poses[..], left, right);
        if let Some(f) = record.as_mut() {
            writeln!(f, "{}", replay::csv_row(&frame))?;
        }
        let intensity = meter.update(&frame);

        let now = now_unix();
        if now != last_sec {
            last_sec = now;
            if scene_dirty || now % 5 == 0 {
                scene = vr
                    .apps()
                    .scene_app()
                    .unwrap_or_else(|| (HOME_KEY.to_owned(), HOME_NAME.to_owned()));
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
                look.visible = vr
                    .place(ov_watch, &panel_in_front(h))
                    .and_then(|()| vr.set_visible(ov_watch, true))
                    .is_ok();
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
                },
                head,
                // Celebrating or pointed at: full brightness wherever the wrist points.
                celebrate_until.is_some() || on_watch,
                if motion { dt } else { f32::INFINITY },
            ),
        }

        let pointer_device = vr.controller(role(other(hand)));
        let pointer_raw = pointer_device.and_then(|i| poses.get(i as usize)).and_then(valid);
        let pointer = tip(&vr, input.as_ref(), other(hand), pointer_raw);
        let pointer = pointer.as_ref();
        let pointing = other(hand);
        let (pressed, changed) = match &input {
            Some(inp) => {
                let take = [Hand::Left, Hand::Right].map(|h| h == pointing && trigger_taken);
                let _ = vr.update_actions(inp.set, inp.hands, take);
                let select = vr.digital(inp.select, inp.hand(pointing));
                let unbound = select.is_none() && pointer.is_some() && !vr.dashboard_visible();
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

        let mut on_panel = false;
        // Where the beam ends, if the pointer is on the watch or the panel.
        let mut beam = None;
        if let (Some(ptr), true) = (pointer, panel_open) {
            let (o, d) = ray(ptr);
            if let Some((uv, hit)) = vr.intersect(ov_panel, o, d) {
                on_panel = true;
                beam = Some((o, hit));
                let (x, y) = uv_to_px(uv, PANEL_PX.0, PANEL_PX.1);
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
            && let Some(p) = pointer
        {
            let (o, d) = ray(p);
            // Only the card counts, not the clear part of the texture under a short card. Once on
            // it, the grown card counts too, so the strip it shows can be pointed at.
            let s = watch.ui.global::<AppState>();
            let tall = on_watch || s.get_hr_lost() || !s.get_panel_seen();
            let card = if tall { WATCH_PX.1 as f32 } else { WATCH_CARD_PX };
            beam = vr
                .intersect(ov_watch, o, d)
                .filter(|(uv, _)| uv_to_px(*uv, WATCH_PX.0, WATCH_PX.1).1 < card)
                .map(|(_, hit)| (o, hit));
        }
        on_watch = !on_panel && beam.is_some();
        let wants = on_watch || on_panel;
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
                if let Err(e) = vr.place(ov_panel, &panel_in_front(h)) {
                    tracing::warn!("{e}");
                }
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
                // The beam rides on the pointing controller and the dot on what it points at, so
                // both move with them at the headset's rate, in step with each other.
                let placed = ride(&vr, ov_laser, &m, pointer_device.zip(pointer_raw))
                    .and_then(|()| vr.set_width(ov_laser, len));
                let carrier = if on_panel { None } else { wrist.zip(wrist_pose) };
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
fn ride(vr: &Vr, ov: u64, m: &Mat, device: Option<(u32, &Mat)>) -> Result<(), fitvr_vr::VrError> {
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
}

fn place_watch(vr: &Vr, ov: u64, look: &mut Look, wrist: Wrist, head: Option<&Mat>, full: bool, dt: f32) {
    let aspect = WATCH_PX.1 as f32 / WATCH_PX.0 as f32;
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
    if wrist.device != look.attached_to || look.hand != Some(wrist.hand) || moved {
        look.attached_to = wrist.device;
        look.hand = Some(wrist.hand);
        look.tip_offset = offset;
        let identity = Mat {
            m: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
        };
        let m = compose(&offset.unwrap_or(identity), &watch_offset(wrist.hand, aspect));
        if let Some(i) = wrist.device
            && let Err(e) = vr.attach(ov, i, &m)
        {
            tracing::warn!("{e}");
        }
    }
    // As on DesktopVR: the watch fades with the turn of the wrist and the look of the eyes, and
    // hides when the controller is at the face.
    let target = match (wrist.tip, head) {
        _ if full => 1.0,
        (Some(t), Some(h)) if near_face(t, h) => 0.0,
        (Some(t), Some(h)) => {
            let w = compose(t, &watch_offset(wrist.hand, aspect));
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

impl From<&fitvr_core::profile::Profile> for Applied {
    fn from(p: &fitvr_core::profile::Profile) -> Self {
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
            match vr.apps().set_auto_launch(setup::APP_KEY, p.auto_start) {
                Ok(()) => tracing::info!("start with SteamVR: {}", p.auto_start),
                Err(e) => tracing::warn!("{e}"),
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
