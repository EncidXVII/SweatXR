// Copyright (C) 2026 EncidXVII
// SPDX-License-Identifier: GPL-3.0-only

//! OpenVR access plus the pure geometry of the watch and panel. Geometry is kept out of the FFI
//! module so it is testable without SteamVR.

mod ffi;

pub use ffi::{
    AppKind, Apps, ERR_NO_SERVER, EVENT_QUIT, EVENT_SCENE_CHANGED, Gpu, HMD, Handle, HmdMatrix34,
    MAX_DEVICES, ROLE_LEFT, ROLE_RIGHT, Server, TrackedDevicePose, Vr, VrError, config_dir, runtime_dir,
    terminate,
};

use sweatxr_core::profile::Hand;
use sweatxr_core::{PoseFrame, Vec3};

pub type Mat = HmdMatrix34;

pub fn position(m: &Mat) -> Vec3 {
    [m.m[0][3], m.m[1][3], m.m[2][3]]
}

/// Column `c` of the rotation: 0 = right, 1 = up, 2 = back (OpenVR looks down -Z).
pub fn axis(m: &Mat, c: usize) -> Vec3 {
    [m.m[0][c], m.m[1][c], m.m[2][c]]
}

pub fn valid(p: &TrackedDevicePose) -> Option<&Mat> {
    (p.pose_is_valid && p.device_is_connected).then_some(&p.device_to_absolute)
}

pub fn frame(t: f64, poses: &[TrackedDevicePose], left: Option<u32>, right: Option<u32>) -> PoseFrame {
    let at = |i: Option<u32>| {
        i.and_then(|i| poses.get(i as usize))
            .and_then(valid)
            .map(position)
    };
    PoseFrame {
        t,
        head: at(Some(HMD)),
        left: at(left),
        right: at(right),
    }
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(a: Vec3) -> Vec3 {
    let l = dot(a, a).sqrt().max(1e-6);
    [a[0] / l, a[1] / l, a[2] / l]
}

pub fn distance(a: Vec3, b: Vec3) -> f32 {
    let d = sub(a, b);
    dot(d, d).sqrt()
}

pub const WATCH_WIDTH_M: f32 = 0.14;
/// Distance from the watch's anchor point to its top edge.
const WATCH_TOP_M: f32 = 0.049;
pub const PANEL_WIDTH_M: f32 = 0.5;
pub const PANEL_DISTANCE_M: f32 = 0.72;
/// Controller this close to the face: the player is aiming or eating; get out of the way.
const NEAR_FACE_M: f32 = 0.15;

/// Watch relative to the controller's tip pose (-Z where it points, +Y its top, under the thumb), as
/// on WristXR: behind the tip, on the back of the hand (left: -X, right: +X) and facing out of
/// it, where a real watch sits. On the controller's top it faced your chest when you turned the
/// wrist to read it. Like a real watch, text runs along the forearm and its top points away from
/// you when you raise the wrist across your chest, palm down, on either hand. `aspect` is the
/// texture's height over width: the top edge stays put whatever the height, so a card that grows,
/// grows toward you.
pub fn watch_offset(hand: Hand, aspect: f32) -> Mat {
    let s = if hand == Hand::Left { 1.0 } else { -1.0 };
    // Columns: text direction (texture x), text up (texture y), face normal.
    let (x, y, z) = ([0.0, 0.0, -s], [0.0, -1.0, 0.0], [-s, 0.0, 0.0]);
    let drop = WATCH_WIDTH_M * aspect * 0.5 - WATCH_TOP_M;
    let p = [-0.04 * s - y[0] * drop, -y[1] * drop, 0.16 - y[2] * drop];
    Mat {
        m: [
            [x[0], y[0], z[0], p[0]],
            [x[1], y[1], z[1], p[1]],
            [x[2], y[2], z[2], p[2]],
        ],
    }
}

fn mul(a: &Mat, b: &Mat) -> Mat {
    let mut m = Mat::default();
    for r in 0..3 {
        for c in 0..4 {
            m.m[r][c] =
                (0..3).map(|k| a.m[r][k] * b.m[k][c]).sum::<f32>() + if c == 3 { a.m[r][3] } else { 0.0 };
        }
    }
    m
}

/// Inverse of a rigid transform.
pub fn inverse(a: &Mat) -> Mat {
    let mut m = Mat::default();
    for r in 0..3 {
        for c in 0..3 {
            m.m[r][c] = a.m[c][r];
        }
        m.m[r][3] = -(0..3).map(|k| a.m[k][r] * a.m[k][3]).sum::<f32>();
    }
    m
}

/// `a` then `b`: the transform `b` expressed under `a`.
pub fn compose(a: &Mat, b: &Mat) -> Mat {
    mul(a, b)
}

/// However it is moved, the watch stays within this of its wrist's tip: slid along the forearm and tilted, never
/// left in the air out of a glance's reach. Room for a second watch up the arm from WristXR's.
pub const WATCH_REACH_M: f32 = 0.4;

/// A pose from a position and a unit quaternion (x, y, z, w), as WristXR saves its panels.
pub fn pose_mat(p: Vec3, q: [f32; 4]) -> Mat {
    let [x, y, z, w] = q;
    Mat {
        m: [
            [
                1.0 - 2.0 * (y * y + z * z),
                2.0 * (x * y - z * w),
                2.0 * (x * z + y * w),
                p[0],
            ],
            [
                2.0 * (x * y + z * w),
                1.0 - 2.0 * (x * x + z * z),
                2.0 * (y * z - x * w),
                p[1],
            ],
            [
                2.0 * (x * z - y * w),
                2.0 * (y * z + x * w),
                1.0 - 2.0 * (x * x + y * y),
                p[2],
            ],
        ],
    }
}

/// Another watch on the same wrist (WristXR's): its pose relative to the controller's tip, its width and its
/// height over width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Neighbour {
    pub pose: Mat,
    pub width_m: f32,
    pub aspect: f32,
}

