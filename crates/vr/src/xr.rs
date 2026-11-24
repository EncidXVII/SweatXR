// Copyright (C) 2026 EncidXVII
// SPDX-License-Identifier: GPL-3.0-only

//! OpenXR overlay backend for Linux runtimes without SteamVR (Monado, WiVRn), after WristXR's: an overlay session
//! (`XR_EXTX_overlay`) on our own Vulkan device, every overlay a quad layer, the controllers through OpenXR actions.
//! It answers the same calls as the OpenVR connection, so the overlay loop runs the same on either. What OpenXR
//! cannot tell (the game in front, another app's overlays, SteamVR's dashboard) comes back empty. The OpenXR loader
//! is opened at run time, so the app still starts where none is installed.

use std::collections::HashMap;
use std::ffi::{CString, c_void};
use std::time::Duration;

use openxr as xr;
use openxr::sys::Handle as _;

use crate::ffi::{EVENT_QUIT, Gpu, HMD, Handle, MAX_DEVICES, ROLE_LEFT, ROLE_RIGHT, TrackedDevicePose};
use crate::ffi::{VrError, VrEvent};
use crate::{Mat, compose, pose_mat, position, quaternion};

/// `sessionLayersPlacement`: above the game's layers (the value WristXR uses).
const LAYERS_PLACEMENT: u32 = 5;
/// Swapchain formats in order of preference, and whether red and blue must be swapped for it:
/// `VK_FORMAT_R8G8B8A8_SRGB`, `_UNORM`, then `VK_FORMAT_B8G8R8A8_SRGB`, `_UNORM`. sRGB first, like the textures
/// SteamVR gets.
const FORMATS: [(u32, bool); 4] = [(43, false), (37, false), (50, true), (44, true)];
/// The clear 1x1 layer shown when nothing else is: Monado misbehaves on frames without layers.
const BLANK: u64 = u64::MAX;
/// The actions the overlay asks for by name (`handle`), by the last part of their OpenVR path; a handle is its
/// place here plus one.
const ACTIONS: [&str; 4] = ["select", "grab", "pointer", "haptic"];
const SELECT: u64 = 1;
const GRAB: u64 = 2;
const POINTER: u64 = 3;
/// The two controllers in `poses`, as OpenVR numbers them by role: the headset is 0.
const LEFT: u32 = ROLE_LEFT;
const RIGHT: u32 = ROLE_RIGHT;

fn xr_err(call: &'static str) -> impl Fn(xr::sys::Result) -> VrError {
    move |r| VrError {
        call,
        code: 0,
        detail: r.to_string(),
    }
}

fn fail(call: &'static str, detail: &str) -> VrError {
    VrError {
        call,
        code: 0,
        detail: detail.into(),
    }
}

/// Select and grab per controller type, as WristXR binds them: the trigger selects, the grip moves the watch. Every
/// type also gets the aim pose (the pointer, SteamVR's tip), the grip pose (the controller itself) and haptics.
const PROFILES: [(&str, &str, Option<&str>); 6] = [
    ("khr/simple_controller", "select/click", None),
    ("valve/index_controller", "trigger/click", Some("squeeze/value")),
    ("oculus/touch_controller", "trigger/value", Some("squeeze/value")),
    ("htc/vive_controller", "trigger/click", Some("squeeze/click")),
    (
        "microsoft/motion_controller",
        "trigger/value",
        Some("squeeze/click"),
    ),
    (
        "bytedance/pico4_controller",
        "trigger/value",
        Some("squeeze/value"),
    ),
];

struct Actions {
    set: xr::ActionSet,
    select: xr::Action<bool>,
    grab: xr::Action<bool>,
    aim: xr::Action<xr::Posef>,
    grip: xr::Action<xr::Posef>,
    haptic: xr::Action<xr::Haptic>,
    hands: [xr::Path; 2],
}

