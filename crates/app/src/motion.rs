// Copyright (C) 2026 EncidXVII
// SPDX-License-Identifier: GPL-3.0-only

//! Whether the UI may animate: Windows' "Animation effects" setting (Settings > Accessibility >
//! Visual effects), as WristXR reads it. Off keeps every state change and drops the movement.

#[cfg(windows)]
pub fn animations_enabled() -> bool {
    const SPI_GETCLIENTAREAANIMATION: u32 = 0x1042;
    #[link(name = "user32")]
    unsafe extern "system" {
        fn SystemParametersInfoW(action: u32, param: u32, value: *mut core::ffi::c_void, ini: u32) -> i32;
    }
    let mut on: i32 = 1;
    // SAFETY: plain query into an i32 (BOOL) we own.
    let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut on).cast(), 0) };
    // On when the query fails: the setting is unknown, not off.
    ok == 0 || on != 0
}

#[cfg(not(windows))]
pub fn animations_enabled() -> bool {
    true
}
