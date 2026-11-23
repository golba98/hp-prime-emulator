#!/usr/bin/env bash
# Launch the HP Prime Native IDE specifically placed on the ASUS monitor (HDMI-1 at x=7680..11520)

# Unset WAYLAND_DISPLAY so winit connects via XWayland to DISPLAY :0, allowing precise multi-monitor positioning
unset WAYLAND_DISPLAY
export DISPLAY=:0
export PPL_WINDOW_POS_X=7850
export PPL_WINDOW_POS_Y=200

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN_PATH="${PPL_BIN:-}"
if [ -z "$BIN_PATH" ]; then
    for candidate in \
        "${CARGO_TARGET_DIR:-$REPO_ROOT/target}/release/ppl" \
        "$REPO_ROOT/../.cache/prime-emulator-target/release/ppl" \
        "$REPO_ROOT/../.cache/prime-emulator-target-compact/release/ppl"; do
        if [ -f "$candidate" ]; then
            BIN_PATH="$candidate"
            break
        fi
    done
fi
if [ -z "$BIN_PATH" ] || [ ! -x "$BIN_PATH" ]; then
    echo "Release ppl binary not found; build with scripts/emulator/run.sh build --release --bin ppl or set PPL_BIN." >&2
    exit 1
fi

echo "Launching Native IDE onto ASUS monitor (HDMI-1 at x=7850)..."
"$BIN_PATH" gui "$@" &
PID=$!

# Move and position window onto the ASUS monitor using wmctrl
for i in {1..30}; do
    if wmctrl -l | grep -q "HP Prime"; then
        wmctrl -r "HP Prime PPL Emulator" -b remove,maximized_vert,maximized_horz
        wmctrl -r "HP Prime PPL Emulator" -e 0,7850,200,1920,1080
        echo "Window successfully placed on ASUS monitor."
        break
    fi
    sleep 0.1
done

wait $PID