impl Actions {
    fn new(instance: &xr::Instance) -> xr::Result<Self> {
        let hands = [
            instance.string_to_path(xr::USER_HAND_LEFT)?,
            instance.string_to_path(xr::USER_HAND_RIGHT)?,
        ];
        let set = instance.create_action_set("sweatxr", "SweatXR", 0)?;
        Ok(Self {
            select: set.create_action("select", "Select on watch and panel", &hands)?,
            grab: set.create_action("grab", "Move the watch", &hands)?,
            aim: set.create_action("pointer", "Pointer", &hands)?,
            grip: set.create_action("grip", "Controller", &hands)?,
            haptic: set.create_action("haptic", "Haptic tick", &hands)?,
            set,
            hands,
        })
    }

    /// Suggests the bindings for every controller type; one the runtime rejects is skipped.
    fn suggest(&self, instance: &xr::Instance, pico: bool) {
        for (profile, select, grab) in PROFILES {
            // The Pico 4 profile comes with an extension.
            if profile.starts_with("bytedance/") && !pico {
                continue;
            }
            let mut bindings = Vec::new();
            for hand in ["left", "right"] {
                let path = |p: &str| instance.string_to_path(&format!("/user/hand/{hand}/{p}")).ok();
                bindings.extend(path(&format!("input/{select}")).map(|p| xr::Binding::new(&self.select, p)));
                if let Some(g) = grab {
                    bindings.extend(path(&format!("input/{g}")).map(|p| xr::Binding::new(&self.grab, p)));
                }
                bindings.extend(path("input/aim/pose").map(|p| xr::Binding::new(&self.aim, p)));
                bindings.extend(path("input/grip/pose").map(|p| xr::Binding::new(&self.grip, p)));
                bindings.extend(path("output/haptic").map(|p| xr::Binding::new(&self.haptic, p)));
            }
            let result = instance
                .string_to_path(&format!("/interaction_profiles/{profile}"))
                .and_then(|p| instance.suggest_interaction_profile_bindings(p, &bindings));
            if let Err(e) = result {
                tracing::warn!("OpenXR bindings for {profile} not suggested: {e}");
            }
        }
    }
}

/// What a quad's pose is relative to: the room, or a tracked device (`HMD`, `LEFT`, `RIGHT`).
#[derive(Clone, Copy, PartialEq)]
enum Anchor {
    Stage,
    Device(u32),
}

/// One overlay: where it goes and how it shows, as the OpenVR calls set it, and its swapchain.
struct Layer {
    chain: Option<Chain>,
    /// The `Gpu` texture slot its pictures go through, and that slot's size.
    slot: Option<(usize, (u32, u32))>,
    anchor: Anchor,
    pose: Mat,
    width_m: f32,
    alpha: f32,
    shown: bool,
    order: u32,
}

impl Layer {
    fn new() -> Self {
        Self {
            chain: None,
            slot: None,
            anchor: Anchor::Stage,
            pose: pose_mat([0.0; 3], [0.0, 0.0, 0.0, 1.0]),
            width_m: 1.0,
            alpha: 1.0,
            shown: false,
            order: 0,
        }
    }
}

struct Chain {
    swapchain: xr::Swapchain<xr::Vulkan>,
    images: Vec<u64>,
    size: (u32, u32),
    /// An image has been released, so there is a picture to show.
    ready: bool,
}

/// Everything that holds the session.
struct Live {
    session: xr::Session<xr::Vulkan>,
    waiter: xr::FrameWaiter,
    stream: xr::FrameStream<xr::Vulkan>,
    stage: xr::Space,
    view: xr::Space,
    aim: [xr::Space; 2],
    grip: [xr::Space; 2],
    format: u32,
    bgra: bool,
}

