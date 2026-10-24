#!/usr/bin/env bash
# Run rical-gtk in a private headless sway, type keys into it and save
# screenshots. Nothing appears on the real desktop.
#
# Usage:
#   scripts/screenshot.sh [--size WxH] [--bin PATH] OUTDIR STEP... -- RICAL_GTK_ARGS...
#
# Each STEP is "name:keys", where keys are wtype arguments, e.g.
#   "week:"            screenshot without typing
#   "month:m"          type m, then screenshot
#   "next:-k Right"    press Right, then screenshot
#   "ctrl:-M ctrl 3 -m ctrl"
#   "edit:!touch vdir/x.ics"  run a shell command, then screenshot
# Screenshots are written to OUTDIR/NN-name.png.
#
# Needs sway, grim and wtype (in the dev shell).
set -euo pipefail

size=1280x860
bin=target/debug/rical-gtk
while [[ $# -gt 0 ]]; do
  case $1 in
    --size) size=$2; shift 2 ;;
    --bin) bin=$2; shift 2 ;;
    *) break ;;
  esac
done
outdir=$1; shift
steps=()
while [[ $# -gt 0 && $1 != -- ]]; do steps+=("$1"); shift; done
[[ ${1:-} == -- ]] && shift
mkdir -p "$outdir"

tmp=$(mktemp -d)
sway_pid=
app_pid=
cleanup() {
  [[ -n $app_pid ]] && kill "$app_pid" 2>/dev/null || true
  [[ -n ${kbd_pid:-} ]] && kill "$kbd_pid" 2>/dev/null || true
  [[ -n $sway_pid ]] && kill "$sway_pid" 2>/dev/null || true
  wait 2>/dev/null || true
  rm -rf "$tmp"
}
trap cleanup EXIT

cat > "$tmp/config" <<CONF
output HEADLESS-1 resolution $size bg #777777 solid_color
default_border none
focus_follows_mouse no
exec sh -c 'echo \$WAYLAND_DISPLAY > $tmp/display'
CONF

env -u WAYLAND_DISPLAY -u DISPLAY -u SWAYSOCK \
  WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman \
  sway -c "$tmp/config" > "$tmp/sway.log" 2>&1 &
sway_pid=$!

for _ in $(seq 100); do [[ -s $tmp/display ]] && break; sleep 0.1; done
[[ -s $tmp/display ]] || { echo "sway did not start:" >&2; cat "$tmp/sway.log" >&2; exit 1; }
sock=$(ls "${XDG_RUNTIME_DIR:-/run/user/$UID}"/sway-ipc.*."$sway_pid".sock)

export WAYLAND_DISPLAY=$(cat "$tmp/display")
export GDK_BACKEND=wayland GSK_RENDERER=cairo GTK_A11Y=none
unset DISPLAY

# The headless seat has no keyboard, and each wtype only adds one while it
# runs. Hold one open so the app sees a keyboard from the start.
wtype -s 86400000 &
kbd_pid=$!
sleep 0.3

"$bin" "$@" > "$tmp/app.log" 2>&1 &
app_pid=$!

for _ in $(seq 100); do
  SWAYSOCK=$sock swaymsg -t get_tree 2>/dev/null | grep -q '"app_id": "net.jeffas.Rical"' && break
  kill -0 "$app_pid" 2>/dev/null || { echo "rical-gtk exited:" >&2; cat "$tmp/app.log" >&2; exit 1; }
  sleep 0.1
done
sleep 1

i=0
for step in "${steps[@]}"; do
  name=${step%%:*}
  keys=${step#*:}
  if [[ $keys == '!'* ]]; then
    # "name:!command" runs a shell command instead, e.g. to edit the vdir
    sh -c "${keys:1}"
  elif [[ -n $keys ]]; then
    # shellcheck disable=SC2086
    wtype $keys
  fi
  sleep 0.6
  file=$(printf '%s/%02d-%s.png' "$outdir" "$i" "$name")
  grim "$file"
  echo "$file"
  i=$((i + 1))
done

if [[ -s $tmp/app.log ]]; then
  echo "--- rical-gtk output" >&2
  cat "$tmp/app.log" >&2
fi
