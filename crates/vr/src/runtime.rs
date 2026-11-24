// Copyright (C) 2026 EncidXVII
// SPDX-License-Identifier: GPL-3.0-only

//! Linux: SteamVR through OpenVR when it runs, else an OpenXR runtime that can show overlays (Monado, WiVRn). The
//! overlay's calls go to whichever is in use; SteamVR-only ones (registering, the game in front, another app's
//! overlays, taking buttons from the game) do nothing or come back empty under OpenXR.

use std::cell::RefCell;
use std::path::Path;

use crate::ffi::{self, Apps, Gpu, Handle, HmdMatrix34, MAX_DEVICES, TrackedDevicePose, VrError, VrEvent};
use crate::xr::OpenXr;
use crate::{AppKind, OpenVr};

pub enum Vr {
    OpenVr(OpenVr),
    OpenXr(Box<RefCell<OpenXr>>),
}

fn no_textures() -> VrError {
    VrError {
        call: "Vulkan textures",
        code: 0,
        detail: "the OpenXR session fills its own".into(),
    }
}

impl Vr {
    /// SteamVR (`AppKind::Overlay` or `Background`).
    pub fn init(kind: AppKind) -> Result<Self, VrError> {
        ffi::Vr::init(kind).map(Self::OpenVr)
    }

    /// An OpenXR runtime that can show overlays, with the session started.
    pub fn openxr() -> Result<Self, VrError> {
        OpenXr::open().map(|x| Self::OpenXr(Box::new(RefCell::new(x))))
    }

    pub fn is_openxr(&self) -> bool {
        matches!(self, Self::OpenXr(_))
    }

    /// SteamVR's application half; OpenXR has none.
    pub fn apps(&self) -> Option<&Apps> {
        match self {
            Self::OpenVr(vr) => vr.apps(),
            Self::OpenXr(_) => None,
        }
    }

    /// Under OpenXR, pictures go through `set_raw`, straight into the session's swapchains.
    pub fn gpu(&self) -> Result<Gpu, VrError> {
        match self {
            Self::OpenVr(vr) => vr.gpu(),
            Self::OpenXr(_) => Err(no_textures()),
        }
    }

    pub fn set_texture(&self, h: u64, gpu: &mut Gpu, slot: usize, rgba: &[u8]) -> Result<(), VrError> {
        match self {
            Self::OpenVr(vr) => vr.set_texture(h, gpu, slot, rgba),
            Self::OpenXr(_) => Err(no_textures()),
        }
    }

    pub fn set_raw(&self, h: u64, rgba: &mut [u8], width: u32, height: u32) -> Result<(), VrError> {
        match self {
            Self::OpenVr(vr) => vr.set_raw(h, rgba, width, height),
            Self::OpenXr(xr) => xr.borrow_mut().set_raw(h, rgba, width, height),
        }
    }

    pub fn poses(&self) -> Box<[TrackedDevicePose; MAX_DEVICES]> {
        match self {
            Self::OpenVr(vr) => vr.poses(),
            Self::OpenXr(xr) => xr.borrow_mut().poses(),
        }
    }

    pub fn controller(&self, role: u32) -> Option<u32> {
        match self {
            Self::OpenVr(vr) => vr.controller(role),
            Self::OpenXr(_) => OpenXr::controller(role),
        }
    }

    pub fn poll_event(&self) -> Option<VrEvent> {
        match self {
            Self::OpenVr(vr) => vr.poll_event(),
            Self::OpenXr(xr) => xr.borrow_mut().poll_event(),
        }
    }

    pub fn create_overlay(&self, key: &str, name: &str) -> Result<u64, VrError> {
        match self {
            Self::OpenVr(vr) => vr.create_overlay(key, name),
            Self::OpenXr(xr) => Ok(xr.borrow_mut().create_overlay()),
        }
    }

    /// OpenXR does not show another app's overlays.
    pub fn overlay_exists(&self, key: &str) -> bool {
        match self {
            Self::OpenVr(vr) => vr.overlay_exists(key),
            Self::OpenXr(_) => false,
        }
    }

    pub fn destroy_overlay(&self, h: u64) {
        match self {
            Self::OpenVr(vr) => vr.destroy_overlay(h),
            Self::OpenXr(xr) => xr.borrow_mut().destroy_overlay(h),
        }
    }

