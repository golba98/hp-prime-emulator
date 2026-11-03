# M19.5 — Storage footprint audit

Audit date: 2026-11-03. Source checkpoint: `0a259072fdc85d26fad9c64231ba1182bf4e0566`, branch `feat/g1-usability-overhaul`.

No cache retirement, container prune, worktree removal or historical evidence cleanup was executed. Existing firmware, calculator saves, research images and validated build roots remain in place. The GUI validation uncovered a legacy scratch-capture lifecycle exception, described explicitly under verification. No M20 or hardware feature work was performed.

## Measured scope and accounting

The reported **170 GB** closely matches the repository **56.821 GB** plus sibling Cargo targets **111.94 GB**, approximately **168.76 GB** together. It understates the complete project-related environment: the baseline persistent footprint is **250.035 GB in unique-inode allocation accounting**.

The read-only baseline covers 407 discovered roots. Its exact regular-file allocation is 249,408,004,096 bytes, including 4,841,472 bytes on `/tmp` tmpfs. Subtract tmpfs and add the separately identified project container-layer union of 631,357,440 bytes: **250,034,520,064 bytes**. Regular-file logical size before that container attribution is 249,075,756,778 bytes. Directory metadata and filesystem metadata are not included in this regular-file metric.

All sizes below use decimal GB (10⁹ bytes). The JSON evidence records exact bytes. The broad `du` entry sum was approximately 263.86 GB: counting each `(st_dev, st_ino)` once removes about 14.45 GB of hardlink duplication. The scan avoids following symlinks and crossing VFS devices. Nested directory tables are descriptive, not additive.

This is **not an exclusive physical-space measurement**. `/home` and `/var/tmp` are different Btrfs subvolumes of the same filesystem UUID `7fa282b3-7c57-4177-97e6-a9aab7045167`, with `compress=zstd:1`; do not add their `df` totals. `filefrag` confirms shared/encoded extents in sampled firmware profiles. `compsize` SEARCH_V2 and snapshot enumeration were denied; noninteractive sudo was unavailable. Compression, reflinks and external snapshots prevent exact attribution of physical consumption or recovery. The same file may consume fewer physical bytes than its `st_blocks` accounting, and deleting one reference may free none of its shared extents. The shared filesystem reports 759,359,455,232 used bytes and 256,875,020,288 available bytes at the final environment check; that includes unrelated machine data and must not be charged to this project or subtracted from its logical allocation to infer ownership.

Evidence: [baseline inventory](../../evidence/m19-5/baseline.json), [external attribution](../../evidence/m19-5/external-measurements.json), [environment commands](../../evidence/m19-5/environment-readonly.json), and [final allocation snapshot](../../evidence/m19-5/after.json). The inventory records unknown ownership conservatively rather than declaring all discovered names disposable.

## Ranked category breakdown

| Category | Location | Actual allocation | Required? | Safely reclaimable |
|---|---|---:|---|---|
| Cargo build outputs | 16 populated targets plus one empty target location; exact paths below | 174.933 GB | B: reproducible products; preserve exact referenced binaries first | 115.074 GB of conditional candidates; physical recovery unknown |
| Milestone artifacts | canonical repository `artifacts/` | 30.628 GB | Mixed A/D: protected evidence and copied test profiles | No removal approved |
| Research scratch | canonical repository `scratch/research/` | 26.061 GB | Mixed A/D: source, evidence, firmware, Windows VM chain | No removal approved |
| NAND | 29 old bases and 29 overlays across artifact/cache roots | 8.040 GB | Three distinct bases; preserve backing chains and failed runs | Future reference reduction; not independent physical savings |
| QEMU build roots | 19 top-level roots under `~/.cache`, including three nested builds | 4.604 GB regular files; 4.611 GB `du` including metadata | B products mixed with retained manifests/reproducibility evidence | Separate scoped review only |
| Backups | `/home/k9-vortex/prime-emulator-backups` | 3.304 GB | A: protected backup | 0 authorized |
| Project container layers | six layer directories under `~/.local/share/containers/storage/overlay` | 0.631 GB | Validated chain 0.391 GB; historical project-only layer 0.240 GB | No prune authorized |
| Firmware, saves and IDE state | `~/.local/share/hp-prime-emulator` and `hpprimepplemulator&nativeide` | 0.184 GB | A: protected user resources | 0 authorized |
| Git | canonical `.git/` | about 0.022 GB | A: history and ownership metadata | 0 |

Rows overlap: NAND images and QEMU roots are already represented in their parent roots. **Do not sum this table.** The non-overlapping root list in `baseline.json` reconciles the total. The source/docs/tests/fixtures footprint is approximately 0.11 GB; source is not the cause of the reported growth.

Excluded shared environment storage: Cargo home 1.985 GB, Rustup 2.397 GB, Docker Desktop VM 25.226 GB, unrelated Podman layers, and unrelated browser/AI caches. These are broader machine storage, not exclusively project-owned costs. No shared-image layer was charged twice. The layer-path/identity union is recorded in [project-container-layers.json](../../evidence/m19-5/project-container-layers.json); the 147,582,976-byte Ubuntu base is shared by the project toolchain images and the retained Ubuntu base-image record. Container figures include directory metadata from `du`, unlike the regular-file root inventory.

## Cargo storage and build-cache architecture

The canonical target `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target` accounts for **81.914 GB**: debug 79.540 GB and release 2.371 GB. Debug dependencies account for 44.531 GB, incremental state 22.056 GB, build-script/native outputs 9.502 GB and examples 3.172 GB. There are 6,132 debug dependency entries and 1,226 incremental directories. Revision-specific hashed outputs have accumulated across milestones.

Isolated worktrees have duplicated compilation rather than sharing one target. The accidental `/home/k9-vortex/.cache/.cache/prime-emulator-target` holds another 5.782 GB: the repository-relative target setting resolves differently in cached worktrees. All populated target configurations were measured in [cargo-profiles.json](../../evidence/m19-5/cargo-profiles.json); the table below separates debug, release and alternate target triples. Small root metadata is outside profile subtotals.

