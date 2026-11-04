//! Thin binding over the OpenVR flat C API. The runtime is loaded at run time (the
//! `openvr_api` library next to the exe, else the one inside the SteamVR install), so the build
//! needs no SDK and the app starts fine without SteamVR. All `unsafe` in this crate lives here.

mod tables;
mod vulkan;

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::{Path, PathBuf};

use tables::{IVRApplicationsFn, IVRCompositorFn, IVRInputFn, IVROverlayFn, IVRSystemFn};
pub use vulkan::Gpu;

pub type Slot = *const c_void;

#[repr(C)]
pub struct Texture {
    pub handle: *mut c_void,
    pub texture_type: u32,
    pub color_space: u32,
}

#[repr(C)]
pub struct VulkanTextureData {
    pub image: u64,
    pub device: *mut c_void,
    pub physical_device: *mut c_void,
    pub instance: *mut c_void,
    pub queue: *mut c_void,
    pub queue_family_index: u32,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub sample_count: u32,
}

const TEXTURE_VULKAN: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HmdMatrix34 {
    pub m: [[f32; 4]; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TrackedDevicePose {
    pub device_to_absolute: HmdMatrix34,
    pub velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
    pub tracking_result: u32,
    pub pose_is_valid: bool,
    pub device_is_connected: bool,
}

/// `VREvent_t`. Mis-packed to 4 bytes on Linux in the SDK; mirrored so the size check passes.
#[repr(C)]
#[cfg_attr(unix, repr(packed(4)))]
#[derive(Clone, Copy, Default)]
pub struct VrEvent {
    pub event_type: u32,
    pub tracked_device_index: u32,
    pub event_age_seconds: f32,
    pub data: [u64; 6],
}
const _: () = assert!(size_of::<VrEvent>() == if cfg!(unix) { 60 } else { 64 });

#[repr(C)]
pub struct IntersectionParams {
    pub source: [f32; 3],
    pub direction: [f32; 3],
    pub origin: u32,
}

#[repr(C)]
#[derive(Default)]
pub struct IntersectionResults {
    pub point: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub distance: f32,
}

#[repr(C)]
#[derive(Default)]
pub struct DigitalActionData {
    pub active: bool,
    pub active_origin: u64,
    pub state: bool,
    pub changed: bool,
    pub update_time: f32,
}

#[repr(C)]
#[derive(Default)]
pub struct ActiveActionSet {
    pub action_set: u64,
    pub restricted_to_device: u64,
    pub secondary_action_set: u64,
    pub padding: u32,
    pub priority: i32,
}

pub const MAX_DEVICES: usize = 64;
pub const HMD: u32 = 0;
pub const INVALID_DEVICE: u32 = u32::MAX;
pub const STANDING: u32 = 1;
pub const ROLE_LEFT: u32 = 1;
pub const ROLE_RIGHT: u32 = 2;
pub const EVENT_SCENE_CHANGED: u32 = 404;
pub const EVENT_QUIT: u32 = 700;
const APP_OVERLAY: u32 = 2;
const APP_UTILITY: u32 = 4;
const PROP_NAME: u32 = 0;
const ERR_APP_KEY_EXISTS: u32 = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppKind {
    /// Long-running watch and panel.
    Overlay,
    /// One-shot calls such as registering the manifest.
    Utility,
}

#[derive(Debug)]
pub struct VrError {
    pub call: &'static str,
    pub code: u32,
    pub detail: String,
}

impl std::fmt::Display for VrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} failed ({}){}",
            self.call,
            self.code,
            if self.detail.is_empty() { "" } else { ": " }
        )?;
        f.write_str(&self.detail)
    }
}

impl std::error::Error for VrError {}

fn err(call: &'static str, code: u32) -> VrError {
    VrError {
        call,
        code,
        detail: String::new(),
    }
}

fn check(call: &'static str, code: u32) -> Result<(), VrError> {
    if code == 0 { Ok(()) } else { Err(err(call, code)) }
}

fn cstr(call: &'static str, s: &str) -> Result<CString, VrError> {
    CString::new(s).map_err(|_| VrError {
        call,
        code: 0,
        detail: "string contains NUL".into(),
    })
}

type InitFn = unsafe extern "C" fn(err: *mut u32, kind: u32, startup: *const c_char) -> u32;
type ShutdownFn = unsafe extern "C" fn();
type InterfaceFn = unsafe extern "C" fn(name: *const c_char, err: *mut u32) -> isize;
type ErrorTextFn = unsafe extern "C" fn(err: u32) -> *const c_char;

