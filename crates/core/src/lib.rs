//! Energy model, sessions, storage and export. No VR, Bluetooth or UI dependencies, so every
//! rule here runs in plain unit tests against replayed traces.

pub mod energy;
pub mod export;
mod ffi;
pub mod intensity;
pub mod profile;
pub mod replay;
pub mod storage;
pub mod time;
pub mod tracker;

/// Positions in metres, tracking space. Orientation is not needed for the energy estimate.
pub type Vec3 = [f32; 3];

/// One tracking snapshot. A device is `None` while it is off or has lost tracking.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PoseFrame {
    pub t: f64,
    pub head: Option<Vec3>,
    pub left: Option<Vec3>,
    pub right: Option<Vec3>,
}

/// Live OpenVR or a replayed trace.
pub trait PoseSource {
    fn next_frame(&mut self) -> Option<PoseFrame>;
}
