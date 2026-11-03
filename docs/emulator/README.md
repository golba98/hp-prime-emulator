# HP Prime G1 Firmware Emulator

The Rust emulator executes original G1 firmware under Unicorn. The current CLI
exposes genuine firmware compilation and execution alongside the independent
native Rust engine. Firmware assets are supplied locally and are not distributed.

Start with the [unified CLI report](M8_UNIFIED_CLI_REPORT.md). Firmware execution
currently requires Linux. The desktop IDE offers both engines: Native PPL, and
G1 Firmware, a live original-firmware calculator ([M12](M12_FULL_CALCULATOR_REPORT.md)).

## Implementation milestones

| Report | Scope |
|---|---|
| [M4](M4_FIRMWARE_DISPLAY_REPORT.md) | Genuine LCD, input, calculator arithmetic |
| [M5](M5_PROGRAM_CATALOG_REPORT.md) | Program import, catalog, editor and persistence |
| [M6](M6_FIRMWARE_COMPILER_REPORT.md) | Original PPL compiler and diagnostics |
| [M7](M7_FIRMWARE_RUNTIME_REPORT.md) | Runtime, arguments, output, graphics and errors |
| [M8](M8_UNIFIED_CLI_REPORT.md) | Unified CLI, source identity and managed execution |
| [M12](M12_FULL_CALCULATOR_REPORT.md) | G1 Firmware calculator mode in the IDE: session lifecycle, import, input, evidence suite |
| [M15](M15_PARITY_REPORT.md) | Physical-G1 parity: version-matched 15157 firmware, three-way runner, 77 recorded device results (0 mismatches on the matched build), pending device batch |
| [M14](M14_FUNCTIONALITY_REPORT.md) | The firmware's CAS, graphing, all 18 apps and app computations through the UI; maths data types through the runtime |
| [M13](M13_STORAGE_POWER_REPORT.md) | Storage and power: the firmware's own OFF/sleep/wake, shut down, persistence across restarts, memory reset, memory figures |

Reports record evidence and limitations at each milestone. Earlier blockers and
plans below are historical; consult later reports for their resolution. Raw guest
storage and proprietary firmware remain outside Git.

## Research background

Research was performed against these upstream sources (isolated checkouts under
`scratch/research/`, never merged into project code):

