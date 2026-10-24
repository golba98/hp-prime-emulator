# PPL Compiler Research

Question: can a Rust firmware emulator realistically execute **HP's own PPL
compiler** instead of reimplementing PPL? Evidence assembled in Phase 1 from
the 20260909 firmware image, the hp-prime-kit (commit `7581215`, measured on a
physical G2 with firmware 2.4.15515 and on HP's Virtual Calculator 2.4 build
2025-09-15), the 2013-era official Linux Virtual Calculator binary (local,
symbols intact), ripem, and the hpprime-pplcompile project.

## 1. Where compilation happens on real hardware

The calculator's PPL toolchain is **inside the firmware image** — there is no
separate compiler component:

- `armfir.elf` contains the entire calculator application, including the
  program editor, the PPL language processor, and the CAS (giac). The disk's
  `PROGRAMS/` directory holds only payload programs, not tools.
- The 2013 Virtual Calculator binary (x86 port of the same codebase, symbols
  intact) shows the relevant machinery lives in one process: `CProgram`,
  `EditProgram`, `PrgmVarGet/PrgmVarSet`, `CallPrgmFOR/CASE/IFTE`,
  `ParseExpression`/`TParseData`, `CCompiled::CCompiling::Compile`
  (CAS/formula compiler). No separate `pplc`-style tool exists.

Compilation is triggered by the calculator itself (measurements are from
hp-prime-kit, labeled G2/fw 2.4.15515):

- Saving a program on the calculator writes **source + a "compiled block"**
  into the `.hpprgm`.
- The block is a **cache**, not the authoritative artifact: installing a
  program with one value altered inside its block and source untouched
  produced the *source's* value at run time, and the returned file contained
  a **rebuilt** block ("formats.block-is-a-cache").
- The Connectivity Kit, by contrast, writes source-only files.

## 2. What the "compiled block" actually is

hp-prime-kit's format walk (verified over a 367 KB block, landing exactly at
the source record):

- `.hpprgm` = nested TLV container, magic `7C 61 8A B2`, little-endian.
- The "block" is a **symbol table**: one entry per declared global
  (`0040018B` variable, `0040020B` function) plus a `Main` entry
  (`0040008B`) whose value holds the UTF-16LE source. Constants are
  pre-materialized in the calculator's internal formats:
  - numbers: 8-byte little-endian u64 — bits 0..11 = 12-bit two's-complement
    exponent, bits 12..59 = 12 BCD digits, bits 60..63 = sign nibble (0/9);
    zero = all zeros;
  - matrices follow the `.hpmat` layout (type byte `0x14` at offset 2, rank,
    rows, cols, then numbers).
- Blocks are **not byte-stable** across compiles (uninitialized padding).

**Conclusion:** the on-disk "compiled block" is a serialized
constants/symbol cache — *not* translated program logic. Program logic is
compiled in RAM from source at run (or at program entry), inside armfir. The
storage format therefore tells us the data representation, not an ISA.

## 3. What "compiled PPL" looks like in memory — unknown, deliberately

No public reverse engineering establishes the in-RAM representation of a
compiled PPL program (register VM? AST? token stream? threaded code?).
Evidence that it exists: the calculator's run-time "syntax error at line N"
diagnostics happen on Check/Run, and run-time speed suggests pre-compiled
forms. This is a genuine open research question for Phase 3+, answerable via
firmware tracing: break on the PPL entry services after pressing Check, dump
the guest heap, diff against source.

## 4. Existing `.hpprgm` tooling — packaging vs compilation

| Tool | What it really does | Verdict |
|---|---|---|
| `hpprime-pplcompile` (pavlozs, `cbb85d5`) | Container packer/unpacker; README states explicitly: "pure format converter … without parsing or validating HP PPL syntax" | **Not a compiler** |
| hp-prime-kit `hpprime write` / `read` | Round-trips source through the container, byte-identical to Connectivity Kit output; rejects calculator-written files because their stale block would mismatch | Packaging, very well characterized |
| hp-prime-kit `matrix nums` | Walks the compiled block and decodes symbol-table values | Block *reader*, confirms the cache model |
| Connectivity Kit | Ships source to the device | Packaging |

Our own `ppl-syntax`/`ppl-compiler` crates are a reimplementation used for
the current IDE — under this project's goals they are a compatibility oracle,
not the target execution path.

## 5. Can the original compiler be driven programmatically?

Yes — through firmware execution, which is precisely the architecture this
project targets. Three grades of automation, in increasing fidelity:

1. **Virtual Calculator automation** (works today, no emulator needed):
   hp-prime-kit proves the official Linux/Windows Virtual Calculator (the
   firmware compiled for x86) can be driven by (a) writing files into its
   startup-scanned calculator folder, (b) restarting it, (c) **one manual
   keypress** to launch the generated wrapper program, and (d) reading back
   `M9.hpmat` written on close. Limits: one human keypress per run, no
   in-process API, black-box I/O through matrices, and the VC is an old
   x86 firmware build (our local AppImage is 20130415; the kit targets VC
   2.4/2025-09-15).
2. **HLE emulator with scripted input** (Phase 2 target): boot armfir under
   Unicorn, inject keystrokes/touch through the input queue, capture the LCD
   framebuffer, and read/write the virtual FAT16 disk. PrimU2's TCP
   `key/touch/screenshot` interface plus QuickTransfer's protocol (file
   inject incl. `KEY_INJECT 0x40`, `WRITE_MEM/READ_MEM`) demonstrate this
   works against real firmware. PPL programs are then compiled by HP's code
   exactly as on hardware.
3. **Direct internal invocation** (speculative, Phase 3+): locate the
   firmware's compile-from-source entry (e.g. the function behind the
   editor's Check action) and call it via `ExecuteCustomCode`-style guest
   invocation. Requires reverse engineering; no public evidence yet.

## 6. Feasibility answer

**Yes — the Rust emulator can and should use HP's own PPL compiler**, because
the compiler is inseparable from the firmware we are already executing;
compilation happens in-guest by construction. What we must *not* assume:

- We cannot extract a standalone compiler (none exists as a separate
  artifact).
- We cannot rely on the `.hpprgm` block as compiler output for program logic
  (it is a constants cache).
- Compatibility with a physical G1 is established by the differential-testing
  harness this repo already has (`ppl-compat`, `ppl-device`): run the same
  `.hpprgm` on the emulator and on the real G1 over USB and compare
  screen/output evidence — the same methodology already proven for the
  reimplementation, now pointed at the firmware-execution emulator.

## 7. Open questions and tracing strategy

1. Which guest entry points implement PPL parse/check/run (static: find the
   editor command table in armfir; dynamic: SVC+block tracing from a
   scripted Check keypress — both Phase 3).
2. Where the run-time form of a compiled program lives in the guest heap
   (heap snapshot diffing around program start).
3. Whether firmware 20260909's PPL compiler differs observably from
   20250915 (should be pinned once a 20250915 image is obtained for the
   version-locked test set).
4. Python-app (`main.py`/MicroPython) execution path — separate subsystem,
   same in-guest principle, out of scope for Phase 2.