/// The spot beside another watch on the wrist: turned as it is, top edges level, a centimetre apart along the
/// line its text runs on, on the side further up the arm (away from the controller's tip). `aspect` is our
/// texture's height over width.
pub fn beside(n: &Neighbour, aspect: f32) -> Mat {
    let (x, y, p) = (axis(&n.pose, 0), axis(&n.pose, 1), position(&n.pose));
    let step = (n.width_m + WATCH_WIDTH_M) / 2.0 + 0.01;
    let rise = (n.width_m * n.aspect - WATCH_WIDTH_M * aspect) / 2.0;
    let at = |side: f32| {
        [
            p[0] + x[0] * side * step + y[0] * rise,
            p[1] + x[1] * side * step + y[1] * rise,
            p[2] + x[2] * side * step + y[2] * rise,
        ]
    };
    let (a, b) = (at(1.0), at(-1.0));
    let c = if dot(a, a) >= dot(b, b) { a } else { b };
    let mut m = n.pose;
    (m.m[0][3], m.m[1][3], m.m[2][3]) = (c[0], c[1], c[2]);
    m
}

/// Gripping the watch with the pointing hand: the watch as that hand's ray sees it. Kept while the grip holds, so
/// the watch follows the ray rigidly, as a gripped watch does on WristXR: move the hand to slide it, turn the hand
/// to tilt it.
pub fn grip(ray: &Mat, watch: &Mat) -> Mat {
    compose(&inverse(ray), watch)
}

/// Where a gripped watch sits now, relative to its wrist's tip pose (what the profile keeps), within
/// `WATCH_REACH_M` of that tip.
pub fn gripped_on_wrist(ray: &Mat, grip: &Mat, wrist_tip: &Mat) -> Mat {
    let mut m = compose(&inverse(wrist_tip), &compose(ray, grip));
    let p = position(&m);
    let far = dot(p, p).sqrt();
    if far > WATCH_REACH_M {
        let k = WATCH_REACH_M / far;
        (m.m[0][3], m.m[1][3], m.m[2][3]) = (p[0] * k, p[1] * k, p[2] * k);
    }
    m
}

/// A watch let go within 2 cm and 15 degrees of its default spot is back on it.
pub fn at_default(spot: &Mat, default: &Mat) -> bool {
    distance(position(spot), position(default)) < 0.02
        && dot(axis(spot, 0), axis(default, 0)) > 0.966
        && dot(axis(spot, 2), axis(default, 2)) > 0.966
}

/// Whether the watch is looked at, as on WristXR: its face turned toward the eyes and the eyes
/// on it. Wide to show (a glance from the corner of the eye is enough), wider still to stay, and
/// held for at least `GLANCE_HOLD_S`, so hand tremor and head bob at the edge never make it blink.
#[derive(Clone, Copy, Debug, Default)]
pub struct Glance {
    shown: bool,
    held_s: f32,
}

