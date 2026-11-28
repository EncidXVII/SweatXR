# Changelog

All notable changes to this project are documented here. Format based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Fixed
- Opening SweatXR while the watch already runs no longer starts a second watch process. The second one found the
  watch taken and left at once, but it had replaced the watch's log by then.

## [0.8.2] - 2026-11-28

### Changed
- Windows: the tray icon belongs to the watch. It is there whenever the watch runs in VR, also when SteamVR
  started it and no window is open. A click opens the window; the menu has Start with SteamVR and Quit, which
  stops the watch. The window alone no longer adds an icon.
- Opening SweatXR while SteamVR runs starts the watch, also with Start with SteamVR switched off.

### Fixed
- Opening SweatXR with SteamVR already running did not start the watch: SteamVR counted the window's own helper
  as the watch. The watch only came with the next SteamVR start.
- The watch no longer twitches on a fast-moving hand. It was hung on the wrist again, a few millimetres off, each
  time the hand moved quickly.

## [0.8.1] - 2026-11-28

### Fixed
- With a heart rate strap, standing still with a raised heart rate (a menu, the minutes after a song) no longer
  counts as active minutes: an active second needs movement too.
- On the day the clocks change, the day starts at local midnight. It started an hour off, so a session near
  midnight showed under the wrong day on the Sessions page and in the day's TCX file.
- Deleting history in the middle of a session: what you do next gets a session of its own. It counted for the day
  but showed on no session.

## [0.8.0] - 2026-11-24

### Added
- Linux without SteamVR: SweatXR runs as an OpenXR overlay on Monado and WiVRn (experimental). Start the runtime,
  then `sweatxr --overlay`. The watch, the panel, the pointer, the grip and haptics work through OpenXR. Every game
  counts as "VR game", the game still sees the trigger, the watch does not move aside for WristXR's, and SweatXR
  does not start with the runtime. SteamVR stays first when it runs; `SWEATXR_OPENXR=1` skips it.

### Changed
- Quitting lets the heart rate strap go, so a phone or watch can connect to it at once.
- With Bluetooth switched off, the strap status says "Bluetooth is off" instead of "Connecting…".
- Linux: a strap is saved by its own address, so a Bluetooth dongle that comes back under another name keeps it.
  Straps saved before still connect.
- Linux: the window gives its app id, so Wayland docks show SweatXR's name and icon.
- Linux: GNOME's and KDE's reduce-animations settings are followed, as Windows' already was.
- Nix: the `sweatxr` command keeps its files from garbage collection, so the start with SteamVR keeps working.
- README: what to do when the watch doesn't come with SteamVR, and how to run SweatXR on Monado and WiVRn.
- Linux: without SteamVR installed, the desktop window says how to start SweatXR on Monado or WiVRn.
- Nix: the 0.7.1 file hashes match the release files as published, so `nix run` works again.
- The Arch Linux PKGBUILD lists `openxr` as optional, for Monado and WiVRn.

## [0.7.1] - 2026-11-23

### Changed
- About lists the maker's profiles as GitHub, X, Threads, with X at its new address (x.com/EncidXVII).

### Added
- Linux packaging: a Nix flake (`nix run github:EncidXVII/SweatXR`), and an Arch Linux PKGBUILD in `packaging/aur`,
  both built from the release files.

## [0.7.0] - 2026-11-23

### Added
- The watch can be moved on its wrist, as on WristXR: point at it with the other controller, hold the grip, slide
  it along the forearm or turn the hand to tilt it, and let go. It stays there (kept per hand); let go on its
  default spot, it is back on it.
- With WristXR running and its watch on the same wrist, the watch starts beside WristXR's instead of on top of it:
  turned the same way, top edges level, a centimetre apart, up the arm. It reads where WristXR saved its watch, so
  it follows within two seconds when that one is moved. A watch placed by hand stays where it was put.
- The panel can be gripped and carried, as a panel on WristXR: the gripped spot stays under the beam, the panel
  turns to face you, level, and stays where the grip lets go.
- The pointer works the way WristXR's does: one ray, the pointing hand's tip as it will be when the next frame
  shows, smoothed for a steady hand, and SweatXR's own arithmetic for where it meets the watch and the panel. The
  beam, the dot and the control under them come from that one answer. Before, the beam rode the controller (where
  the hand is now), the dot came from an unpredicted pose and SteamVR's own intersection, and the two parted
  whenever the hand moved.
- Default bindings for Windows Mixed Reality and Steam Frame controllers, the two WristXR has and SweatXR lacked:
  without them the watch and beam went by the controller's raw pose, in another place and at another angle.
- Uninstall: `Uninstall SweatXR.cmd` in the Windows folder, `uninstall.sh` in the Linux archive, or
  `sweatxr --uninstall`. It takes SweatXR off SteamVR's list, so it no longer starts with SteamVR, and deletes
  its settings and workout history; exports in Documents/SweatXR stay. It never starts SteamVR. Same as
  WristXR.