    pub fn set_width(&self, h: u64, metres: f32) -> Result<(), VrError> {
        match self {
            Self::OpenVr(vr) => vr.set_width(h, metres),
            Self::OpenXr(xr) => xr.borrow_mut().set_width(h, metres),
        }
    }

    pub fn set_sort_order(&self, h: u64, order: u32) -> Result<(), VrError> {
        match self {
            Self::OpenVr(vr) => vr.set_sort_order(h, order),
            Self::OpenXr(xr) => xr.borrow_mut().set_sort_order(h, order),
        }
    }

    pub fn wait_frame(&self, timeout_ms: u32) {
        match self {
            Self::OpenVr(vr) => vr.wait_frame(timeout_ms),
            Self::OpenXr(xr) => xr.borrow_mut().wait_frame(timeout_ms),
        }
    }

    pub fn set_alpha(&self, h: u64, alpha: f32) -> Result<(), VrError> {
        match self {
            Self::OpenVr(vr) => vr.set_alpha(h, alpha),
            Self::OpenXr(xr) => xr.borrow_mut().set_alpha(h, alpha),
        }
    }

    pub fn set_visible(&self, h: u64, visible: bool) -> Result<(), VrError> {
        match self {
            Self::OpenVr(vr) => vr.set_visible(h, visible),
            Self::OpenXr(xr) => xr.borrow_mut().set_visible(h, visible),
        }
    }

    pub fn attach(&self, h: u64, device: u32, m: &HmdMatrix34) -> Result<(), VrError> {
        match self {
            Self::OpenVr(vr) => vr.attach(h, device, m),
            Self::OpenXr(xr) => xr.borrow_mut().attach(h, device, m),
        }
    }

    pub fn place(&self, h: u64, m: &HmdMatrix34) -> Result<(), VrError> {
        match self {
            Self::OpenVr(vr) => vr.place(h, m),
            Self::OpenXr(xr) => xr.borrow_mut().place(h, m),
        }
    }

    /// OpenXR's actions are made with the session.
    pub fn set_action_manifest(&self, path: &Path) -> Result<(), VrError> {
        match self {
            Self::OpenVr(vr) => vr.set_action_manifest(path),
            Self::OpenXr(_) => Ok(()),
        }
    }

    pub fn handle(&self, kind: Handle, path: &str) -> Result<u64, VrError> {
        match self {
            Self::OpenVr(vr) => vr.handle(kind, path),
            Self::OpenXr(_) => OpenXr::handle(kind, path),
        }
    }

    /// An OpenXR overlay cannot take buttons from the game, so `take` only counts on SteamVR.
    pub fn update_actions(&self, set: u64, hands: [u64; 2], take: [bool; 2]) -> Result<(), VrError> {
        match self {
            Self::OpenVr(vr) => vr.update_actions(set, hands, take),
            Self::OpenXr(xr) => {
                xr.borrow().update_actions();
                Ok(())
            }
        }
    }

    pub fn dashboard_visible(&self) -> bool {
        match self {
            Self::OpenVr(vr) => vr.dashboard_visible(),
            Self::OpenXr(_) => false,
        }
    }

    pub fn digital(&self, action: u64, source: u64) -> Option<(bool, bool)> {
        match self {
            Self::OpenVr(vr) => vr.digital(action, source),
            Self::OpenXr(xr) => xr.borrow().digital(action, source),
        }
    }

    pub fn pose(&self, action: u64, source: u64) -> Option<HmdMatrix34> {
        match self {
            Self::OpenVr(vr) => vr.pose(action, source),
            Self::OpenXr(xr) => xr.borrow().pose(action, source),
        }
    }

    /// OpenXR predicts every pose for when the frame shows.
    pub fn pose_next(&self, action: u64, source: u64) -> Option<HmdMatrix34> {
        match self {
            Self::OpenVr(vr) => vr.pose_next(action, source),
            Self::OpenXr(xr) => xr.borrow().pose(action, source),
        }
    }

    pub fn haptic(&self, action: u64, source: u64, seconds: f32, amplitude: f32) {
        match self {
            Self::OpenVr(vr) => vr.haptic(action, source, seconds, amplitude),
            Self::OpenXr(xr) => xr.borrow().haptic(source, seconds, amplitude),
        }
    }
}
