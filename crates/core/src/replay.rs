// Copyright (C) 2026 EncidXVII
// SPDX-License-Identifier: GPL-3.0-only

//! Recorded and synthetic tracking traces. They stand in for a headset in every automated test.

use crate::{PoseFrame, PoseSource, Vec3};

pub struct Replay {
    frames: std::vec::IntoIter<PoseFrame>,
}

impl Replay {
    pub fn new(frames: Vec<PoseFrame>) -> Self {
        Self {
            frames: frames.into_iter(),
        }
    }

    /// Format written by `to_csv` (and by `sweatxr --overlay --record`).
    pub fn from_csv(text: &str) -> Option<Self> {
        let mut frames = Vec::new();
        for line in text.lines().skip(1).filter(|l| !l.is_empty()) {
            let v: Vec<&str> = line.split(',').collect();
            if v.len() != 10 {
                return None;
            }
            let dev = |i: usize| -> Option<Option<Vec3>> {
                if v[i].is_empty() {
                    return Some(None);
                }
                Some(Some([
                    v[i].parse().ok()?,
                    v[i + 1].parse().ok()?,
                    v[i + 2].parse().ok()?,
                ]))
            };
            frames.push(PoseFrame {
                t: v[0].parse().ok()?,
                head: dev(1)?,
                left: dev(4)?,
                right: dev(7)?,
            });
        }
        Some(Self::new(frames))
    }
}

pub const CSV_HEADER: &str = "t,hx,hy,hz,lx,ly,lz,rx,ry,rz";

pub fn csv_row(f: &PoseFrame) -> String {
    let dev = |d: Option<Vec3>| d.map_or(",,".to_owned(), |p| format!("{},{},{}", p[0], p[1], p[2]));
    format!("{},{},{},{}", f.t, dev(f.head), dev(f.left), dev(f.right))
}

impl PoseSource for Replay {
    fn next_frame(&mut self) -> Option<PoseFrame> {
        self.frames.next()
    }
}

/// Movement patterns for the replay gate. Amplitudes in metres, frequencies in Hz.
#[derive(Clone, Copy, Debug)]
pub enum Pattern {
    Idle,
    Sway,
    /// 20 s of punching, 10 s rest.
    BoxingBursts,
    SustainedCardio,
}

pub const RATE_HZ: f64 = 30.0;

pub fn synth(pattern: Pattern, seconds: f64, t0: f64) -> Vec<PoseFrame> {
    let n = (seconds * RATE_HZ) as usize;
    let mut noise = Noise(0x9e37_79b9);
    (0..n)
        .map(|i| {
            let t = i as f64 / RATE_HZ;
            let (head, hand, f_head, f_hand) = match pattern {
                Pattern::Idle => (0.0, 0.0, 0.0, 0.0),
                Pattern::Sway => (0.04, 0.05, 0.4, 0.4),
                Pattern::BoxingBursts if t % 30.0 < 20.0 => (0.06, 0.32, 1.0, 2.2),
                Pattern::BoxingBursts => (0.01, 0.02, 0.3, 0.3),
                Pattern::SustainedCardio => (0.03, 0.06, 1.6, 1.6),
            };
            let s = |a: f64, f: f64, phase: f64| (a * (std::f64::consts::TAU * f * t + phase).sin()) as f32;
            let mut at = |base: Vec3, a: f64, f: f64, phase: f64| -> Vec3 {
                [
                    base[0] + noise.mm(),
                    base[1] + s(a, f, phase) + noise.mm(),
                    base[2] + s(a, f, phase + 1.0) + noise.mm(),
                ]
            };
            PoseFrame {
                t: t0 + t,
                head: Some(at([0.0, 1.65, 0.0], head, f_head, 0.0)),
                left: Some(at([-0.25, 1.2, -0.3], hand, f_hand, 0.0)),
                right: Some(at([0.25, 1.2, -0.3], hand, f_hand, std::f64::consts::PI)),
            }
        })
        .collect()
}

/// Sub-millimetre jitter, like a real tracker at rest.
struct Noise(u32);

impl Noise {
    fn mm(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32 - 0.5) * 0.0005
    }
}
