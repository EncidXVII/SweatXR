//! Windows tray icon: a click opens the window, the right-click menu has Open and Quit. The icon
//! is drawn in code (accent ring) so there is no image decoder or asset to ship.

use fitvr_ui::DesktopWindow;
use slint::ComponentHandle;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

const SIZE: u32 = 32;

pub struct Tray {
    _icon: TrayIcon,
    open: MenuItem,
    quit: MenuItem,
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
        let open = MenuItem::new("Open FitnessVR", true, None);
        let quit = MenuItem::new("Quit", true, None);
        let menu = Menu::new();
        menu.append(&open)?;
        menu.append(&quit)?;
        let icon = TrayIconBuilder::new()
            .with_tooltip("FitnessVR")
            .with_icon(Icon::from_rgba(ring(), SIZE, SIZE)?)
            .with_menu(Box::new(menu))
            // Left click opens the window; the menu is on the right button, as elsewhere on Windows.
            .with_menu_on_left_click(false)
            .build()?;
        Ok(Self {
            _icon: icon,
            open,
            quit,
        })
    }

    /// Handles clicks since the last call. True when the player chose Quit.
    pub fn poll(&self, ui: &DesktopWindow) -> bool {
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
