// Copyright (C) 2026 EncidXVII
// SPDX-License-Identifier: GPL-3.0-only

//! Thin binding over the OpenVR flat C API. The runtime is loaded at run time (the
//! `openvr_api` library next to the exe, else the one inside the SteamVR install), so the build
//! needs no SDK and the app starts fine without SteamVR. All `unsafe` in this crate lives here and, on Linux, in
//! the OpenXR backend (`xr`).

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

/// `InputPoseActionData_t`.
#[repr(C)]
#[derive(Default)]
pub struct PoseActionData {
    pub active: bool,
    pub active_origin: u64,
    pub pose: TrackedDevicePose,
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

/// `k_nActionSetOverlayGlobalPriorityMin`: input bound at this priority is not delivered to the game.
pub const OVERLAY_PRIORITY: i32 = 0x0100_0000;
pub const MAX_DEVICES: usize = 64;
pub const HMD: u32 = 0;
pub const INVALID_DEVICE: u32 = u32::MAX;
pub const STANDING: u32 = 1;
pub const ROLE_LEFT: u32 = 1;
pub const ROLE_RIGHT: u32 = 2;
pub const EVENT_SCENE_CHANGED: u32 = 404;
pub const EVENT_QUIT: u32 = 700;
const APP_OVERLAY: u32 = 2;
const APP_BACKGROUND: u32 = 3;
const APP_UTILITY: u32 = 4;
const PROP_NAME: u32 = 0;
const ERR_APP_KEY_EXISTS: u32 = 100;
const ERR_APP_ALREADY_RUNNING: u32 = 106;
const ERR_APP_ALREADY_STARTING: u32 = 110;
/// VR_Init code for a background app when SteamVR is not running.
pub const ERR_NO_SERVER: u32 = 121;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppKind {
    /// Long-running watch and panel.
    Overlay,
    /// Connects only if SteamVR is already running; never starts it.
    Background,
    /// Works without SteamVR running; only IVRApplications is available.
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

    /// `PROCESSENTRY32W`.
    #[repr(C)]
    struct ProcessEntry {
        size: u32,
        usage: u32,
        pid: u32,
        heap: usize,
        module: u32,
        threads: u32,
        parent: u32,
        priority: i32,
        flags: u32,
        exe: [u16; 260],
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> *mut c_void;
        fn Process32FirstW(snapshot: *mut c_void, entry: *mut ProcessEntry) -> i32;
        fn Process32NextW(snapshot: *mut c_void, entry: *mut ProcessEntry) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn WaitForSingleObject(handle: *mut c_void, ms: u32) -> u32;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn GetCurrentProcess() -> *mut c_void;
        fn TerminateProcess(process: *mut c_void, code: u32) -> i32;
    }

    const SNAP_PROCESS: u32 = 2;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const WAIT_TIMEOUT: u32 = 258;
    pub const SERVER: &str = "vrserver.exe";

    /// A running process, held open so its end can be seen.
    pub struct Process(*mut c_void);

    // SAFETY: a process handle may be waited on from any thread.
    unsafe impl Send for Process {}

    impl Process {
        /// The first running process with this executable name (case-insensitive).
        pub fn find(exe: &str) -> Option<Self> {
            // SAFETY: a snapshot walk with an entry of the size it states; the snapshot is closed on every path
            // and the process handle is owned by the returned value.
            unsafe {
                let snap = CreateToolhelp32Snapshot(SNAP_PROCESS, 0);
                if snap.is_null() || snap as isize == -1 {
                    return None;
                }
                let mut e: ProcessEntry = std::mem::zeroed();
                e.size = size_of::<ProcessEntry>() as u32;
                let mut pid = None;
                let mut more = Process32FirstW(snap, &mut e) != 0;
                while more {
                    let n = e.exe.iter().position(|c| *c == 0).unwrap_or(e.exe.len());
                    if String::from_utf16_lossy(&e.exe[..n]).eq_ignore_ascii_case(exe) {
                        pid = Some(e.pid);
                        break;
                    }
                    more = Process32NextW(snap, &mut e) != 0;
                }
                CloseHandle(snap);
                Self::open(pid?)
            }
        }

        /// The process with this id, if there is one.
        pub fn open(pid: u32) -> Option<Self> {
            // SAFETY: plain value call; the handle is owned by the returned value.
            let handle = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
            (!handle.is_null()).then_some(Self(handle))
        }

        pub fn alive(&self) -> bool {
            // SAFETY: a zero-timeout wait on a handle we own.
            unsafe { WaitForSingleObject(self.0, 0) == WAIT_TIMEOUT }
        }
    }

    impl Drop for Process {
        fn drop(&mut self) {
            // SAFETY: the handle came from OpenProcess and is closed once.
            unsafe { CloseHandle(self.0) };
        }
    }

    pub fn terminate() -> ! {
        // SAFETY: ends our own process; nothing runs after it.
        unsafe { TerminateProcess(GetCurrentProcess(), 0) };
        std::process::exit(0)
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

    unsafe extern "C" {
        fn _exit(code: c_int) -> !;
    }

    pub const SERVER: &str = "vrserver";

    /// A running process, by its `/proc` entry (Linux; nowhere else is one found).
    pub struct Process(std::path::PathBuf);

    impl Process {
        /// The first running process with this name (`comm`: the executable's name, cut at 15 characters).
        pub fn find(exe: &str) -> Option<Self> {
            std::fs::read_dir("/proc")
                .ok()?
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy().parse::<u32>().is_ok())
                })
                .find(|p| std::fs::read_to_string(p.join("comm")).is_ok_and(|c| c.trim_end() == exe))
                .map(Self)
        }

