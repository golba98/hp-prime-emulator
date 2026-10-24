#!/bin/bash
# Run the IDE with an automation script under a private Xvfb and wait for it
# to exit. Usage: run_ide_auto.sh <script.json> [timeout_secs]
# Environment: PPL_XVFB_SCREEN (default 1280x900x24) sets the virtual screen;
# WINIT_X11_SCALE_FACTOR (e.g. 1.5, 2) emulates a high-DPI display.
set -u
SCREEN="${PPL_XVFB_SCREEN:-1280x900x24}"
mkdir -p /tmp/ppl-ide-e2e
SCRIPT="$(readlink -f "$1")"
TIMEOUT="${2:-90}"
for n in $(seq 300 380); do
  if [ ! -e "/tmp/.X11-unix/X$n" ] && [ ! -e "/tmp/.X$n-lock" ]; then DISP=:$n; break; fi
done
export DISPLAY="$DISP"
unset WAYLAND_DISPLAY
Xvfb "$DISP" -screen 0 "$SCREEN" -nolisten tcp >/tmp/ppl-ide-e2e/xvfb.log 2>&1 &
XVFB_PID=$!
for i in $(seq 1 40); do [ -e "/tmp/.X11-unix/X${DISP#:}" ] && break; sleep 0.2; done
sleep 0.5

export WINIT_UNIX_BACKEND=x11
export LIBGL_ALWAYS_SOFTWARE=1
export NO_COLOR=1
export PPL_IDE_AUTOMATION="$SCRIPT"
cd "/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator"
T0=$(date +%s)
# The release binary lives in Cargo's target directory, which
# .cargo/config.toml may move outside the repository; PPL_BIN overrides.
TARGET_DIR="$(cargo metadata --format-version 1 --no-deps 2>/dev/null \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])' 2>/dev/null)"
PPL_BIN="${PPL_BIN:-${TARGET_DIR:-./target}/release/ppl}"
timeout "$TIMEOUT" "$PPL_BIN" gui >/tmp/ppl-ide-e2e/ide_run.log 2>&1
CODE=$?
T1=$(date +%s)
kill -TERM "$XVFB_PID" 2>/dev/null
echo "exit=$CODE elapsed=$((T1-T0))s display=$DISP screen=$SCREEN scale=${WINIT_X11_SCALE_FACTOR:-native}"
