#!/usr/bin/env bash
# Time one of the apps in a private headless sway: how long until its window
# shows, until it is ready (its own `perf` log line), the CPU it uses idle
# afterwards and the memory it holds.
#   scripts/perf.sh [--idle SECS] [--settle SECS] [--no-ready] BIN APP_ID STATE_DIR
# Prints: window_ms ready_ms idle_cpu_ms rss_mb
#
# The app takes its settings from the environment (RICAL_VDIR,
# RICAL_CONTACTS_CONTACTS, RICAL_MAIL_MAIL...). Its config, state and
# cache go under STATE_DIR: run twice with the same one to time it with a
# warm cache. Make something big to time with scripts/demo-big.sh, e.g.
#   scripts/demo-big.sh /var/tmp/big 50000 20000 50000
#   RICAL_VDIR=/var/tmp/big/cal RICAL_NO_NOTIFICATIONS=true \
#     scripts/perf.sh target/release/rical-calendar net.jeffas.Rical /tmp/s
# Needs sway (in the dev shell). With PERF_OUT set, the idle time is
# recorded by `perf record` into that file instead. --no-ready is for builds
# from before the `perf` lines: the window counts as ready.
set -euo pipefail
idle=5 settle=12 wait_ready=600
while [[ $# -gt 0 ]]; do
  case $1 in
    --idle) idle=$2; shift 2 ;;
    --settle) settle=$2; shift 2 ;;
    --no-ready) wait_ready=0; shift ;;
    *) break ;;
  esac
done
bin=$(realpath "${1:?usage: perf.sh [--idle S] [--settle S] [--no-ready] BIN APP_ID STATE_DIR}")
app_id=${2:?app id}
st=${3:?state dir}
mkdir -p "$st"
export XDG_STATE_HOME=$st/state XDG_CONFIG_HOME=$st/config XDG_DATA_HOME=$st/data XDG_CACHE_HOME=$st/cache

tmp=$(mktemp -d)
pids=()
cleanup() {
  for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done
  wait 2>/dev/null || true
  rm -rf "$tmp"
}
trap cleanup EXIT
cat > "$tmp/config" <<CONF
output HEADLESS-1 resolution 1400x900 bg #777777 solid_color
exec sh -c 'echo \$WAYLAND_DISPLAY > $tmp/display'
CONF
env -u WAYLAND_DISPLAY -u DISPLAY -u SWAYSOCK \
  WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman \
  sway -c "$tmp/config" > "$tmp/sway.log" 2>&1 &
sway_pid=$!
pids+=("$sway_pid")
for _ in $(seq 100); do [[ -s $tmp/display ]] && break; sleep 0.05; done
sock=$(ls "${XDG_RUNTIME_DIR:-/run/user/$UID}"/sway-ipc.*."$sway_pid".sock)
export WAYLAND_DISPLAY=$(cat "$tmp/display") GDK_BACKEND=wayland GSK_RENDERER=cairo GTK_A11Y=none
unset DISPLAY

logs() { find "$st/state" -name rical.log 2>/dev/null || true; }
for l in $(logs); do : > "$l"; done
t0=$(date +%s%N)
dbus-run-session -- "$bin" > "$tmp/app.log" 2>&1 &
pids+=($!)
window=""
for _ in $(seq 3000); do
  if SWAYSOCK=$sock swaymsg -t get_tree 2>/dev/null | grep -q "\"app_id\": \"$app_id\""; then
    window=$(( ($(date +%s%N) - t0) / 1000000 ))
    break
  fi
  sleep 0.01
done
[[ -n $window ]] || { echo "no window:" >&2; cat "$tmp/app.log" >&2; exit 1; }
pid=$(pgrep -n -f "^$bin" || true)

# ready: the app's own line; builds without one count the window
ready=$window
for _ in $(seq "$wait_ready"); do
  l=$(logs | head -1)
  if [[ -n $l ]] && grep -q "ready at" "$l"; then
    ready=$(grep "ready at" "$l" | head -1 |
      sed -E 's/.*ready at ([0-9.]+)(ms|s|µs).*/\1 \2/' |
      awk '{ if ($2 == "s") print int($1 * 1000); else if ($2 == "ms") print int($1); else print 0 }')
    break
  fi
  sleep 0.05
done

sleep "$settle"
ticks=$(getconf CLK_TCK)
cpu() { awk '{print $14 + $15}' "/proc/$pid/stat"; }
c0=$(cpu)
if [[ -n ${PERF_OUT:-} ]]; then
  perf record -q -e cpu-clock:u -g -o "$PERF_OUT" -p "$pid" -- sleep "$idle" > /dev/null 2>&1 || true
else
  sleep "$idle"
fi
c1=$(cpu)
rss=$(awk '/VmRSS/ {print int($2 / 1024)}' "/proc/$pid/status")
echo "$window $ready $(( (c1 - c0) * 1000 / ticks )) $rss"