        pub fn open(pid: u32) -> Option<Self> {
            let p = Self(std::path::PathBuf::from(format!("/proc/{pid}")));
            p.alive().then_some(p)
        }

        pub fn alive(&self) -> bool {
            self.0.exists()
        }
    }

    pub fn terminate() -> ! {
        // SAFETY: ends our own process without running exit handlers; nothing runs after it.
        unsafe { _exit(0) }
    }
}

/// SteamVR's server process, to see it end when it could not say so (a crash, a kill): no quit event comes then.
pub struct Server(os::Process);

impl Server {
    pub fn find() -> Option<Self> {
        os::Process::find(os::SERVER).map(Self)
    }

    pub fn alive(&self) -> bool {
        self.0.alive()
    }
}

/// Ends this process at once, skipping teardown: for when SteamVR is gone and one of its calls never returns.
/// Everything worth keeping must already be saved.
pub fn terminate() -> ! {
    os::terminate()
}

/// SteamVR install dir from `openvrpaths.vrpath`, which SteamVR rewrites on every start.
pub fn runtime_dir() -> Option<PathBuf> {
    vrpath("runtime")
}

/// SteamVR's config folder (`appconfig.json`, `vrappconfig`), from `openvrpaths.vrpath`.
pub fn config_dir() -> Option<PathBuf> {
    vrpath("config")
}

fn vrpath(entry: &str) -> Option<PathBuf> {
    let base = if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))?
    };
    let text = std::fs::read_to_string(base.join("openvr").join("openvrpaths.vrpath")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    json[entry].as_array()?.first()?.as_str().map(PathBuf::from)
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

/// A live OpenVR connection with the full set of interfaces. Shuts down on drop (via `apps`).
pub struct Vr {
    sys: &'static IVRSystemFn,
    overlay: &'static IVROverlayFn,
    input: &'static IVRInputFn,
    interface: InterfaceFn,
    apps: Apps,
}

/// An OpenVR connection that only manages applications: registering, auto-start, launching.
/// Utility apps may not ask for anything else. Shuts down on drop.
pub struct Apps {
    t: &'static IVRApplicationsFn,
    shutdown: ShutdownFn,
}

/// Loads the runtime and calls VR_Init. On success the caller owns one VR_Shutdown.
fn connect(kind: AppKind) -> Result<(InterfaceFn, ShutdownFn), VrError> {
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
            std::mem::transmute::<*mut c_void, ErrorTextFn>(sym(c"VR_GetVRInitErrorAsEnglishDescription")?),
        )
    };
    let mut code = 0u32;
    let app_type = match kind {
        AppKind::Overlay => APP_OVERLAY,
        AppKind::Background => APP_BACKGROUND,
        AppKind::Utility => APP_UTILITY,
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
    Ok((interface, shutdown))
}

impl Apps {
    pub fn init(kind: AppKind) -> Result<Self, VrError> {
        let (interface, shutdown) = connect(kind)?;
        Self::from(interface, shutdown)
    }

    /// Takes ownership of the connection: shuts it down if the table is missing.
    fn from(interface: InterfaceFn, shutdown: ShutdownFn) -> Result<Self, VrError> {
        match fn_table(interface, tables::IVRAPPLICATIONS) {
            // SAFETY: the table stays valid until VR_ShutdownInternal, which only Drop calls.
            Ok(t) => Ok(Self {
                t: unsafe { &*t.cast() },
                shutdown,
            }),
            Err(e) => {
                // SAFETY: init succeeded, so shutdown is valid and balances it.
                unsafe { shutdown() };
                Err(e)
            }
        }
    }

