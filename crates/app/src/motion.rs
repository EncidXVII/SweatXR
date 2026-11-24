// Copyright (C) 2026 EncidXVII
// SPDX-License-Identifier: GPL-3.0-only

//! Whether the UI may animate: Windows' "Animation effects" setting (Settings > Accessibility >
//! Visual effects), as WristXR reads it; on Linux GNOME's enable-animations and KDE's animation
//! speed set to instant. Off keeps every state change and drops the movement.

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
    let gnome = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "enable-animations"])
        .output()
        .ok()
        .is_some_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "false");
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")));
    let kde = config
        .and_then(|d| std::fs::read_to_string(d.join("kdeglobals")).ok())
        .is_some_and(|t| {
            t.lines()
                .any(|l| l.replace(' ', "") == "AnimationDurationFactor=0")
        });
    !(gnome || kde)
}
