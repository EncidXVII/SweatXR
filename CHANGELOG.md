# Changelog

All notable changes to this project are documented here. Format based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [1.0.0]

First release.

### Added
- SteamVR wrist watch overlay: goal ring, calories (estimated), active minutes, heart rate.
  Dims when you are not looking, full brightness after a short glance, hides near your face.
  A lime dot shows while a session is being counted.
- VR panel opened by pointing at the watch: Today (per-game list), Week (bars, streak),
  Settings (profile, watch hand, units, auto-start, heart rate strap). Haptic tick on hover and
  click; the button is rebindable in SteamVR.
- Desktop app: Today, History (day, week, month, per game), Export, Settings. Tray icon on
  Windows.
- Calorie estimate from heart rate (Keytel et al. 2005) with a Bluetooth LE strap, or from head
  and hand motion without one, with personal calibration after 20 minutes with a strap.
- Sessions per game, active minutes, daily goal and streaks, all stored locally in SQLite.
- Export to TCX (today) and CSV (everything).
- Starts with SteamVR. Windows x64 (zip), Linux x64 and arm64 (AppImage and tar.gz), each with
  a SHA-256 file.
