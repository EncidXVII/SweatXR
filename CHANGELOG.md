# Changelog

All notable changes to this project are documented here. Format based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed
- New look in line with DesktopVR: neutral graphite greys, soft rounded surfaces, Anton headings over
  Barlow (bundled, with a semibold weight for chosen tabs and filled buttons).
  The watch is the DesktopVR card (14 cm, 560 px): ring with today's kcal, a pill naming the game
  being counted, active minutes and heart rate. A hint or warning shows as one line under it.
- The VR panel is DesktopVR's 800 x 600 rail menu (50 cm at arm's length): Today, Week, You,
  Strap, Setup. Settings are split across the rail so no page needs sub-tabs. First run has a
  second step that teaches opening the panel from the watch.
- The desktop app has a left nav; Today adds this week's chart, History shows burned, active and
  best day over the chart, Settings is one page in two columns with Open folder.
- Motion: presses answer on trigger-down, hovers ease, segment choices and switches slide, the
  watch card grows down for a hint and back, the ring and bars fill to new values, the goal card
  fades in, the panel fades in when opened. All of it follows Windows' "Animation effects".

### Fixed
- The watch sat at a different angle on each controller model and read sideways when you raised
  your wrist. It now uses the controller's tip pose, like DesktopVR: text runs along the forearm,
  top away from you, on either hand. The laser starts from the tip too.
- Glancing at the watch brightens it, as on DesktopVR, at a wide angle (a look from the corner of
  the eye is enough). Once bright it holds and needs a clear look away to dim, so it no longer
  flickers at the edge. It dims to 35 % when you look away.
- Closing the FitnessVR window quits it; it no longer keeps running unseen in the background.
- The VR overlay never starts SteamVR itself: started any other way while SteamVR is off, it
  leaves at once.
- Pointing just below a short watch card no longer counts as pointing at the watch.
- The strap dropping mid-session now says so on the watch; estimates continue from movement.

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