/// Shows when the wrist is turned to within about 50 degrees of the eyes, the face within about 66
/// degrees of them and the watch within 52 degrees of where the head looks; stays until those pass
/// about 61, 81 and 70 degrees. The wrist's turn is its roll around the forearm alone; the face and
/// look angles are what they always were, so only the wrist has to turn less far. As WristXR.
/// (roll, facing, looking), cosines.
const GLANCE_SHOW: (f32, f32, f32) = (0.64, 0.4, 0.62);
const GLANCE_KEEP: (f32, f32, f32) = (0.48, 0.15, 0.34);
const GLANCE_HOLD_S: f32 = 0.3;

impl Glance {
    /// Steps by `dt` seconds; true while the watch is looked at.
    pub fn update(&mut self, head: &Mat, watch: &Mat, dt: f32) -> bool {
        let to_eye = sub(position(head), position(watch));
        let (roll, facing, looking) = if dot(to_eye, to_eye) < 1e-8 {
            (1.0, 1.0, 1.0)
        } else {
            let to_eye = norm(to_eye);
            // Around the forearm (the text runs along it); looking straight down it, the face angle.
            let arm = axis(watch, 0);
            let along = dot(to_eye, arm);
            let across = sub(to_eye, [arm[0] * along, arm[1] * along, arm[2] * along]);
            let toward = if dot(across, across) > 0.04 {
                norm(across)
            } else {
                to_eye
            };
            let normal = axis(watch, 2);
            (
                dot(normal, toward),
                dot(normal, to_eye),
                dot(axis(head, 2), to_eye),
            )
        };
        self.held_s += dt;
        let (r, f, l) = if self.shown { GLANCE_KEEP } else { GLANCE_SHOW };
        let want = roll > r && facing > f && looking > l;
        if want != self.shown && (want || self.held_s >= GLANCE_HOLD_S) {
            self.shown = want;
            self.held_s = 0.0;
        }
        self.shown
    }
}

/// The controller is at the face: the watch hides.
pub fn near_face(controller: &Mat, head: &Mat) -> bool {
    distance(position(controller), position(head)) < NEAR_FACE_M
}

/// Panel in front of the head at eye height minus a bit, upright, facing the player.
pub fn panel_in_front(head: &Mat) -> Mat {
    let back = axis(head, 2);
    let flat = norm([back[0], 0.0, back[2]]);
    let p = position(head);
    let center = [
        p[0] - flat[0] * PANEL_DISTANCE_M,
        p[1] - 0.1,
        p[2] - flat[2] * PANEL_DISTANCE_M,
    ];
    let up = [0.0, 1.0, 0.0];
    let right = cross(up, flat);
    Mat {
        m: [
            [right[0], up[0], flat[0], center[0]],
            [right[1], up[1], flat[1], center[1]],
            [right[2], up[2], flat[2], center[2]],
        ],
    }
}

/// Pointer ray of a controller: origin and forward (-Z).
pub fn ray(controller: &Mat) -> (Vec3, Vec3) {
    let z = axis(controller, 2);
    (position(controller), [-z[0], -z[1], -z[2]])
}

/// Laser texture: a strip this many texels long and 2 high. SteamVR keeps texel aspect, so at
/// a 0.7 m reach the beam is 0.7 / 256 m = 2.7 mm thick.
pub const LASER_PX: (u32, u32) = (512, 2);

/// Beam in the accent colour fading in from the controller, straight RGBA.
pub fn laser_texture(rgb: [u8; 3]) -> Vec<u8> {
    let (w, h) = LASER_PX;
    (0..w * h)
        .flat_map(|i| {
            let t = (i % w) as f32 / (w - 1) as f32;
            [rgb[0], rgb[1], rgb[2], (40.0 + 200.0 * t) as u8]
        })
        .collect()
}

/// The pointer dot: its own overlay, moved in the same frame as the beam, so the two never drift
/// apart the way a dot drawn into the panel's picture does (that one shows a frame or more late).
pub const DOT_PX: u32 = 32;
/// About the size of the old in-picture dot on the panel (16 of its 800 px).
pub const DOT_WIDTH_M: f32 = 0.011;

/// Accent disc with a dark rim, antialiased, straight RGBA.
pub fn dot_texture(rgb: [u8; 3]) -> Vec<u8> {
    let c = (DOT_PX as f32 - 1.0) / 2.0;
    (0..DOT_PX * DOT_PX)
        .flat_map(|i| {
            let (x, y) = ((i % DOT_PX) as f32 - c, (i / DOT_PX) as f32 - c);
            let r = (x * x + y * y).sqrt();
            let alpha = (c + 0.5 - r).clamp(0.0, 1.0);
            // Rim: the outer eighth, in the background grey, so the dot reads on a lit control.
            let rim = (r - c * 0.75).clamp(0.0, 1.0);
            let mix = |a: u8, b: u8| (f32::from(a) * (1.0 - rim) + f32::from(b) * rim) as u8;
            [
                mix(rgb[0], 0x0B),
                mix(rgb[1], 0x0B),
                mix(rgb[2], 0x0C),
                (alpha * 255.0) as u8,
            ]
        })
        .collect()
}

