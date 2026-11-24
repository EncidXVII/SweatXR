// Copyright (C) 2026 EncidXVII
// SPDX-License-Identifier: GPL-3.0-only

//! Heart rate sources: a BLE strap (live) or a scripted fake (tests, demos).

mod ble;
mod parse;

pub use ble::{BleHr, legacy_id};
pub use parse::{Contact, Measurement, ParseError, parse};

use std::time::{Duration, Instant};

/// A reading older than this is a dropped strap, not a heart rate.
pub const STALE: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    /// No strap chosen.
    #[default]
    Off,
    NoAdapter,
    /// The adapter is there but switched off.
    BluetoothOff,
    Scanning,
    Connecting,
    Connected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
}

pub trait HrSource {
    /// Current bpm, or `None` when there is no fresh reading with skin contact.
    fn bpm(&self) -> Option<u16>;
    fn status(&self) -> Status;
}

/// Plays one value per second, looping. `None` entries simulate dropouts.
pub struct FakeHr {
    values: Vec<Option<u16>>,
    start: Instant,
}

impl FakeHr {
    pub fn new(values: Vec<Option<u16>>) -> Self {
        Self {
            values,
            start: Instant::now(),
        }
    }

    /// Warm-up, two intervals and a dropout: exercises every HR path in about four minutes.
    pub fn workout() -> Self {
        let mut v: Vec<Option<u16>> = (0..60).map(|i| Some(85 + i)).collect();
        v.extend((0..60).map(|i| Some(150 - (i % 20))));
        v.extend([None; 10]);
        v.extend((0..60).map(|i| Some(160 - i / 2)));
        v.extend((0..60).map(|i| Some(130 - i / 2)));
        Self::new(v)
    }

    pub fn value_at(&self, secs: u64) -> Option<u16> {
        if self.values.is_empty() {
            return None;
        }
        self.values[(secs % self.values.len() as u64) as usize]
    }
}

impl HrSource for FakeHr {
    fn bpm(&self) -> Option<u16> {
        self.value_at(self.start.elapsed().as_secs())
    }

    fn status(&self) -> Status {
        Status::Connected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_loops_and_drops() {
        let f = FakeHr::new(vec![Some(100), None]);
        assert_eq!(
            (f.value_at(0), f.value_at(1), f.value_at(2)),
            (Some(100), None, Some(100))
        );
        assert!(FakeHr::workout().values.contains(&None));
    }
}
