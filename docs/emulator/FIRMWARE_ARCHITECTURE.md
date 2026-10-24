# Firmware Architecture Report

Evidence sources: official **HP Prime G1 Firmware 2.4.2 (2026-09-09)**
package (`HP_Prime_Calculator_Firmware_20260909.zip`, from
[hpcalc.org/details/7469](https://www.hpcalc.org/details/7469), HP's
Moravia distribution mirror), statically analyzed under
`scratch/research/firmware/`; the [TI-Planet HP wiki](https://wiki.tiplanet.org/HP_Prime/Firmware_files)
for the 2013-era layout; ripem's `tools/osrom2elf` for the OS-ROM header; and
PrimU2's loader code. Everything marked *verified* below was confirmed on the
20260909 image with our own parsing. The firmware is proprietary HP content:
kept in the gitignored research tree only, never committed or redistributed.

## 1. Hardware context (G1)

- SoC: **Samsung S3C2416**, ARM926EJ-S (**ARMv5TEJ**, ARM+Thumb, no
  exclusives). Verified by the OS-ROM magic `V5J\0` + `2416`, qemuPrime's
  `s3c2416` machine, and PrimU2's S3C2416 peripheral model.
- Peripherals evidenced by emulation/wire knowledge: S3C INTC at
  `0x4A000000`, PWM timers at `0x51000000`, RTC at `0x57000000`, LCD
  controller block at `0x4C800000`, NT11002 touch controller (qemuPrime),
  NAND flash (KRNLLIB exports `nand_*`), USB device + OHCI host.
- G1 hardware revisions: **VerA / VerC** (queryable through the `ARCH`
  device IOCTL 260; hpcalc release notes say "revision A or C").
- OS lineage: the runtime library set (`SDKLIB.DLL`, `KRNLLIB.DLL`,
  `COREDLL.DLL`, `MD5DLL.DLL`) is **PE32 for MS Windows CE 4.20, ARMv4** —
  the platform OS is a WinCE 4.20 derivative ("BESTA"/"BXC" platform naming).

## 2. Update package structure (20260909, verified)

```
HP_Prime_Calculator_Firmware_20260909.zip   (11,436,320 B)
  sha256: 36ce8bfa87ab887cfcebe188c881ac73b4214e0db5ab839b350a2498262b7367
├── PRIME_APP.DAT        33,554,432 B  (32 MiB application disk)
├── PRIME_MASTER.DAT      4,194,304 B  (4 MiB, purpose unverified)
├── PRIME_OS.ROM          1,048,576 B  (1 MiB kernel image)
├── Prime_FW.md5                         (component MD5 manifest)
├── release_info.txt + release_info_*.html
└── version_20260909
```

2013 packages additionally shipped `armfir.elf`, `BESTAARM.ROM`,
`BXCBOOT0.BIN`, `MASTER.DAT`, and a `firmware.zip` with boot code; the
bootloader files are not present in the 20260909 user-facing package.
The official distribution channel is HP's Connectivity Kit "Firmware" folder
(via [updates.moravia-consulting.com](https://updates.moravia-consulting.com/)).

### Version map (public firmware)

| Version | Date | Notes |
|---|---|---|
| 2.4.15515 | 2025-09-15 | PrimU2's tested target ("20250915") |
| 2.4.15714 (beta) | 2026-08-05 | hpcalc.org listing |
| 2.4.2 | 2026-09-09 | analyzed here; G1 revision A or C |

Only G1 packages matter for this project; G2 (different SoC: i.MX 6ULL) is
out of scope for Phase 2.

## 3. PRIME_OS.ROM — kernel image (verified)

24-byte header, then a flat ARM binary:

```c
struct OSROM_Hdr {          // little-endian
  uint32_t e_entry;         // 0x30000020
  uint8_t  pad1[4];
  uint32_t e_size_copy;      // 0x00100000
  uint32_t e_loadaddr;       // 0x30000000
  uint32_t e_size;          // 0x00100000
  char     e_ident[8];       // "V5J\0" "2416"
  uint8_t  pad2[4];
};
```

- Loads at **0x30000000**, entry `0x30000020` (right after the header).
- ripem (3rd-party firmware) builds replacements for this exact format and
  boot chain, proving the bootloader loads it from NAND to 0x30000000.
- PrimU2 maps the image at 0x30000000 RWX **but never enters it** — the HLE
  starts at `armfir.elf`'s entry instead. Whether armfir actually requires
  live kernel state at boot (vs. only SVC dispatch) is the central open
  question for a Rust HLE (see PHASE2 plan, milestone M2).

## 4. PRIME_APP.DAT — application disk (verified)

- Header at offset 0: `"APDSKXXX.DAT"` + `"V1.00"`; **FAT16 volume at byte
  offset 8192** — the TI-Planet "8 KB offset" claim holds for the current
  firmware (OEM label `"BXC.OS.1"`, 512 B sectors, 1 sector/cluster, 2 FATs ×
  254 sectors, 512 root entries, 65,504 sectors ≈ 32 MiB, media 0xF8).
  Geometry fields (heads/sectors/track) are zero, so Linux `mount -o
  loop,offset=8192` works but `mtools` needs `MTOOLS_SKIP_CHECK=1`.
- 2013 packages named this file `APPSDISK.DAT`; the layout is otherwise the
  same scheme. Name changes across versions mean loaders should probe the
  header magic rather than the filename.

### Disk contents (20260909, listed via mtools)

```
/                        BESTABFS.IND, FIRSTRUN.INI, APPSLIST.{INF,MAP,MD5}
/WINDOW/SYSTEM/          SDKLIB.DLL (30,208), KRNLLIB.DLL (5,632),
                         COREDLL.DLL (558,592), MD5DLL.DLL (6,144)
/PROGRAMS/TOOLS/         (tools, as in 2013: bestafir.exe/.dat, hello.exe/.dat)
/PROGRAMS/MISC/          armfir.elf (8,306,652), armfir.dat (10,612,736),
                         diagnose.exe (265,728), diagnose.dat (250,880)
```

`APPSLIST.MD5` is the integrity manifest TI-Planet found must be updated when
modifying disk files (relevant only for firmware-modification experiments,
which are out of scope).

## 5. armfir.elf (verified, 20260909)

```
ELF32 LSB executable, ARM, EABI5, statically linked, stripped
1 × PT_LOAD: off 0x34, vaddr 0x30600000, filesz 0x76F634, memsz 0x7756C0,
             flags R+X+W (single RWX segment), align 8
Entry: 0x30604AE8
Sections: ER_RO, ER_RW, ER_ZI (ARM RealView scatter naming)
```

- ~7.7 MiB of code+data plus ~24 KiB ZI (bss). One RWX segment simplifies
  HLE loading (PrimU2's 16 MiB hard-coded per-segment allocation works for
  this image but is not general).
- Mixed ARM/Thumb: entry code contains `blx` interworking calls; PrimU2 runs
  thread engines in Thumb mode.
- **armfir.dat** (10.6 MiB, same directory): begins `FF FF FF FF` followed by
  a table of u32 offset/size pairs — a companion blob (likely the same image
  in another encoding, or resources); purpose unverified. It is *not* needed
  by PrimU2 to boot (PrimU2 only reads armfir.elf).

## 6. SDKLIB.DLL and the syscall layer (verified — key insight)

`SDKLIB.DLL` is a PE32 WinCE 4.20 ARM DLL with base `0x10000000` and
**exactly 742 named exports** — the same count and order as PrimU2's service
enum (`0x10000..0x102E5`). `armfir.elf` statically contains a 742-entry stub
table at VA `0x307FBBEC`:

```
svc #0x10000+N     ; (ef 01 xx xx)
stmdb sp!, {r0}    ; reserve slot
stmdb sp!, {lr}    ; push return address
```

…which is exactly the protocol PrimU2's `svc_hook` implements (LR from `[sp]`,
`sp += 8` on return, result in `r0`). This proves the HLE boundary: the
firmware application talks to the platform through these 742 SVCs; a host
that services them does not need to execute `PRIME_OS.ROM`, `COREDLL.DLL`,
or `KRNLLIB.DLL` — unless armfir calls into them by address rather than SVC
(undetermined; KRNLLIB export names like `nand_read_page` suggest
direct-linked platform code exists).

- `KRNLLIB.DLL`: 104 exports, base `0x31800000` (DB/UI/NAND helpers).
- `COREDLL.DLL`: 558 KiB WinCE core.
- PrimU2's `syscalls_sdk.json` names 721 of the 742; IDs `0x102D1..0x102E4`
  are unnamed in the JSON but present in the enum and the DLL.

## 7. Boot chain (device) vs HLE start (emulator)

Device (inferred from ripem + bootloader file names, not fully verified):

```
BXCBOOT0.BIN / BESTAARM.ROM (NAND boot partitions)
  → load PRIME_OS.ROM to 0x30000000, enter 0x30000020
  → mount NAND/APP disk, load armfir.elf to 0x30600000
  → enter 0x30604AE8 with platform services via SDKLIB SVCs
```

PrimU2 (verified): map PRIME_OS.ROM (inert), map armfir.elf PT_LOADs, create
one thread at `0x30604AE8` with a 4 MiB stack, dispatch SVCs on the host.

## 8. Unresolved questions

1. Does armfir, in practice, ever jump *into* PRIME_OS.ROM or the DLL images
   (vs. SVC-only)? Requires boot tracing (Phase 2, milestone M2 gate).
2. What is `PRIME_MASTER.DAT` (4 MiB)? (Possibly the master calibration/
   license partition mirrored from NAND.)
3. Exact purpose and load semantics of `armfir.dat`.
4. Whether the LCD panel interface the firmware configures is genuinely
   32bpp (as PrimU2 emulates) or 16bpp with a PrimU2-specific host convention.
5. Which of the 21 unnamed service IDs (`0x102D1..0x102E4`) are, and whether
   the 20250915 target uses the same numbering as 20260909 (the 742/SDKLIB
   match suggests numbering is stable, but this should be pinned per version
   in Phase 2).
6. G2 differences (i.MX 6ULL, ARMv7-A) — explicitly out of scope for Phase 2
   but recorded because "Prime" firmware packages on HP's portal mix G1/G2.
