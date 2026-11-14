//! `fitvr --uninstall` (`Uninstall FitnessVR.cmd` in the Windows zip, `uninstall.sh` in the Linux archive): there
//! is no installer, so this undoes what running it set up. It takes FitnessVR off SteamVR's list (no more start
//! with SteamVR) and deletes its data: settings and workout history. Exported files (Documents/FitnessVR) stay, and
//! the program itself is the user's to delete. Never starts SteamVR.
use std::error::Error;
use std::path::Path;

use fitvr_vr::{AppKind, Apps, ERR_NO_SERVER, config_dir};

use crate::setup::{APP_KEY, Dirs};

/// What happened, for the wrapper scripts to show (a Windows GUI program has no console to print to).
pub fn report_path() -> std::path::PathBuf {
    std::env::temp_dir().join("fitvr-uninstall.txt")
}

pub fn run(dirs: &Dirs) -> Result<(), Box<dyn Error>> {
    let result = uninstall(dirs);
    let text = match &result {
        Ok(()) => "FitnessVR is off SteamVR's list and its data is deleted.".to_owned(),
        Err(e) => format!("Not everything could be removed: {e}"),
    };
    let _ = std::fs::write(report_path(), format!("{text}\n"));
    result
}

fn uninstall(dirs: &Dirs) -> Result<(), Box<dyn Error>> {
    let manifest = dirs.data.join("fitvr.vrmanifest");
    // A background connection never starts SteamVR, so it doubles as an "is it running" check.
    match Apps::init(AppKind::Background) {
        Ok(apps) => {
            if apps.running(APP_KEY) {
                return Err(
                    "the FitnessVR watch is running in SteamVR. Quit SteamVR, then uninstall.".into(),
                );
            }
            let _ = apps.set_auto_launch(APP_KEY, false);
            let _ = apps.remove_manifest(&manifest);
        }
        Err(e) if e.code == ERR_NO_SERVER => {
            // As registering does with SteamVR off; SteamVR's list file is edited too, in case that did not stick.
            if let Ok(apps) = Apps::init(AppKind::Utility) {
                let _ = apps.set_auto_launch(APP_KEY, false);
                let _ = apps.remove_manifest(&manifest);
            }
            if let Some(config) = config_dir() {
                drop_manifest(&config.join("appconfig.json"), &manifest)?;
            }
        }
        Err(e) => return Err(e.into()),
    }
    if let Some(config) = config_dir() {
        let settings = config.join("vrappconfig").join(format!("{APP_KEY}.vrappconfig"));
        match std::fs::remove_file(&settings) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(format!("{}: {e}", settings.display()).into());
            }
            _ => {}
        }
    }
    match std::fs::remove_dir_all(&dirs.data) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            return Err(format!(
                "{}: {e}. Quit FitnessVR (tray icon > Quit), then try again.",
                dirs.data.display()
            )
            .into());
        }
        _ => {}
    }
    // Windows keeps the data in `...\FitnessVR\data`: the emptied `FitnessVR` folder goes too (never anything else).
    if let Some(parent) = dirs
        .data
        .parent()
        .filter(|p| p.file_name().is_some_and(|n| n == "FitnessVR"))
    {
        let _ = std::fs::remove_dir(parent);
    }
    Ok(())
}

/// Drops the manifest from SteamVR's `appconfig.json` list (used with SteamVR off).
fn drop_manifest(file: &Path, manifest: &Path) -> Result<(), Box<dyn Error>> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return Ok(());
    };
    let mut json: serde_json::Value = serde_json::from_str(&text)?;
    let norm = |p: &str| p.replace('\\', "/").to_lowercase();
    let ours = norm(&manifest.to_string_lossy());
    if let Some(list) = json["manifest_paths"].as_array_mut() {
        let before = list.len();
        list.retain(|p| p.as_str().is_none_or(|p| norm(p) != ours));
        if list.len() != before {
            std::fs::write(file, serde_json::to_string_pretty(&json)?)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uninstalling_drops_only_our_manifest_from_steamvrs_list() {
        let dir = std::env::temp_dir().join(format!("fitvr-appconfig-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let file = dir.join("appconfig.json");
        let ours = r"C:\Users\Someone\AppData\Roaming\FitnessVR\data\fitvr.vrmanifest";
        let theirs = r"C:\Program Files\Other\other.vrmanifest";
        let list = serde_json::json!({ "manifest_paths": [ours.to_uppercase().replace('\\', "/"), theirs] });
        std::fs::write(&file, list.to_string()).expect("write");
        drop_manifest(&file, Path::new(ours)).expect("drop");
        let back: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&file).expect("read")).expect("json");
        assert_eq!(back["manifest_paths"], serde_json::json!([theirs]));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
