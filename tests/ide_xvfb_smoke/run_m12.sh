#!/bin/bash
# M12: run one G1 Firmware calculator scenario of the IDE under a private Xvfb,
# with an isolated HOME/XDG and a fresh calculator profile seeded from the
# configured initialized data. The user's profile and config are never used.
# Usage: run_m12.sh <scenario> [timeout_secs]   (scripts/script_m12_<scenario>.json)
# Environment: M12_RUN (evidence directory, default artifacts/m12/<UTC time>),
# PPL_M12_CONFIG (asset config, default ~/.config/hp-prime-emulator/firmware.toml),
# M12_PROFILE (a calculator profile directory kept across runs, e.g. to
# reopen the IDE on the same calculator; seeded when new, never deleted).
set -u
HERE="$(dirname "$(readlink -f "$0")")"
ROOT="$(readlink -f "$HERE/../..")"
NAME="$1"
TIMEOUT="${2:-300}"
# A scenario name, or the path of any other firmware script (whose dump
# paths are its own).
SCRIPT="$HERE/scripts/script_m12_$NAME.json"
[ -f "$SCRIPT" ] || SCRIPT="$HERE/scripts/script_$NAME.json"   # e.g. m13_persist
if [ -f "$NAME" ]; then SCRIPT="$(readlink -f "$NAME")"; NAME="$(basename "$NAME" .json)"; fi
[ -f "$SCRIPT" ] || { echo "no scenario $SCRIPT" >&2; exit 2; }
CONFIG="${PPL_M12_CONFIG:-$HOME/.config/hp-prime-emulator/firmware.toml}"
asset() {
  python3 -c 'import sys,tomllib; print(tomllib.load(open(sys.argv[1],"rb")).get(sys.argv[2],""))' "$CONFIG" "$1"
}
TARGET_DIR="$(cd "$ROOT" && cargo metadata --format-version 1 --no-deps 2>/dev/null \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
export PPL_BIN="${PPL_BIN:-$TARGET_DIR/release/ppl}"
export PPL_FIRMWARE="$(asset firmware)"
export PPL_FACTORY_A="$(asset factory_a)"
export PPL_OS_ROM="$(asset os_rom)"
export PPL_FIRMWARE_INIT_DATA="$(asset init_data)"
# The workspace's helper, never an older installed one.
export PPL_FIRMWARE_RUNNER="$TARGET_DIR/release/prime-emulator"
RUN="${M12_RUN:-$ROOT/artifacts/m12/$(date -u +%Y%m%dT%H%M%SZ)}"
OUT="$RUN/$NAME"
mkdir -p "$OUT"
ISO="$(mktemp -d /tmp/ppl-m12-XXXXXX)"
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
export HOME="$ISO/home" XDG_CONFIG_HOME="$ISO/config" XDG_DATA_HOME="$ISO/data"
export PPL_FIRMWARE_PROFILE="${M12_PROFILE:-$ISO/profile}"
mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME"
E2E=/tmp/ppl-ide-e2e/m12/$NAME
rm -rf "$E2E"; mkdir -p "$E2E"
pgrep -af "prime-emulator session" >"$OUT/pids-before.txt" || true
"$HERE/run_ide_auto.sh" "$SCRIPT" "$TIMEOUT" | tee "$OUT/run.txt"
# Helpers of this profile still alive after the IDE exited (must be none).
sleep 1
pgrep -af "prime-emulator session.*$ISO" >"$OUT/leftover-pids.txt" || true
pgrep -af "prime-emulator session" >"$OUT/pids-after.txt" || true
cp -r "$E2E"/. "$OUT"/ 2>/dev/null || true
cp /tmp/ppl-ide-e2e/ide_run.log "$OUT/ide_run.log" 2>/dev/null || true
for f in session.log ide-reports; do
  [ -e "$PPL_FIRMWARE_PROFILE/$f" ] && cp -r "$PPL_FIRMWARE_PROFILE/$f" "$OUT/"
done
ls "$PPL_FIRMWARE_PROFILE/C/DATA" >"$OUT/profile-programs.txt" 2>/dev/null || true
rm -rf "$ISO"
echo "evidence: $OUT"
