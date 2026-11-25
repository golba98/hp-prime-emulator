#!/bin/bash
# Run the IDE with an automation script under a private Xvfb and wait for it
# to exit. Usage: run_ide_auto.sh <script.json> [timeout_secs]
# Environment: PPL_XVFB_SCREEN (default 1280x900x24) sets the virtual screen;
# WINIT_X11_SCALE_FACTOR (e.g. 1.5, 2) emulates a high-DPI display.
# PPL_IDE_PID_FILE can record the GUI PID for an isolated crash/reopen test.
set -u
SCREEN="${PPL_XVFB_SCREEN:-1280x900x24}"
LOG_DIR="${PPL_IDE_LOG_DIR:-/tmp/ppl-ide-e2e}"
mkdir -p "$LOG_DIR"
SCRIPT="$(readlink -f "$1")"
TIMEOUT="${2:-90}"
# Serialize allocation across IDE scenarios, including different test workspaces.
display_lock_dir="/tmp/ppl-ide-xvfb-$UID"
mkdir -p -m 700 "$display_lock_dir" || exit 1
DISP=
for n in $(seq 300 380); do
  exec {display_lock_fd}>"$display_lock_dir/X$n" || exit 1
  if flock -n "$display_lock_fd" &&
      [ ! -e "/tmp/.X11-unix/X$n" ] && [ ! -e "/tmp/.X$n-lock" ]; then
    DISP=:$n
    break
  fi
  exec {display_lock_fd}>&-
done
if [ -z "$DISP" ]; then
  printf '%s\n' 'No free display for the Native IDE test' >&2
  exit 1
fi
export DISPLAY="$DISP"
unset WAYLAND_DISPLAY
Xvfb "$DISP" -screen 0 "$SCREEN" -nolisten tcp >"$LOG_DIR/xvfb.log" 2>&1 &
XVFB_PID=$!
for i in $(seq 1 40); do
  [ -e "/tmp/.X11-unix/X${DISP#:}" ] && break
  kill -0 "$XVFB_PID" 2>/dev/null || exit 1
  sleep 0.2
done
sleep 0.5

export WINIT_UNIX_BACKEND=x11
export LIBGL_ALWAYS_SOFTWARE=1
export NO_COLOR=1
export PPL_IDE_AUTOMATION="$SCRIPT"
# Run from this checkout (any worktree), not a hard-coded path.
cd "$(dirname "$(readlink -f "$0")")/../.."
T0=$(date +%s)
# The release binary lives in Cargo's target directory, which
# .cargo/config.toml may move outside the repository; PPL_BIN overrides.
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps 2>/dev/null \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])' 2>/dev/null)"
PPL_BIN="${PPL_BIN:-${TARGET_DIR:-./target}/release/ppl}"
# Without a window manager nothing focuses the window; the automation only
# advances while it is focused.
python3 tests/ide_xvfb_smoke/x11_focus.py "HP Prime PPL Emulator" "$TIMEOUT" &
FOCUS_PID=$!
timeout "$TIMEOUT" bash -c '
  if [[ -n "${PPL_IDE_PID_FILE:-}" ]]; then
    printf "%s\n" "$$" >"$PPL_IDE_PID_FILE" || exit 1
  fi
  exec "$@"
' ppl-ide-run "$PPL_BIN" gui >"$LOG_DIR/ide_run.log" 2>&1
CODE=$?
kill "$FOCUS_PID" 2>/dev/null
T1=$(date +%s)
kill -TERM "$XVFB_PID" 2>/dev/null
wait "$XVFB_PID" 2>/dev/null
echo "exit=$CODE elapsed=$((T1-T0))s display=$DISP screen=$SCREEN scale=${WINIT_X11_SCALE_FACTOR:-native}"
exit "$CODE"
