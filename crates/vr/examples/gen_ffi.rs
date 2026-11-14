//! Regenerates src/ffi/tables.rs from tools/openvr_capi.h:
//!
//!     cargo run -p fitvr-vr --example gen_ffi
//!
//! It writes the file itself: a shell redirect would empty it before this crate rebuilds.
//!
//! Only the slots we call get a typed signature. Every other slot is an opaque pointer: all
//! slots are function pointers, so the layout stays exact without binding the whole API. The
//! interface version strings come from the header too, so updating it is one command.

use std::process::exit;

const USED: &[(&str, &[(&str, &str)])] = &[
    (
        "IVRSystem",
        &[
            (
                "GetDeviceToAbsoluteTrackingPose",
                "(origin: u32, predicted_s: f32, poses: *mut TrackedDevicePose, count: u32)",
            ),
            ("GetTrackedDeviceIndexForControllerRole", "(role: u32) -> u32"),
            ("PollNextEvent", "(event: *mut VrEvent, size: u32) -> bool"),
            (
                "GetOutputDevice",
                "(device: *mut u64, texture_type: u32, instance: *mut c_void)",
            ),
        ],
    ),
    (
        "IVROverlay",
        &[
            (
                "CreateOverlay",
                "(key: *const c_char, name: *const c_char, handle: *mut u64) -> u32",
            ),
            ("DestroyOverlay", "(handle: u64) -> u32"),
            ("SetOverlayAlpha", "(handle: u64, alpha: f32) -> u32"),
            ("SetOverlayWidthInMeters", "(handle: u64, width: f32) -> u32"),
            ("SetOverlaySortOrder", "(handle: u64, order: u32) -> u32"),
            ("WaitFrameSync", "(timeout_ms: u32) -> u32"),
            ("IsDashboardVisible", "() -> bool"),
            (
                "SetOverlayTransformAbsolute",
                "(handle: u64, origin: u32, m: *const HmdMatrix34) -> u32",
            ),
            (
                "SetOverlayTransformTrackedDeviceRelative",
                "(handle: u64, device: u32, m: *const HmdMatrix34) -> u32",
            ),
            ("ShowOverlay", "(handle: u64) -> u32"),
            ("HideOverlay", "(handle: u64) -> u32"),
            (
                "ComputeOverlayIntersection",
                "(handle: u64, params: *const IntersectionParams, results: *mut IntersectionResults) -> bool",
            ),
            (
                "SetOverlayTexture",
                "(handle: u64, texture: *const Texture) -> u32",
            ),
            (
                "SetOverlayRaw",
                "(handle: u64, buffer: *mut c_void, width: u32, height: u32, bytes_per_pixel: u32) -> u32",
            ),
        ],
    ),
    (
        "IVRCompositor",
        &[
            (
                "GetVulkanInstanceExtensionsRequired",
                "(value: *mut c_char, size: u32) -> u32",
            ),
            (
                "GetVulkanDeviceExtensionsRequired",
                "(physical_device: *mut c_void, value: *mut c_char, size: u32) -> u32",
            ),
        ],
    ),
    (
        "IVRApplications",
        &[
            (
                "AddApplicationManifest",
                "(path: *const c_char, temporary: bool) -> u32",
            ),
            ("RemoveApplicationManifest", "(path: *const c_char) -> u32"),
            ("IdentifyApplication", "(pid: u32, key: *const c_char) -> u32"),
            (
                "GetApplicationKeyByProcessId",
                "(pid: u32, key: *mut c_char, len: u32) -> u32",
            ),
            (
                "GetApplicationPropertyString",
                "(key: *const c_char, prop: u32, value: *mut c_char, len: u32, err: *mut u32) -> u32",
            ),
            (
                "SetApplicationAutoLaunch",
                "(key: *const c_char, auto_launch: bool) -> u32",
            ),
            ("GetCurrentSceneProcessId", "() -> u32"),
            ("LaunchApplication", "(key: *const c_char) -> u32"),
            ("GetApplicationProcessId", "(key: *const c_char) -> u32"),
        ],
    ),
    (
        "IVRInput",
        &[
            ("SetActionManifestPath", "(path: *const c_char) -> u32"),
            (
                "GetActionSetHandle",
                "(name: *const c_char, handle: *mut u64) -> u32",
            ),
            (
                "GetActionHandle",
                "(name: *const c_char, handle: *mut u64) -> u32",
            ),
            (
                "GetInputSourceHandle",
                "(path: *const c_char, handle: *mut u64) -> u32",
            ),
            (
                "UpdateActionState",
                "(sets: *mut ActiveActionSet, set_size: u32, count: u32) -> u32",
            ),
            (
                "GetDigitalActionData",
                "(action: u64, data: *mut DigitalActionData, size: u32, restrict: u64) -> u32",
            ),
            (
                "GetPoseActionDataRelativeToNow",
                "(action: u64, origin: u32, seconds_from_now: f32, data: *mut PoseActionData, size: u32, restrict: u64) -> u32",
            ),
            (
                "TriggerHapticVibrationAction",
                "(action: u64, start_s: f32, duration_s: f32, frequency: f32, amplitude: f32, restrict: u64) -> u32",
            ),
        ],
    ),
];

fn fail(msg: String) -> ! {
    eprintln!("gen_ffi: {msg}");
    exit(1)
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = text.find(start)? + start.len();
    Some(&text[from..from + text[from..].find(end)?])
}

fn main() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools/openvr_capi.h");
    let header = std::fs::read_to_string(path).unwrap_or_else(|e| fail(format!("{path}: {e}")));
    let mut out = String::from(
        "// Generated by `cargo run -p fitvr-vr --example gen_ffi` from tools/openvr_capi.h. Do not edit.\n\n\
         use super::{ActiveActionSet, DigitalActionData, HmdMatrix34, IntersectionParams, IntersectionResults, PoseActionData};\n\
         use super::{Slot, Texture, TrackedDevicePose, VrEvent};\n\
         use std::ffi::{c_char, c_void};\n",
    );
    for (table, sigs) in USED {
        let version = between(&header, &format!("{table}_Version = \""), "\"")
            .unwrap_or_else(|| fail(format!("{table}: no version string")));
        let body = between(&header, &format!("struct VR_{table}_FnTable"), "};")
            .unwrap_or_else(|| fail(format!("{table}: no function table")));
        let names: Vec<&str> = body
            .split("OPENVR_FNTABLE_CALLTYPE *")
            .skip(1)
            .filter_map(|s| s.split(')').next())
            .collect();
        let last = sigs
            .iter()
            .map(|(n, _)| {
                names
                    .iter()
                    .position(|x| x == n)
                    .unwrap_or_else(|| fail(format!("{table}: no {n}")))
            })
            .max()
            .unwrap_or(0);
        out += &format!(
            "\npub const {}: &str = \"FnTable:{version}\\0\";\n\n#[repr(C)]\n#[allow(non_snake_case)]\npub struct {table}Fn {{\n",
            table.to_uppercase()
        );
        for name in &names[..=last] {
            let ty = sigs.iter().find(|(n, _)| n == name).map_or_else(
                || "Slot".to_owned(),
                |(_, sig)| format!("unsafe extern \"C\" fn{sig}"),
            );
            out += &format!("    pub {name}: {ty},\n");
        }
        out += "}\n";
    }
    let target = concat!(env!("CARGO_MANIFEST_DIR"), "/src/ffi/tables.rs");
    std::fs::write(target, out).unwrap_or_else(|e| fail(format!("{target}: {e}")));
}
