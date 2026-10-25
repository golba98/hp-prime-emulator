#!/usr/bin/env bash
# Extract a proprietary HP Prime G1 factory A: FAT16 image for local tests.
#
# Usage: scripts/emulator/prepare_factory_a.sh <PRIME_APP.DAT|app.img> <out-dir>
# Keep firmware and extracted files outside the repository. mcopy's -n flag
# preserves existing output files. The PRIME_APP.DAT container may have an
# 8 KiB header; the FAT boot-sector OEM id identifies the volume's offset.
set -euo pipefail

find_volume_offset() {
  local image="$1"
  local oem_offset
  # Consume grep's output fully: head would cause SIGPIPE under pipefail.
  oem_offset=$(LC_ALL=C grep -abo 'BXC.OS.1' "$image" |
    awk -F: 'NR == 1 { first = $1 } END { if (NR) print first }') || return 1
  [[ -n "$oem_offset" && "$oem_offset" -ge 3 ]] || return 1
  printf '%s\n' "$((oem_offset - 3))"
}

main() {
  if [[ $# -ne 2 ]]; then
    echo "usage: $0 <PRIME_APP.DAT|app.img> <out-dir>" >&2
    return 2
  fi
  local image="$1"
  local output="$2"
  local offset
  if ! command -v mcopy >/dev/null 2>&1; then
    echo "mtools (mcopy) is required" >&2
    return 2
  fi
  if [[ ! -f "$image" || ! -r "$image" ]]; then
    echo "cannot read factory image: $image" >&2
    return 1
  fi
  if ! offset=$(find_volume_offset "$image"); then
    echo "no FAT volume (OEM id BXC.OS.1) found in $image" >&2
    return 1
  fi
  mkdir -p "$output/A"
  MTOOLS_SKIP_CHECK=1 mcopy -s -n -i "$image@@$offset" '::/*' "$output/A/"
  echo "factory A: tree in $output/A (FAT volume at byte $offset of $image)"
  ls -la "$output/A/programs/misc"
}

main "$@"
