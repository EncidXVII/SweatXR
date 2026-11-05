# Third-party notices

## Bundled files

- **Anton** font (`crates/ui/ui/fonts/Anton-Regular.ttf`), Copyright 2020 The Anton Project
  Authors. SIL Open Font License 1.1, full text in `crates/ui/ui/fonts/Anton-OFL.txt`.
- **Ubuntu** font, Regular (`crates/ui/ui/fonts/Ubuntu-Regular.ttf`), Copyright 2010-2011 Canonical
  Ltd. Ubuntu Font Licence 1.0, https://ubuntu.com/legal/font-licence.
- **openvr_capi.h** (`tools/openvr_capi.h`), Copyright Valve Corporation. BSD 3-Clause
  License. Used only to generate `crates/vr/src/ffi/tables.rs`.

## Runtime

OpenVR (`openvr_api`) is loaded from the user's SteamVR install and not redistributed.

## Rust crates

Compiled-in crates and their licenses: run `cargo deny list` in the repository. All are
GPL-3.0 compatible (MIT, Apache-2.0, BSD, ISC, Zlib, MPL-2.0, Unicode-3.0, BSL-1.0, CC0, and
Slint under its GPL-3.0 option); `cargo deny check licenses` enforces this in CI.
