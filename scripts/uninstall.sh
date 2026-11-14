#!/bin/sh
# Undoes install.sh: takes FitnessVR off SteamVR's list, deletes its data (settings, workout history) and the
# installed binary. Exports in ~/Documents/FitnessVR stay. Quit FitnessVR and SteamVR first.
set -eu
dest="${XDG_DATA_HOME:-$HOME/.local/share}/fitvr"
here=$(cd "$(dirname "$0")" && pwd)
bin="$dest/fitvr"
[ -x "$bin" ] || bin="$here/fitvr"
printf 'Uninstall FitnessVR (settings and workout history are deleted)? [y/N] '
read -r answer
case "$answer" in y|Y) ;; *) exit 0 ;; esac
"$bin" --uninstall
rm -rf "$dest"
echo "FitnessVR is uninstalled. Exports in ~/Documents/FitnessVR stay."
