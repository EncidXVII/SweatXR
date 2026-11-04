//! OpenVR access plus the pure geometry of the watch and panel. Geometry is kept out of the FFI
//! module so it is testable without SteamVR.

mod ffi;

pub use ffi::{
    AppKind, EVENT_QUIT, EVENT_SCENE_CHANGED, Gpu, HMD, Handle, HmdMatrix34, MAX_DEVICES, ROLE_LEFT,
    ROLE_RIGHT, TrackedDevicePose, Vr, VrError, runtime_dir,
};

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

/// Watch transform relative to its controller: on top of the wrist, face up, 12 o'clock
/// toward the fingers. Same for both hands because controller space is mirrored by SteamVR.
pub const WATCH_OFFSET: Mat = Mat {
    m: [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.02],
        [0.0, -1.0, 0.0, 0.12],
    ],
};
pub const WATCH_WIDTH_M: f32 = 0.08;
pub const PANEL_WIDTH_M: f32 = 0.8;
pub const PANEL_DISTANCE_M: f32 = 0.7;
/// Wrist turned toward the eyes: angle between watch normal and the line to the head < ~45 deg.
const GLANCE_COS: f32 = 0.7;
/// Controller this close to the face: the player is aiming or eating; get out of the way.
const NEAR_FACE_M: f32 = 0.15;

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

/// What the watch should do this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WatchLook {
    Hidden,
    /// Facing the eyes: full opacity once held past the glance delay.
    Facing,
    Away,
}

pub fn watch_look(controller: &Mat, head: &Mat) -> WatchLook {
    let watch = mul(controller, &WATCH_OFFSET);
    let to_head = norm(sub(position(head), position(&watch)));
    if distance(position(controller), position(head)) < NEAR_FACE_M {
        WatchLook::Hidden
    } else if dot(axis(&watch, 2), to_head) > GLANCE_COS {
        WatchLook::Facing
    } else {
        WatchLook::Away
    }
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

    #[test]
    fn watch_turned_up_under_the_eyes_is_facing() {
        // Hand held level 40 cm below and in front of the eyes: watch face points up at them.
        let head = at([0.0, 1.7, 0.0]);
        assert_eq!(watch_look(&at([0.0, 1.3, -0.25]), &head), WatchLook::Facing);
        // Same hand rolled 180 degrees: face points at the floor.
        let mut rolled = at([0.0, 1.3, -0.25]);
        (rolled.m[0][0], rolled.m[1][1]) = (-1.0, -1.0);
        assert_eq!(watch_look(&rolled, &head), WatchLook::Away);
        assert_eq!(watch_look(&at([0.0, 1.65, -0.05]), &head), WatchLook::Hidden);
    }

    #[test]
    fn ray_points_forward() {
        let (o, d) = ray(&at([1.0, 1.0, 1.0]));
        assert_eq!((o, d), ([1.0, 1.0, 1.0], [0.0, 0.0, -1.0]));
        assert_eq!(uv_to_px([0.5, 1.0], 100, 50), (50.0, 0.0));
    }
}
