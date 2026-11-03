# M19 NAND and executed synthetic boot chain

The research overlay executes self-authored ARM926 NAND transactions and an
actual first-stage to second-stage handoff. **PRIME_OS kernel boot: not attempted
— matching authorized boot artifacts are missing.** No proprietary firmware,
physical calculator, USB operation or production Rust HLE change is involved.

## Evidence and behavior

The pinned qemuPrime source is commit
`32e891a9fc568de3110343c3e35b9ec0d23f6f44`, VERSION2.10.1. Apply overlays0001–0004
in order. The NAND component remains GPL research code. The generic QEMU NAND
core and other machines are untouched. Overlay0004 replaces the broken
board-specific adapter with bounded page, OOB, command and backend handling.

The primary register contract is Samsung's [S3C2416 User's Manual Rev1.10,
October2008](https://www.armdesigner.com/uploadfile/download/Samsung_ARM9_S3C2416X_datasheet.pdf),
SHA256 `a45058dc61e1c90f226da236dcd1412b38dbbe3b02c4cd4271f92d469e2122fe`,
chapter7: NFDATA widths, ECC comparison inputs/output registers, reset/lock state
and status bits. The [Linux v5.15 NAND driver's
syndrome](https://github.com/torvalds/linux/blob/v5.15/drivers/mtd/nand/raw/s3c2410.c)
independently anchors the MAIN512 parity/correction ordering. This driver is
family evidence; the Samsung2416 manual establishes the exact register layout.

The implemented research chip retains the fork's EC/DA profile:256MiB main,
2048+64 bytes/page,64pages/block,131072pages, two column and three row cycles.
It is a synthetic profile, not a measured G1 flash identity. Supported sequences
are ID90, read00/30, random-read05/E0, program80/10, erase60/D0, status70 and
resetFF. Data bus widths1/2/4 perform ordered byte cycles. Program ANDs new data;
erase restores main and OOB toFF. Invalid sequences and addresses fail without
buffer overrun or flash modification. Soft/tight lock bounds and reset defaults
are explicit. Status reports busy, write protection and failure. Host media
errors propagate to the flash failure bit.

Busy work commits through virtual-clock callbacks. Read1000ns, program200000ns
and erase2000000ns are configurable research latencies, not measured G1 values.
Flash reset and machine reset cancel a pending program. R/B transition polling
is modeled; NAND interrupt delivery, DMA, alternate edge behavior, power-loss
atomicity, wear and real bad-block failure probabilities are unsupported.

Generated MAIN ECC is at0x34/38; stored comparison bytes are written to0x14/18.
INIT and main lock control generation. Independent bit-parity equations check
fixed vectors without the QEMU lookup table. Genuine ARM execution covers a
nontrivial512byte parity0x5f99a596, clean0, single-data error byte123/bit1
(status0x3d91), guest software repair, ECC area error3 and double-data error2.
NFDATA never silently repairs data. The code's2048byte address-parity extension
is a research derivation and has no independently established hardware vector;
it is not promoted to validated2048byte ECC. Spare ECC and4/8bit ECC remain
unsupported with diagnostics; generated4/8bit values never pretend to be valid.
OOB raw transport/persistence is independently exercised regardless of ECC.

Bad blocks are represented by physical OOB bytes, without automatic remapping
or invented chip-side failures. The guest sets/reads a synthetic marker at
page256, then applies its explicit test policy to skip to page320. This is not
an inferred G1 filesystem or proprietary boot bad-block layout.

## Storage isolation

The runner accepts no existing flash input. It creates a complete276824064byte
FF raw interleaved base, hashes it and bind-mounts it read-only. Sparse holes
would contain00, so they are deliberately avoided. Writes go to a disposable
qcow2 version2 overlay; its virtual size preserves full geometry while actual
writes remain below the existing8MiB file limit. Tests reopen the overlay in a
fresh QEMU process and preserve both main/OOB bytes. The base hash is unchanged
at completion. Exact-size main-only images are readable with synthetic FF OOB;
writable main-only/OOB persistence is explicitly unsupported. Original firmware
packages/storage images are never used as test media.

## Actual executed commands and observations

Build command (with existing local pinned DTC reference and owned output):

```sh
python3 scripts/emulator/full_system_build.py \
  --reference '/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/scratch/research/qemuPrime' \
  --dtc-reference /home/k9-vortex/.cache/prime-m17-fullsystem-build/dtc-reference \
  --overlay research/full-system/patches/0001-intc-reset-fiq.patch \
  --overlay research/full-system/patches/0002-s3c2416-timer.patch \
  --overlay research/full-system/patches/0003-armv5-alignment.patch \
  --overlay research/full-system/patches/0004-s3c2416-nand.patch \
  --output /home/k9-vortex/.cache/prime-m19-nand-build --jobs 4
```

The first build correctly failed for a missing old-QEMU include. Subsequent
owned incremental rebuilds produced the final overlay binary
`cfbfedab20b523cd529ed8e95efeaef15bc1f4943531da16de687bd359398b9c`.
The exact final overlay SHA is
`6050f47c30433f812f79a7f74acb3078d174c7e79f4f83e8d988568998182c6b`.
A clean integrated build is the coordinator's independent reproduction gate;
this owner result is honestly an incremental build of the recorded final source.

Commands executed with that binary:

```sh
python3 scripts/emulator/full_system_nand.py --build-root /home/k9-vortex/.cache/prime-m19-nand-build --case nand --output /home/k9-vortex/.cache/prime-m19-nand-release --timeout 30
python3 scripts/emulator/full_system_nand.py --build-root /home/k9-vortex/.cache/prime-m19-nand-build --case nand_ecc --output /home/k9-vortex/.cache/prime-m19-nand-ecc-release --timeout 30
python3 scripts/emulator/full_system_nand.py --build-root /home/k9-vortex/.cache/prime-m19-nand-build --case boot_chain --output /home/k9-vortex/.cache/prime-m19-nand-boot-release --timeout 30
PYTHONPATH=scripts/emulator python3 -m unittest scripts/emulator/test_full_system_nand.py -v
```

The compiler is clang22.1.8 (Fedora22.1.8-4.fc44). Compiler/linker paths, versions,
binary hashes, assembly/C/linker source hashes, ELF hashes, exact QEMU/container
argument vectors, register snapshots, mailboxes, instruction translations, UART,
QMP and output hashes are retained in the run folders. Machine-readable captures
are in `evidence/m19-nand.json`; CI replay is not a new ARM execution.

| Case | Host device tests | Actual ARM observations |
|---|---|---|
| nand |15 controls: erasedFF, page/OOB roundtrip,1-to-0 programming, no register fall-through, physical bad-block marker, erase main/OOB, final flash byte, malformed sequence/bounds, flash/machine reset cancellation, lock-tight, independent512 ECC vectors, mixed data widths, fresh-process persistence and read-only refusal | Entry0x30000000; three pages including last physical page each verify all2112bytes; checksum808224; guest bad-block skip256→320; UART exactly `M19 NAND\n`; terminal mailbox0x600d0019. |
| nand_ecc | Independent reference evaluated by host validator | Same ARM entry; mailbox parity0x5f99a596, clean0, correction descriptor15761, repaired checksum65284, ECC error3, double error2; UART exactly `M19 ECC\n`. |
| boot_chain | Main-only bootstrap copying across OOB and bootstrap reconstruction at QMP reset | No `-kernel`; CPU starts0 then steps4. Stage1 code is deliberately at0x1000, requiring OOB stripping. Guest reads stage2 from physical page4 through NAND MMIO into0x30010000, verifies CRC0xab6fb861 over96bytes, then calls it. Stage2 sets0x5a190002 and UART `M19 BOOT2\n`; terminal PC0x30010040. GDB never writes PC, payload, handler or success markers. |

Eight Python replay/guard tests pass. A real SIGTERM cancellation during active
qtest commands produces a failed result with `NAND experiment cancelled` and
independently verifies the owned container is absent. This result is retained
separately and never counted as successful NAND/guest execution.

Initial unsuccessful experiments remain under the owned cache: ELF unwind-table
entry mismatch, missing block backend attachment, and a positive-byte-count
`blk_pread` result initially misclassified as failure. The successful round-trip
control required correcting each issue; no assertion was removed or weakened.

## Boot boundary

The board copies first8KiB of NAND **main** data to address0, also at reset.
This is an explicit synthetic bootstrap policy, not Samsung iROM emulation:
actual ROM code, ROM ECC format, NAND boot-selection straps, real bad-block
selection and proprietary stages remain prerequisites. DRAM is already modeled
RAM; the guest sets its stack but does not prove a physical DRAM training
sequence. The executed handoff establishes NAND loading/integrity/control flow.
It is neither BXCBOOT nor PRIME_OS nor armfir startup, and never calculator
physical parity.
