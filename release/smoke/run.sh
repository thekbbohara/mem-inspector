#!/bin/sh
# Inside the smoke container. A fresh HOME with fake data, no settings, no
# Claude: the app must show the first-run picker, pick a folder on a second
# disk, then scan the fake home.
set -u
export HOME=$(mktemp -d) DISPLAY=:99 APPIMAGE_EXTRACT_AND_RUN=1 TMPDIR=/tmp
export XDG_RUNTIME_DIR=$(mktemp -d) NO_AT_BRIDGE=1
cd "$HOME"
python3 /scripts/rehearsal-home.py /tmp/fake >/dev/null
# The fake data becomes this HOME's content.
cp -a /tmp/fake/home/. "$HOME"/ && rm -rf /tmp/fake
mkdir -p /second-disk/Archive
Xvfb :99 -screen 0 1280x800x24 >/dev/null 2>&1 &
sleep 2
shot() { import -window root "/shots/$1.png"; echo "shot $1"; }
click() { xdotool mousemove "$1" "$2" click 1; sleep "${3:-1}"; }
dbus-run-session -- /app/Mem.AppImage >/shots/app.log 2>&1 &
sleep 12
shot 1-first-run
# "Choose folder" opens the native GTK folder dialog.
xdotool search --name "Mem Inspector" >/shots/windows.txt 2>&1
click "${CHOOSE_X:-813}" "${CHOOSE_Y:-440}" 3
shot 2-folder-dialog
# Typing "/" opens GTK's location bar; the rest goes in once it is open.
xdotool type "/"; sleep 1
xdotool type --delay 40 "second-disk/Archive"; sleep 1
shot 3-folder-typed
xdotool key Return; sleep 2
# Still open (it navigated into the folder): confirm with Open.
xdotool search --name "Where should personal files go" >/dev/null 2>&1 && { xdotool key Return; sleep 2; }
sleep 2
shot 4-folder-chosen
echo "settings: $(cat "$HOME/.config/mem-inspector/settings.json" 2>&1)"
click "${CONTINUE_X:-843}" "${CONTINUE_Y:-595}" 2
sleep 40
shot 5-scanned
echo "--- app log"; grep -v "^$" /shots/app.log | tail -30
