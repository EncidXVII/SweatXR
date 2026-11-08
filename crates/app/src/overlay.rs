//! The in-VR process: a watch on the wrist and a panel opened from it. Polls poses at 50 Hz,
//! ticks the tracker at 1 Hz, redraws the watch at most 10 Hz and only when something changed.

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
use fitvr_ui::{AppState, Feedback, PanelView, WatchView};
use fitvr_vr::{
    AppKind, EVENT_QUIT, EVENT_SCENE_CHANGED, Gpu, HMD, Handle, LASER_PX, Mat, PANEL_WIDTH_M, ROLE_LEFT,
    ROLE_RIGHT, Vr, WATCH_OFFSET, WATCH_WIDTH_M, WatchLook, laser, laser_texture, panel_in_front, position,
    ray, uv_to_px, valid, watch_look,
};
use slint::ComponentHandle;

use crate::Opts;
use crate::app::{App, Hr, Queue, wire};
use crate::setup::{self, ACTION_HAPTIC, ACTION_SELECT, ACTION_SET, Dirs};

const POLL: Duration = Duration::from_millis(20);
const WATCH_MIN_FRAME: Duration = Duration::from_millis(100);
const GLANCE_HOLD: Duration = Duration::from_millis(300);
/// How long the watch shows the lime GOAL face.
const CELEBRATE: Duration = Duration::from_secs(4);
const ALPHA_AWAY: f32 = 0.4;
const HOME_KEY: &str = "steamvr.home";
const HOME_NAME: &str = "SteamVR Home";
const WATCH_PX: (u32, u32) = (512, 320);
const PANEL_PX: (u32, u32) = (1200, 800);

#[derive(Clone, Copy)]
enum Buzz {
    Hover,
    Press,
}

struct Input {
    set: u64,
    select: u64,
    haptic: u64,
    hands: [u64; 2],
}

impl Input {
    fn new(vr: &Vr, dirs: &Dirs) -> Result<Self, Box<dyn Error>> {
        vr.set_action_manifest(&setup::write_action_manifest(dirs)?)?;
        Ok(Self {
            set: vr.handle(Handle::ActionSet, ACTION_SET)?,
            select: vr.handle(Handle::Action, ACTION_SELECT)?,
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
    facing_since: Option<Instant>,
    alpha: f32,
    visible: bool,
}

pub fn run(dirs: &Dirs, opts: &Opts) -> Result<(), Box<dyn Error>> {
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
    let mut watch = Surface::new(WatchView::new, WATCH_PX.0, WATCH_PX.1)?;
    let mut panel = Surface::new(PanelView::new, PANEL_PX.0, PANEL_PX.1)?;
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
    let mut tex = Textures::new(&vr);
    tex.upload(&vr, ov_laser, 2, &mut laser_texture(), LASER_PX);
    let mut laser_on = false;
    // None until the first tick, so a goal already met at launch does not celebrate again.
    let mut goal_met: Option<bool> = None;
    let mut celebrate_until: Option<Instant> = None;

    let started = Instant::now();
    let mut meter = IntensityMeter::new(app.profile.height_cm);
    let mut tracker = Tracker::new();
    let mut scene = (HOME_KEY.to_owned(), HOME_NAME.to_owned());
    let (mut last_sec, mut last_watch_frame) = (0i64, Instant::now() - WATCH_MIN_FRAME);
    let mut look = Look {
        alpha: -1.0,
        ..Look::default()
    };
    let (mut panel_open, mut select_down, mut scene_dirty, mut watch_stale) = (false, false, true, false);
    let (mut loops, mut redraws) = (0u64, 0u64);

    'main: loop {
        let loop_start = Instant::now();
        while let Some(ev) = vr.poll_event() {
            match ev.event_type {
                EVENT_QUIT => break 'main,
                EVENT_SCENE_CHANGED => scene_dirty = true,
                _ => {}
            }
        }
        let hand = app.profile.watch_hand;
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
            if now % 5 == 0
                && let Some(old) = app.reload_profile()
            {
                apply_profile_change(&vr, &app, &old);
            }
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
                wrist,
                wrist_pose,
                head,
                celebrate_until.is_some(),
            ),
        }

        let pointer = vr
            .controller(role(other(hand)))
            .and_then(|i| poses.get(i as usize))
            .and_then(valid);
        let (pressed, changed) = match &input {
            Some(inp) => {
                let _ = vr.update_actions(inp.set);
                vr.digital(inp.select, inp.hand(other(hand))).unwrap_or_default()
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
                let s = panel.ui.global::<AppState>();
                s.set_cursor_visible(true);
                s.set_cursor_x(x);
                s.set_cursor_y(y);
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
            beam = vr.intersect(ov_watch, o, d).map(|(_, hit)| (o, hit));
        }
        let on_watch = !on_panel && beam.is_some();
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
            }
            let _ = vr.set_visible(ov_panel, panel_open);
            buzz.set(Some(Buzz::Press));
        }

