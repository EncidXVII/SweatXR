// Copyright (C) 2026 EncidXVII
// SPDX-License-Identifier: GPL-3.0-only

//! Folders, SteamVR registration (.vrmanifest + auto-start) and the input action manifest.

use std::error::Error;
use std::path::{Path, PathBuf};

use sweatxr_vr::{AppKind, Apps, ERR_NO_SERVER};

pub const APP_KEY: &str = "local.sweatxr.overlay";
/// The watch's overlay. While it exists, the overlay process runs.
pub const WATCH_KEY: &str = "sweatxr.watch";
pub const ACTION_SET: &str = "/actions/main";
pub const ACTION_SELECT: &str = "/actions/main/in/select";
pub const ACTION_HAPTIC: &str = "/actions/main/out/haptic";
/// The controller's tip pose: the same point and axes on every controller model (WristXR uses it too), so the
/// watch sits and turns the same way on Index, Touch and Vive wands. The raw device pose differs per model.
pub const ACTION_POINTER: &str = "/actions/main/in/pointer";
/// Default bindings: trigger click selects, both hands. Players rebind in SteamVR's binding UI.
/// Grip on the pointing hand, held while its beam is on the watch: the watch moves with that hand (as on WristXR).
pub const ACTION_GRAB: &str = "/actions/main/in/grab";
/// The controllers WristXR has bindings for, and a generic one. A controller without a binding file gets no tip
/// pose, and the watch and beam then go by its raw pose: some other place and angle on every model.
const CONTROLLERS: [&str; 6] = [
    "knuckles",
    "oculus_touch",
    "vive_controller",
    "holographic_controller",
    "frame_controller",
    "generic",
];

pub struct Dirs {
    pub data: PathBuf,
    pub export: PathBuf,
}

impl Dirs {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        // SWEATXR_DATA_DIR keeps smoke tests and soaks away from the player's real history.
        let data = match std::env::var_os("SWEATXR_DATA_DIR") {
            Some(d) => PathBuf::from(d),
            None => directories::ProjectDirs::from("", "", "SweatXR")
                .ok_or("no home directory")?
                .data_dir()
                .to_path_buf(),
        };
        std::fs::create_dir_all(&data)?;
        let docs = directories::UserDirs::new().and_then(|u| u.document_dir().map(Path::to_path_buf));
        let export = docs.unwrap_or_else(|| data.clone()).join("SweatXR");
        Ok(Self { data, export })
    }

    pub fn db(&self) -> PathBuf {
        self.data.join("sweatxr.db")
    }
}

fn write_json(path: &Path, value: &serde_json::Value) -> std::io::Result<()> {
    std::fs::write(
        path,
        serde_json::to_string_pretty(value).map_err(std::io::Error::other)?,
    )
}

/// The app icon SteamVR shows next to SweatXR in its startup and overlay lists.
const STEAMVR_IMAGE: &[u8] = include_bytes!("../../../packaging/brand/sweatxr-256.png");

/// Writes the manifest pointing at this exact executable, so a moved install re-registers itself.
pub fn write_vrmanifest(dirs: &Dirs) -> Result<PathBuf, Box<dyn Error>> {
    let image = dirs.data.join("sweatxr.png");
    std::fs::write(&image, STEAMVR_IMAGE)?;
    // Inside an AppImage the exe sits in a mount whose path changes every run; $APPIMAGE is the
    // stable file SteamVR must launch.
    let exe = match std::env::var_os("APPIMAGE") {
        Some(p) => PathBuf::from(p),
        None => std::env::current_exe()?,
    };
    let path = dirs.data.join("sweatxr.vrmanifest");
    let exe_key = if cfg!(windows) {
        "binary_path_windows"
    } else {
        "binary_path_linux"
    };
    write_json(
        &path,
        &serde_json::json!({
            "source": "builtin",
            "applications": [{
                "app_key": APP_KEY,
                "launch_type": "binary",
                exe_key: exe,
                "arguments": "--overlay",
                "image_path": image,
                "is_dashboard_overlay": true,
                "strings": { "en_us": {
                    "name": "SweatXR",
                    "description": "Calories, active minutes and heart rate on your wrist in every game."
                }}
            }]
        }),
    )?;
    Ok(path)
}