    /// (app key, display name) of the running game, if any.
    pub fn scene_app(&self) -> Option<(String, String)> {
        // SAFETY: plain value call.
        let pid = unsafe { (self.t.GetCurrentSceneProcessId)() };
        if pid == 0 {
            return None;
        }
        let mut key = [0 as c_char; 128];
        // SAFETY: buffer of the advertised length; OpenVR NUL-terminates.
        let code = unsafe { (self.t.GetApplicationKeyByProcessId)(pid, key.as_mut_ptr(), key.len() as u32) };
        if code != 0 {
            return None;
        }
        let mut name = [0 as c_char; 256];
        let mut perr = 0u32;
        // SAFETY: as above; `key` came back NUL-terminated.
        unsafe {
            (self.t.GetApplicationPropertyString)(
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
        match unsafe { (self.t.AddApplicationManifest)(p.as_ptr(), false) } {
            0 | ERR_APP_KEY_EXISTS => Ok(()),
            code => Err(err("AddApplicationManifest", code)),
        }
    }

    /// Takes the app off SteamVR's list (it no longer starts with SteamVR).
    pub fn remove_manifest(&self, path: &Path) -> Result<(), VrError> {
        let p = cstr("RemoveApplicationManifest", &path.to_string_lossy())?;
        // SAFETY: valid C string.
        check("RemoveApplicationManifest", unsafe {
            (self.t.RemoveApplicationManifest)(p.as_ptr())
        })
    }

    /// Tells SteamVR this process is the app `key`. Before the action manifest, so its bindings are filed under
    /// the app's own key even when SteamVR did not start it (else the buttons only work after a restart).
    pub fn identify(&self, key: &str) -> Result<(), VrError> {
        let k = cstr("IdentifyApplication", key)?;
        // SAFETY: valid C string; plain value call.
        check("IdentifyApplication", unsafe {
            (self.t.IdentifyApplication)(std::process::id(), k.as_ptr())
        })
    }

    pub fn set_auto_launch(&self, key: &str, on: bool) -> Result<(), VrError> {
        let k = cstr("SetApplicationAutoLaunch", key)?;
        // SAFETY: valid C string.
        check("SetApplicationAutoLaunch", unsafe {
            (self.t.SetApplicationAutoLaunch)(k.as_ptr(), on)
        })
    }

    /// Whether this app's process is there, other than the one asking. SteamVR takes any process started from the
    /// app's executable for the app, this one included, and goes on naming one that was ended without telling it
    /// (Task Manager, a crash), so the process itself is looked up too.
    pub fn running(&self, key: &str) -> bool {
        // SAFETY: valid C string; plain value call.
        let pid = cstr("GetApplicationProcessId", key)
            .map_or(0, |k| unsafe { (self.t.GetApplicationProcessId)(k.as_ptr()) });
        pid != 0 && pid != std::process::id() && os::Process::open(pid).is_some_and(|p| p.alive())
    }

    /// Asks SteamVR to start a registered app now. False when SteamVR holds it for running or starting already
    /// and so started nothing.
    pub fn launch(&self, key: &str) -> Result<bool, VrError> {
        let k = cstr("LaunchApplication", key)?;
        // SAFETY: valid C string.
        match unsafe { (self.t.LaunchApplication)(k.as_ptr()) } {
            0 => Ok(true),
            ERR_APP_ALREADY_RUNNING | ERR_APP_ALREADY_STARTING => Ok(false),
            code => Err(err("LaunchApplication", code)),
        }
    }
}

impl Drop for Apps {
    fn drop(&mut self) {
        // SAFETY: balances the successful VR_InitInternal2 in `connect`; tables are not used after.
        unsafe { (self.shutdown)() };
    }
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

    /// Full connection for the overlay (`AppKind::Overlay` or `Background`).
    pub fn init(kind: AppKind) -> Result<Self, VrError> {
        let (interface, shutdown) = connect(kind)?;
        // From here `apps` owns the connection: an early return shuts it down.
        let apps = Apps::from(interface, shutdown)?;
        let table = |name: &'static str| fn_table(interface, name);
        let (sys, overlay, input) = (
            table(tables::IVRSYSTEM)?,
            table(tables::IVROVERLAY)?,
            table(tables::IVRINPUT)?,
        );
        // SAFETY: function tables stay valid until VR_ShutdownInternal, which only Apps' Drop
        // calls, and their layout is generated from the same header version we request.
        unsafe {
            Ok(Self {
                sys: &*sys.cast(),
                overlay: &*overlay.cast(),
                input: &*input.cast(),
                interface,
                apps,
            })
        }
    }

    /// The application half of this connection (registering, scene app, launching). Always there on SteamVR; an
    /// OpenXR runtime has none (see `Vr` on Linux).
    pub fn apps(&self) -> Option<&Apps> {
        Some(&self.apps)
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

    /// Whether an overlay with this key exists right now, whichever app made it (WristXR's watch).
    pub fn overlay_exists(&self, key: &str) -> bool {
        let Ok(k) = cstr("FindOverlay", key) else {
            return false;
        };
        let mut handle = 0u64;
        // SAFETY: valid C string and out pointer; an unknown key only returns an error code.
        unsafe { (self.overlay.FindOverlay)(k.as_ptr(), &mut handle) == 0 }
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

    /// Higher draws on top of overlays at the same depth (the pointer dot over the panel).
    pub fn set_sort_order(&self, h: u64, order: u32) -> Result<(), VrError> {
        // SAFETY: plain value call.
        check("SetOverlaySortOrder", unsafe {
            (self.overlay.SetOverlaySortOrder)(h, order)
        })
    }

    /// Blocks until the compositor starts its next frame, so poses and overlay moves keep the
    /// headset's own rate. Sleeps for the timeout instead when there is no frame to wait on.
    pub fn wait_frame(&self, timeout_ms: u32) {
        // SAFETY: plain value call.
        if unsafe { (self.overlay.WaitFrameSync)(timeout_ms) } != 0 {
            std::thread::sleep(std::time::Duration::from_millis(u64::from(timeout_ms)));
        }
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
    /// (UV, world point) where a ray hits the overlay, if it does.
    pub fn intersect(&self, h: u64, source: [f32; 3], direction: [f32; 3]) -> Option<([f32; 2], [f32; 3])> {
        let params = IntersectionParams {
            source,
            direction,
            origin: STANDING,
        };
        let mut res = IntersectionResults::default();
        // SAFETY: both structs are valid for the call.
        unsafe { (self.overlay.ComputeOverlayIntersection)(h, &params, &mut res) }
            .then_some((res.uv, res.point))
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

    /// Updates the action set once per hand (`hands`: their input source handles). A hand marked in `take` gets
    /// overlay priority, so the game stops seeing that hand's bound buttons until it is let go of.
    pub fn update_actions(&self, set: u64, hands: [u64; 2], take: [bool; 2]) -> Result<(), VrError> {
        let mut active = [0, 1].map(|i| ActiveActionSet {
            action_set: set,
            restricted_to_device: hands[i],
            priority: if take[i] { OVERLAY_PRIORITY } else { 0 },
            ..Default::default()
        });
        // SAFETY: two sets of the advertised size.
        check("UpdateActionState", unsafe {
            (self.input.UpdateActionState)(active.as_mut_ptr(), size_of::<ActiveActionSet>() as u32, 2)
        })
    }

    /// SteamVR's dashboard is open (it takes the controller buttons while it is).
    pub fn dashboard_visible(&self) -> bool {
        // SAFETY: plain value call.
        unsafe { (self.overlay.IsDashboardVisible)() }
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

    /// Pose action on one hand (standing space), at the same moment as `poses()`.
    pub fn pose(&self, action: u64, source: u64) -> Option<HmdMatrix34> {
        let mut d = PoseActionData::default();
        // SAFETY: struct of the advertised size.
        let code = unsafe {
            (self.input.GetPoseActionDataRelativeToNow)(
                action,
                STANDING,
                0.0,
                &mut d,
                size_of::<PoseActionData>() as u32,
                source,
            )
        };
        (code == 0 && d.active && d.pose.pose_is_valid).then_some(d.pose.device_to_absolute)
    }

    /// Pose action on one hand (standing space) as it will be when the next frame shows: what the pointer goes by,
    /// as on WristXR, so what it draws this frame is where the hand is when the frame is seen.
    pub fn pose_next(&self, action: u64, source: u64) -> Option<HmdMatrix34> {
        let mut d = PoseActionData::default();
        // SAFETY: struct of the advertised size.
        let code = unsafe {
            (self.input.GetPoseActionDataForNextFrame)(
                action,
                STANDING,
                &mut d,
                size_of::<PoseActionData>() as u32,
                source,
            )
        };
        (code == 0 && d.active && d.pose.pose_is_valid).then_some(d.pose.device_to_absolute)
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

#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests {
    use super::os::Process;

    #[test]
    fn finds_a_running_process_by_name_and_sees_it_alive() {
        let me = if cfg!(windows) {
            let exe = std::env::current_exe().expect("exe");
            exe.file_name().expect("name").to_string_lossy().into_owned()
        } else {
            std::fs::read_to_string("/proc/self/comm")
                .expect("comm")
                .trim_end()
                .to_owned()
        };
        assert!(Process::find(&me).is_some_and(|p| p.alive()), "{me}");
        assert!(Process::find("sweatxr-no-such-process").is_none());
    }
}
