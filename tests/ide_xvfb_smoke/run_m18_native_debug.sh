#!/bin/bash
# Native only. No calculator enumeration or device jobs; isolated window/profile/logs.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
M18_OUTPUT="${PPL_M18_OUTPUT:-${TMPDIR:-/tmp}/ppl-m18-native-debug}"
mkdir -p "$M18_OUTPUT/config" "$M18_OUTPUT/data"
M18_OUTPUT="$(realpath "$M18_OUTPUT")"
python3 - "$HERE/scripts/script_m18_native_debug.json" "$M18_OUTPUT" <<'PY'
import sys
from pathlib import Path
Path(sys.argv[2], 'scenario.json').write_text(Path(sys.argv[1]).read_text().replace('__M18_OUTPUT__', sys.argv[2]))
PY
export XDG_CONFIG_HOME="$M18_OUTPUT/config" XDG_DATA_HOME="$M18_OUTPUT/data"
export PPL_IDE_NO_DEVICE=1 PPL_ENGINE=native PPL_IDE_AUTOMATION="$M18_OUTPUT/scenario.json"
export WINIT_UNIX_BACKEND=x11 LIBGL_ALWAYS_SOFTWARE=1 NO_COLOR=1
unset WAYLAND_DISPLAY
Xvfb -displayfd 3 -screen 0 1280x900x24 -nolisten tcp 3>"$M18_OUTPUT/display" >"$M18_OUTPUT/xvfb.log" 2>&1 &
M18_XVFB_PID=$!
M18_FOCUS_PID=""
trap 'kill "$M18_XVFB_PID" ${M18_FOCUS_PID:+"$M18_FOCUS_PID"} 2>/dev/null || true' EXIT
for _ in $(seq 1 50); do [ -s "$M18_OUTPUT/display" ] && break; sleep .1; done
export DISPLAY=":$(cat "$M18_OUTPUT/display")"
python3 "$HERE/x11_focus.py" "HP Prime PPL Emulator" 60 >"$M18_OUTPUT/focus.log" 2>&1 &
M18_FOCUS_PID=$!
M18_TARGET="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
timeout 60 "${PPL_BIN:-$M18_TARGET/debug/ppl}" gui >"$M18_OUTPUT/ide.log" 2>&1
python3 "$HERE/assert_m18_native_debug.py" "$M18_OUTPUT/dump.json"