| Target location | Debug GB | Release GB | Other target/profile GB |
|---|---:|---:|---|
| `/var/tmp/hp-prime-pr21-build` | 4.552 | 1.332 | x86_64-pc-windows-gnu: 0.204816 |
| `/home/k9-vortex/.cache/prime-m16-recovery-target` | 4.761 | 0.612 | — |
| `/home/k9-vortex/.cache/prime-m16-stateful-target` | 4.949 | 0.612 | — |
| `/home/k9-vortex/.cache/prime-m17-native-target` | 9.254 | 1.466 | — |
| `/home/k9-vortex/.cache/prime-m17-runtime-target` | 8.676 | 0.615 | — |
| `/home/k9-vortex/.cache/prime-m18-native-target` | 9.306 | 0.619 | — |
| `/home/k9-vortex/.cache/prime-m18-snapshot-target` | 5.246 | 0.000 | — |
| `/home/k9-vortex/.cache/prime-m19-hardware-target` | 5.003 | 0.000 | — |
| `/home/k9-vortex/.cache/.cache/prime-emulator-target` | 5.042 | 0.740 | — |
| `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target` | 79.540 | 2.371 | — |
| `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target-m12` | 5.168 | 1.332 | — |
| `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target-main` | 0.000 | 0.903 | — |
| `/home/k9-vortex/Development/4-Rust/.cache/prime-keyboard-target-agent` | 5.373 | 1.335 | — |
| `/home/k9-vortex/Development/4-Rust/.cache/prime-next-core-target` | 5.842 | 0.000 | — |
| `/home/k9-vortex/Development/4-Rust/.cache/prime-next-differential-target` | 4.799 | 0.000 | — |
| `/home/k9-vortex/Development/4-Rust/.cache/prime-next-native-target` | 5.278 | 0.000 | — |

The canonical target contains eight Unicorn native-library output variants, including duplicate build/install archives inside each variant. Cargo.lock retains Unicorn 2.1.5, but its default feature set enabled all architectures. The production emulator only uses ARM/Thumb. Disabling default features and enabling `arch_arm` produces a measured debug `libunicorn.a` of **22,330,426 bytes**, compared with **334,665,014 bytes** for the sampled all-architecture archive. No guest semantics or release optimization settings changed.

Implemented build policy, selected by the user:

- Dev and test profiles use `debug=1`, `incremental=false`. Line information remains; complete Rust local-variable debugging requires the explicit `dev-full` profile (`debug=2`, incremental enabled). PPL source debugging remains covered by its tests.
- `scripts/emulator/run.sh` respects explicit `CARGO_TARGET_DIR`, normalizes it to an absolute space-free path and derives its shared default from Git's common metadata. Worktrees using the wrapper no longer accidentally create a new target based on their parent directory.
- Use one canonical target for sequential compatible jobs. Cargo locks serialize shared-target builds. For genuinely simultaneous jobs or distinct toolchain/configuration experiments, use explicitly owned target slots; do not assume a shared target makes concurrent builds independent.
- Historical worktrees running plain Cargo can still resolve the old relative `.cargo/config.toml`. Use the wrapper or an explicit target there. No global Cargo configuration or old target was rewritten.

A fresh compact target was used for verification; existing targets were preserved. It contains workspace tests, all-target Clippy and release binaries in approximately **8.523 GB**, versus 81.914 GB in the accumulated canonical target. This compares a fresh compact campaign with historical accumulation, not identical generations with only one setting changed. The initial test compilation finished in 34.74 seconds and release compilation in 57.12 seconds on this host with dependencies already downloaded. Exact historical reconstruction may need the retained branch, lockfile, toolchain and flags; these timings are not universal cold-download estimates.

## QEMU and container growth

There are **19 top-level build roots and three nested reproducibility builds**, not 19 independently installed toolchains. Top-level accounting includes the nested builds. Typical completed roots occupy about 257 MB (`source` about 84 MB plus `build` about 173 MB); failed exports remain around 82–84 MB. The M17 root includes the three nested builds and the pinned DTC reference. Failed builds and their diagnostics were retained.

Keep `scratch/research/qemuPrime`, DTC at `~/.cache/prime-m17-fullsystem-build/dtc-reference`, and the original validated `~/.cache/prime-m19-integrated-build-04`. Its executable SHA256 is `e474ec9f608ea44aaf4934d3dafaf8546a78c8985c2d4bf148f667c262e777b3`; the pinned QEMU source revision is `32e891a9fc568de3110343c3e35b9ec0d23f6f44`. The validated Xenial toolchain image is `ebb5b50b3fee1ce2879db02927a7f64437f9ab7b6bb7e1750fdb52d08aa3b2b7`.

The new `full_system_build.py --reuse-build PATH` verifies the reference pin, DTC revision, ordered overlay hashes, configure flags, Containerfile hash, actual executable digest and current runtime image identity. A matching recipe publishes a **new** compact runtime bundle; it does not recompile, export another source tree or overwrite an existing destination. Mismatches fail before creating output. Fresh full builds remain available. Bundles retain GPL licenses, compiler/package records, a manifest and a recipe fingerprint. Keep the original sources and overlays; a compact binary bundle is not a replacement for corresponding source.

The verified new bundle `/home/k9-vortex/.cache/prime-m19-5-runtime` occupies **43,122,688 bytes** and executes the exact original validated binary. The original approximately 257 MB build remains untouched.

| Top-level QEMU root | Regular-file allocation GB | Manifest status |
|---|---:|---|
| `/home/k9-vortex/.cache/prime-m17-fullsystem-build` | 1.028518 | built |
| `/home/k9-vortex/.cache/prime-m18-hardware-alignment-build` | 0.256434 | built |
| `/home/k9-vortex/.cache/prime-m18-hardware-build` | 0.256438 | built |
| `/home/k9-vortex/.cache/prime-m18-hardware-final-build` | 0.256438 | built |
| `/home/k9-vortex/.cache/prime-m18-hardware-published-build` | 0.083706 | failed |
| `/home/k9-vortex/.cache/prime-m18-hardware-published-build-02` | 0.256446 | built |
| `/home/k9-vortex/.cache/prime-m19-clock-build` | 0.082239 | failed |
| `/home/k9-vortex/.cache/prime-m19-clock-build-02` | 0.256475 | built |
| `/home/k9-vortex/.cache/prime-m19-clock-final-build` | 0.256479 | built |
| `/home/k9-vortex/.cache/prime-m19-clock-published-build` | 0.256479 | built |
| `/home/k9-vortex/.cache/prime-m19-clock-published-final-build` | 0.083722 | failed |
| `/home/k9-vortex/.cache/prime-m19-clock-published-final-build-02` | 0.256487 | built |
| `/home/k9-vortex/.cache/prime-m19-display-build` | 0.082239 | failed |
| `/home/k9-vortex/.cache/prime-m19-display-build-02` | 0.256471 | built |
| `/home/k9-vortex/.cache/prime-m19-integrated-build` | 0.082239 | failed |
| `/home/k9-vortex/.cache/prime-m19-integrated-build-02` | 0.083739 | failed |
| `/home/k9-vortex/.cache/prime-m19-integrated-build-03` | 0.256614 | built |
| `/home/k9-vortex/.cache/prime-m19-integrated-build-04` | 0.256614 | built |
| `/home/k9-vortex/.cache/prime-m19-nand-build` | 0.256541 | built |

