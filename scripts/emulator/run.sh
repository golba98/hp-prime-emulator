#!/usr/bin/env bash
# Cargo build/run helper for the Rust workspace.
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
# Resolve the shared default from Git's canonical common directory, not a
# temporary worktree's parent. Explicit isolated slots take precedence.
COMMON_DIR="$(git -C "$REPO_ROOT" rev-parse --path-format=absolute --git-common-dir)"
CANONICAL_ROOT="$(dirname "$COMMON_DIR")"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${CANONICAL_ROOT}/../.cache/prime-emulator-target}"
case "$CARGO_TARGET_DIR" in /*) ;; *) CARGO_TARGET_DIR="$REPO_ROOT/$CARGO_TARGET_DIR";; esac
export CARGO_TARGET_DIR="$(realpath -m "$CARGO_TARGET_DIR")"
case "$CARGO_TARGET_DIR" in *" "*) echo "Unicorn requires a space-free CARGO_TARGET_DIR" >&2; exit 2;; esac
mkdir -p "$CARGO_TARGET_DIR"

cd "$REPO_ROOT"
exec cargo "$@"
