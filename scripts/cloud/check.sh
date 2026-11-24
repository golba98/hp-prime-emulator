#!/usr/bin/env bash
# Public-source validation. Calculator resources and physical USB access stay local.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/hp-prime-cloud-target}"
case "$CARGO_TARGET_DIR" in *" "*) echo "Use a space-free CARGO_TARGET_DIR for Unicorn." >&2; exit 2;; esac
check_state=$(mktemp -d)
trap 'rm -rf -- "$check_state"' EXIT
export PPL_CONFIG=/dev/null PPL_IDE_NO_DEVICE=1
export XDG_CONFIG_HOME="$check_state/config" XDG_DATA_HOME="$check_state/data"
export XDG_CACHE_HOME="$check_state/cache"
unset PPL_FIRMWARE PPL_OS_ROM PPL_FACTORY_A PPL_FIRMWARE_INIT_DATA PPL_FIRMWARE_RUNNER
unset PPL_PRIME_FONT_RESOURCE PPL_PRIME_HELP_RESOURCE PPL_CAS_HELPER
unset PRIME_TEST_FIRMWARE PRIME_TEST_OS_ROM PRIME_TEST_INIT_DATA PRIME_TEST_FONT_RESOURCE
python3 scripts/maintenance/check_documentation.py
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked --workspace
cargo test --locked --workspace
# Unit tests above include the GUI model; this also exercises the real X11 window.
export PPL_BIN="$CARGO_TARGET_DIR/debug/ppl"
export PPL_IDE_LOG_DIR="$check_state/gui-log"
export PPL_NATIVE_PROFILE="$check_state/native-profile"
bash tests/ide_xvfb_smoke/run_ide_auto.sh tests/ide_xvfb_smoke/scripts/script_native_side_input.json 90
python3 tests/ide_xvfb_smoke/assert_native_side_input.py
