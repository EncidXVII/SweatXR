# Releasing SweatXR

## The repository page (once)

What search engines and assistants read first is the repository's own page, so it says what the README says:

1. **Name**: `SweatXR` (GitHub redirects the old address after a rename).
2. **About**:
   - Description: `Free, open-source VR fitness and calorie tracker for every SteamVR game: calories, active minutes and heart rate zones on a wrist watch, Bluetooth heart rate straps, history per game, TCX and CSV export. No account, no cloud.`
   - Topics (20 at most): `vr`, `virtual-reality`, `steamvr`, `openvr`, `vr-fitness`, `fitness-tracker`, `calorie-counter`, `calories`,
     `heart-rate`, `bluetooth-le`, `exergaming`, `workout`, `vr-overlay`, `tcx`, `valve-index`, `meta-quest`,
     `steam-frame`, `linux`, `rust`, `slint`.
3. **Settings > General > Social preview**: upload `docs/img/og.png` (the card shown when the link is shared).
4. **Security**: turn on Private vulnerability reporting (`SECURITY.md` sends reports there) and Dependabot alerts.
5. Features: Issues on, Discussions on (Q&A, Show and tell), Wiki off.

## Each release

1. Everything committed, the changelog's top section dated, the version the same in every manifest.
2. The checks in [CONTRIBUTING.md](CONTRIBUTING.md) pass, and the build has been used in a headset.
3. Build the packages with the packaging scripts; each carries `LICENSE`, `NOTICE` and `THIRD_PARTY_NOTICES.md`.
4. Tag `vX.Y.Z` and push the tag: the `ci` workflow builds every package and its SHA-256 file. Publish them from
   your own account, with the changelog section as the notes:
   `gh run download <run id> -D assets`, then
   `gh release create vX.Y.Z assets/*/* --title "SweatXR X.Y.Z" --notes-file notes.md`.
5. Check the README's download links and the app's update check against the new release.
6. In `flake.nix`, set `version` and each Linux file's `hash` (its `.sha256` in SRI form: `nix hash convert --hash-algo
   sha256 <hex>`), then push: the Nix workflow builds it.