### Fixed
- The overlay can no longer crowd out the game. Its loop never runs faster than 500 times a second, even if
  SteamVR's frame wait returns at once (a loop that floods SteamVR starves the game until the headset freezes,
  as a GPU wait in WristXR once did), and it waits at most a second for the GPU instead of for ever.
- When SteamVR crashes or is killed, the watch leaves with it. It used to stay behind unseen, holding the strap
  and the database, while the next SteamVR started a second one. If a SteamVR call never returns, the process
  is ended after 5 s, which also gives back a trigger it was holding. Same as WristXR.
- A save that failed (a busy database) no longer leaves the session pointing at a row that was never written.
- Pulling the trigger on the watch or the panel no longer also pulls it in the game: while the pointing hand is
  on them, its trigger is SweatXR's alone. It changes hands only once let go, so a trigger held in the game
  never freezes there. The other hand is never touched. Same as WristXR.
- The very first launch gets its trigger: SweatXR tells SteamVR which app it is before loading its controls
  (they could otherwise work only after a restart), and loads them again once if a tracked controller's trigger
  still never arrives. Same as WristXR.
- The pointer dot no longer lags behind the beam: it is its own overlay, moved in the same frame, and the beam
  rides on the controller, so both keep up with your hand at the headset's rate. The overlay now runs once per
  headset frame instead of 50 times a second. Beam and dot take the theme's accent.
- "Start with SteamVR" now reaches SteamVR as soon as you flip it, in the desktop window or in VR (turned on
  while SteamVR runs, the watch starts at once). It no longer flips back: neither window writes the other's old
  settings back over a change any more.
- Tray: Quit removes the icon and closes SweatXR. A left click opens the window; the menu is on the right
  button.

### Changed
- The watch sits on the back of the wrist, facing out of it, where a real watch is. It was on the controller's top,
  under the thumb, so it faced your chest when you turned the wrist to read it. Same as WristXR.
- The watch shows from a wider look: within about 52 degrees of where you look (was 45), and stays to 70 (was
  63). The wrist's turn is unchanged. Same as WristXR.
- New logo: a heartbeat in straight, leaning bars, in the same family as WristXR's and PortXR's. It is the
  app and window icon, the tray icon (lime on a dark taskbar, dark on a light one, switching with Windows), the
  picture SteamVR shows next to SweatXR, the Linux icon, and it sits beside the name in the desktop window in
  the theme's accent. Every icon is drawn from one script, packaging/brand/marks.py.
- The watch needs a quarter less turn of the wrist: its roll around the forearm must be within about 50 degrees
  of your eyes to show and 61 to stay. Where your arm and head are counts exactly as before. Same as WristXR.

### Added
- Themes: Lime, Mint, Aqua, Sky, Violet, Pink, Amber or White as the accent (Setup in VR, Settings > Look on
  the desktop), the same eight as WristXR. The watch, panel and desktop window recolour at once; warning red
  stays red, and dark text on every accent is at least 4.5:1.

### Changed
- New look in line with WristXR: neutral graphite greys, soft rounded surfaces, Anton headings over
  Barlow (bundled, with a semibold weight for chosen tabs and filled buttons).
  The watch is the WristXR card (14 cm, 560 px): ring with today's kcal, a pill naming the game
  being counted, active minutes and heart rate. A hint or warning shows as one line under it.
- The VR panel is WristXR's 800 x 600 rail menu (50 cm at arm's length): Today, Week, You,
  Strap, Setup. Settings are split across the rail so no page needs sub-tabs. First run has a
  second step that teaches opening the panel from the watch.
- The desktop app has a left nav; Today adds this week's chart, History shows burned, active and
  best day over the chart, Settings is one page in two columns with Open folder.
- Motion: presses answer on trigger-down, hovers ease, segment choices and switches slide, the
  watch card grows down for a hint and back, the ring and bars fill to new values, the goal card
  fades in, the panel fades in when opened. All of it follows Windows' "Animation effects".

### Fixed
- The watch sat at a different angle on each controller model and read sideways when you raised
  your wrist. It now uses the controller's tip pose, like WristXR: text runs along the forearm,
  top away from you, on either hand. The laser starts from the tip too.
- Glancing at the watch brightens it, as on WristXR, at a wide angle (a look from the corner of
  the eye is enough). Once bright it holds and needs a clear look away to dim, so it no longer
  flickers at the edge. It dims to 35 % when you look away.
- Closing the SweatXR window quits it; it no longer keeps running unseen in the background.
- The VR overlay never starts SteamVR itself: started any other way while SteamVR is off, it
  leaves at once.
- Pointing just below a short watch card no longer counts as pointing at the watch.
- The strap dropping mid-session now says so on the watch; estimates continue from movement.

## [Test builds]

The first builds, before the public release.

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
