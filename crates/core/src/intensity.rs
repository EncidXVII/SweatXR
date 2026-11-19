//! Motion intensity: 1 s EMA of weighted head + hand linear speed, normalised to body height so
//! a tall player's longer reach does not read as more effort. Speeds are frame-invariant, so a
//! recentred or streamed tracking space (Steam Link, Frame) needs no special handling.

use crate::{PoseFrame, Vec3};

const W_HEAD: f32 = 0.3;
const W_HAND: f32 = 0.35;
const TAU_S: f32 = 1.0;
const REF_HEIGHT_M: f32 = 1.7;
/// Faster than any human limb: a tracking glitch or a recentre, not movement.
const MAX_SPEED: f32 = 12.0;
/// Longer gaps (pause, sleep, lost tracking) carry no usable speed.
const MAX_GAP_S: f64 = 0.5;

pub struct IntensityMeter {
    last: Option<PoseFrame>,
    ema: f32,
    height_scale: f32,
}

impl IntensityMeter {
    pub fn new(height_cm: f32) -> Self {
        Self {
            last: None,
            ema: 0.0,
            height_scale: height_scale(height_cm),
        }
    }

    /// The player edited their height: later frames are scaled by it, the running average stays.
    pub fn set_height(&mut self, height_cm: f32) {
        self.height_scale = height_scale(height_cm);
    }

    pub fn update(&mut self, f: &PoseFrame) -> f32 {
        if let Some(prev) = self.last.replace(*f) {
            let dt = f.t - prev.t;
            if dt > 0.0 && dt < MAX_GAP_S {
                let dt = dt as f32;
                let parts = [
                    (prev.head, f.head, W_HEAD),
                    (prev.left, f.left, W_HAND),
                    (prev.right, f.right, W_HAND),
                ];
                let (mut sum, mut weight) = (0.0, 0.0);
                for (a, b, w) in parts {
                    if let (Some(a), Some(b)) = (a, b) {
                        let v = dist(a, b) / dt;
                        if v < MAX_SPEED {
                            sum += w * v;
                            weight += w;
                        }
                    }
                }
                // Renormalise over the devices we saw so losing one controller does not halve the score.
                let raw = if weight > 0.0 {
                    sum / weight / self.height_scale
                } else {
                    0.0
                };
                self.ema += (1.0 - (-dt / TAU_S).exp()) * (raw - self.ema);
            }
        }
        self.ema
    }
}

fn height_scale(height_cm: f32) -> f32 {
    (height_cm / 100.0 / REF_HEIGHT_M).max(0.5)
}

fn dist(a: Vec3, b: Vec3) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}