pub struct OpenXr {
    // Fields drop in this order: the session and the swapchains first, the Vulkan device they run on (`gpu`) after.
    live: Live,
    layers: HashMap<u64, Layer>,
    gpu: Gpu,
    actions: Actions,
    instance: xr::Instance,
    blend: xr::EnvironmentBlendMode,
    /// The runtime can fade layers (`XR_KHR_composition_layer_color_scale_bias`); without it a dimmed overlay shows
    /// at full brightness.
    color_scale: bool,
    running: bool,
    /// A frame is begun (`wait_frame`) and not yet ended.
    begun: bool,
    /// Display time of the frame begun: poses are predicted for it.
    time: Option<xr::Time>,
    /// The headset and the controllers as last located (`poses`), for drawing far layers first.
    located: [Option<Mat>; 3],
    next_key: u64,
}

fn to_mat(p: xr::Posef) -> Mat {
    let (o, v) = (p.orientation, p.position);
    pose_mat([v.x, v.y, v.z], [o.x, o.y, o.z, o.w])
}

fn to_xr(m: &Mat) -> xr::Posef {
    let [x, y, z, w] = quaternion(m);
    let p = position(m);
    xr::Posef {
        orientation: xr::Quaternionf { x, y, z, w },
        position: xr::Vector3f {
            x: p[0],
            y: p[1],
            z: p[2],
        },
    }
}

fn located(l: &xr::SpaceLocation) -> bool {
    l.location_flags
        .contains(xr::SpaceLocationFlags::ORIENTATION_VALID | xr::SpaceLocationFlags::POSITION_VALID)
}

/// One of OpenXR's space-separated Vulkan extension lists.
fn extensions(list: xr::Result<String>) -> Vec<CString> {
    match list {
        Ok(s) => s
            .split_whitespace()
            .filter_map(|e| CString::new(e).ok())
            .collect(),
        Err(e) => {
            tracing::warn!("OpenXR Vulkan extensions unknown: {e}");
            Vec::new()
        }
    }
}

impl OpenXr {
    /// Opens the runtime, makes a Vulkan device on the GPU the headset is on and starts the overlay session on it.
    pub fn open() -> Result<Self, VrError> {
        // SAFETY: the system's OpenXR loader (`libopenxr_loader.so.1`), which follows the OpenXR specification.
        let entry = unsafe { xr::Entry::load(&()) }
            .map_err(|e| fail("load OpenXR", &format!("no OpenXR loader ({e})")))?;
        let available = entry
            .enumerate_extensions()
            .map_err(xr_err("xrEnumerateInstanceExtensionProperties"))?;
        if !available.extx_overlay {
            return Err(fail(
                "OpenXR",
                "the runtime cannot show overlays (no XR_EXTX_overlay)",
            ));
        }
        if !available.khr_vulkan_enable {
            return Err(fail(
                "OpenXR",
                "the runtime has no Vulkan support (no XR_KHR_vulkan_enable)",
            ));
        }
        let mut wanted = xr::ExtensionSet::default();
        wanted.extx_overlay = true;
        wanted.khr_vulkan_enable = true;
        wanted.khr_composition_layer_color_scale_bias = available.khr_composition_layer_color_scale_bias;
        wanted.bd_controller_interaction = available.bd_controller_interaction;
        let app = xr::ApplicationInfo {
            application_name: "SweatXR",
            engine_name: "SweatXR",
            ..Default::default()
        };
        let instance = entry
            .create_instance(&app, &wanted, &[], &())
            .map_err(xr_err("xrCreateInstance"))?;
        let system = instance
            .system(xr::FormFactor::HEAD_MOUNTED_DISPLAY)
            .map_err(xr_err("xrGetSystem"))?;
        // The spec asks for the graphics requirements before the graphics device and the session.
        let needs = instance
            .graphics_requirements::<xr::Vulkan>(system)
            .map_err(xr_err("xrGetVulkanGraphicsRequirementsKHR"))?;
        let min = needs.min_api_version_supported;
        if (min.major(), min.minor()) > (1, 1) {
            let need = format!("the runtime needs Vulkan {}.{}", min.major(), min.minor());
            return Err(fail("OpenXR", &need));
        }
        let blend = instance
            .enumerate_environment_blend_modes(system, xr::ViewConfigurationType::PRIMARY_STEREO)
            .ok()
            .and_then(|m| m.first().copied())
            .unwrap_or(xr::EnvironmentBlendMode::OPAQUE);
        let actions = Actions::new(&instance).map_err(xr_err("OpenXR actions"))?;
        actions.suggest(&instance, available.bd_controller_interaction);
        let gpu = Gpu::new(
            &extensions(instance.vulkan_legacy_instance_extensions(system)),
            // SAFETY: `vk` is the live VkInstance Gpu::new just made with the extensions asked for above.
            |vk| unsafe { instance.vulkan_graphics_device(system, vk.cast_const()) }.map_or(0, |d| d as u64),
            |_| extensions(instance.vulkan_legacy_device_extensions(system)),
        )?;
        let live = start(&instance, system, &actions, &gpu)?;
        let name = instance.properties().map(|p| p.runtime_name).unwrap_or_default();
        tracing::info!("OpenXR overlay on {name}");
        let mut xr = Self {
            live,
            layers: HashMap::new(),
            gpu,
            actions,
            instance,
            blend,
            color_scale: available.khr_composition_layer_color_scale_bias,
            running: false,
            begun: false,
            time: None,
            located: [None; 3],
            next_key: 0,
        };
        // Clear and tiny, a metre in front of the eyes.
        let mut blank = Layer::new();
        blank.anchor = Anchor::Device(HMD);
        blank.pose = pose_mat([0.0, 0.0, -1.0], [0.0, 0.0, 0.0, 1.0]);
        blank.width_m = 0.01;
        xr.layers.insert(BLANK, blank);
        xr.set_raw(BLANK, &[0; 4], 1, 1)?;
        Ok(xr)
    }

