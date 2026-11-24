# Third-party notices

## Bundled files

- **Anton** font (`crates/ui/ui/fonts/Anton-Regular.ttf`), Copyright 2020 The Anton Project
  Authors. SIL Open Font License 1.1, full text in `crates/ui/ui/fonts/Anton-OFL.txt`.
- **Barlow** font, Regular and SemiBold (`crates/ui/ui/fonts/Barlow-*.ttf`), Copyright 2017 The Barlow Project
  Authors. SIL Open Font License 1.1, full text in `crates/ui/ui/fonts/Barlow-OFL.txt`.
- **openvr_capi.h** (`tools/openvr_capi.h`), Copyright Valve Corporation. BSD 3-Clause
  License. Used only to generate `crates/vr/src/ffi/tables.rs`.

## Runtime

OpenVR (`openvr_api`) is loaded from the user's SteamVR install and not redistributed. On Linux without SteamVR,
the system's OpenXR loader (`libopenxr_loader.so.1`) is loaded the same way.

## Rust crates

Compiled-in crates and their licenses: run `cargo deny list` in the repository. All are
GPL-3.0 compatible (MIT, Apache-2.0, BSD, ISC, Zlib, MPL-2.0, Unicode-3.0, BSL-1.0, CC0, and
Slint under its GPL-3.0 option); `cargo deny check licenses` enforces this in CI.
