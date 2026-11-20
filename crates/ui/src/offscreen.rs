// Copyright (C) 2026 EncidXVII
// SPDX-License-Identifier: GPL-3.0-only

//! Renders Slint windows into RGBA buffers with the software renderer, for overlay textures and
//! snapshot tests. Each `Surface` owns one window; only changed regions are repainted.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize, PlatformError};

thread_local! {
    /// The window the next component will be created in. Slint asks the platform for a window
    /// from inside `Component::new`, so it is handed over through here.
    static NEXT: RefCell<Option<Rc<MinimalSoftwareWindow>>> = const { RefCell::new(None) };
}

struct Offscreen {
    start: Instant,
}

impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        NEXT.with(|n| n.borrow_mut().take())
            .map(|w| w as Rc<dyn WindowAdapter>)
            .ok_or_else(|| PlatformError::Other("create windows through Surface::new".into()))
    }

    fn duration_since_start(&self) -> Duration {
        self.start.elapsed()
    }
}

/// Use the offscreen platform for this process. Must run before any component is created; a
/// second call is a no-op.
pub fn install() {
    let _ = slint::platform::set_platform(Box::new(Offscreen {
        start: Instant::now(),
    }));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pointer {
    Move,
    Down,
    Up,
    Leave,
    /// Wheel, in logical pixels; negative scrolls the content up (towards the end).
    Scroll(i32),
}

pub struct Surface<C: ComponentHandle> {
    pub ui: C,
    window: Rc<MinimalSoftwareWindow>,
    pixels: Vec<PremultipliedRgbaColor>,
    rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl<C: ComponentHandle> Surface<C> {
    pub fn new(
        make: impl FnOnce() -> Result<C, PlatformError>,
        width: u32,
        height: u32,
    ) -> Result<Self, PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        window.set_size(PhysicalSize::new(width, height));
        NEXT.with(|n| *n.borrow_mut() = Some(window.clone()));
        let ui = make()?;
        ui.show()?;
        let n = (width * height) as usize;
        Ok(Self {
            ui,
            window,
            pixels: vec![PremultipliedRgbaColor::default(); n],
            rgba: vec![0; n * 4],
            width,
            height,
        })
    }

    /// Repaints if anything changed; true when `frame` holds a new image.
    pub fn render(&mut self) -> bool {
        slint::platform::update_timers_and_animations();
        let (pixels, stride) = (&mut self.pixels, self.width as usize);
        let mut dirty = None;
        if !self
            .window
            .draw_if_needed(|r| dirty = Some(r.render(pixels, stride)))
        {
            return false;
        }
        // Convert only what was repainted: a full 1200x800 pass cost ~2 ms, the repaint itself
        // a fraction of that.
        let (x0, y0, w, h) = dirty.map_or((0, 0, self.width, self.height), |d| {
            let (o, s) = (d.bounding_box_origin(), d.bounding_box_size());
            (o.x.max(0) as u32, o.y.max(0) as u32, s.width, s.height)
        });
        let (x1, y1) = (
            (x0 + w).min(self.width) as usize,
            (y0 + h).min(self.height) as usize,
        );
        let rgba = self.rgba.as_chunks_mut::<4>().0;
        for y in y0 as usize..y1 {
            let row = y * stride;
            for (out, p) in rgba[row + x0 as usize..row + x1]
                .iter_mut()
                .zip(&self.pixels[row + x0 as usize..row + x1])
            {
                *out = match p.alpha {
                    255 => [p.red, p.green, p.blue, 255],
                    0 => [0; 4],
                    a => {
                        let un = |c: u8| (u16::from(c) * 255 / u16::from(a)) as u8;
                        [un(p.red), un(p.green), un(p.blue), a]
                    }
                };
            }
        }
        true
    }

    /// The last rendered frame as straight RGBA, rows top to bottom.
    pub fn frame(&mut self) -> &mut [u8] {
        &mut self.rgba
    }

    pub fn pointer(&self, x: f32, y: f32, kind: Pointer) {
        let position = LogicalPosition::new(x, y);
        let button = PointerEventButton::Left;
        let event = match kind {
            Pointer::Move => WindowEvent::PointerMoved { position },
            Pointer::Down => WindowEvent::PointerPressed { position, button },
            Pointer::Up => WindowEvent::PointerReleased { position, button },
            Pointer::Leave => WindowEvent::PointerExited,
            Pointer::Scroll(dy) => WindowEvent::PointerScrolled {
                position,
                delta_x: 0.0,
                delta_y: dy as f32,
            },
        };
        let _ = self.window.window().dispatch_event_with_result(event);
    }
}