    pub fn create_overlay(&mut self) -> u64 {
        self.next_key += 1;
        self.layers.insert(self.next_key, Layer::new());
        self.next_key
    }

    pub fn destroy_overlay(&mut self, h: u64) {
        self.layers.remove(&h);
    }

    fn layer(&mut self, h: u64) -> Result<&mut Layer, VrError> {
        self.layers
            .get_mut(&h)
            .ok_or_else(|| fail("overlay", "no such overlay"))
    }

    pub fn set_width(&mut self, h: u64, metres: f32) -> Result<(), VrError> {
        self.layer(h)?.width_m = metres;
        Ok(())
    }

    pub fn set_sort_order(&mut self, h: u64, order: u32) -> Result<(), VrError> {
        self.layer(h)?.order = order;
        Ok(())
    }

    pub fn set_alpha(&mut self, h: u64, alpha: f32) -> Result<(), VrError> {
        self.layer(h)?.alpha = alpha;
        Ok(())
    }

    pub fn set_visible(&mut self, h: u64, visible: bool) -> Result<(), VrError> {
        self.layer(h)?.shown = visible;
        Ok(())
    }

    /// Relative to a device (`HMD`, `LEFT`, `RIGHT`): the runtime moves the quad with it between our frames.
    pub fn attach(&mut self, h: u64, device: u32, m: &Mat) -> Result<(), VrError> {
        let l = self.layer(h)?;
        l.anchor = Anchor::Device(device);
        l.pose = *m;
        Ok(())
    }

    pub fn place(&mut self, h: u64, m: &Mat) -> Result<(), VrError> {
        let l = self.layer(h)?;
        l.anchor = Anchor::Stage;
        l.pose = *m;
        Ok(())
    }

