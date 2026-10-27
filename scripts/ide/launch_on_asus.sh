#!/usr/bin/env bash
# Launch the HP Prime Native IDE specifically placed on the ASUS monitor (HDMI-1 at x=7680..11520)

# Unset WAYLAND_DISPLAY so winit connects via XWayland to DISPLAY :0, allowing precise multi-monitor positioning
unset WAYLAND_DISPLAY
export DISPLAY=:0
export PPL_WINDOW_POS_X=7850
export PPL_WINDOW_POS_Y=200

BIN_PATH="/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/../.cache/prime-emulator-target/release/ppl"
if [ ! -f "$BIN_PATH" ]; then
    BIN_PATH="/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/target/release/ppl"
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