/// The dot at `at`, turned to face the eye and lifted 2 mm toward it so it sits on top of what it
/// points at.
pub fn dot_pose(at: Vec3, eye: Vec3) -> Mat {
    let z = norm(sub(eye, at));
    let up = if z[1].abs() > 0.99 {
        [0.0, 0.0, 1.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let x = norm(cross(up, z));
    let y = cross(z, x);
    let p = [at[0] + z[0] * 0.002, at[1] + z[1] * 0.002, at[2] + z[2] * 0.002];
    Mat {
        m: [
            [x[0], y[0], z[0], p[0]],
            [x[1], y[1], z[1], p[1]],
            [x[2], y[2], z[2], p[2]],
        ],
    }
}

/// Overlay transform for a beam from `from` to `to`, its flat side turned toward the eyes.
/// Returns the transform and the beam length (the overlay width).
pub fn laser(from: Vec3, to: Vec3, eye: Vec3) -> (Mat, f32) {
    let len = distance(from, to).max(1e-3);
    let x = norm(sub(to, from));
    let mid = [
        (from[0] + to[0]) / 2.0,
        (from[1] + to[1]) / 2.0,
        (from[2] + to[2]) / 2.0,
    ];
    let view = sub(eye, mid);
    // Normal: the part of the view direction perpendicular to the beam.
    let z = norm(sub(
        view,
        [x[0] * dot(view, x), x[1] * dot(view, x), x[2] * dot(view, x)],
    ));
    let y = cross(z, x);
    let m = Mat {
        m: [
            [x[0], y[0], z[0], mid[0]],
            [x[1], y[1], z[1], mid[1]],
            [x[2], y[2], z[2], mid[2]],
        ],
    };
    (m, len)
}

/// Overlay UV (origin bottom-left) to texture pixels (origin top-left).
pub fn uv_to_px(uv: [f32; 2], width: u32, height: u32) -> (f32, f32) {
    (uv[0] * width as f32, (1.0 - uv[1]) * height as f32)
}

/// Where a ray (from `o` along the unit `d`) meets a flat overlay `width_m` wide and `height_m` high whose middle
/// is at `pose` (x to its right, y up, z out of its face): the point on its picture, 0..1 from the top-left corner,
/// and in the room. None off the overlay, behind the ray's start or edge-on. Our own arithmetic on the poses we
/// placed the overlays with, as on WristXR: the beam, the dot and the control under them all come from one ray
/// and one answer, whatever moment SteamVR would have measured at.
pub fn hit(pose: &Mat, width_m: f32, height_m: f32, o: Vec3, d: Vec3) -> Option<([f32; 2], Vec3)> {
    let rel = sub(o, position(pose));
    let (x, y, z) = (axis(pose, 0), axis(pose, 1), axis(pose, 2));
    let (lo, ld) = (
        [dot(rel, x), dot(rel, y), dot(rel, z)],
        [dot(d, x), dot(d, y), dot(d, z)],
    );
    if ld[2].abs() < 1e-6 {
        return None;
    }
    let t = -lo[2] / ld[2];
    let uv = [
        (lo[0] + ld[0] * t) / width_m + 0.5,
        0.5 - (lo[1] + ld[1] * t) / height_m,
    ];
    let inside = t > 0.0 && (0.0..=1.0).contains(&uv[0]) && (0.0..=1.0).contains(&uv[1]);
    inside.then_some((uv, [o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t]))
}

/// Ray smoothing, WristXR's: a hand that turns less than this in a frame (tremor, tracking noise) is smoothed,
/// a faster one passes straight through, so aiming never lags.
pub const SMOOTH_FULL_RAD: f32 = 0.02;
/// The steadiest hand's share of the new ray each frame.
pub const SMOOTH_MIN: f32 = 0.25;

/// The ray to use this frame, from last frame's and the controller's new pose.
pub fn smooth(prev: &Mat, raw: &Mat) -> Mat {
    let turn = dot(axis(prev, 2), axis(raw, 2)).clamp(-1.0, 1.0).acos();
    let moved = distance(position(prev), position(raw));
    let t = (turn / SMOOTH_FULL_RAD).max(moved / 0.02).clamp(SMOOTH_MIN, 1.0);
    let mix = |a: Vec3, b: Vec3| {
        [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ]
    };
    let z = norm(mix(axis(prev, 2), axis(raw, 2)));
    let x = norm(cross(mix(axis(prev, 1), axis(raw, 1)), z));
    let y = cross(z, x);
    let p = mix(position(prev), position(raw));
    Mat {
        m: [
            [x[0], y[0], z[0], p[0]],
            [x[1], y[1], z[1], p[1]],
            [x[2], y[2], z[2], p[2]],
        ],
    }
}

/// An upright panel with its middle at `center`, turned to face `eye` (level: it never tips).
pub fn facing(center: Vec3, eye: Vec3) -> Mat {
    let to_eye = sub(eye, center);
    let flat = if to_eye[0].abs() + to_eye[2].abs() < 1e-5 {
        [0.0, 0.0, 1.0]
    } else {
        norm([to_eye[0], 0.0, to_eye[2]])
    };
    let up = [0.0, 1.0, 0.0];
    let right = cross(up, flat);
    Mat {
        m: [
            [right[0], up[0], flat[0], center[0]],
            [right[1], up[1], flat[1], center[1]],
            [right[2], up[2], flat[2], center[2]],
        ],
    }
}

/// A gripped panel, as a held one behaves on WristXR: the gripped spot (`local`: metres right and up of its
/// middle) stays under the ray at `reach` along it, and the panel turns to face `eye` from where its middle was
/// (`was`), so a grip near an edge never skews it.
pub fn carried(was: &Mat, local: [f32; 2], o: Vec3, d: Vec3, reach: f32, eye: Vec3) -> Mat {
    let mut m = facing(position(was), eye);
    let (x, y) = (axis(&m, 0), axis(&m, 1));
    for i in 0..3 {
        m.m[i][3] = o[i] + d[i] * reach - x[i] * local[0] - y[i] * local[1];
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: Mat = Mat {
        m: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
    };

    fn at(p: Vec3) -> Mat {
        let mut m = IDENTITY;
        (m.m[0][3], m.m[1][3], m.m[2][3]) = (p[0], p[1], p[2]);
        m
    }

    fn close(a: &Mat, b: &Mat) -> bool {
        (0..3).all(|r| (0..4).all(|c| (a.m[r][c] - b.m[r][c]).abs() < 1e-4))
    }

    #[test]
    fn a_gripped_watch_follows_the_pointing_hand_and_stays_on_its_wrist() {
        let aspect = 0.5;
        let tip = pointing([-0.2, 1.1, -0.3], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
        let spot = watch_offset(Hand::Left, aspect);
        let watch = compose(&tip, &spot);
        // The other hand, to the right of the wrist and pointing left at it.
        let ray = pointing([0.2, 1.1, -0.14], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let held = grip(&ray, &watch);
        // Nothing moved: it is where it was.
        assert!(close(&gripped_on_wrist(&ray, &held, &tip), &spot));
        assert!(at_default(&gripped_on_wrist(&ray, &held, &tip), &spot));
        // The pointing hand moves 5 cm up the forearm (the wrist's +Z): so does the watch, turned as it was.
        let mut moved_ray = ray;
        moved_ray.m[2][3] += 0.05;
        let moved = gripped_on_wrist(&moved_ray, &held, &tip);
        assert!(
            (position(&moved)[2] - position(&spot)[2] - 0.05).abs() < 1e-4,
            "{:?}",
            position(&moved)
        );
        assert!(dot(axis(&moved, 2), axis(&spot, 2)) > 0.999 && !at_default(&moved, &spot));
        // The wrist moves with the watch on it: the watch keeps its place on the wrist.
        let mut tip_later = tip;
        tip_later.m[0][3] += 0.4;
        let mut ray_later = moved_ray;
        ray_later.m[0][3] += 0.4;
        assert!(close(&gripped_on_wrist(&ray_later, &held, &tip_later), &moved));
        // Carried a metre off, it stays within reach of the wrist.
        let mut far_ray = ray;
        far_ray.m[1][3] += 1.0;
        let far = position(&gripped_on_wrist(&far_ray, &held, &tip));
        assert!((dot(far, far).sqrt() - WATCH_REACH_M).abs() < 1e-4);
    }

    /// Quaternion (x, y, z, w) of a turn about an axis, and the product `a` after `b`.
    fn turn(axis_: Vec3, angle: f32) -> [f32; 4] {
        let s = (angle / 2.0).sin();
        [axis_[0] * s, axis_[1] * s, axis_[2] * s, (angle / 2.0).cos()]
    }
    fn after(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
        let ([ax, ay, az, aw], [bx, by, bz, bw]) = (a, b);
        [
            aw * bx + ax * bw + ay * bz - az * by,
            aw * by - ax * bz + ay * bw + az * bx,
            aw * bz + ax * by - ay * bx + az * bw,
            aw * bw - ax * bx - ay * by - az * bz,
        ]
    }

    #[test]
    fn the_ray_finds_the_point_on_the_picture_it_shows() {
        // A panel 0.5 m wide and 0.25 high, 1 m ahead, facing back at us (+Z).
        let panel = at([0.0, 1.5, -1.0]);
        let (o, ahead) = ([0.0, 1.5, 0.0], [0.0, 0.0, -1.0]);
        let (uv, p) = hit(&panel, 0.5, 0.25, o, ahead).expect("the middle");
        assert!((uv[0] - 0.5).abs() < 1e-5 && (uv[1] - 0.5).abs() < 1e-5 && (p[2] + 1.0).abs() < 1e-5);
        // Up and to the right of the middle is right of and above it on the picture (v runs down from the top).
        let (uv, p) = hit(&panel, 0.5, 0.25, [0.1, 1.55, 0.0], ahead).expect("upper right");
        assert!((uv[0] - 0.7).abs() < 1e-5 && (uv[1] - 0.3).abs() < 1e-5, "{uv:?}");
        assert!((p[0] - 0.1).abs() < 1e-5 && (p[1] - 1.55).abs() < 1e-5);
        // Past an edge, pointing away, or along the panel: nothing.
        assert!(hit(&panel, 0.5, 0.25, [0.3, 1.5, 0.0], ahead).is_none());
        assert!(hit(&panel, 0.5, 0.25, o, [0.0, 0.0, 1.0]).is_none());
        assert!(hit(&panel, 0.5, 0.25, o, [1.0, 0.0, 0.0]).is_none());
    }

    #[test]
    fn a_steady_hand_is_smoothed_and_a_fast_one_is_not() {
        let prev = pointing([0.0, 1.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
        // A tremor: 1 mm and a few thousandths of a radian. A quarter of the way there.
        let mut near = prev;
        near.m[0][3] += 0.001;
        let s = smooth(&prev, &near);
        assert!((s.m[0][3] - 0.00025).abs() < 1e-6, "{}", s.m[0][3]);
        // A real move: 10 cm and a quarter turn. Straight through, and still a rigid pose.
        let far = pointing([0.1, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let s = smooth(&prev, &far);
        assert!(close(&s, &far));
        assert!(
            (dot(axis(&s, 0), axis(&s, 1))).abs() < 1e-5
                && (dot(axis(&s, 2), axis(&s, 2)) - 1.0).abs() < 1e-5
        );
    }

    #[test]
    fn a_carried_panel_keeps_the_gripped_spot_under_the_ray_and_faces_you() {
        let eye = [0.0, 1.6, 0.0];
        let was = facing([0.0, 1.5, -0.7], eye);
        // Gripped 10 cm right of and 5 cm above its middle; the hand then swings the ray to the right.
        let (o, d, reach) = ([0.2, 1.2, 0.0], norm([0.6, 0.3, -0.7]), 0.9);
        let now = carried(&was, [0.1, 0.05], o, d, reach, eye);
        let spot = [o[0] + d[0] * reach, o[1] + d[1] * reach, o[2] + d[2] * reach];
        let (uv, p) = hit(&now, 0.5, 0.375, o, d).expect("still under the ray");
        assert!(distance(p, spot) < 1e-4);
        assert!(
            (uv[0] - 0.7).abs() < 1e-3 && (uv[1] - (0.5 - 0.05 / 0.375)).abs() < 1e-3,
            "{uv:?}"
        );
        // Upright, its face toward the eyes (as seen from where it was: it finishes the turn over the next frames).
        assert!((axis(&now, 1)[1] - 1.0).abs() < 1e-5);
        assert!(dot(axis(&now, 2), norm(sub(eye, position(&now)))) > 0.5);
        let settled = carried(&now, [0.1, 0.05], o, d, reach, eye);
        assert!(dot(axis(&settled, 2), norm(sub(eye, position(&settled)))) > 0.9);
    }

    #[test]
    fn wristxr_saves_its_watch_the_way_ours_is_placed() {
        // WristXR's default left-hand watch (Panel::on_wrist): a quarter turn about Z, face up, a quarter turn
        // about Z again; 4 cm to the back of the hand, 16 cm behind the tip, its top edge 4.9 cm from the anchor.
        use std::f32::consts::FRAC_PI_2;
        let aspect = 0.5;
        let to_back = turn([0.0, 0.0, 1.0], FRAC_PI_2);
        let q = after(
            to_back,
            after(
                turn([1.0, 0.0, 0.0], -FRAC_PI_2),
                turn([0.0, 0.0, 1.0], FRAC_PI_2),
            ),
        );
        let ours = watch_offset(Hand::Left, aspect);
        let theirs = pose_mat(position(&ours), q);
        assert!(close(&theirs, &ours), "{theirs:?} vs {ours:?}");
    }

    #[test]
    fn our_watch_sits_beside_another_up_the_arm_with_level_tops() {
        // WristXR's watch on its default spot, taller than ours (open).
        let theirs = Neighbour {
            pose: watch_offset(Hand::Left, 1.5),
            width_m: 0.14,
            aspect: 1.5,
        };
        let ours = beside(&theirs, 0.5);
        let (p, q) = (position(&ours), position(&theirs.pose));
        // 15 cm further behind the tip (+Z: up the arm), turned the same way.
        assert!((p[2] - q[2] - 0.15).abs() < 1e-4, "{p:?} vs {q:?}");
        assert!(dot(axis(&ours, 2), axis(&theirs.pose, 2)) > 0.999);
        // Top edges level: each centre plus half its height along its up.
        let top = |m: &Mat, h: f32| dot(position(m), axis(m, 1)) + h / 2.0;
        assert!((top(&ours, 0.14 * 0.5) - top(&theirs.pose, 0.14 * 1.5)).abs() < 1e-4);
        // On the right hand the text runs the other way: still up the arm.
        let right = Neighbour {
            pose: watch_offset(Hand::Right, 0.5),
            width_m: 0.14,
            aspect: 0.5,
        };
        assert!(position(&beside(&right, 0.5))[2] > position(&right.pose)[2] + 0.1);
    }

    #[test]
    fn panel_faces_the_player() {
        let head = at([0.0, 1.7, 0.0]);
        let panel = panel_in_front(&head);
        let p = position(&panel);
        assert!((p[2] + PANEL_DISTANCE_M).abs() < 1e-5, "in front is -Z: {p:?}");
        let to_head = norm(sub(position(&head), p));
        assert!(dot(axis(&panel, 2), to_head) > 0.95);
    }

    /// Controller pointing along `along` (its -Z) with its top (+Y, under the thumb) toward `top`.
    fn pointing(at_: Vec3, along: Vec3, top: Vec3) -> Mat {
        let z = [-along[0], -along[1], -along[2]];
        let y = top;
        let x = cross(y, z);
        Mat {
            m: [
                [x[0], y[0], z[0], at_[0]],
                [x[1], y[1], z[1], at_[1]],
                [x[2], y[2], z[2], at_[2]],
            ],
        }
    }

    /// Wrist raised across the chest, palm down: the left hand points to the user's right (+X),
    /// the right hand to the left, the controller's top (under the thumb) toward the user and the
    /// back of the hand up. Text runs along the arm, reading left to right, top away from the user
    /// (-Z), face up. Same rule on both hands.
    #[test]
    fn watch_reads_like_a_watch_on_both_hands() {
        for (hand, along) in [(Hand::Left, [1.0, 0.0, 0.0]), (Hand::Right, [-1.0, 0.0, 0.0])] {
            let ctrl = pointing([0.0, 1.2, -0.3], along, [0.0, 0.0, 1.0]);
            let w = compose(&ctrl, &watch_offset(hand, 0.58));
            assert!(
                dot(axis(&w, 0), [1.0, 0.0, 0.0]) > 0.99,
                "{hand:?}: text runs {:?}",
                axis(&w, 0)
            );
            assert!(
                dot(axis(&w, 1), [0.0, 0.0, -1.0]) > 0.99,
                "{hand:?}: top points {:?}",
                axis(&w, 1)
            );
            assert!(
                dot(axis(&w, 2), [0.0, 1.0, 0.0]) > 0.99,
                "{hand:?}: faces {:?}",
                axis(&w, 2)
            );
        }
    }

    #[test]
    fn watch_top_edge_stays_put_when_it_grows() {
        let top = |aspect: f32| {
            let m = watch_offset(Hand::Left, aspect);
            let half = WATCH_WIDTH_M * aspect * 0.5;
            let y = axis(&m, 1);
            [
                m.m[0][3] + y[0] * half,
                m.m[1][3] + y[1] * half,
                m.m[2][3] + y[2] * half,
            ]
        };
        assert!(distance(top(0.41), top(0.58)) < 1e-5);
    }

    #[test]
    fn watch_shows_only_when_looked_at() {
        let eye = [0.0, 1.6, 0.0];
        let watch_at = [0.0, 1.2, -0.3];
        let face_up = pointing(watch_at, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
        let watch = compose(&face_up, &watch_offset(Hand::Left, 0.58));
        // Head with its -Z on the target, pitched as needed.
        let head_at = |target: Vec3| {
            let f = norm(sub(target, eye));
            let z = [-f[0], -f[1], -f[2]];
            let x = norm(cross([0.0, 1.0, 0.0], z));
            let y = cross(z, x);
            Mat {
                m: [
                    [x[0], y[0], z[0], eye[0]],
                    [x[1], y[1], z[1], eye[1]],
                    [x[2], y[2], z[2], eye[2]],
                ],
            }
        };
        let mut g = Glance::default();
        assert!(
            !g.update(&head_at([0.0, 1.6, -2.0]), &watch, 0.016),
            "looking ahead"
        );
        assert!(
            g.update(&head_at(position(&watch)), &watch, 0.016),
            "looking at the face"
        );
        let away = head_at([0.0, 2.6, -1.0]);
        assert!(g.update(&away, &watch, 0.1), "held, never a blink");
        assert!(!g.update(&away, &watch, 0.3), "then gone");
        // Rolled half a turn around the forearm.
        let palm_up = compose(
            &face_up,
            &Mat {
                m: [[-1.0, 0.0, 0.0, 0.0], [0.0, -1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
            },
        );
        let turned = compose(&palm_up, &watch_offset(Hand::Left, 0.58));
        assert!(
            !Glance::default().update(&head_at(position(&turned)), &turned, 0.016),
            "face turned away"
        );
        assert!(near_face(&at([0.0, 1.65, -0.05]), &at(eye)));
    }

    #[test]
    fn inverse_undoes_a_transform() {
        let a = compose(
            &pointing([0.3, 1.1, -0.2], [0.6, 0.0, -0.8], [0.0, 1.0, 0.0]),
            &watch_offset(Hand::Right, 0.5),
        );
        let back = compose(&inverse(&a), &a);
        for r in 0..3 {
            for c in 0..4 {
                let want = if r == c { 1.0 } else { 0.0 };
                assert!((back.m[r][c] - want).abs() < 1e-5, "{back:?}");
            }
        }
    }

    #[test]
    fn laser_spans_the_ray_and_faces_the_eye() {
        let (m, len) = laser([0.0, 1.0, 0.0], [0.0, 1.0, -1.0], [0.0, 1.6, 0.2]);
        assert!((len - 1.0).abs() < 1e-5);
        assert_eq!(position(&m), [0.0, 1.0, -0.5]);
        assert!(
            dot(axis(&m, 0), [0.0, 0.0, -1.0]) > 0.999,
            "beam axis along the ray"
        );
        assert!(axis(&m, 2)[1] > 0.9, "flat side up toward the eye above it");
    }

    #[test]
    fn dot_faces_the_eye_just_in_front_of_the_hit() {
        let m = dot_pose([0.0, 1.0, -1.0], [0.0, 1.0, 0.0]);
        assert!(dot(axis(&m, 2), [0.0, 0.0, 1.0]) > 0.999, "faces the eye");
        assert!((position(&m)[2] + 0.998).abs() < 1e-5, "2 mm toward the eye");
        assert!(dot(axis(&m, 1), [0.0, 1.0, 0.0]) > 0.999, "upright");
        let px = dot_texture([0xC6, 0xFF, 0x1A]);
        let at = |x: u32, y: u32| px[((y * DOT_PX + x) * 4) as usize..][..4].to_vec();
        assert_eq!(at(16, 16), [0xC6, 0xFF, 0x1A, 255], "accent centre");
        assert_eq!(at(0, 0)[3], 0, "clear corner");
    }

    #[test]
    fn ray_points_forward() {
        let (o, d) = ray(&at([1.0, 1.0, 1.0]));
        assert_eq!((o, d), ([1.0, 1.0, 1.0], [0.0, 0.0, -1.0]));
        assert_eq!(uv_to_px([0.5, 1.0], 100, 50), (50.0, 0.0));
    }
}