    /// Copies straight RGBA8 into the overlay's swapchain and hands it to the runtime, to be shown from the next
    /// frame on.
    pub fn set_raw(&mut self, h: u64, rgba: &[u8], w: u32, height: u32) -> Result<(), VrError> {
        let live = &self.live;
        let layer = self
            .layers
            .get_mut(&h)
            .ok_or_else(|| fail("overlay", "no such overlay"))?;
        if layer.slot.is_none_or(|(_, size)| size != (w, height)) {
            layer.slot = Some((self.gpu.add_texture(w, height)?, (w, height)));
        }
        let Some((slot, _)) = layer.slot else {
            return Err(fail("overlay", "no texture"));
        };
        if layer.chain.as_ref().is_none_or(|c| c.size != (w, height)) {
            layer.chain = None;
            let swapchain = live
                .session
                .create_swapchain(&xr::SwapchainCreateInfo {
                    create_flags: xr::SwapchainCreateFlags::EMPTY,
                    usage_flags: xr::SwapchainUsageFlags::COLOR_ATTACHMENT
                        | xr::SwapchainUsageFlags::TRANSFER_DST,
                    format: live.format,
                    sample_count: 1,
                    width: w,
                    height,
                    face_count: 1,
                    array_size: 1,
                    mip_count: 1,
                })
                .map_err(xr_err("xrCreateSwapchain"))?;
            let images = swapchain
                .enumerate_images()
                .map_err(xr_err("xrEnumerateSwapchainImages"))?;
            layer.chain = Some(Chain {
                swapchain,
                images,
                size: (w, height),
                ready: false,
            });
        }
        let Some(chain) = layer.chain.as_mut() else {
            return Err(fail("overlay", "no swapchain"));
        };
        let i = chain
            .swapchain
            .acquire_image()
            .map_err(xr_err("xrAcquireSwapchainImage"))?;
        if let Err(e) = chain.swapchain.wait_image(xr::Duration::INFINITE) {
            // Acquired but never waited for: the swapchain cannot go on, so a new one comes next time.
            layer.chain = None;
            return Err(xr_err("xrWaitSwapchainImage")(e));
        }
        let copied = match chain.images.get(i as usize) {
            Some(&image) => self.gpu.fill(slot, rgba, image, live.bgra),
            None => Err(fail("overlay", "swapchain image out of range")),
        };
        // Released even when the copy failed, so the swapchain is never left holding an image.
        chain
            .swapchain
            .release_image()
            .map_err(xr_err("xrReleaseSwapchainImage"))?;
        copied?;
        chain.ready = true;
        Ok(())
    }

    /// The headset (`HMD`) and the controllers (`LEFT`, `RIGHT`) in the room, as they will be when the frame
    /// begun shows. Nothing is valid before the session runs.
    pub fn poses(&mut self) -> Box<[TrackedDevicePose; MAX_DEVICES]> {
        let mut poses = Box::new([TrackedDevicePose::default(); MAX_DEVICES]);
        self.located = [None; 3];
        let Some(time) = self.time.filter(|_| self.running) else {
            return poses;
        };
        let live = &self.live;
        let spaces = [&live.view, &live.grip[0], &live.grip[1]];
        for (i, space) in spaces.into_iter().enumerate() {
            let Some(l) = space.locate(&live.stage, time).ok().filter(located) else {
                continue;
            };
            let m = to_mat(l.pose);
            self.located[i] = Some(m);
            poses[i] = TrackedDevicePose {
                device_to_absolute: m,
                pose_is_valid: true,
                device_is_connected: true,
                ..TrackedDevicePose::default()
            };
        }
        poses
    }

    pub fn controller(role: u32) -> Option<u32> {
        [LEFT, RIGHT].contains(&role).then_some(role)
    }

    /// Follows the session through its states and passes on the end of it as OpenVR's quit event.
    pub fn poll_event(&mut self) -> Option<VrEvent> {
        let quit = VrEvent {
            event_type: EVENT_QUIT,
            ..VrEvent::default()
        };
        let mut buf = xr::EventDataBuffer::new();
        loop {
            let event = match self.instance.poll_event(&mut buf) {
                Ok(Some(e)) => e,
                Ok(None) => return None,
                Err(e) => {
                    tracing::warn!("OpenXR runtime gone: {e}");
                    return Some(quit);
                }
            };
            match event {
                xr::Event::SessionStateChanged(s) => {
                    if !self.session_state(s.state()) {
                        return Some(quit);
                    }
                }
                xr::Event::InstanceLossPending(_) => return Some(quit),
                _ => {}
            }
        }
    }

