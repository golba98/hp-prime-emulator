#!/bin/bash
# Native-only, isolated preferences and outputs. Requires a built release ppl.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
M17_OUTPUT="${PPL_M17_OUTPUT:-${TMPDIR:-/tmp}/ppl-m17-native-linkage}"
mkdir -p "$M17_OUTPUT/config" "$M17_OUTPUT/data"
M17_OUTPUT="$(realpath "$M17_OUTPUT")"
python3 - "$HERE/scripts/script_m17_native_linkage.json" "$M17_OUTPUT" <<'PY'
import sys
from pathlib import Path
Path(sys.argv[2], 'scenario.json').write_text(Path(sys.argv[1]).read_text().replace('__M17_OUTPUT__', sys.argv[2]))
PY
export XDG_CONFIG_HOME="$M17_OUTPUT/config" XDG_DATA_HOME="$M17_OUTPUT/data"
"$HERE/run_ide_auto.sh" "$M17_OUTPUT/scenario.json" 60
python3 "$HERE/assert_m17_native_linkage.py" "$M17_OUTPUT/dump.json"
