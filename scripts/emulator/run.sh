#!/usr/bin/env bash
# Build/run helper for the prime-emulator crate.
#
# Why this exists: Unicorn 2.x's bundled CMake build fails when the build
# directory path contains a space, and this repository's checkout path
# contains one. Redirecting CARGO_TARGET_DIR to a space-free location
# outside the repository makes all cargo subcommands work unchanged
# (`build`, `check`, `test`, `clippy`, `fmt`, `run`).
#
# Usage:
#   scripts/emulator/run.sh <cargo args...>
#   scripts/emulator/run.sh run -- --firmware ...     # run the emulator
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Space-free build dir one level above the repository.
export CARGO_TARGET_DIR="${REPO_ROOT}/../.cache/prime-emulator-target"
mkdir -p "$CARGO_TARGET_DIR"

cd "$REPO_ROOT"
exec cargo "$@"
