// Copyright (C) 2026 EncidXVII
// SPDX-License-Identifier: GPL-3.0-only

//! WristXR's watch, for when WristXR runs beside SweatXR: both put a watch on the same spot of the same wrist, so
//! ours then starts beside it. WristXR saves its panels in its settings file, the watch among them with its pose
//! relative to the controller's tip, the frame our own watch is placed in; it saves again whenever the watch is
//! moved, so ours follows. Only read, never written; anything unexpected in the file means no neighbour.

use sweatxr_core::profile::Hand;
use sweatxr_vr::{Neighbour, pose_mat};

pub struct Watch {
    /// Its overlay's key in SteamVR: there while WristXR runs.
    pub key: String,
    pub hand: Hand,
    pub neighbour: Neighbour,
}

/// The shown watch in WristXR's state.json, if it has one on a hand.
pub fn parse(text: &str) -> Option<Watch> {
    let state: serde_json::Value = serde_json::from_str(text).ok()?;
    let watch = state
        .get("panels")?
        .as_array()?
        .iter()
        .find(|p| p["source"] == "Wrist")?;
    if watch["visible"] == false {
        return None;
    }
    let hand = match watch["anchor"]["Hand"].as_str()? {
        "Left" => Hand::Left,
        "Right" => Hand::Right,
        _ => return None,
    };
    let number = |v: &serde_json::Value| v.as_f64().map(|n| n as f32).filter(|n| n.is_finite());
    let (pos, rot) = (&watch["pose"]["pos"], &watch["pose"]["rot"]);
    let p = [number(&pos["x"])?, number(&pos["y"])?, number(&pos["z"])?];
    let q = [
        number(&rot["x"])?,
        number(&rot["y"])?,
        number(&rot["z"])?,
        number(&rot["w"])?,
    ];
    let length = q.iter().map(|v| v * v).sum::<f32>().sqrt();
    let (width_m, aspect) = (number(&watch["width_m"])?, number(&watch["aspect"])?);
    // A watch is a few centimetres across and a unit turn: anything else is not one to sit beside.
    if (length - 1.0).abs() > 0.01 || !(0.02..=0.5).contains(&width_m) || !(0.1..=4.0).contains(&aspect) {
        return None;
    }
    Some(Watch {
        key: format!("wxr.panel.{}", watch["id"].as_u64()?),
        hand,
        neighbour: Neighbour {
            pose: pose_mat(p, q.map(|v| v / length)),
            width_m,
            aspect,
        },
    })
}

/// WristXR's watch as its settings file has it now.
pub fn load() -> Option<Watch> {
    let dirs = directories::ProjectDirs::from("", "", "WristXR")?;
    parse(&std::fs::read_to_string(dirs.config_dir().join("state.json")).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sweatxr_vr::position;

    const STATE: &str = r#"{"dominant":"Right","panels":[
        {"id":1,"source":"Keyboard","anchor":"World","pose":{"pos":{"x":0.0,"y":1.0,"z":-1.0},"rot":{"x":0.0,"y":0.0,"z":0.0,"w":1.0}},"width_m":0.6,"aspect":0.4,"visible":true},
        {"id":2,"source":"Wrist","anchor":{"Hand":"Left"},"pose":{"pos":{"x":-0.047,"y":0.077,"z":0.142},"rot":{"x":-0.7537606,"y":0.21293032,"z":0.5768866,"w":0.23174891}},"width_m":0.14,"aspect":1.5,"visible":true},
        {"id":3,"source":{"Monitor":0},"anchor":"World","pose":{"pos":{"x":0.0,"y":1.5,"z":-1.0},"rot":{"x":0.0,"y":0.0,"z":0.0,"w":1.0}},"width_m":1.5,"aspect":0.5625,"visible":true}]}"#;

    #[test]
    fn reads_the_watch_wristxr_saved() {
        let w = parse(STATE).expect("a watch");
        assert_eq!((w.key.as_str(), w.hand), ("wxr.panel.2", Hand::Left));
        assert_eq!((w.neighbour.width_m, w.neighbour.aspect), (0.14, 1.5));
        let p = position(&w.neighbour.pose);
        assert!((p[0] + 0.047).abs() < 1e-6 && (p[2] - 0.142).abs() < 1e-6);
    }

    #[test]
    fn anything_else_is_no_neighbour() {
        assert!(parse("").is_none() && parse("{}").is_none() && parse(r#"{"panels":[]}"#).is_none());
        // Hidden, held in the room, or with numbers no watch has.
        assert!(
            parse(&STATE.replace(
                r#""aspect":1.5,"visible":true"#,
                r#""aspect":1.5,"visible":false"#
            ))
            .is_none()
        );
        assert!(parse(&STATE.replace(r#"{"Hand":"Left"}"#, r#""World""#)).is_none());
        assert!(parse(&STATE.replace(r#""w":0.23174891"#, r#""w":7.0"#)).is_none());
        assert!(parse(&STATE.replace(r#""width_m":0.14"#, r#""width_m":14.0"#)).is_none());
    }
}
