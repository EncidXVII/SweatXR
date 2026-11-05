//! design-tokens.json is the single source of colours, type, spacing and motion. It becomes the
//! `Tokens` Slint global here, so a value can never drift between the watch, panel and desktop.

use std::collections::HashMap;
use std::error::Error;
use std::fmt::Write;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
    let tokens_path = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?).join("../../design-tokens.json");
    println!("cargo:rerun-if-changed={}", tokens_path.display());
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&tokens_path)?)?;

    let mut s = String::from(
        "// Generated from design-tokens.json by build.rs.
         export global Mode {
    in-out property <bool> desktop;
    in-out property <bool> reduce-motion;
}
         export global Tokens {
",
    );
    let group = |name: &str| json[name].as_object().ok_or(format!("tokens: missing {name}"));
    let mut line = |ty: &str, name: String, value: String| {
        writeln!(s, "    out property <{ty}> {}: {value};", name.replace('_', "-"))
    };
    for (k, v) in group("color")? {
        line(
            "color",
            k.clone(),
            v.as_str().ok_or("color must be a string")?.to_owned(),
        )?;
    }
    for (k, v) in group("font")? {
        line("string", format!("font-{k}"), format!("{v}"))?;
    }
    // VR values are the default; `desktop` overrides apply when Mode.desktop is set, so a
    // desk-distance window gets denser type and controls from the same tokens.
    for (prefix, unit) in [("type", "px"), ("space", "px"), ("radius", "px"), ("size", "px")] {
        for (k, v) in group(prefix)? {
            let value = match json["desktop"][prefix].get(k) {
                Some(d) => format!("Mode.desktop ? {d}{unit} : {v}{unit}"),
                None => format!("{v}{unit}"),
            };
            line("length", format!("{prefix}-{k}"), value)?;
        }
    }
    for (k, v) in group("motion")? {
        line(
            "duration",
            format!("motion-{}", k.trim_end_matches("_ms")),
            format!("{v}ms"),
        )?;
    }
    s.push_str("}\n");

    let out = PathBuf::from(std::env::var("OUT_DIR")?).join("tokens.slint");
    std::fs::write(&out, s)?;
    let config = slint_build::CompilerConfiguration::new()
        .with_library_paths(HashMap::from([("tokens".to_owned(), out)]));
    slint_build::compile_with_config("ui/app.slint", config)?;
    Ok(())
}