| Source | Commit | Date | License |
|---|---|---|---|
| [PrimU2](https://github.com/telecomadm1145/PrimeU2) (reference HLE) | `461d994` | 2026-06-19 | GPL-2.0 |
| [qemuPrime](https://github.com/opcod3/qemuPrime) (S3C2416 machine for QEMU) | `32e891a` | 2021-08-17 | GPL-2.0 (QEMU) |
| [ripem](https://github.com/boricj/ripem) (3rd-party firmware, PRIME_OS.ROM format) | `531a570` | 2017-08-20 | BSD-style (LICENSE) |
| [hp-prime-kit](https://github.com/JordiRigau/hp-prime-kit) (PPL reference + .hpprgm format) | `7581215` | 2026-09-29 | (repo LICENSE) |
| [hpprime-pplcompile](https://github.com/pavlozs/hpprime-pplcompile) (.hpprgm container packer) | `cbb85d5` | 2026-05-24 | (repo) |

Firmware analyzed: official **HP Prime G1 Firmware 2.4.2 (2026-09-09)**
obtained from [hpcalc.org](https://www.hpcalc.org/details/7469) (HP's
distribution channel). Proprietary HP content — kept out of git
(`scratch/research/firmware/`), never redistributed.

## Documents

| Document | Contents |
|---|---|
| [PHASE1_RESEARCH_REPORT.md](PHASE1_RESEARCH_REPORT.md) | Executive summary, validation report, open questions |
| [PRIMU2_SOURCE_AUDIT.md](PRIMU2_SOURCE_AUDIT.md) | How PrimU2 actually works, file by file, with source evidence |
| [FIRMWARE_ARCHITECTURE.md](FIRMWARE_ARCHITECTURE.md) | G1 firmware structure, verified formats, memory map |
| [PPL_COMPILER_RESEARCH.md](PPL_COMPILER_RESEARCH.md) | What "compiling PPL" means on real hardware; feasibility of reusing HP's compiler |
| [FEDORA_COMPATIBILITY.md](FEDORA_COMPATIBILITY.md) | Windows-dependency audit, Fedora toolchain evidence |
| [EMULATION_ARCHITECTURE.md](EMULATION_ARCHITECTURE.md) | Architecture comparison (A/B/C), recommendation, Rust crate layout |
| [SVC_COVERAGE.md](SVC_COVERAGE.md) | Service ID inventory: 742 declared, 85 implemented, gaps |
| [PHASE2_IMPLEMENTATION_PLAN.md](PHASE2_IMPLEMENTATION_PLAN.md) | Milestones, risks, success criteria for Phase 2 |
| [M2_IMPLEMENTATION_REPORT.md](M2_IMPLEMENTATION_REPORT.md) | M2 outcome: what was built, verified startup state, findings |
| [M2_EXECUTION_TRACE.md](M2_EXECUTION_TRACE.md) | Working command, live boot trace, quantitative evidence |
| [M2_SVC_IMPLEMENTATIONS.md](M2_SVC_IMPLEMENTATIONS.md) | Service-by-service ledger with live call counts |
| [M2_VALIDATION_REPORT.md](M2_VALIDATION_REPORT.md) | All test commands and results (V1–V5) |
| [M2_KNOWN_BLOCKERS.md](M2_KNOWN_BLOCKERS.md) | The exact remaining blocker and the next executable step |
| [M3_EVENT_LOOP_REPORT.md](M3_EVENT_LOOP_REPORT.md) | M3 outcome: event-loop root-cause analysis, verified GetEvent heartbeat, rejected hypotheses |

## Key conclusions (short form)

1. **The HLE approach is validated by evidence.** The firmware's OS services
   are ARM `SVC` instructions (`svc #0x10000+N`) whose numbering is the export
   ordinal table of `SDKLIB.DLL` — 742 services, verified in the 20260909
   firmware image. Intercepting them from Unicorn is exactly what PrimU2 does,
   and the mechanism was reproduced in Rust on Fedora (test T-SVC-1, passing).
2. **Rust-on-Fedora is feasible today**: `unicorn-engine` 2.1.5 builds and
   executes ARM code, with two constraints (path-with-spaces breaks Unicorn's
   CMake; `-latomic` needed on this toolchain).
3. **PrimU2 implements only 85/742 services (11.5%)** and hard-codes a
   Windows-only UI/bridge layer; a Rust reimplementation of the architecture is
   preferred over porting PrimU2 (GPL-2.0 also constrains code reuse).
4. **A `.hpprgm` "compiled block" is not executable bytecode** — it is a
   symbol-table cache of pre-materialized constants that the calculator
   rebuilds from source. Therefore "run HP's own PPL compiler" means executing
   firmware's compile-from-source path inside the emulated environment, not
   extracting a standalone compiler.
5. **Recommended Phase 2 milestone**: headless Rust emulator core that boots
   `armfir.elf` under Unicorn with the SVC dispatch layer, a virtual FAT16
   disk, and a scripted input/display interface — no GUI.

### Desktop setup and recovery

Choose **G1 Firmware**, then **Configure firmware…** in the calculator controls.
Settings can browse to the user-supplied ARM ELF, factory A directory, optional
OS ROM and initialized A/C storage, helper executable and calculator profile.
**Validate firmware resources** checks file presence and the ARM ELF header;
startup additionally validates executable/helper and firmware capabilities.
Paths are saved with the IDE configuration and apply on the next session start.
The emulator does not download or redistribute HP firmware. Native PPL remains
available from the unavailable-session controls without these resources.

Normal OFF remains the save operation. The session publishes a versioned,
checksummed storage checkpoint only after the firmware finishes OFF. On an
unclean restart it restores the last intact compatible generation and archives
interrupted storage under the profile's `.recovery` directory. A startup
baseline can recover the initial storage when no OFF checkpoint exists. Corrupt
or incompatible generations never overwrite the current storage; startup stops
if no safe generation remains. Keep these files when diagnosing recovery.

This does **not** checkpoint live CPU/RAM state or unsaved edits. Changes since
OFF may be lost. Checkpoints are currently retained without automatic pruning;
allow disk space for complete copies of calculator storage. They are recovery
copies, not a substitute for exporting important programs.

### Experimental memory budget

`prime-emulator run --heap-budget-mib 32 ...` limits guest heap allocations while
keeping the existing mapped arena. The default remains 256 MiB for development.
Allocation failure returns null to firmware and failed realloc leaves its old
block intact. The budget excludes firmware mappings, stacks and host resources;
it is **not** an assertion of physical 32 MB G1 equivalence. Use it to investigate
allocation pressure, and record the budget alongside any parity result.
