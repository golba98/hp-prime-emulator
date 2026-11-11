#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$(readlink -f "$0")")"
# Install the Giac development package separately. The compiler and linker
# options can also point to an unpacked user-owned dependency prefix.
${CXX:-c++} -std=c++17 -O2 -DHAVE_CONFIG_H ${CPPFLAGS:-} main.cc ${LDFLAGS:-} -lgiac -o "${PPL_CAS_OUTPUT:-ppl-giac-helper}"
