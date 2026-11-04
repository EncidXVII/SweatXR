#!/bin/sh
# Install for the current user (Linux desktop, Steam Deck, Steam Frame in Developer Mode).
# Copies the binary to ~/.local/share/fitvr and registers it with SteamVR so it starts with it.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
dest="${XDG_DATA_HOME:-$HOME/.local/share}/fitvr"
mkdir -p "$dest"
install -m 755 "$here/fitvr" "$dest/fitvr"
echo "Installed to $dest/fitvr"
if "$dest/fitvr" --register; then
    echo "Registered with SteamVR. It starts with SteamVR from now on."
else
    echo "SteamVR is not running. Start it once, then run: $dest/fitvr --register"
fi