#[cfg(windows)]
mod os {
    use std::ffi::{c_char, c_void};

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
    }

    pub const LIB: &str = "openvr_api.dll";
    pub const RUNTIME_SUBDIR: &str = "bin/win64";

    pub fn open(path: &std::path::Path) -> *mut c_void {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: `wide` is NUL-terminated and outlives the call.
        unsafe { LoadLibraryW(wide.as_ptr()) }
    }

    pub fn sym(lib: *mut c_void, name: &std::ffi::CStr) -> *mut c_void {
        // SAFETY: `lib` is a live module handle and `name` is NUL-terminated.
        unsafe { GetProcAddress(lib, name.as_ptr()) }
    }
}

#[cfg(unix)]
mod os {
    use std::ffi::{c_char, c_int, c_void};

    #[link(name = "dl")]
    unsafe extern "C" {
        fn dlopen(name: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(lib: *mut c_void, name: *const c_char) -> *mut c_void;
    }

    pub const LIB: &str = "libopenvr_api.so";
    #[cfg(target_arch = "aarch64")]
    pub const RUNTIME_SUBDIR: &str = "bin/linuxarm64";
    #[cfg(not(target_arch = "aarch64"))]
    pub const RUNTIME_SUBDIR: &str = "bin/linux64";

    pub fn open(path: &std::path::Path) -> *mut c_void {
        use std::os::unix::ffi::OsStrExt;
        let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
            return std::ptr::null_mut();
        };
        // SAFETY: `c` is NUL-terminated; RTLD_NOW (2) resolves everything up front.
        unsafe { dlopen(c.as_ptr(), 2) }
    }

    pub fn sym(lib: *mut c_void, name: &std::ffi::CStr) -> *mut c_void {
        // SAFETY: `lib` is a live handle from dlopen and `name` is NUL-terminated.
        unsafe { dlsym(lib, name.as_ptr()) }
    }
}

/// SteamVR install dir from `openvrpaths.vrpath`, which SteamVR rewrites on every start.
pub fn runtime_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))?
    };
    let text = std::fs::read_to_string(base.join("openvr").join("openvrpaths.vrpath")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    json["runtime"].as_array()?.first()?.as_str().map(PathBuf::from)
}

fn load() -> Result<*mut c_void, VrError> {
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        candidates.push(dir.join(os::LIB));
    }
    if let Some(rt) = runtime_dir() {
        candidates.push(rt.join(os::RUNTIME_SUBDIR).join(os::LIB));
    }
    candidates
        .into_iter()
        .map(|p| os::open(&p))
        .find(|h| !h.is_null())
        .ok_or_else(|| VrError {
            call: "load openvr_api",
            code: 0,
            detail: "SteamVR not installed".into(),
        })
}

/// A live OpenVR connection. Shuts down on drop.
pub struct Vr {
    sys: &'static IVRSystemFn,
    overlay: &'static IVROverlayFn,
    apps: &'static IVRApplicationsFn,
    input: &'static IVRInputFn,
    interface: InterfaceFn,
    shutdown: ShutdownFn,
}

fn fn_table(interface: InterfaceFn, name: &'static str) -> Result<*const c_void, VrError> {
    let mut code = 0u32;
    // SAFETY: `name` is a NUL-terminated "FnTable:" interface string from the header.
    let p = unsafe { interface(name.as_ptr().cast(), &mut code) } as *const c_void;
    if code != 0 || p.is_null() {
        Err(err("VR_GetGenericInterface", code))
    } else {
        Ok(p)
    }
}

