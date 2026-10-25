#!/usr/bin/env bash
# Print the meaningful repository structure, excluding generated artifacts.
#
# Usage: scripts/maintenance/show-tree.sh [depth]
#
# Excluded: Cargo build cache (target/), git internals, agent/scratch output,
# Python caches and virtualenvs. Read-only; never modifies or deletes files.
set -euo pipefail

DEPTH="${1:-4}"
if [[ ! "$DEPTH" =~ ^[1-9][0-9]*$ ]]; then
  echo "error: depth must be a positive integer" >&2
  exit 2
fi
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

if ! command -v tree >/dev/null 2>&1; then
  echo "error: the 'tree' utility is required (apt install tree)" >&2
  exit 1
fi

cd "$ROOT"
exec tree -a -L "$DEPTH" \
  -I 'target|.git|.agents|.claude|__pycache__|.venv|node_modules|scratch'
