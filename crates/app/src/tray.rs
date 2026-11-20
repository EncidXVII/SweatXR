// Copyright (C) 2026 EncidXVII
// SPDX-License-Identifier: GPL-3.0-only

//! Windows tray icon: a click opens the window, the right-click menu has Open and Quit. The icon
//! is the SweatXR mark from the exe's icons (build.rs): lime on a dark taskbar, dark ink on a
//! light one, swapped when Windows switches.

use std::time::{Duration, Instant};

use slint::ComponentHandle;
use sweatxr_ui::DesktopWindow;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

const SIZE: u32 = 32;
/// How often the tray checks whether the taskbar turned light or dark.
const THEME_CHECK: Duration = Duration::from_secs(2);

pub struct Tray {
    icon: TrayIcon,
    light: bool,
    checked: Instant,
    open: MenuItem,
    quit: MenuItem,
}

/// The mark at the tray's size from the exe's icon groups 2 (lime) and 3 (ink). A build without
/// the icons gets the drawn accent ring instead.
fn icon(light: bool) -> Result<Icon, tray_icon::BadIcon> {
    let size = small_icon_px();
    Icon::from_resource(if light { 3 } else { 2 }, Some((size, size)))
        .or_else(|_| Icon::from_rgba(ring(), SIZE, SIZE))
}

/// The size Windows draws small icons at (the tray), in pixels at the current display scale.
fn small_icon_px() -> u32 {
    const SM_CXSMICON: i32 = 49;
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetSystemMetrics(index: i32) -> i32;
    }
    // SAFETY: plain read of a system value.
    u32::try_from(unsafe { GetSystemMetrics(SM_CXSMICON) })
        .unwrap_or(16)
        .max(16)
}

/// Whether Windows draws the taskbar light (Settings > Personalisation > Colours: "Choose your
/// default Windows mode").
fn light_taskbar() -> bool {
    // HKEY_CURRENT_USER is the handle value 0x80000001, sign-extended as Windows defines it.
    const HKEY_CURRENT_USER: isize = 0x8000_0001_u32.cast_signed() as isize;
    const RRF_RT_REG_DWORD: u32 = 0x18;
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegGetValueW(
            key: isize,
            subkey: *const u16,
            value: *const u16,
            flags: u32,
            kind: *mut u32,
            data: *mut core::ffi::c_void,
            len: *mut u32,
        ) -> i32;
    }
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("SystemUsesLightTheme");
    let (mut value, mut len) = (0u32, 4u32);
    // SAFETY: NUL-terminated strings that outlive the call; the DWORD lands in `value`, at most `len` bytes.
    let err = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            core::ptr::null_mut(),
            (&raw mut value).cast(),
            &raw mut len,
        )
    };
    err == 0 && value == 1
}

fn ring() -> Vec<u8> {
    let c = (SIZE as f32 - 1.0) / 2.0;
    (0..SIZE * SIZE)
        .flat_map(|i| {
            let (x, y) = ((i % SIZE) as f32 - c, (i / SIZE) as f32 - c);
            let r = (x * x + y * y).sqrt();
            if (10.0..=15.5).contains(&r) {
                [0xC6, 0xFF, 0x1A, 0xFF]
            } else {
                [0, 0, 0, 0]
            }
        })
        .collect()
}

impl Tray {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let open = MenuItem::new("Open SweatXR", true, None);
        let quit = MenuItem::new("Quit", true, None);
        let menu = Menu::new();
        menu.append(&open)?;
        menu.append(&quit)?;
        let light = light_taskbar();
        let icon = TrayIconBuilder::new()
            .with_tooltip("SweatXR")
            .with_icon(icon(light)?)
            .with_menu(Box::new(menu))
            // Left click opens the window; the menu is on the right button, as elsewhere on Windows.
            .with_menu_on_left_click(false)
            .build()?;
        Ok(Self {
            icon,
            light,
            checked: Instant::now(),
            open,
            quit,
        })
    }

    /// Swaps the icon when the taskbar turns light or dark (checked every couple of seconds).
    fn follow_taskbar(&mut self) {
        if self.checked.elapsed() < THEME_CHECK {
            return;
        }
        self.checked = Instant::now();
        let light = light_taskbar();
        if light != self.light
            && let Ok(icon) = icon(light)
            && self.icon.set_icon(Some(icon)).is_ok()
        {
            self.light = light;
        }
    }

    /// Handles clicks since the last call. True when the player chose Quit.
    pub fn poll(&mut self, ui: &DesktopWindow) -> bool {
        self.follow_taskbar();
        while let Ok(e) = MenuEvent::receiver().try_recv() {
            if e.id == *self.quit.id() {
                return true;
            }
            if e.id == *self.open.id() {
                let _ = ui.show();
            }
        }
        while let Ok(e) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
            {
                let _ = ui.show();
            }
        }
        false
    }
}
