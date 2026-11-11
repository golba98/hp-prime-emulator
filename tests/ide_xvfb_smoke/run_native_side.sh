#!/usr/bin/env bash
# Real desktop native acceptance. All profiles and logs remain in the printed directory.
set -euo pipefail
NATIVE_TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/prime-native-side-XXXXXX")"
NATIVE_TEST_HERE="$(cd "$(dirname "$0")" && pwd)"
export PPL_IDE_NO_DEVICE=1
# Discover the installed algebra helper before isolating application data.
if [[ -z "${PPL_CAS_HELPER:-}" ]]; then
  NATIVE_TEST_HELPER="${XDG_DATA_HOME:-$HOME/.local/share}/hp-prime-emulator/cas/ppl-giac-helper"
  if [[ -x "$NATIVE_TEST_HELPER" ]]; then export PPL_CAS_HELPER="$NATIVE_TEST_HELPER"; fi
fi
for scenario in input state_create state_reopen graph dialogs user integer clipboard models solve graph_context; do
  profile="$scenario"
  if [[ "$scenario" == state_* ]]; then profile=state; fi
  export PPL_NATIVE_PROFILE="$NATIVE_TEST_ROOT/$profile/memory"
  export XDG_CONFIG_HOME="$NATIVE_TEST_ROOT/$profile/config"
  export XDG_DATA_HOME="$NATIVE_TEST_ROOT/$profile/data"
  export PPL_IDE_LOG_DIR="$NATIVE_TEST_ROOT/$scenario/log"
  bash "$NATIVE_TEST_HERE/run_ide_auto.sh" "$NATIVE_TEST_HERE/scripts/script_native_side_$scenario.json" 60
done
python3 "$NATIVE_TEST_HERE/assert_native_side_input.py"
python3 "$NATIVE_TEST_HERE/assert_native_side_workflows.py"
printf 'Preserved Native desktop evidence: %s\n' "$NATIVE_TEST_ROOT"