        match (beam, head) {
            (Some((from, to)), Some(h)) => {
                let (m, len) = laser(from, to, position(h));
                let placed = vr.place(ov_laser, &m).and_then(|()| vr.set_width(ov_laser, len));
                if placed.is_ok() && !laser_on {
                    laser_on = vr.set_visible(ov_laser, true).is_ok();
                }
            }
            _ if laser_on => laser_on = vr.set_visible(ov_laser, false).is_err(),
            _ => {}
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
        std::thread::sleep(POLL.saturating_sub(loop_start.elapsed()));
    }

    if let Err(e) = app.store.apply(&tracker.finish()) {
        tracing::warn!("store: {e}");
    }
    vr.destroy_overlay(ov_watch);
    vr.destroy_overlay(ov_panel);
    vr.destroy_overlay(ov_laser);
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

/// GPU textures when Vulkan works (slot 0 watch, 1 panel, 2 laser), else SetOverlayRaw, which leaks
/// inside SteamVR and is only a fallback.
struct Textures(Option<Gpu>);

impl Textures {
    fn new(vr: &Vr) -> Self {
        let gpu = vr.gpu().and_then(|mut g| {
            g.add_texture(WATCH_PX.0, WATCH_PX.1)?;
            g.add_texture(PANEL_PX.0, PANEL_PX.1)?;
            g.add_texture(LASER_PX.0, LASER_PX.1)?;
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

fn place_watch(
    vr: &Vr,
    ov: u64,
    look: &mut Look,
    wrist: Option<u32>,
    wrist_pose: Option<&Mat>,
    head: Option<&Mat>,
    full: bool,
) {
    // Re-attach when SteamVR hands the role to another device (controller swap, restart).
    if wrist != look.attached_to {
        look.attached_to = wrist;
        if let Some(i) = wrist
            && let Err(e) = vr.attach(ov, i, &WATCH_OFFSET)
        {
            tracing::warn!("{e}");
        }
    }
    let state = match (wrist_pose, head) {
        (Some(w), Some(h)) => watch_look(w, h),
        _ => WatchLook::Hidden,
    };
    let alpha = match state {
        // Celebrating: full brightness wherever the wrist points.
        _ if full => 1.0,
        WatchLook::Hidden => 0.0,
        WatchLook::Facing => {
            let since = *look.facing_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= GLANCE_HOLD {
                1.0
            } else {
                ALPHA_AWAY
            }
        }
        WatchLook::Away => ALPHA_AWAY,
    };
    if state != WatchLook::Facing {
        look.facing_since = None;
    }
    let visible = alpha > 0.0;
    if visible != look.visible {
        look.visible = visible;
        let _ = vr.set_visible(ov, visible);
    }
    if (alpha - look.alpha).abs() > f32::EPSILON {
        look.alpha = alpha;
        let _ = vr.set_alpha(ov, alpha);
    }
}

fn apply_profile_change(vr: &Vr, app: &App, old: &fitvr_core::profile::Profile) {
    if old.auto_start != app.profile.auto_start
        && let Err(e) = vr.apps().set_auto_launch(setup::APP_KEY, app.profile.auto_start)
    {
        tracing::warn!("{e}");
    }
    if old.hr_device != app.profile.hr_device
        && let Hr::Live(b) = &app.hr
    {
        b.use_device(app.profile.hr_device.clone());
    }
}
