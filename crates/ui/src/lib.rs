//! Slint views shared by the VR watch, the VR panel and the desktop window, plus the code that
//! turns stored totals into what they show.

slint::include_modules!();
include!(concat!(env!("OUT_DIR"), "/accents.rs"));

/// The theme's accent as RGB; an unknown theme falls back to the first (lime).
pub fn accent_rgb(theme: u8) -> [u8; 3] {
    ACCENTS.get(usize::from(theme)).copied().unwrap_or(ACCENTS[0])
}

pub mod offscreen;
pub mod view;