    /// False once the runtime wants us gone.
    fn session_state(&mut self, state: xr::SessionState) -> bool {
        tracing::info!("OpenXR session {state:?}");
        match state {
            xr::SessionState::READY => {
                match self.live.session.begin(xr::ViewConfigurationType::PRIMARY_STEREO) {
                    Ok(_) => self.running = true,
                    Err(e) => tracing::warn!("xrBeginSession failed: {e}"),
                }
            }
            xr::SessionState::STOPPING => {
                self.end_frame();
                self.running = false;
                self.time = None;
                if let Err(e) = self.live.session.end() {
                    tracing::warn!("xrEndSession failed: {e}");
                }
            }
            xr::SessionState::EXITING | xr::SessionState::LOSS_PENDING => return false,
            _ => {}
        }
        true
    }

    /// Ends the frame begun, then waits for and begins the next one. Without a running session, sleeps
    /// `timeout_ms` instead.
    pub fn wait_frame(&mut self, timeout_ms: u32) {
        self.end_frame();
        let pause = Duration::from_millis(u64::from(timeout_ms));
        if !self.running {
            std::thread::sleep(pause);
            return;
        }
        match self.live.waiter.wait() {
            Ok(state) => match self.live.stream.begin() {
                Ok(()) => {
                    self.time = Some(state.predicted_display_time);
                    self.begun = true;
                }
                Err(e) => tracing::warn!("xrBeginFrame failed: {e}"),
            },
            Err(e) => {
                tracing::warn!("xrWaitFrame failed: {e}");
                std::thread::sleep(pause);
            }
        }
    }

    /// Submits the frame begun, if one is: every shown overlay with a picture, by sort order and back to front
    /// within one, since layers do not test depth against each other.
    fn end_frame(&mut self) {
        if !std::mem::take(&mut self.begun) {
            return;
        }
        let Some(time) = self.time else { return };
        let Live {
            stream,
            stage,
            view,
            grip,
            ..
        } = &mut self.live;
        let located = self.located;
        // Where a layer is in the room, as far as the last poses tell.
        let world = |l: &Layer| match l.anchor {
            Anchor::Stage => Some(position(&l.pose)),
            Anchor::Device(d) => located
                .get(d as usize)
                .copied()
                .flatten()
                .map(|m| position(&compose(&m, &l.pose))),
        };
        let eye = located[HMD as usize].map_or([0.0; 3], |m| position(&m));
        let distance = |l: &Layer| world(l).map_or(0.0, |p| crate::distance(p, eye));
        let ready = |l: &Layer| l.chain.as_ref().is_some_and(|c| c.ready);
        let mut list: Vec<&Layer> = self
            .layers
            .iter()
            .filter(|(k, l)| **k != BLANK && l.shown && l.alpha > 0.001 && ready(l))
            .map(|(_, l)| l)
            .collect();
        list.sort_by(|a, b| a.order.cmp(&b.order).then(distance(b).total_cmp(&distance(a))));
        if list.is_empty() {
            list.extend(self.layers.get(&BLANK).filter(|l| ready(l)));
        }
        let fades: Vec<xr::sys::CompositionLayerColorScaleBiasKHR> = list
            .iter()
            .map(|l| xr::sys::CompositionLayerColorScaleBiasKHR {
                ty: xr::sys::CompositionLayerColorScaleBiasKHR::TYPE,
                next: std::ptr::null(),
                color_scale: xr::Color4f {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: l.alpha.min(1.0),
                },
                color_bias: xr::Color4f {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0,
                },
            })
            .collect();
        let mut quads = Vec::with_capacity(list.len());
        for (l, fade) in list.iter().zip(&fades) {
            let Some(c) = &l.chain else { continue };
            let space = match l.anchor {
                Anchor::Stage => &*stage,
                Anchor::Device(LEFT) => &grip[0],
                Anchor::Device(RIGHT) => &grip[1],
                Anchor::Device(_) => &*view,
            };
            let rect = xr::Rect2Di {
                offset: xr::Offset2Di { x: 0, y: 0 },
                extent: xr::Extent2Di {
                    width: c.size.0 as i32,
                    height: c.size.1 as i32,
                },
            };
            // OpenVR keeps the picture's aspect: the height follows from the width.
            let height_m = l.width_m * c.size.1 as f32 / c.size.0 as f32;
            let quad = xr::CompositionLayerQuad::new()
                .layer_flags(
                    xr::CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA
                        | xr::CompositionLayerFlags::UNPREMULTIPLIED_ALPHA,
                )
                .space(space)
                .eye_visibility(xr::EyeVisibility::BOTH)
                .sub_image(
                    xr::SwapchainSubImage::new()
                        .swapchain(&c.swapchain)
                        .image_rect(rect),
                )
                .pose(to_xr(&l.pose))
                .size(xr::Extent2Df {
                    width: l.width_m,
                    height: height_m,
                });
            if !self.color_scale || l.alpha >= 1.0 {
                quads.push(quad);
                continue;
            }
            // A dimmed watch or a panel fading in: the layer's alpha.
            let mut raw = quad.into_raw();
            raw.next = (fade as *const xr::sys::CompositionLayerColorScaleBiasKHR).cast();
            // SAFETY: the same quad with a well-formed extension struct chained on; `fades` outlives the call below.
            quads.push(unsafe { xr::CompositionLayerQuad::from_raw(raw) });
        }
        let layers: Vec<&xr::CompositionLayerBase<'_, xr::Vulkan>> = quads.iter().map(|q| &**q).collect();
        if let Err(e) = stream.end(time, self.blend, &layers) {
            tracing::warn!("xrEndFrame failed: {e}");
        }
    }