pub fn write_action_manifest(dirs: &Dirs) -> std::io::Result<PathBuf> {
    let dir = dirs.data.join("input");
    std::fs::create_dir_all(&dir)?;
    let hands = ["left", "right"];
    for c in CONTROLLERS {
        let mut sources: Vec<_> = hands
            .iter()
            .map(|h| serde_json::json!({ "path": format!("/user/hand/{h}/input/trigger"), "mode": "button", "inputs": { "click": { "output": ACTION_SELECT } } }))
            .collect();
        // Every named controller has a grip; the generic one is not known to.
        if c != "generic" {
            sources.extend(hands.iter().map(|h| serde_json::json!({ "path": format!("/user/hand/{h}/input/grip"), "mode": "button", "inputs": { "click": { "output": ACTION_GRAB } } })));
        }
        let haptics: Vec<_> =
            hands.iter().map(|h| serde_json::json!({ "output": ACTION_HAPTIC, "path": format!("/user/hand/{h}/output/haptic") })).collect();
        let poses: Vec<_> =
            hands.iter().map(|h| serde_json::json!({ "output": ACTION_POINTER, "path": format!("/user/hand/{h}/pose/tip") })).collect();
        write_json(
            &dir.join(format!("bindings_{c}.json")),
            &serde_json::json!({
                "controller_type": c, "name": "SweatXR defaults", "description": "Trigger selects on the watch and panel; grip moves the watch.",
                "bindings": { ACTION_SET: { "sources": sources, "haptics": haptics, "poses": poses } }
            }),
        )?;
    }
    let path = dir.join("actions.json");
    write_json(
        &path,
        &serde_json::json!({
            "actions": [
                { "name": ACTION_SELECT, "type": "boolean" },
                { "name": ACTION_GRAB, "type": "boolean" },
                { "name": ACTION_POINTER, "type": "pose" },
                { "name": ACTION_HAPTIC, "type": "vibration" }
            ],
            "action_sets": [{ "name": ACTION_SET, "usage": "leftright" }],
            "default_bindings": CONTROLLERS.iter().map(|c| serde_json::json!({ "controller_type": c, "binding_url": format!("bindings_{c}.json") })).collect::<Vec<_>>(),
            "localization": [{ "language_tag": "en_US", ACTION_SET: "SweatXR", ACTION_SELECT: "Select on watch and panel", ACTION_GRAB: "Move the watch", ACTION_POINTER: "Pointer", ACTION_HAPTIC: "Haptic tick" }]
        }),
    )?;
    Ok(path)
}

pub fn register(apps: &Apps, dirs: &Dirs, auto_start: bool) -> Result<(), Box<dyn Error>> {
    apps.add_manifest(&write_vrmanifest(dirs)?)?;
    apps.set_auto_launch(APP_KEY, auto_start)?;
    Ok(())
}

/// `sweatxr --register`, spawned by the desktop app. Registers the overlay with SteamVR so it
/// starts with it. If SteamVR is already running and auto-start is on, also starts the overlay
/// now, so nobody has to restart SteamVR after installing. With `start` (the window was just
/// opened) it starts the overlay with auto-start off too: that is how the watch is started by hand.
pub fn register_utility(dirs: &Dirs, start: bool) -> Result<(), Box<dyn Error>> {
    let auto_start = sweatxr_core::storage::Store::open(&dirs.db())?
        .profile()?
        .auto_start;
    // A background connection never starts SteamVR, so it doubles as an "is it running" check.
    match Apps::init(AppKind::Background) {
        Ok(apps) => {
            register(&apps, dirs, auto_start)?;
            if !auto_start && !start {
                tracing::info!("registered with SteamVR; auto-start is off, overlay not started");
            } else if apps.overlay_exists(WATCH_KEY) || apps.running(APP_KEY) {
                tracing::info!("registered with SteamVR; overlay already running");
            } else if apps.launch(APP_KEY)? {
                tracing::info!("registered with SteamVR and started the overlay");
            } else {
                // SteamVR still counts an overlay that was ended behind its back, and starts no second one.
                std::process::Command::new(std::env::current_exe()?)
                    .arg("--overlay")
                    .spawn()?;
                tracing::info!(
                    "registered with SteamVR; it held the overlay for running, so started it here"
                );
            }
        }
        Err(e) if e.code == ERR_NO_SERVER => {
            // Utility apps may only use IVRApplications, which is all registering needs.
            register(&Apps::init(AppKind::Utility)?, dirs, auto_start)?;
            tracing::info!(
                "SteamVR not running; registered, overlay starts with SteamVR (auto-start {auto_start})"
            );
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}