/// Reads one of OpenVR's space-separated extension lists into owned strings.
fn extension_list(read: impl Fn(*mut c_char, u32) -> u32) -> Vec<CString> {
    let len = read(std::ptr::null_mut(), 0);
    let mut buf = vec![0 as c_char; len as usize + 1];
    read(buf.as_mut_ptr(), buf.len() as u32);
    // SAFETY: OpenVR NUL-terminates within the buffer it was given; it was zeroed anyway.
    let text = unsafe { CStr::from_ptr(buf.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    text.split_whitespace()
        .filter_map(|e| CString::new(e).ok())
        .collect()
}

impl Vr {
    /// Vulkan device SteamVR can read textures from. Lives as long as the overlays using it.
    pub fn gpu(&self) -> Result<Gpu, VrError> {
        let ptr = fn_table(self.interface, tables::IVRCOMPOSITOR)?;
        // SAFETY: valid until shutdown, like the other tables.
        let comp: &IVRCompositorFn = unsafe { &*ptr.cast() };
        // SAFETY (both closures): the buffer pointer/size pairs come from `extension_list`.
        let instance_ext = extension_list(|b, n| unsafe { (comp.GetVulkanInstanceExtensionsRequired)(b, n) });
        Gpu::new(
            &instance_ext,
            |instance| {
                let mut dev = 0u64;
                // SAFETY: `instance` is the live VkInstance Gpu::new just created.
                unsafe { (self.sys.GetOutputDevice)(&mut dev, TEXTURE_VULKAN, instance) };
                dev
            },
            |physical| {
                extension_list(|b, n| unsafe { (comp.GetVulkanDeviceExtensionsRequired)(physical, b, n) })
            },
        )
    }

    /// Copies RGBA into the overlay's GPU texture and hands it to SteamVR.
    pub fn set_texture(&self, h: u64, gpu: &mut Gpu, slot: usize, rgba: &[u8]) -> Result<(), VrError> {
        let mut data = gpu.upload(slot, rgba)?;
        let tex = Texture {
            handle: (&raw mut data).cast(),
            texture_type: TEXTURE_VULKAN,
            color_space: 0,
        };
        // SAFETY: `tex` and `data` live across the call; SteamVR reads the image, which stays
        // alive (owned by `gpu`) and in TRANSFER_SRC_OPTIMAL layout as required.
        check("SetOverlayTexture", unsafe {
            (self.overlay.SetOverlayTexture)(h, &tex)
        })
    }

    pub fn init(kind: AppKind) -> Result<Self, VrError> {
        let lib = load()?;
        let sym = |name: &CStr| -> Result<*mut c_void, VrError> {
            let p = os::sym(lib, name);
            if p.is_null() {
                Err(VrError {
                    call: "resolve symbol",
                    code: 0,
                    detail: format!("{name:?}"),
                })
            } else {
                Ok(p)
            }
        };
        // SAFETY: the symbols come from openvr_api and have these exact C signatures
        // (openvr_capi.h); a data pointer and a function pointer have the same size here.
        let (init, shutdown, interface, error_text) = unsafe {
            (
                std::mem::transmute::<*mut c_void, InitFn>(sym(c"VR_InitInternal2")?),
                std::mem::transmute::<*mut c_void, ShutdownFn>(sym(c"VR_ShutdownInternal")?),
                std::mem::transmute::<*mut c_void, InterfaceFn>(sym(c"VR_GetGenericInterface")?),
                std::mem::transmute::<*mut c_void, ErrorTextFn>(sym(
                    c"VR_GetVRInitErrorAsEnglishDescription",
                )?),
            )
        };
        let mut code = 0u32;
        let app_type = if kind == AppKind::Overlay {
            APP_OVERLAY
        } else {
            APP_UTILITY
        };
        // SAFETY: `code` is a valid out pointer; a null startup info is allowed.
        unsafe { init(&mut code, app_type, std::ptr::null()) };
        if code != 0 {
            // SAFETY: returns a static NUL-terminated string for any code.
            let text = unsafe { CStr::from_ptr(error_text(code)) }
                .to_string_lossy()
                .into_owned();
            return Err(VrError {
                call: "VR_Init",
                code,
                detail: text,
            });
        }
        let table = |name: &'static str| fn_table(interface, name);
        let tables = (|| {
            Ok((
                table(tables::IVRSYSTEM)?,
                table(tables::IVROVERLAY)?,
                table(tables::IVRAPPLICATIONS)?,
                table(tables::IVRINPUT)?,
            ))
        })();
        let (sys, overlay, apps, input) = match tables {
            Ok(t) => t,
            Err(e) => {
                // SAFETY: init succeeded, so shutdown is valid and balances it.
                unsafe { shutdown() };
                return Err(e);
            }
        };
        // SAFETY: function tables stay valid until VR_ShutdownInternal, which only Drop calls,
        // and their layout is generated from the same header version we request.
        unsafe {
            Ok(Self {
                sys: &*sys.cast(),
                overlay: &*overlay.cast(),
                apps: &*apps.cast(),
                input: &*input.cast(),
                interface,
                shutdown,
            })
        }
    }

    pub fn poses(&self) -> Box<[TrackedDevicePose; MAX_DEVICES]> {
        let mut poses = Box::new([TrackedDevicePose::default(); MAX_DEVICES]);
        // SAFETY: the buffer holds exactly MAX_DEVICES poses.
        unsafe {
            (self.sys.GetDeviceToAbsoluteTrackingPose)(STANDING, 0.0, poses.as_mut_ptr(), MAX_DEVICES as u32)
        };
        poses
    }

    pub fn controller(&self, role: u32) -> Option<u32> {
        // SAFETY: plain value call.
        let i = unsafe { (self.sys.GetTrackedDeviceIndexForControllerRole)(role) };
        (i != INVALID_DEVICE && (i as usize) < MAX_DEVICES).then_some(i)
    }

    pub fn poll_event(&self) -> Option<VrEvent> {
        let mut ev = VrEvent::default();
        // SAFETY: `ev` is a VREvent_t of exactly the size we pass.
        unsafe { (self.sys.PollNextEvent)(&mut ev, size_of::<VrEvent>() as u32) }.then_some(ev)
    }

    pub fn create_overlay(&self, key: &str, name: &str) -> Result<u64, VrError> {
        let (k, n) = (cstr("CreateOverlay", key)?, cstr("CreateOverlay", name)?);
        let mut handle = 0u64;
        // SAFETY: valid C strings and out pointer.
        check("CreateOverlay", unsafe {
            (self.overlay.CreateOverlay)(k.as_ptr(), n.as_ptr(), &mut handle)
        })?;
        Ok(handle)
    }

    pub fn destroy_overlay(&self, h: u64) {
        // SAFETY: plain value call; an unknown handle only returns an error code.
        let _ = unsafe { (self.overlay.DestroyOverlay)(h) };
    }

    pub fn set_width(&self, h: u64, metres: f32) -> Result<(), VrError> {
        // SAFETY: plain value call.
        check("SetOverlayWidthInMeters", unsafe {
            (self.overlay.SetOverlayWidthInMeters)(h, metres)
        })
    }

    pub fn set_alpha(&self, h: u64, alpha: f32) -> Result<(), VrError> {
        // SAFETY: plain value call.
        check("SetOverlayAlpha", unsafe {
            (self.overlay.SetOverlayAlpha)(h, alpha)
        })
    }

    pub fn set_visible(&self, h: u64, visible: bool) -> Result<(), VrError> {
        // SAFETY: plain value calls.
        let code = unsafe {
            if visible {
                (self.overlay.ShowOverlay)(h)
            } else {
                (self.overlay.HideOverlay)(h)
            }
        };
        check("Show/HideOverlay", code)
    }

    pub fn attach(&self, h: u64, device: u32, m: &HmdMatrix34) -> Result<(), VrError> {
        // SAFETY: `m` is a valid matrix for the duration of the call.
        check("SetOverlayTransformTrackedDeviceRelative", unsafe {
            (self.overlay.SetOverlayTransformTrackedDeviceRelative)(h, device, m)
        })
    }

    pub fn place(&self, h: u64, m: &HmdMatrix34) -> Result<(), VrError> {
        // SAFETY: as above.
        check("SetOverlayTransformAbsolute", unsafe {
            (self.overlay.SetOverlayTransformAbsolute)(h, STANDING, m)
        })
    }

    /// Uploads straight RGBA8. SteamVR copies the buffer before returning.
    pub fn set_raw(&self, h: u64, rgba: &mut [u8], width: u32, height: u32) -> Result<(), VrError> {
        if rgba.len() != (width * height * 4) as usize {
            return Err(VrError {
                call: "SetOverlayRaw",
                code: 0,
                detail: "buffer size".into(),
            });
        }
        // SAFETY: the buffer holds width * height RGBA pixels, checked above.
        check("SetOverlayRaw", unsafe {
            (self.overlay.SetOverlayRaw)(h, rgba.as_mut_ptr().cast(), width, height, 4)
        })
    }

    /// UV of where a ray hits the overlay, if it does.
    pub fn intersect(&self, h: u64, source: [f32; 3], direction: [f32; 3]) -> Option<[f32; 2]> {
        let params = IntersectionParams {
            source,
            direction,
            origin: STANDING,
        };
        let mut res = IntersectionResults::default();
        // SAFETY: both structs are valid for the call.
        unsafe { (self.overlay.ComputeOverlayIntersection)(h, &params, &mut res) }.then_some(res.uv)
    }

    /// (app key, display name) of the running game, if any.
    pub fn scene_app(&self) -> Option<(String, String)> {
        // SAFETY: plain value call.
        let pid = unsafe { (self.apps.GetCurrentSceneProcessId)() };
        if pid == 0 {
            return None;
        }
        let mut key = [0 as c_char; 128];
        // SAFETY: buffer of the advertised length; OpenVR NUL-terminates.
        let code =
            unsafe { (self.apps.GetApplicationKeyByProcessId)(pid, key.as_mut_ptr(), key.len() as u32) };
        if code != 0 {
            return None;
        }
        let mut name = [0 as c_char; 256];
        let mut perr = 0u32;
        // SAFETY: as above; `key` came back NUL-terminated.
        unsafe {
            (self.apps.GetApplicationPropertyString)(
                key.as_ptr(),
                PROP_NAME,
                name.as_mut_ptr(),
                name.len() as u32,
                &mut perr,
            )
        };
        // SAFETY: both buffers are NUL-terminated by OpenVR (zero-initialised otherwise).
        let (key, name) = unsafe { (CStr::from_ptr(key.as_ptr()), CStr::from_ptr(name.as_ptr())) };
        let key = key.to_string_lossy().into_owned();
        let name = if perr == 0 && !name.is_empty() {
            name.to_string_lossy().into_owned()
        } else {
            key.clone()
        };
        Some((key, name))
    }

    pub fn add_manifest(&self, path: &Path) -> Result<(), VrError> {
        let p = cstr("AddApplicationManifest", &path.to_string_lossy())?;
        // SAFETY: valid C string.
        match unsafe { (self.apps.AddApplicationManifest)(p.as_ptr(), false) } {
            0 | ERR_APP_KEY_EXISTS => Ok(()),
            code => Err(err("AddApplicationManifest", code)),
        }
    }

    pub fn set_auto_launch(&self, key: &str, on: bool) -> Result<(), VrError> {
        let k = cstr("SetApplicationAutoLaunch", key)?;
        // SAFETY: valid C string.
        check("SetApplicationAutoLaunch", unsafe {
            (self.apps.SetApplicationAutoLaunch)(k.as_ptr(), on)
        })
    }

    pub fn set_action_manifest(&self, path: &Path) -> Result<(), VrError> {
        let p = cstr("SetActionManifestPath", &path.to_string_lossy())?;
        // SAFETY: valid C string.
        check("SetActionManifestPath", unsafe {
            (self.input.SetActionManifestPath)(p.as_ptr())
        })
    }

    /// Handle for an action set, action or input source path.
    pub fn handle(&self, kind: Handle, path: &str) -> Result<u64, VrError> {
        let p = cstr("input handle", path)?;
        let mut h = 0u64;
        // SAFETY: valid C string and out pointer.
        let code = unsafe {
            match kind {
                Handle::ActionSet => (self.input.GetActionSetHandle)(p.as_ptr(), &mut h),
                Handle::Action => (self.input.GetActionHandle)(p.as_ptr(), &mut h),
                Handle::Source => (self.input.GetInputSourceHandle)(p.as_ptr(), &mut h),
            }
        };
        check("input handle", code)?;
        Ok(h)
    }

    pub fn update_actions(&self, set: u64) -> Result<(), VrError> {
        let mut active = ActiveActionSet {
            action_set: set,
            ..Default::default()
        };
        // SAFETY: one set of the advertised size.
        check("UpdateActionState", unsafe {
            (self.input.UpdateActionState)(&mut active, size_of::<ActiveActionSet>() as u32, 1)
        })
    }

    /// (pressed, changed since last update) for a boolean action on one hand.
    pub fn digital(&self, action: u64, source: u64) -> Option<(bool, bool)> {
        let mut d = DigitalActionData::default();
        // SAFETY: struct of the advertised size.
        let code = unsafe {
            (self.input.GetDigitalActionData)(action, &mut d, size_of::<DigitalActionData>() as u32, source)
        };
        (code == 0 && d.active).then_some((d.state, d.changed))
    }

    pub fn haptic(&self, action: u64, source: u64, seconds: f32, amplitude: f32) {
        // SAFETY: plain value call; failures only mean no buzz.
        let _ = unsafe {
            (self.input.TriggerHapticVibrationAction)(action, 0.0, seconds, 160.0, amplitude, source)
        };
    }
}

#[derive(Clone, Copy)]
pub enum Handle {
    ActionSet,
    Action,
    Source,
}

impl Drop for Vr {
    fn drop(&mut self) {
        // SAFETY: balances the successful VR_InitInternal2 in `init`; tables are not used after.
        unsafe { (self.shutdown)() };
    }
}
