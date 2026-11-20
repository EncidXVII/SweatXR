// Copyright (C) 2026 EncidXVII
// SPDX-License-Identifier: GPL-3.0-only

//! The user's body data and preferences. Stored as one row; every field has a safe default so
//! the app works before first run is finished.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sex {
    Male,
    Female,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hand {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Units {
    Metric,
    Imperial,
}

/// Number of accent themes (design-tokens.json "themes").
pub const THEMES: u8 = 8;

/// Per-user motion intensity -> kcal/min line, fitted while a heart rate strap was worn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Calibration {
    pub slope: f32,
    pub intercept: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub weight_kg: f32,
    pub height_cm: f32,
    pub age: u32,
    pub sex: Sex,
    pub goal_kcal: u32,
    pub resting_hr: u32,
    pub watch_hand: Hand,
    pub units: Units,
    pub auto_start: bool,
    pub hr_device: Option<String>,
    pub calibration: Option<Calibration>,
    pub onboarded: bool,
    /// The player has opened the VR panel at least once, so the watch stops teaching the gesture.
    pub panel_seen: bool,
    /// Accent colour, an index into design-tokens.json "themes" (0 = Lime).
    pub theme: u8,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            weight_kg: 70.0,
            height_cm: 170.0,
            age: 30,
            sex: Sex::Male,
            goal_kcal: 300,
            resting_hr: 60,
            watch_hand: Hand::Left,
            units: Units::Metric,
            auto_start: true,
            hr_device: None,
            calibration: None,
            onboarded: false,
            panel_seen: false,
            theme: 0,
        }
    }
}

impl Profile {
    /// Clamp UI edits to the ranges the equations were fitted on.
    pub fn clamped(mut self) -> Self {
        self.weight_kg = self.weight_kg.clamp(30.0, 250.0);
        self.height_cm = self.height_cm.clamp(120.0, 230.0);
        self.age = self.age.clamp(13, 99);
        self.goal_kcal = self.goal_kcal.clamp(50, 3000);
        self.resting_hr = self.resting_hr.clamp(35, 110);
        self.theme = self.theme.min(THEMES - 1);
        self
    }
}
