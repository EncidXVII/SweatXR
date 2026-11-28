// Copyright (C) 2026 EncidXVII
// SPDX-License-Identifier: GPL-3.0-only

//! Windows tray icon of the watch in VR, there for as long as the watch runs: a click opens the window, the
//! right-click menu has Open, Start with SteamVR and Quit (which stops the watch). The icon is the SweatXR mark
//! from the exe's icons (build.rs): lime on a dark taskbar, dark ink on a light one, swapped when Windows
//! switches. The window has no icon of its own: it is on the taskbar while it is open.

use std::time::{Duration, Instant};

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

const SIZE: u32 = 32;
/// How often the tray checks whether the taskbar turned light or dark.
const THEME_CHECK: Duration = Duration::from_secs(2);

pub struct Tray {
    icon: TrayIcon,
    light: bool,
    checked: Instant,
    open: MenuItem,
    autostart: CheckMenuItem,
    quit: MenuItem,
}

pub enum Cmd {
    Open,
    AutoStart(bool),
    Quit,
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

/// The window class winit gives its windows; with the title it tells our window from any other named SweatXR.
const WINDOW_CLASS: &str = "Window Class";

#[repr(C)]
struct Msg {
    hwnd: isize,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt: [i32; 2],
    private: u32,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn PeekMessageW(msg: *mut Msg, hwnd: isize, min: u32, max: u32, remove: u32) -> i32;
    fn TranslateMessage(msg: *const Msg) -> i32;
    fn DispatchMessageW(msg: *const Msg) -> isize;
    fn FindWindowW(class: *const u16, title: *const u16) -> isize;
    fn ShowWindow(hwnd: isize, cmd: i32) -> i32;
    fn IsIconic(hwnd: isize) -> i32;
    fn SetForegroundWindow(hwnd: isize) -> i32;
}

/// The icon and its menu get their clicks as window messages; the watch's loop has no other reader for them.
fn pump() {
    const PM_REMOVE: u32 = 1;
    // SAFETY: a zeroed MSG is valid to fill; only this thread's messages are read and handed on.
    unsafe {
        let mut msg: Msg = core::mem::zeroed();
        while PeekMessageW(&raw mut msg, 0, 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&raw const msg);
            DispatchMessageW(&raw const msg);
        }
    }
}

/// Brings SweatXR's window to the front, or starts it when it is not open.
pub fn open_window() {
    const SW_RESTORE: i32 = 9;
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (class, title) = (wide(WINDOW_CLASS), wide("SweatXR"));
    // SAFETY: NUL-terminated strings that outlive the call; the handle is used at once, and a window that has
    // closed since only makes the calls fail.
    let found = unsafe {
        let hwnd = FindWindowW(class.as_ptr(), title.as_ptr());
        if hwnd != 0 {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            SetForegroundWindow(hwnd);
        }
        hwnd != 0
    };
    if found {
        return;
    }
    if let Err(e) = std::env::current_exe().and_then(|exe| std::process::Command::new(exe).spawn()) {
        tracing::warn!("open SweatXR: {e}");
    }
}

impl Tray {
    pub fn new(autostart: bool) -> Result<Self, Box<dyn std::error::Error>> {
        let open = MenuItem::new("Open SweatXR", true, None);
        let autostart = CheckMenuItem::new("Start with SteamVR", true, autostart, None);
        let quit = MenuItem::new("Quit", true, None);
        let menu = Menu::new();
        menu.append_items(&[&open, &autostart, &PredefinedMenuItem::separator(), &quit])?;
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
            autostart,
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

    /// Keeps the "Start with SteamVR" tick in step with the setting, wherever it was changed.
    pub fn set_autostart(&self, on: bool) {
        if self.autostart.is_checked() != on {
            self.autostart.set_checked(on);
        }
    }

    /// The next thing chosen on the icon or its menu, if any. Call every frame, until None.
    pub fn poll(&mut self) -> Option<Cmd> {
        pump();
        self.follow_taskbar();
        while let Ok(e) = MenuEvent::receiver().try_recv() {
            if e.id == *self.quit.id() {
                return Some(Cmd::Quit);
            }
            if e.id == *self.autostart.id() {
                return Some(Cmd::AutoStart(self.autostart.is_checked()));
            }
            if e.id == *self.open.id() {
                return Some(Cmd::Open);
            }
        }
        while let Ok(e) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
            {
                return Some(Cmd::Open);
            }
        }
        None
    }
}