Nested roots are `prime-m17-fullsystem-build/repro-build`, `overlay-repro` and `published-overlay-build`. Exact nested sizes and manifests are in [qemu-nested-builds.json](../../evidence/m19-5/qemu-nested-builds.json); top-level identities are in [qemu-builds.json](../../evidence/m19-5/qemu-builds.json).

Rootless Podman storage totals **9.746 GB** in allocation accounting. The project-image chain union is **0.631 GB**; other known images account for about 5.660 GB; 3.236 GB unreferenced by current image records remain **D — unknown**. Absence from the current image list does not establish safe ownership. Four unrelated unlinked volumes are also protected. No running or stopped container existed at either inventory check. No generic prune, layer deletion or image removal was run. Runtime test containers are removed by their existing scoped lifecycle.

The largest research file is the Windows `research-overlay.qcow2`, 12.155 GB allocated, with an 80 GiB virtual capacity. Its backing file is `scratch/research/official-software-20261025/windows/research.qcow2`; the evaluation ISO occupies 5.113 GB. Keep the backing chain and provenance records. Virtual capacity is not current disk consumption, and the backing file cannot be removed independently.

## NAND bases, profiles and evidence

All **29 old 276,824,064-byte NAND bases** were fully rehashed without modification. They account for 8,027,897,856 bytes; the 29 private overlays together account for only 12,242,944 bytes. Exact paths, device/inode identities and hashes are in [nand-bases.json](../../evidence/m19-5/nand-bases.json).

| SHA256 | Count | Meaning |
|---|---:|---|
| `fbee2e0f41769c042340dff3211f37f3933c097684bce976a69222767d10656b` | 21 | Erased synthetic NAND |
| `1785ecc15bbfb2fbe9d1334afa8db549ae24ac1a0179ca243e7b95aa65d88ae0` | 7 | Current synthetic boot payload |
| `42368b43f80916ff95a5d27216e02662611b0431e44f0467f95e8bac1c51b435` | 1 | Earlier synthetic boot payload |

The final inventory also lists the largest sparse/compression candidates. For example, an old `debug/deps/app_suite-1e339db95435f089.tmpc47e0fd` has logical length 117,738,568 bytes but only 9,633,792 allocated bytes; its name suggests an interrupted link output, but that does not prove inactivity or authorize removal. Extent inspection is needed to distinguish holes from compression.

Sparse raw holes read as zero and cannot represent erased NAND FF bytes without changing semantics. Geometry, interleaved OOB bytes, ECC vectors and the test payload remain unchanged.

Future runs atomically publish immutable mode-0400, content-addressed bases under `~/.cache/hp-prime-emulator/research/nand` (override `PPL_RESEARCH_CACHE`). Per-run `base.raw` is a regular hardlink, compatible with existing validators and relative qcow2 backing filenames. Cross-filesystem fallback creates a new private read-only copy. Each writable qcow2 overlay remains private. Existing corrupt cache entries fail closed and are never repaired or overwritten. Tests cover concurrent publication, corruption, existing-output refusal and cross-filesystem ownership.

Two new base variants used by this validation account for 553,648,128 bytes once; multiple run aliases share their inodes. Older 29 bases were not migrated, replaced or removed. Failed/cancelled overlays remain diagnostic evidence rather than being silently pruned. The old repeated bases are future reference-deduplication candidates only after each run's backing-chain and evidence requirements are reviewed.

There are **1,673 `armfir.elf` copies**, about 13.877 GB before extent-sharing deduplication. A sampled benchmark profile has shared/encoded extents and an independent inode: existing copying already benefits from reflinks. That is not 13.877 GB of independently reclaimable physical space. Never replace writable firmware profiles with hardlinks. User firmware, saves, OFF persistence, validated recovery generations and unknown snapshots remain protected.

`artifacts/diagnostic-parity` (6.253 GB), `consolidation-preserved` (4.883 GB), M19 (4.332 GB), M17 (2.335 GB), M14 (2.210 GB) and M18 (1.332 GB) are major retained groups. Consolidation-preserved evidence may be unique even where its old worktree is gone. Screenshots total approximately 0.393 GB by extension; logs are not the dominant cost. Keep physical references, failed-test evidence, deletion/replacement journals, recovery references and reproduction manifests.

## Classification and exact cleanup proposal

A means essential/protected, B rebuildable, C confirmed disposable temporary data, D unknown or unsafe. Baseline automatic root classification finds A **3.488 GB**, B **174.933 GB**, C **0**, D **70.987 GB**, before tmpfs exclusion and the separate container attribution. Source/history and irreplaceable evidence inside mixed D roots remain protected. **A is a conservative recognized subset, not the whole essential set.**

Conservatively retain the complete **75.102 GB non-Cargo baseline pool**, including mixed/unknown data and project container layers. No confirmed orphan is asserted. Finished-looking names, age and missing visible Cargo processes are insufficient: three process fd directories were inaccessible. Ownership claims and open files require a fresh review immediately before any later removal.

[cleanup-proposal.json](../../evidence/m19-5/cleanup-proposal.json) lists exact paths, byte sizes, purposes, sharing uncertainty, recovery cost, guards and proposed argv. It is a **dry-run proposal, not authorization**. The first phase covers **115,074,306,048 bytes** of allocation: 15 historical Cargo targets and only the canonical `debug/incremental` directory. The canonical target's remaining products and all validated QEMU roots are retained in that phase. Current compact outputs are not candidates.

