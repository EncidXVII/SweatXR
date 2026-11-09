//! OpenVR access plus the pure geometry of the watch and panel. Geometry is kept out of the FFI
//! module so it is testable without SteamVR.

mod ffi;

pub use ffi::{
    AppKind, Apps, ERR_NO_SERVER, EVENT_QUIT, EVENT_SCENE_CHANGED, Gpu, HMD, Handle, HmdMatrix34,
    MAX_DEVICES, ROLE_LEFT, ROLE_RIGHT, TrackedDevicePose, Vr, VrError, runtime_dir,
};

use fitvr_core::profile::Hand;
use fitvr_core::{PoseFrame, Vec3};

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

/// Watch relative to the controller's tip pose (-Z where it points, +Y up out of the back of the
/// hand), as on DesktopVR: behind the tip, facing up out of the controller. Like a real watch, text
/// runs along the forearm and its top points away from you when you raise the wrist across your
/// chest, on either hand. `aspect` is the texture's height over width: the top edge stays put
/// whatever the height, so a card that grows, grows toward you.
pub fn watch_offset(hand: Hand, aspect: f32) -> Mat {
    let s = if hand == Hand::Left { 1.0 } else { -1.0 };
    // Columns: text direction (texture x), text up (texture y), face normal.
    let (x, y, z) = ([0.0, 0.0, -s], [-s, 0.0, 0.0], [0.0, 1.0, 0.0]);
    let drop = WATCH_WIDTH_M * aspect * 0.5 - WATCH_TOP_M;
    let p = [-y[0] * drop, 0.04 - y[1] * drop, 0.16 - y[2] * drop];
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

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How visible the watch should be (0..1), as on DesktopVR: its face turned toward the eyes and
/// the head looking at it. Smooth, so the watch fades in and out with the turn of the wrist.
pub fn gaze_visibility(head: &Mat, watch: &Mat) -> f32 {
    let to_eye = sub(position(head), position(watch));
    if dot(to_eye, to_eye) < 1e-8 {
        return 1.0;
    }
    let to_eye = norm(to_eye);
    let facing = dot(axis(watch, 2), to_eye);
    let back = axis(head, 2);
    let looking = dot(back, to_eye);
    smoothstep(0.55, 0.7, facing) * smoothstep(0.8, 0.9, looking)
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

/// Lime beam fading in from the controller, straight RGBA.
pub fn laser_texture() -> Vec<u8> {
    let (w, h) = LASER_PX;
    (0..w * h)
        .flat_map(|i| {
            let t = (i % w) as f32 / (w - 1) as f32;
            [0xC6, 0xFF, 0x1A, (40.0 + 200.0 * t) as u8]
        })
        .collect()
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

    #[test]
    fn panel_faces_the_player() {
        let head = at([0.0, 1.7, 0.0]);
        let panel = panel_in_front(&head);
        let p = position(&panel);
        assert!((p[2] + PANEL_DISTANCE_M).abs() < 1e-5, "in front is -Z: {p:?}");
        let to_head = norm(sub(position(&head), p));
        assert!(dot(axis(&panel, 2), to_head) > 0.95);
    }

    /// Controller pointing along `along` (its -Z), back of the hand up.
    fn pointing(at_: Vec3, along: Vec3) -> Mat {
        let z = [-along[0], -along[1], -along[2]];
        let y = [0.0, 1.0, 0.0];
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
    /// the right hand to the left. Text runs along the arm, reading left to right, top away from
    /// the user (-Z), face up. Same rule on both hands.
    #[test]
    fn watch_reads_like_a_watch_on_both_hands() {
        for (hand, along) in [(Hand::Left, [1.0, 0.0, 0.0]), (Hand::Right, [-1.0, 0.0, 0.0])] {
            let ctrl = pointing([0.0, 1.2, -0.3], along);
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
        let face_up = pointing(watch_at, [1.0, 0.0, 0.0]);
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
        assert!(
            gaze_visibility(&head_at(position(&watch)), &watch) > 0.9,
            "looking at the face"
        );
        assert!(
            gaze_visibility(&head_at([0.0, 1.6, -2.0]), &watch) < 0.01,
            "looking ahead"
        );
        let palm_up = compose(
            &pointing(watch_at, [1.0, 0.0, 0.0]),
            &Mat {
                m: [[1.0, 0.0, 0.0, 0.0], [0.0, -1.0, 0.0, 0.0], [0.0, 0.0, -1.0, 0.0]],
            },
        );
        let turned = compose(&palm_up, &watch_offset(Hand::Left, 0.58));
        assert!(
            gaze_visibility(&head_at(position(&turned)), &turned) < 0.01,
            "face turned away"
        );
        assert!(near_face(&at([0.0, 1.65, -0.05]), &at(eye)));
    }

    #[test]
    fn inverse_undoes_a_transform() {
        let a = compose(
            &pointing([0.3, 1.1, -0.2], [0.6, 0.0, -0.8]),
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
    fn ray_points_forward() {
        let (o, d) = ray(&at([1.0, 1.0, 1.0]));
        assert_eq!((o, d), ([1.0, 1.0, 1.0], [0.0, 0.0, -1.0]));
        assert_eq!(uv_to_px([0.5, 1.0], 100, 50), (50.0, 0.0));
    }
}
