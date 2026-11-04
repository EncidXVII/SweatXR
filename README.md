# FitnessVR

A watch on your wrist in every SteamVR game: calories, active minutes, heart rate and a daily
goal ring. History per day and per game. Free, open source (GPL-3.0), and all data stays on
your computer.

Runs on Windows x64, Linux x64 and Linux arm64 (Steam Frame on-device). Quest works through
Steam Link or Virtual Desktop on a PC. Quest standalone is not supported.

## Install in 3 steps

1. Download the archive for your system from the latest release and check its `.sha256`.
2. Unpack it anywhere. On Linux or Steam Frame run `./install.sh`; on Windows keep the folder
   where it is.
3. Start SteamVR, then start `fitvr` once. It registers itself and from then on starts with
   SteamVR. Turn that off in Settings > Device > Start with SteamVR.

In VR, look at your left wrist (change the hand in Settings). Point at the watch with the other
controller and pull the trigger to open the panel. Rebind the button in SteamVR's controller
bindings under FitnessVR.

Heart rate: any Bluetooth LE strap that uses the standard heart rate service. Pair it in
Settings > Device > Scan, then pick it. Pairing from the desktop app saves the choice; the VR
overlay connects.

## Privacy

No account, no telemetry, no network access. Everything is in one SQLite file:
`%APPDATA%\FitnessVR\data\fitvr.db` on Windows, `~/.local/share/fitnessvr/fitvr.db` on Linux.
Exports go to `Documents/FitnessVR`.

## Accuracy

All numbers are estimates. This is not a medical device.

- With a heart rate strap: Keytel et al. (2005) prediction equation from heart rate, weight,
  age and sex. Expect roughly 20% error against a dedicated fitness watch.
- Without one: head and hand speed mapped to METs. Expect roughly 35% error. After 20 minutes
  of play with a strap, the app fits motion to your own heart-rate data and uses that fit when
  the strap is off.
- Only time in play counts. Standing in menus does not, and the last idle minutes before a
  session ends are dropped.

## Build

```bash
cargo build --release -p fitvr
```

Linux needs `libdbus-1-dev pkg-config libxkbcommon-dev libfontconfig1-dev`. No OpenVR SDK is
needed: the runtime is loaded from the SteamVR install at start.

Tests: `cargo test --workspace`. UI snapshots live in `crates/ui/tests/snapshots`; review
changes with `cargo insta review`. SteamVR smoke test without a headset (Windows):
`powershell -ExecutionPolicy Bypass -File scripts\smoke.ps1 -Seconds 1800`.

## License

GPL-3.0-only. See [LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
