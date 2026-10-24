#!/usr/bin/env bash
# Extract the HP Prime G1 factory A: drive (PRIME_APP.DAT, a FAT16 image)
# into a directory, for prime-emulator data directories and the M4 tests.
#
# Usage: scripts/emulator/prepare_factory_a.sh <PRIME_APP.DAT or app.img> <out-dir>
#
# <out-dir>/A gets armfir.elf, armfir.dat (fonts/images), the system DLLs
# and FIRSTRUN.INI. The firmware is proprietary: keep <out-dir> outside the
# repository (e.g. /tmp/prime-factory) and never commit it.
#
# PRIME_APP.DAT from the 20260909 release carries an 8 KiB header before
# the FAT16 volume (app.img in the Phase 1 extraction is the bare volume);
# the boot sector is located by its "BXC.OS.1" OEM id.
set -euo pipefail

if [ $# -ne 2 ]; then
  echo "usage: $0 <PRIME_APP.DAT|app.img> <out-dir>" >&2
  exit 2
fi
image="$1"
out="$2"
command -v mcopy >/dev/null || { echo "mtools (mcopy) is required" >&2; exit 2; }

# Offset of the FAT boot sector: the OEM id sits at byte 3 of it.
oem_offset=$(grep -abo "BXC.OS.1" "$image" | head -1 | cut -d: -f1 || true)
if [ -z "$oem_offset" ]; then
  echo "no FAT volume (OEM id BXC.OS.1) found in $image" >&2
  exit 1
fi
offset=$((oem_offset - 3))

mkdir -p "$out/A"
MTOOLS_SKIP_CHECK=1 mcopy -s -n -i "$image@@$offset" '::/*' "$out/A/"
echo "factory A: tree in $out/A (FAT volume at byte $offset of $image)"
ls -la "$out/A/programs/misc"
