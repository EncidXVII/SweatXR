# Security

SweatXR runs locally, makes no network calls and stores data only in a SQLite file in your
user folder. The parts most worth scrutiny are the OpenVR and Vulkan bindings
(`crates/vr/src/ffi`), the Bluetooth packet parser (`crates/hr/src/parse.rs`) and the files
written into SteamVR's configuration when the app registers itself.

## Reporting a vulnerability

Please report privately through GitHub: **Security > Report a vulnerability** on the repository
page. Do not open a public issue. Expect a first reply within a week. Fixes ship in the next
release and the report is credited unless you prefer otherwise.

## Supported versions

Only the latest release gets fixes.