    /// Handles for the overlay's actions and hands, by their OpenVR paths: the action set is the one set, actions go
    /// by the last part of their path (`ACTIONS`), hands are 0 (left) and 1 (right).
    pub fn handle(kind: Handle, path: &str) -> Result<u64, VrError> {
        let unknown = || fail("input handle", path);
        match kind {
            Handle::ActionSet => Ok(0),
            Handle::Action => {
                let name = path.rsplit('/').next().unwrap_or_default();
                let i = ACTIONS.iter().position(|a| *a == name).ok_or_else(unknown)?;
                Ok(i as u64 + 1)
            }
            Handle::Source => match path {
                xr::USER_HAND_LEFT => Ok(0),
                xr::USER_HAND_RIGHT => Ok(1),
                _ => Err(unknown()),
            },
        }
    }

    /// Reads the controllers for this frame. An OpenXR overlay cannot keep a button from the game, so nothing is
    /// taken from it.
    pub fn update_actions(&self) {
        if !self.running {
            return;
        }
        if let Err(e) = self
            .live
            .session
            .sync_actions(&[xr::ActiveActionSet::new(&self.actions.set)])
        {
            tracing::debug!("xrSyncActions failed: {e}");
        }
    }

    fn hand(&self, source: u64) -> Option<xr::Path> {
        self.actions.hands.get(usize::try_from(source).ok()?).copied()
    }

    /// (pressed, changed since the last update) for select or grab on one hand.
    pub fn digital(&self, action: u64, source: u64) -> Option<(bool, bool)> {
        let a = match action {
            SELECT => &self.actions.select,
            GRAB => &self.actions.grab,
            _ => return None,
        };
        let s = a.state(&self.live.session, self.hand(source)?).ok()?;
        s.is_active
            .then_some((s.current_state, s.changed_since_last_sync))
    }

    /// The pointer pose of one hand (OpenXR's aim pose, SteamVR's tip) in the room, when the frame begun shows.
    pub fn pose(&self, action: u64, source: u64) -> Option<Mat> {
        let time = self.time.filter(|_| self.running && action == POINTER)?;
        let space = self.live.aim.get(usize::try_from(source).ok()?)?;
        let l = space.locate(&self.live.stage, time).ok().filter(located)?;
        Some(to_mat(l.pose))
    }

