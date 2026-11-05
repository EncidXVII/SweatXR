# Changelog

All notable changes to this project are documented here. Format based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [1.0.0]

First release.

### Added
- SteamVR wrist watch overlay: goal ring with today's calories (estimated) inside, active
  minutes, heart rate and heart rate zone. Dims when you are not looking, full brightness after
  a short glance, hides near your face. "LIVE" shows while a session is being counted. Until you
  first open the panel, the watch shows how to open it.
- Goal moment: the watch turns lime and both controllers buzz when today's goal is reached.
- Laser pointer from your other controller whenever you aim at the watch or panel.
- VR panel opened by pointing at the watch: Today (per-game list), Week (bars, streak),
  Settings (body, watch, strap, data). Haptic tick on hover and
  click; the button is rebindable in SteamVR.
- Desktop app: Today, History (day, week or 30 days, step back through past periods, per game),
  Sessions (each day's sessions with per-session TCX export), Settings. Denser layout than in VR.
  Tray icon on Windows.
- Settings for resting heart rate, personal calibration progress, and deleting all history.
- Calorie estimate from heart rate (Keytel et al. 2005) with a Bluetooth LE strap, or from head
  and hand motion without one, with personal calibration after 20 minutes with a strap.
- Sessions per game, active minutes, daily goal and streaks, all stored locally in SQLite.
- Export to TCX (a session or a whole day) and CSV (everything).
- Starts with SteamVR. Windows x64 (zip), Linux x64 and arm64 (AppImage and tar.gz), each with
  a SHA-256 file.
