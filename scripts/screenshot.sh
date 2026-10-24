#!/usr/bin/env bash
# Run rical-gtk in a private headless sway, drive it with the keyboard and
# pointer, and save screenshots. Nothing appears on the real desktop.
#
# Usage:
#   scripts/screenshot.sh [--size WxH] [--bin PATH] OUTDIR STEP... -- RICAL_GTK_ARGS...
#
# Each STEP is "name:action"; the action runs, then a screenshot is saved to
# OUTDIR/NN-name.png. Actions:
#   "week:"                     nothing, just a screenshot
#   "month:m"                   wtype arguments: type m
#   "next:-k Right"             press Right
#   "ctrl:-M ctrl 3 -m ctrl"    Ctrl+3
#   "drag:%move 550 515; press; move 600 560; release"
#                               drive the pointer (see tools/wlpointer)
#   "float:@floating enable"    sway commands, ";" separated
#   "edit:!touch vdir/x.ics"    a shell command
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

pointer=tools/wlpointer/target/debug/wlpointer
[[ -x $pointer ]] || cargo build -q --manifest-path tools/wlpointer/Cargo.toml

tmp=$(mktemp -d)
pids=()
cleanup() {
  exec 3>&- 2>/dev/null || true
  for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
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
pids+=("$sway_pid")

for _ in $(seq 100); do [[ -s $tmp/display ]] && break; sleep 0.1; done
[[ -s $tmp/display ]] || { echo "sway did not start:" >&2; cat "$tmp/sway.log" >&2; exit 1; }
sock=$(ls "${XDG_RUNTIME_DIR:-/run/user/$UID}"/sway-ipc.*."$sway_pid".sock)

export WAYLAND_DISPLAY=$(cat "$tmp/display")
export GDK_BACKEND=wayland GSK_RENDERER=cairo GTK_A11Y=none
unset DISPLAY

# The headless seat has no keyboard or pointer, and each wtype only adds a
# keyboard while it runs. Hold both open so the app sees them from the start.
wtype -s 86400000 &
pids+=($!)
mkfifo "$tmp/pointer"
"$pointer" < "$tmp/pointer" &
pids+=($!)
exec 3> "$tmp/pointer"
sleep 0.3

"$bin" "$@" > "$tmp/app.log" 2>&1 &
app_pid=$!
pids+=("$app_pid")

for _ in $(seq 100); do
  SWAYSOCK=$sock swaymsg -t get_tree 2>/dev/null | grep -q '"app_id": "net.jeffas.Rical"' && break
  kill -0 "$app_pid" 2>/dev/null || { echo "rical-gtk exited:" >&2; cat "$tmp/app.log" >&2; exit 1; }
  sleep 0.1
done
sleep 1

i=0
for step in "${steps[@]}"; do
  name=${step%%:*}
  action=${step#*:}
  case $action in
    '%'*)
      IFS=';' read -ra cmds <<< "${action:1}"
      for cmd in "${cmds[@]}"; do
        echo "$cmd" >&3
        sleep 0.08
      done
      ;;
    '@'*)
      IFS=';' read -ra cmds <<< "${action:1}"
      for cmd in "${cmds[@]}"; do
        # shellcheck disable=SC2086
        SWAYSOCK=$sock swaymsg $cmd > /dev/null
      done
      ;;
    '!'*)
      sh -c "${action:1}"
      ;;
    '') ;;
    *)
      # shellcheck disable=SC2086
      wtype $action
      ;;
  esac
  sleep 0.6
  file=$(printf '%s/%02d-%s.png' "$outdir" "$i" "$name")
  grim "$file"
  echo "$file"
  i=$((i + 1))
done

if ! kill -0 "$app_pid" 2>/dev/null; then
  echo "rical-gtk exited early" >&2
fi
if [[ -s $tmp/app.log ]]; then
  echo "--- rical-gtk output" >&2
  cat "$tmp/app.log" >&2
fi
