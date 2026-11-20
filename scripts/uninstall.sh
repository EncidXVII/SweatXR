#!/bin/sh
# Copyright (C) 2026 EncidXVII
# SPDX-License-Identifier: GPL-3.0-only

# Undoes install.sh: takes SweatXR off SteamVR's list, deletes its data (settings, workout history) and the
# installed binary. Exports in ~/Documents/SweatXR stay. Quit SweatXR and SteamVR first.
set -eu
dest="${XDG_DATA_HOME:-$HOME/.local/share}/sweatxr"
here=$(cd "$(dirname "$0")" && pwd)
bin="$dest/sweatxr"
[ -x "$bin" ] || bin="$here/sweatxr"
printf 'Uninstall SweatXR (settings and workout history are deleted)? [y/N] '
read -r answer
case "$answer" in y|Y) ;; *) exit 0 ;; esac
"$bin" --uninstall
rm -rf "$dest"
echo "SweatXR is uninstalled. Exports in ~/Documents/SweatXR stay."
