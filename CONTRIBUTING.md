# Contributing

Thanks for helping. Bug reports, accuracy comparisons and pull requests are all welcome.

## Most useful right now

- **Accuracy reports.** Play 10 minutes with a heart rate strap and a fitness watch or strap app
  running, then file an "Accuracy report" issue with both numbers. Rhythm, boxing and slow games
  all help.
- **Headset coverage.** Does the watch sit well on your controllers (Index, Touch, Frame, Vive)?
- **Steam Frame and Linux testing.**

## Build and test

```bash
cargo build --release -p fitvr
cargo test --workspace
```

Linux needs `libdbus-1-dev pkg-config libxkbcommon-dev libfontconfig1-dev`. No OpenVR SDK is
needed; the runtime is loaded from your SteamVR install.

Before a pull request, run what CI runs:

```bash
cargo fmt --all --check
```

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

```bash
cargo deny check
```

UI changes update the PNG snapshots in `crates/ui/tests/snapshots`. Review them with
`cargo insta review` and commit the new images with your change.

Windows, no headset needed: `scripts\smoke.ps1` runs the overlay against SteamVR's null driver
and checks memory, CPU and the log. It edits SteamVR's settings and restores them at the end.

## Layout

| Crate | What lives there |
|---|---|
| `crates/core` | energy model, sessions, SQLite storage, TCX/CSV export, replay traces |
| `crates/hr` | Bluetooth LE heart rate and the packet parser |
| `crates/vr` | OpenVR binding (runtime-loaded), Vulkan overlay textures, watch/panel geometry |
| `crates/ui` | Slint views shared by watch, panel and desktop; tokens from `design-tokens.json` |
| `crates/app` | the `fitvr` binary: overlay, desktop window, tray, SteamVR registration |

## Rules

- `unsafe` only inside an `ffi` module, each block with a `// SAFETY:` comment.
- No `unwrap()` outside tests; errors are logged or shown, never swallowed silently.
- Colours, type sizes and spacing come from `design-tokens.json` only (a test enforces it).
  Text in VR is at least 24 px, controls at least 48 px, contrast WCAG AA.
- Calories are always called estimates in the UI.
- No telemetry and no network calls.
- Comments explain why, not what. No commented-out code.
- Do not decompile or copy other fitness apps. Behaviour may be observed; code may not be reused.
- New dependencies need a reason in the pull request and must pass `cargo deny`.

## Regenerating the OpenVR binding

After updating `tools/openvr_capi.h`, add any newly used functions to
`crates/vr/examples/gen_ffi.rs`, then run `cargo run -p fitvr-vr --example gen_ffi`. It writes
`crates/vr/src/ffi/tables.rs` itself.

By contributing you agree that your contribution is licensed under GPL-3.0-only.
