//! Folders, SteamVR registration (.vrmanifest + auto-start) and the input action manifest.

use std::error::Error;
use std::path::{Path, PathBuf};

use fitvr_vr::{AppKind, Vr};

pub const APP_KEY: &str = "local.fitnessvr.overlay";
pub const ACTION_SET: &str = "/actions/main";
pub const ACTION_SELECT: &str = "/actions/main/in/select";
pub const ACTION_HAPTIC: &str = "/actions/main/out/haptic";
/// Default bindings: trigger click selects, both hands. Players rebind in SteamVR's binding UI.
const CONTROLLERS: [&str; 4] = ["knuckles", "oculus_touch", "vive_controller", "generic"];

pub struct Dirs {
    pub data: PathBuf,
    pub export: PathBuf,
}

impl Dirs {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        // FITVR_DATA_DIR keeps smoke tests and soaks away from the player's real history.
        let data = match std::env::var_os("FITVR_DATA_DIR") {
            Some(d) => PathBuf::from(d),
            None => directories::ProjectDirs::from("", "", "FitnessVR")
                .ok_or("no home directory")?
                .data_dir()
                .to_path_buf(),
        };
        std::fs::create_dir_all(&data)?;
        let docs = directories::UserDirs::new().and_then(|u| u.document_dir().map(Path::to_path_buf));
        let export = docs.unwrap_or_else(|| data.clone()).join("FitnessVR");
        Ok(Self { data, export })
    }

    pub fn db(&self) -> PathBuf {
        self.data.join("fitvr.db")
    }
}

fn write_json(path: &Path, value: &serde_json::Value) -> std::io::Result<()> {
    std::fs::write(
        path,
        serde_json::to_string_pretty(value).map_err(std::io::Error::other)?,
    )
}

/// Writes the manifest pointing at this exact executable, so a moved install re-registers itself.
pub fn write_vrmanifest(dirs: &Dirs) -> Result<PathBuf, Box<dyn Error>> {
    // Inside an AppImage the exe sits in a mount whose path changes every run; $APPIMAGE is the
    // stable file SteamVR must launch.
    let exe = match std::env::var_os("APPIMAGE") {
        Some(p) => PathBuf::from(p),
        None => std::env::current_exe()?,
    };
    let path = dirs.data.join("fitvr.vrmanifest");
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
                "is_dashboard_overlay": true,
                "strings": { "en_us": {
                    "name": "FitnessVR",
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
        let sources: Vec<_> = hands
            .iter()
            .map(|h| serde_json::json!({ "path": format!("/user/hand/{h}/input/trigger"), "mode": "button", "inputs": { "click": { "output": ACTION_SELECT } } }))
            .collect();
        let haptics: Vec<_> =
            hands.iter().map(|h| serde_json::json!({ "output": ACTION_HAPTIC, "path": format!("/user/hand/{h}/output/haptic") })).collect();
        write_json(
            &dir.join(format!("bindings_{c}.json")),
            &serde_json::json!({
                "controller_type": c, "name": "FitnessVR defaults", "description": "Trigger selects on the watch and panel.",
                "bindings": { ACTION_SET: { "sources": sources, "haptics": haptics } }
            }),
        )?;
    }
    let path = dir.join("actions.json");
    write_json(
        &path,
        &serde_json::json!({
            "actions": [{ "name": ACTION_SELECT, "type": "boolean" }, { "name": ACTION_HAPTIC, "type": "vibration" }],
            "action_sets": [{ "name": ACTION_SET, "usage": "leftright" }],
            "default_bindings": CONTROLLERS.iter().map(|c| serde_json::json!({ "controller_type": c, "binding_url": format!("bindings_{c}.json") })).collect::<Vec<_>>(),
            "localization": [{ "language_tag": "en_US", ACTION_SET: "FitnessVR", ACTION_SELECT: "Select on watch and panel", ACTION_HAPTIC: "Haptic tick" }]
        }),
    )?;
    Ok(path)
}

pub fn register(vr: &Vr, dirs: &Dirs, auto_start: bool) -> Result<(), Box<dyn Error>> {
    vr.add_manifest(&write_vrmanifest(dirs)?)?;
    vr.set_auto_launch(APP_KEY, auto_start)?;
    Ok(())
}

/// `fitvr --register`: run by the desktop app so auto-start works before the first VR session.
pub fn register_utility(dirs: &Dirs) -> Result<(), Box<dyn Error>> {
    let auto_start = fitvr_core::storage::Store::open(&dirs.db())?
        .profile()?
        .auto_start;
    let vr = Vr::init(AppKind::Utility)?;
    register(&vr, dirs, auto_start)?;
    tracing::info!("registered with SteamVR, auto-start {auto_start}");
    Ok(())
}