    pub fn haptic(&self, source: u64, seconds: f32, amplitude: f32) {
        let Some(hand) = self.hand(source) else { return };
        let pulse = xr::HapticVibration::new()
            .amplitude(amplitude)
            .frequency(160.0)
            .duration(xr::Duration::from_nanos((seconds * 1e9) as i64));
        if let Err(e) = self
            .actions
            .haptic
            .apply_feedback(&self.live.session, hand, &pulse)
        {
            tracing::debug!("xrApplyHapticFeedback failed: {e}");
        }
    }
}

/// Starts the overlay session on our Vulkan device, with the room, the head and both hands' spaces.
fn start(
    instance: &xr::Instance,
    system: xr::SystemId,
    actions: &Actions,
    gpu: &Gpu,
) -> Result<Live, VrError> {
    let (vk_instance, physical, device, family) = gpu.handles();
    let overlay = xr::sys::SessionCreateInfoOverlayEXTX {
        ty: xr::sys::SessionCreateInfoOverlayEXTX::TYPE,
        next: std::ptr::null(),
        create_flags: Default::default(),
        session_layers_placement: LAYERS_PLACEMENT,
    };
    let binding = xr::sys::GraphicsBindingVulkanKHR {
        ty: xr::sys::GraphicsBindingVulkanKHR::TYPE,
        next: (&raw const overlay).cast(),
        instance: vk_instance as *const c_void,
        physical_device: physical as *const c_void,
        device: device as *const c_void,
        queue_family_index: family,
        queue_index: 0,
    };
    let info = xr::sys::SessionCreateInfo {
        ty: xr::sys::SessionCreateInfo::TYPE,
        next: (&raw const binding).cast(),
        create_flags: Default::default(),
        system_id: system,
    };
    let mut handle = xr::sys::Session::NULL;
    // SAFETY: a well-formed create-info chain (all of it alive for the call) with live Vulkan handles from the
    // device the runtime picked; the instance's own function table.
    let r = unsafe { (instance.fp().create_session)(instance.as_raw(), &info, &mut handle) };
    if r.into_raw() < 0 {
        return Err(xr_err("xrCreateSession (overlay)")(r));
    }
    // SAFETY: a new session of this instance, made for Vulkan, not inside a frame.
    let (session, waiter, stream) =
        unsafe { xr::Session::<xr::Vulkan>::from_raw(instance.clone(), handle, Box::new(())) };
    let formats = session
        .enumerate_swapchain_formats()
        .map_err(xr_err("xrEnumerateSwapchainFormats"))?;
    let (format, bgra) = FORMATS
        .into_iter()
        .find(|(f, _)| formats.contains(f))
        .ok_or_else(|| fail("OpenXR", "the runtime offers no 8-bit RGBA or BGRA swapchains"))?;
    let space = |ty| session.create_reference_space(ty, xr::Posef::IDENTITY);
    let stage = space(xr::ReferenceSpaceType::STAGE).map_err(xr_err("xrCreateReferenceSpace (stage)"))?;
    let view = space(xr::ReferenceSpaceType::VIEW).map_err(xr_err("xrCreateReferenceSpace (view)"))?;
    session
        .attach_action_sets(&[&actions.set])
        .map_err(xr_err("xrAttachSessionActionSets"))?;
    let hand = |action: &xr::Action<xr::Posef>, i: usize| {
        action
            .create_space(&session, actions.hands[i], xr::Posef::IDENTITY)
            .map_err(xr_err("xrCreateActionSpace"))
    };
    let aim = [hand(&actions.aim, 0)?, hand(&actions.aim, 1)?];
    let grip = [hand(&actions.grip, 0)?, hand(&actions.grip, 1)?];
    Ok(Live {
        session,
        waiter,
        stream,
        stage,
        view,
        aim,
        grip,
        format,
        bgra,
    })
}