| Candidate path | Allocation GB | Owner/purpose | Risk / later action |
|---|---:|---|---|
| `/var/tmp/hp-prime-pr21-build` | 6.089 | project Cargo fingerprints; historical build slot | Conditional: preserve references, owner release, activity check and explicit approval |
| `/home/k9-vortex/.cache/prime-m16-recovery-target` | 5.372 | project Cargo fingerprints; historical build slot | Conditional: preserve references, owner release, activity check and explicit approval |
| `/home/k9-vortex/.cache/prime-m16-stateful-target` | 5.561 | project Cargo fingerprints; historical build slot | Conditional: preserve references, owner release, activity check and explicit approval |
| `/home/k9-vortex/.cache/prime-m17-native-target` | 10.720 | project Cargo fingerprints; historical build slot | Conditional: preserve references, owner release, activity check and explicit approval |
| `/home/k9-vortex/.cache/prime-m17-runtime-target` | 9.291 | project Cargo fingerprints; historical build slot | Conditional: preserve references, owner release, activity check and explicit approval |
| `/home/k9-vortex/.cache/prime-m18-native-target` | 9.925 | project Cargo fingerprints; historical build slot | Conditional: preserve references, owner release, activity check and explicit approval |
| `/home/k9-vortex/.cache/prime-m18-snapshot-target` | 5.246 | project Cargo fingerprints; historical build slot | Conditional: preserve references, owner release, activity check and explicit approval |
| `/home/k9-vortex/.cache/prime-m19-hardware-target` | 5.003 | project Cargo fingerprints; historical build slot | Conditional: preserve references, owner release, activity check and explicit approval |
| `/home/k9-vortex/.cache/.cache/prime-emulator-target` | 5.782 | project Cargo fingerprints; historical build slot | Conditional: preserve references, owner release, activity check and explicit approval |
| `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target-m12` | 6.500 | project Cargo fingerprints; historical build slot | Conditional: preserve references, owner release, activity check and explicit approval |
| `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target-main` | 0.903 | project Cargo fingerprints; historical build slot | Conditional: preserve references, owner release, activity check and explicit approval |
| `/home/k9-vortex/Development/4-Rust/.cache/prime-keyboard-target-agent` | 6.709 | project Cargo fingerprints; historical build slot | Conditional: preserve references, owner release, activity check and explicit approval |
| `/home/k9-vortex/Development/4-Rust/.cache/prime-next-core-target` | 5.842 | project Cargo fingerprints; historical build slot | Conditional: preserve references, owner release, activity check and explicit approval |
| `/home/k9-vortex/Development/4-Rust/.cache/prime-next-differential-target` | 4.799 | project Cargo fingerprints; historical build slot | Conditional: preserve references, owner release, activity check and explicit approval |
| `/home/k9-vortex/Development/4-Rust/.cache/prime-next-native-target` | 5.278 | project Cargo fingerprints; historical build slot | Conditional: preserve references, owner release, activity check and explicit approval |
| `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/incremental` | 22.056 | canonical compiler incremental cache | Conditional: preserve references, owner release, activity check and explicit approval |

Each historical target can be recreated from its retained branch/lockfile/toolchain and downloaded dependencies. The fresh current build demonstrates tens-of-seconds compilation here, but exact old binaries may require historical flags or resources; rebuilding current source does not reproduce their hashes. Before any proposed `cargo clean --target-dir EXACT_PATH`, preserve every exact binary/manifest still referenced by benchmarks, recovery records or research manifests. Renew device/inode, backing-chain, owner and open-file checks. The incremental candidate must be handled as that exact child path, never its parent or a wildcard. The JSON records that separate manual action; it is not executed by the doctor.

After this phase, baseline allocation would be **134.960 GB**, or approximately **144.5 GB** including the new validation footprint. Physical bytes recovered remain unknown. A second, separately reviewed retirement of the rest of the 174.933 GB historical Cargo pool could approach the measured retained pool plus one compact active target, roughly **85 GB** with this campaign's outputs. That broader canonical-target retirement is **not** included in the first-phase command list.

Unknown Podman layers, worktrees without owner release, proprietary resources, calculator saves, VM backing chains, recovery generations and unique evidence have no cleanup command. QEMU exports and identical historical NAND bases may offer additional logical reductions after reference review, but their physical savings are not added to Cargo candidates.

## Preventative architecture and retention

Implemented:

- Compact Rust defaults and ARM-only Unicorn; explicit full-debug profile and owned parallel-cache slots.
- Canonical shared-target resolution in the wrapper, with explicit overrides honored.
- Verified QEMU recipe reuse and new-only small runtime bundles; no toolchain rebuild on a verified cache hit.
- Shared immutable synthetic NAND bases with private writable overlays.
- Read-only `scripts/emulator/storage_doctor.py`: category sizes, unique-inode totals, logical/allocation distinction, top files/roots, directories above 1 GB, growth comparison, rebuildable/retained totals, uncertainty and configurable warning budgets. There is no destructive interface.
- M12/M14 GUI scenarios now relocate capture paths into a newly created run output, refuse an existing run directory, use private logs, propagate GUI failure, and clean only their newly created private profile on success. Failure records retain the new profile path. Unit tests cover relocation and traversal refusal; actual GUI execution verifies unchanged legacy captures.

Recommended retention policy, **not automatic eviction**: permanent physical/reference/reproduction evidence; retained important failures; recent successful summaries and representative captures; explicitly disposable successful-run workspaces. Use content-addressing only for verified immutable data. Use existing reflink copying for writable firmware profiles, private qcow2 overlays for NAND, and lossless compression for cold evidence after provenance review. Never compress/move files out from under live consumers or silently rotate recovery generations. Existing QEMU runners already limit each diagnostic file to 8 MiB; no old log was truncated and no snapshot was evicted.

Usage:

```bash
python3 scripts/emulator/storage_doctor.py --profile standard
python3 scripts/emulator/storage_doctor.py --compare evidence/m19-5/baseline.json --format markdown
python3 scripts/emulator/storage_doctor.py --root /exact/owned/path --budget-gb 20
scripts/emulator/run.sh test --workspace
scripts/emulator/run.sh build --profile dev-full
```

