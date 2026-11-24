#!/usr/bin/env bash
# Install public build and headless GUI dependencies in a Debian/Ubuntu environment.
set -euo pipefail
if ! command -v apt-get >/dev/null; then
  echo "Install the dependencies listed in docs/development/SCRIPTS.md for this distribution." >&2
  exit 2
fi
if [ "$(id -u)" -eq 0 ]; then
  elevate=()
else
  elevate=(sudo)
fi
"${elevate[@]}" apt-get update
"${elevate[@]}" apt-get install -y build-essential clang libclang-dev cmake libudev-dev pkg-config \
  python3 python3-pil xvfb xauth libx11-6 libxext6 libxrandr2 libxi6 libxcursor1 \
  libxkbcommon0 libgl1-mesa-dri libegl1-mesa-dev
rustup component add rustfmt clippy
cargo fetch --locked
