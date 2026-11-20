#!/bin/sh
# Copyright (C) 2026 EncidXVII
# SPDX-License-Identifier: GPL-3.0-only

# Install for the current user (Linux desktop, Steam Deck, Steam Frame in Developer Mode).
# Copies the binary to ~/.local/share/sweatxr and registers it with SteamVR so it starts with it.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
dest="${XDG_DATA_HOME:-$HOME/.local/share}/sweatxr"
mkdir -p "$dest"
install -m 755 "$here/sweatxr" "$dest/sweatxr"
echo "Installed to $dest/sweatxr"
if "$dest/sweatxr" --register; then
    echo "Registered with SteamVR. It starts with SteamVR from now on."
else
    echo "SteamVR is not running. Start it once, then run: $dest/sweatxr --register"
fi