Discovery is conservative and name-assisted; it does not inspect every unrelated home cache or automatically classify shared container layers. The separate container attribution in this audit remains necessary. New locations can be supplied explicitly with `--root`. A/D data are retained pending review, C is zero until independently proven, and budget overruns only warn. No periodic cleanup task or background service was installed.

## Development storage budgets

Planning budgets use measured allocation accounting and retain the **75.10 GB historical non-Cargo pool**. They assume separately approved retirement of historical Cargo products, not deletion of evidence. Shared Cargo/Rustup installations and unrelated container/VM storage remain outside these project budgets. The measured full validation target is about 8.52 GB; a separate simultaneous slot adds approximately that much during a comparable campaign.

| Configuration | Steady-state planning ceiling | Temporary peak | Basis |
|---|---:|---:|---|
| Minimal development | 90 GB | about 99 GB | Retained pool + one compact target (~8.52 GB) + recent output and headroom; Rust HLE/IDE |
| Standard development | 100 GB | about 109 GB | Retained pool + up to 15 GB active Rust outputs + recent firmware profiles/evidence and headroom |
| Full research | 115 GB | about 125 GB | Retained pool + up to 25 GB active Rust slots + ~1 GB research source/runtime + ~1 GB synthetic base variants + ~5 GB recent evidence and headroom |

These are measured-data planning envelopes, not physical-space guarantees or achieved reductions. Existing retained pool sizes dominate even minimal development; a clean installation without historical research would differ. Peaks permit one additional measured compact build plus a roughly 257 MB QEMU rebuild and 277 MB new NAND variant where needed. The doctor defaults to these 90/100/115 GB ceilings; override per campaign. Its baseline file preserves the earlier provisional 85/95/110 values rather than rewriting the original measurement.

## Verification, evidence and limitations

The tests use the new compact target `/home/k9-vortex/.cache/prime-m19-5-compact-target`; no old build tree was cleaned to obtain a passing run.

| Check | Observation |
|---|---|
| `cargo test --workspace` | 1,919 passed, 0 failed, 6 existing ignored; includes Native linked debugging, state machines, typed fixtures and synthetic snapshot checks |
| `cargo fmt --all -- --check` | Pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | Pass |
| `cargo build --release -p ppl-cli -p prime-emulator` | Pass; GUI and helper use these new release binaries |
| Configured `prime-emulator` app_suite, firmware_recovery, firmware_session | 10 tests passed with real local firmware paths set; 19 application cases match expected classification, one retained fireworks-g1 XFAIL |
| Actual CAS and Function GUI | 24 existing glyph/frame/lifecycle assertions passed; Function frames match retained reference below status bar |
| Updated private-path Function GUI | Eight assertions passed, no helper outlived the IDE, all 193 old scratch-file hashes unchanged; repeat output refused before launch; forced application failure propagates exit 1 and retains its owned profile |
| Emulator Python tooling | 64 tests passed, including 11 new storage/lifecycle tests |
| New Python Ruff checks and shell syntax | Pass |
| QEMU compact runtime | Exact validated executable hash retained; genuine ARM smoke, NAND, NAND ECC and synthetic boot-chain cases pass |
| Invalid QEMU reuse recipe | Exit 2, no output created |

Workspace tests alone may return early when firmware assets are not configured; the separate configured three-binary run above supplies those assets and completes the actual firmware cases. Existing M19 historical physical references were not changed; no new physical-calculator parity is claimed. Native linked/debug and snapshot tests are software regressions, not new physical evidence. The FullSystem cases execute self-authored ARM guests; they are not HP Prime kernel boot. Kernel boot was not attempted in this storage milestone.

Commands and exact hardware observations are retained in [hardware-tests.json](../../artifacts/m19-5/hardware-tests.json) and the per-case `result.json`/logs under `artifacts/m19-5/`. These use QEMU 2.10.1 fork revision and executable/image identities above, the same six M19 overlays, the unchanged synthetic NAND geometry and the guest ELF hashes recorded by each runner. Firmware logs, source test inputs, GUI dumps/frames, build identities and check summaries remain locally under `artifacts/m19-5`; concise checks are tracked in [verification.json](../../evidence/m19-5/verification.json).

**Validation exception:** the first CAS/Function run used the pre-existing GUI harness, which deletes/reuses fixed `/tmp/ppl-ide-e2e/m12/<scenario>` scratch captures and rewrites its shared scratch logs. The baseline contained that scratch root. Consequently this audit cannot claim every pre-existing temporary capture was preserved. No cache retirement or historical milestone-evidence cleanup was executed. The harness was subsequently fixed and revalidated with private output and preserved legacy-file hashes; the limitation is not hidden by a blanket “nothing deleted” claim. One intermediate private-path check used an unrecognized USB-disable variable; the final private-path run uses the actual `PPL_IDE_NO_DEVICE=1` switch. No agent-initiated calculator send/delete or firmware-write operation was performed.

No full performance benchmark campaign was launched: it would unnecessarily recreate evidence/profile growth for a storage-only change. No performance improvement is inferred from the size reduction. The main remaining limitations are unprivileged Btrfs physical accounting, mixed historical provenance, unreadable process fd directories and non-M12 legacy GUI callers that still use the generic runner's default scratch-log path. Those callers can opt into `PPL_IDE_LOG_DIR`; no claim is made that every historical test harness has been migrated.

## Final storage summary

**Current footprint:** 259.576 GB allocation accounting (259,575,992,320 bytes), versus baseline 250.035 GB.

**Essential/retained pending provenance:** 76.121 GB, including unknown data; recognized A subset 3.488 GB.

**Rebuildable:** 183.455 GB of Cargo outputs, including the new compact validation target. QEMU build products offer further rebuildable subsets inside retained mixed roots.

**Safely reclaimable candidates:** 115.074 GB of allocation, conditional on explicit authorization and the documented reference/activity guards; actual exclusive physical recovery is unknown. No orphan is certified automatically.

**Potential footprint after approved first-phase cleanup:** 144.502 GB in the same metric.

**Proposed steady-state budget:** 90 GB minimal / 100 GB standard / 115 GB full research, after separately approved historical build retirement.

