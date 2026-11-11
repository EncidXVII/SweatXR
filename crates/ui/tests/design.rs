//! AUTO GATE 3 (rules): WCAG AA contrast for every listed token pair, minimum text size, and no
//! colour or font size in the views that bypasses the tokens.

use std::path::Path;

fn tokens() -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../design-tokens.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("tokens")).expect("json")
}

fn luminance(hex: &str) -> f64 {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).expect("hex");
    let lin = |c: u32| {
        let c = f64::from(c) / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(v >> 16 & 0xFF) + 0.7152 * lin(v >> 8 & 0xFF) + 0.0722 * lin(v & 0xFF)
}

fn contrast(a: &str, b: &str) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

#[test]
fn token_pairs_meet_wcag_aa() {
    let t = tokens();
    let color = |k: &serde_json::Value| {
        t["color"][k.as_str().expect("key")]
            .as_str()
            .expect("color")
            .to_owned()
    };
    for (list, min) in [("text_pairs", 4.5), ("ui_pairs", 3.0)] {
        for pair in t["checks"][list].as_array().expect("pairs") {
            let (fg, bg) = (color(&pair[0]), color(&pair[1]));
            let c = contrast(&fg, &bg);
            assert!(c >= min, "{pair}: {c:.2} < {min}");
        }
    }
}

#[test]
fn every_theme_accent_works_where_lime_does() {
    let t = tokens();
    let color = |k: &str| t["color"][k].as_str().expect("color").to_owned();
    let themes = t["themes"].as_array().expect("themes");
    assert_eq!(themes.len(), usize::from(fitvr_core::profile::THEMES));
    assert_eq!(
        themes[0]["accent"], t["color"]["accent"],
        "the first theme is the default accent"
    );
    for theme in themes {
        let (accent, pressed) = (
            theme["accent"].as_str().expect("accent"),
            theme["pressed"].as_str().expect("pressed"),
        );
        let name = &theme["name"];
        for (fg, bg, min) in [
            (color("on_accent"), accent.to_owned(), 4.5),
            (color("on_accent"), pressed.to_owned(), 4.5),
            (accent.to_owned(), color("bg"), 4.5),
            (accent.to_owned(), color("surface"), 3.0),
            (accent.to_owned(), color("raised"), 3.0),
        ] {
            let c = contrast(&fg, &bg);
            assert!(c >= min, "{name}: {fg} on {bg} is {c:.2} < {min}");
        }
    }
}

#[test]
fn text_is_large_enough_for_vr() {
    let t = tokens();
    let min = t["checks"]["min_text_px"].as_u64().expect("min");
    for (k, v) in t["type"].as_object().expect("type") {
        assert!(v.as_u64().expect("px") >= min, "type {k} is {v}px, under {min}px");
    }
    assert!(t["size"]["control"].as_u64() >= t["size"]["target_min"].as_u64());
}

#[test]
fn views_only_use_tokens() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    for entry in std::fs::read_dir(dir).expect("ui dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_none_or(|e| e != "slint") {
            continue;
        }
        for (n, line) in std::fs::read_to_string(&path).expect("read").lines().enumerate() {
            let code = line.split("//").next().unwrap_or_default();
            let hex = code
                .find('#')
                .is_some_and(|i| code[i + 1..].starts_with(|c: char| c.is_ascii_hexdigit()));
            // `root.` passes a size through a component property, which callers set from Tokens.
            let raw_size =
                code.contains("font-size:") && !code.contains("Tokens.") && !code.contains("root.");
            assert!(
                !hex && !raw_size,
                "{}:{}: use Tokens: {line}",
                path.display(),
                n + 1
            );
        }
    }
}
