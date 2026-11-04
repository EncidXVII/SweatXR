//! One binary, three jobs:
//!   fitvr                  desktop window (and tray icon on Windows)
//!   fitvr --overlay        the in-VR watch and panel; SteamVR starts this via the .vrmanifest
//!   fitvr --register       register with SteamVR (auto-start follows the setting)
//! Overlay options: --fake-hr (scripted strap), --record FILE (pose trace), --exit-after SECS,
//! --pin-watch (watch fixed in front of the headset, for tests without controllers).

#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod desktop;
mod overlay;
mod setup;
#[cfg(windows)]
mod tray;

use std::error::Error;
use std::path::PathBuf;

#[derive(Default)]
pub struct Opts {
    pub fake_hr: bool,
    pub record: Option<PathBuf>,
    pub exit_after: Option<u64>,
    /// Test harness: watch fixed in front of the headset, always shown (no controllers needed).
    pub pin_watch: bool,
}

enum Mode {
    Desktop,
    Overlay(Opts),
    Register,
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Mode, String> {
    let mut mode = None;
    let mut opts = Opts::default();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--overlay" => mode = Some("overlay"),
            "--register" => mode = Some("register"),
            "--fake-hr" => opts.fake_hr = true,
            "--pin-watch" => opts.pin_watch = true,
            "--record" => opts.record = Some(args.next().ok_or("--record needs a file")?.into()),
            "--exit-after" => {
                opts.exit_after = Some(
                    args.next()
                        .and_then(|s| s.parse().ok())
                        .ok_or("--exit-after needs seconds")?,
                )
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(match mode {
        Some("overlay") => Mode::Overlay(opts),
        Some(_) => Mode::Register,
        None => Mode::Desktop,
    })
}

fn main() {
    let mode = parse(std::env::args().skip(1));
    let dirs = setup::Dirs::new();
    // One log per mode: the desktop app spawns `--register`, which must not truncate its log.
    let log_name = match &mode {
        Ok(Mode::Overlay(_)) => "overlay.log",
        Ok(Mode::Register) => "register.log",
        _ => "desktop.log",
    };
    let log = dirs
        .as_ref()
        .ok()
        .and_then(|d| std::fs::File::create(d.data.join(log_name)).ok());
    let builder = tracing_subscriber::fmt().with_ansi(false).with_target(false);
    match log {
        Some(f) => builder.with_writer(std::sync::Mutex::new(f)).init(),
        None => builder.with_writer(std::io::stderr).init(),
    }

    let result: Result<(), Box<dyn Error>> = (|| {
        let dirs = dirs?;
        match mode? {
            Mode::Desktop => desktop::run(&dirs),
            Mode::Overlay(opts) => overlay::run(&dirs, &opts),
            Mode::Register => setup::register_utility(&dirs),
        }
    })();
    if let Err(e) = result {
        tracing::error!("{e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> impl Iterator<Item = String> {
        s.split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn parses_modes() {
        assert!(matches!(parse(args("")), Ok(Mode::Desktop)));
        assert!(matches!(parse(args("--register")), Ok(Mode::Register)));
        let Ok(Mode::Overlay(o)) = parse(args("--overlay --fake-hr --exit-after 30")) else {
            panic!("overlay")
        };
        assert!(o.fake_hr && o.exit_after == Some(30));
        assert!(parse(args("--exit-after")).is_err());
        assert!(parse(args("--nope")).is_err());
    }
}