Validation added approximately 9.541 GB while existing builds were retained: compact Cargo 8.523 GB, new synthetic NAND bases 0.554 GB, runtime bundle 0.043 GB and about 0.422 GB of new repository profiles/evidence. This milestone did not recover the proposed space. The scan is a point-in-time observation; its own JSON, this report and the final checkpoint add small metadata/report differences afterward.

## Ten largest independent roots — final snapshot

Root accounting is globally deduplicated by inode. These are project-discovered roots; mixed ownership remains D until reviewed.

| Allocation GB | Location |
|---:|---|
| 81.914 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target` |
| 57.243 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator` |
| 10.720 | `/home/k9-vortex/.cache/prime-m17-native-target` |
| 9.925 | `/home/k9-vortex/.cache/prime-m18-native-target` |
| 9.291 | `/home/k9-vortex/.cache/prime-m17-runtime-target` |
| 8.523 | `/home/k9-vortex/.cache/prime-m19-5-compact-target` |
| 6.709 | `/home/k9-vortex/Development/4-Rust/.cache/prime-keyboard-target-agent` |
| 6.500 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target-m12` |
| 6.089 | `/var/tmp/hp-prime-pr21-build` |
| 5.842 | `/home/k9-vortex/Development/4-Rust/.cache/prime-next-core-target` |

## Twenty largest project files — final snapshot

| Allocation GB | Logical GB | Links | File |
|---:|---:|---:|---|
| 12.155 | 12.156 | 1 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/scratch/research/device-management-20261027/windows/research-overlay.qcow2` |
| 5.113 | 5.113 | 1 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/scratch/research/official-software-20261025/windows/windows-evaluation.iso` |
| 3.302 | 3.302 | 1 | `/home/k9-vortex/prime-emulator-backups/prime-emulator-complete-20261024.tar.zst` |
| 0.337 | 0.337 | 1 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/deps/libunicorn_engine_sys-5b7a7fecb102eeb1.rlib` |
| 0.337 | 0.337 | 1 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/deps/libunicorn_engine_sys-fb17d09f56764aa2.rlib` |
| 0.337 | 0.337 | 1 | `/home/k9-vortex/.cache/.cache/prime-emulator-target/debug/deps/libunicorn_engine_sys-fb17d09f56764aa2.rlib` |
| 0.337 | 0.337 | 1 | `/home/k9-vortex/.cache/prime-m17-runtime-target/debug/deps/libunicorn_engine_sys-fb17d09f56764aa2.rlib` |
| 0.337 | 0.337 | 1 | `/home/k9-vortex/.cache/prime-m18-snapshot-target/debug/deps/libunicorn_engine_sys-fb17d09f56764aa2.rlib` |
| 0.337 | 0.337 | 1 | `/home/k9-vortex/.cache/prime-m19-hardware-target/debug/deps/libunicorn_engine_sys-fb17d09f56764aa2.rlib` |
| 0.335 | 0.335 | 1 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-377d58fadc7c75f5/out/build/libunicorn.a` |
| 0.335 | 0.335 | 1 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-377d58fadc7c75f5/out/lib64/libunicorn.a` |
| 0.335 | 0.335 | 1 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-e7e5884a444956c6/out/build/libunicorn.a` |
| 0.335 | 0.335 | 1 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-e7e5884a444956c6/out/lib64/libunicorn.a` |
| 0.335 | 0.335 | 1 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-93caf569331be6a2/out/build/libunicorn.a` |
| 0.335 | 0.335 | 1 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-93caf569331be6a2/out/lib64/libunicorn.a` |
| 0.335 | 0.335 | 1 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-caa7d0492c2fbeef/out/build/libunicorn.a` |
| 0.335 | 0.335 | 1 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-caa7d0492c2fbeef/out/lib64/libunicorn.a` |
| 0.335 | 0.335 | 1 | `/home/k9-vortex/.cache/prime-m18-snapshot-target/debug/build/unicorn-engine-sys-caa7d0492c2fbeef/out/build/libunicorn.a` |
| 0.335 | 0.335 | 1 | `/home/k9-vortex/.cache/prime-m18-snapshot-target/debug/build/unicorn-engine-sys-caa7d0492c2fbeef/out/lib64/libunicorn.a` |
| 0.335 | 0.335 | 1 | `/home/k9-vortex/.cache/prime-m18-snapshot-target/debug/build/unicorn-engine-sys-93caf569331be6a2/out/build/libunicorn.a` |

## All directories above 1 GB — final snapshot

156 nested directory rows; they must not be summed.

| Allocation GB | Location |
|---:|---|
| 81.914 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target` |
| 79.540 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug` |
| 57.243 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator` |
| 43.976 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/deps` |
| 31.049 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts` |
| 26.061 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/scratch/research` |
| 26.061 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/scratch` |
| 22.056 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/incremental` |
| 13.170 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/scratch/research/device-management-20261027` |
| 13.110 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/scratch/research/device-management-20261027/windows` |
| 10.720 | `/home/k9-vortex/.cache/prime-m17-native-target` |
| 9.925 | `/home/k9-vortex/.cache/prime-m18-native-target` |
| 9.502 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/build` |
| 9.306 | `/home/k9-vortex/.cache/prime-m18-native-target/debug` |
| 9.291 | `/home/k9-vortex/.cache/prime-m17-runtime-target` |
| 9.254 | `/home/k9-vortex/.cache/prime-m17-native-target/debug` |
| 8.676 | `/home/k9-vortex/.cache/prime-m17-runtime-target/debug` |
| 8.523 | `/home/k9-vortex/.cache/prime-m19-5-compact-target` |
| 7.441 | `/home/k9-vortex/.cache/prime-m19-5-compact-target/debug` |
| 6.709 | `/home/k9-vortex/Development/4-Rust/.cache/prime-keyboard-target-agent` |
| 6.500 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target-m12` |
| 6.434 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/scratch/research/official-software-20261025` |
| 6.253 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/diagnostic-parity` |
| 6.206 | `/home/k9-vortex/.cache/prime-m19-5-compact-target/debug/deps` |
| 6.089 | `/var/tmp/hp-prime-pr21-build` |
| 5.842 | `/home/k9-vortex/Development/4-Rust/.cache/prime-next-core-target` |
| 5.842 | `/home/k9-vortex/Development/4-Rust/.cache/prime-next-core-target/debug` |
| 5.782 | `/home/k9-vortex/.cache/.cache/prime-emulator-target` |
| 5.561 | `/home/k9-vortex/.cache/prime-m16-stateful-target` |
| 5.490 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/scratch/research/official-software-20261025/windows` |
| 5.373 | `/home/k9-vortex/Development/4-Rust/.cache/prime-keyboard-target-agent/debug` |
| 5.372 | `/home/k9-vortex/.cache/prime-m16-recovery-target` |
| 5.278 | `/home/k9-vortex/Development/4-Rust/.cache/prime-next-native-target` |
| 5.278 | `/home/k9-vortex/Development/4-Rust/.cache/prime-next-native-target/debug` |
| 5.246 | `/home/k9-vortex/.cache/prime-m18-snapshot-target` |
| 5.246 | `/home/k9-vortex/.cache/prime-m18-snapshot-target/debug` |
| 5.168 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target-m12/debug` |
| 5.042 | `/home/k9-vortex/.cache/.cache/prime-emulator-target/debug` |
| 5.003 | `/home/k9-vortex/.cache/prime-m19-hardware-target` |
| 5.003 | `/home/k9-vortex/.cache/prime-m19-hardware-target/debug` |
| 4.949 | `/home/k9-vortex/.cache/prime-m16-stateful-target/debug` |
| 4.883 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/consolidation-preserved` |
| 4.799 | `/home/k9-vortex/Development/4-Rust/.cache/prime-next-differential-target` |
| 4.799 | `/home/k9-vortex/Development/4-Rust/.cache/prime-next-differential-target/debug` |
| 4.761 | `/home/k9-vortex/.cache/prime-m16-recovery-target/debug` |
| 4.552 | `/var/tmp/hp-prime-pr21-build/debug` |
| 4.332 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/m19` |
| 3.874 | `/home/k9-vortex/.cache/prime-m17-runtime-target/debug/build` |
| 3.851 | `/home/k9-vortex/.cache/prime-m18-native-target/debug/deps` |
| 3.743 | `/home/k9-vortex/.cache/prime-m17-native-target/debug/deps` |
| 3.470 | `/home/k9-vortex/.cache/prime-m17-native-target/debug/incremental` |
| 3.395 | `/home/k9-vortex/.cache/prime-m18-snapshot-target/debug/build` |
| 3.386 | `/home/k9-vortex/.cache/.cache/prime-emulator-target/debug/build` |
| 3.386 | `/home/k9-vortex/.cache/prime-m19-hardware-target/debug/build` |
| 3.319 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/consolidation-preserved/hp-prime-file-compat` |
| 3.319 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/consolidation-preserved/hp-prime-file-compat/artifacts` |
| 3.304 | `/home/k9-vortex/prime-emulator-backups` |
| 3.172 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/examples` |
| 3.140 | `/home/k9-vortex/.cache/prime-m18-native-target/debug/incremental` |
| 3.108 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/scratch/research/m7-private` |
| 3.071 | `/home/k9-vortex/Development/4-Rust/.cache/prime-next-core-target/debug/deps` |
| 3.054 | `/var/tmp/hp-prime-pr21-build/debug/deps` |
| 2.971 | `/home/k9-vortex/.cache/prime-m17-runtime-target/debug/deps` |
| 2.718 | `/home/k9-vortex/Development/4-Rust/.cache/prime-next-native-target/debug/deps` |
| 2.605 | `/home/k9-vortex/Development/4-Rust/.cache/prime-keyboard-target-agent/debug/deps` |
| 2.410 | `/home/k9-vortex/Development/4-Rust/.cache/prime-next-differential-target/debug/deps` |
| 2.371 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/release` |
| 2.335 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/m17` |
| 2.223 | `/home/k9-vortex/.cache/prime-m16-recovery-target/debug/build` |
| 2.210 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/m14` |
| 2.143 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/consolidation` |
| 2.130 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target-m12/debug/deps` |
| 1.905 | `/home/k9-vortex/.cache/prime-m18-native-target/debug/build` |
| 1.860 | `/home/k9-vortex/.cache/prime-m16-stateful-target/debug/incremental` |
| 1.777 | `/home/k9-vortex/Development/4-Rust/.cache/prime-next-differential-target/debug/incremental` |
| 1.762 | `/home/k9-vortex/Development/4-Rust/.cache/prime-next-native-target/debug/incremental` |
| 1.758 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/scratch/research/m8-private` |
| 1.710 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/release/deps` |
| 1.631 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-caa7d0492c2fbeef` |
| 1.631 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-93caf569331be6a2` |
| 1.631 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-e7e5884a444956c6` |
| 1.631 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-377d58fadc7c75f5` |
| 1.631 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-caa7d0492c2fbeef/out` |
| 1.631 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-93caf569331be6a2/out` |
| 1.631 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-e7e5884a444956c6/out` |
| 1.631 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-377d58fadc7c75f5/out` |
| 1.631 | `/home/k9-vortex/.cache/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-caa7d0492c2fbeef` |
| 1.631 | `/home/k9-vortex/.cache/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-93caf569331be6a2` |
| 1.631 | `/home/k9-vortex/.cache/prime-m18-snapshot-target/debug/build/unicorn-engine-sys-93caf569331be6a2` |
| 1.631 | `/home/k9-vortex/.cache/prime-m18-snapshot-target/debug/build/unicorn-engine-sys-caa7d0492c2fbeef` |
| 1.631 | `/home/k9-vortex/.cache/prime-m19-hardware-target/debug/build/unicorn-engine-sys-caa7d0492c2fbeef` |
| 1.631 | `/home/k9-vortex/.cache/prime-m19-hardware-target/debug/build/unicorn-engine-sys-93caf569331be6a2` |
| 1.631 | `/home/k9-vortex/.cache/prime-m17-runtime-target/debug/build/unicorn-engine-sys-93caf569331be6a2` |
| 1.631 | `/home/k9-vortex/.cache/prime-m18-native-target/debug/build/unicorn-engine-sys-caa7d0492c2fbeef` |
| 1.631 | `/home/k9-vortex/.cache/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-caa7d0492c2fbeef/out` |
| 1.631 | `/home/k9-vortex/.cache/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-93caf569331be6a2/out` |
| 1.631 | `/home/k9-vortex/.cache/prime-m18-snapshot-target/debug/build/unicorn-engine-sys-93caf569331be6a2/out` |
| 1.631 | `/home/k9-vortex/.cache/prime-m18-snapshot-target/debug/build/unicorn-engine-sys-caa7d0492c2fbeef/out` |
| 1.631 | `/home/k9-vortex/.cache/prime-m19-hardware-target/debug/build/unicorn-engine-sys-caa7d0492c2fbeef/out` |
| 1.631 | `/home/k9-vortex/.cache/prime-m19-hardware-target/debug/build/unicorn-engine-sys-93caf569331be6a2/out` |
| 1.631 | `/home/k9-vortex/.cache/prime-m17-runtime-target/debug/build/unicorn-engine-sys-93caf569331be6a2/out` |
| 1.631 | `/home/k9-vortex/.cache/prime-m18-native-target/debug/build/unicorn-engine-sys-caa7d0492c2fbeef/out` |
| 1.605 | `/home/k9-vortex/.cache/prime-m17-runtime-target/debug/incremental` |
| 1.603 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/consolidation/firmware-files` |
| 1.603 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/consolidation-preserved/hp-prime-file-compat/artifacts/hpprgm-final-capture` |
| 1.603 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/consolidation-preserved/hp-prime-file-compat/artifacts/hpprgm-capture` |
| 1.568 | `/home/k9-vortex/.cache/prime-m17-native-target/debug/build` |
| 1.564 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/consolidation-preserved/hp-prime-location-parity/artifacts/location-parity` |
| 1.564 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/consolidation-preserved/hp-prime-location-parity/artifacts` |
| 1.564 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/consolidation-preserved/hp-prime-location-parity` |
| 1.560 | `/home/k9-vortex/Development/4-Rust/.cache/prime-keyboard-target-agent/debug/build` |
| 1.556 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target-m12/debug/build` |
| 1.556 | `/home/k9-vortex/.cache/prime-m16-stateful-target/debug/deps` |
| 1.546 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/diagnostic-parity/fresh-final-20261027T145924Z` |
| 1.489 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/scratch/research/m8-private/baseline` |
| 1.466 | `/home/k9-vortex/.cache/prime-m17-native-target/release` |
| 1.437 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/scratch/research/m7-private/runtime` |
| 1.403 | `/home/k9-vortex/.cache/prime-m16-stateful-target/debug/build` |
| 1.395 | `/home/k9-vortex/Development/4-Rust/.cache/prime-next-core-target/debug/build` |
| 1.374 | `/var/tmp/hp-prime-pr21-build/debug/build` |
| 1.365 | `/home/k9-vortex/.cache/prime-m16-recovery-target/debug/deps` |
| 1.351 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/g1-resolution/20261028T200940Z` |
| 1.351 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/g1-resolution` |
| 1.335 | `/home/k9-vortex/Development/4-Rust/.cache/prime-keyboard-target-agent/release` |
| 1.332 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/m18` |
| 1.332 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target-m12/release` |
| 1.332 | `/var/tmp/hp-prime-pr21-build/release` |
| 1.299 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/diagnostic-parity/fresh-20261027T134819Z` |
| 1.196 | `/home/k9-vortex/Development/4-Rust/.cache/prime-keyboard-target-agent/debug/incremental` |
| 1.152 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-caa7d0492c2fbeef/out/build` |
| 1.152 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-93caf569331be6a2/out/build` |
| 1.152 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-e7e5884a444956c6/out/build` |
| 1.152 | `/home/k9-vortex/Development/4-Rust/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-377d58fadc7c75f5/out/build` |
| 1.152 | `/home/k9-vortex/.cache/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-caa7d0492c2fbeef/out/build` |
| 1.152 | `/home/k9-vortex/.cache/.cache/prime-emulator-target/debug/build/unicorn-engine-sys-93caf569331be6a2/out/build` |
| 1.152 | `/home/k9-vortex/.cache/prime-m18-snapshot-target/debug/build/unicorn-engine-sys-93caf569331be6a2/out/build` |
| 1.152 | `/home/k9-vortex/.cache/prime-m18-snapshot-target/debug/build/unicorn-engine-sys-caa7d0492c2fbeef/out/build` |
| 1.152 | `/home/k9-vortex/.cache/prime-m19-hardware-target/debug/build/unicorn-engine-sys-caa7d0492c2fbeef/out/build` |
| 1.152 | `/home/k9-vortex/.cache/prime-m19-hardware-target/debug/build/unicorn-engine-sys-93caf569331be6a2/out/build` |
| 1.152 | `/home/k9-vortex/.cache/prime-m17-runtime-target/debug/build/unicorn-engine-sys-93caf569331be6a2/out/build` |
| 1.152 | `/home/k9-vortex/.cache/prime-m18-native-target/debug/build/unicorn-engine-sys-caa7d0492c2fbeef/out/build` |
| 1.103 | `/home/k9-vortex/.cache/prime-m17-native-target/release/deps` |
| 1.086 | `/home/k9-vortex/.cache/prime-m18-snapshot-target/debug/deps` |
| 1.082 | `/home/k9-vortex/.cache/prime-m19-5-compact-target/release` |
| 1.068 | `/home/k9-vortex/.cache/.cache/prime-emulator-target/debug/deps` |
| 1.046 | `/home/k9-vortex/Development/4-Rust/.cache/prime-next-core-target/debug/incremental` |
| 1.040 | `/home/k9-vortex/.cache/prime-m16-recovery-target/debug/incremental` |
| 1.036 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/m19/bench-final` |
| 1.036 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/m18/bench-final` |
| 1.036 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/m17/bench-final` |
| 1.030 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/scratch/research/m7-private/compiler` |
| 1.029 | `/home/k9-vortex/.cache/prime-m17-fullsystem-build` |
| 1.025 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/g1-closure/20261028T212023Z` |
| 1.025 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/g1-closure` |
| 1.018 | `/home/k9-vortex/.cache/prime-m19-hardware-target/debug/deps` |
| 1.016 | `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator/artifacts/g1-closure/20261028T212023Z/firmware-codec` |
