# M19.5 — Authorized Rust build cleanup

User authorization covered verified obsolete Rust compilation artifacts. No source, Git history, firmware, saves, QEMU builds, containers or retained research evidence was deleted. This follow-up supersedes the original audit's “no cleanup authorized” status for the scoped Rust artifacts only.

## Measured results

| Metric | Exact bytes | Decimal GB |
|---|---:|---:|
| Project allocation before | 259576496128 | 259.576 |
| Project allocation after verification | 118579867648 | 118.580 |
| Net project allocation reduction | 140996628480 | 140.997 |
| Observed filesystem available-space increase | 67843559424 | 67.844 |
| Retained active Rust cache | 17250852864 | 17.251 |

The allocation figures count unique regular-file inodes, exclude tmpfs and add the previously measured project container-layer union once. The filesystem figure is an observed whole-filesystem change, not exact project-exclusive physical recovery: Btrfs compression, reflinks, snapshots and unrelated writes prevent that attribution. Do not add these metrics together. The first post-cleanup scan recorded 109.811 GB before later preservation work and verification growth. The final snapshot includes verification outputs and new evidence; small report/Git metadata additions follow the scan.

## Removed and preserved

Removed 202,204 verified compilation files plus 120 obsolete binary copies after independent archival. Deletion was restricted to the 16 historical targets from the audit, with root identity/UID checks, project fingerprints, held Cargo locks, process/open-file checks and per-file identity rechecks. Sources and worktrees were left in place. Three unreadable process-fd directories were recorded; they were not treated as evidence of inactivity. No affected target process was observed, and Cargo locks were held throughout removal.

Preserved all original compact-target contents from cleanup. The original working CLI/helper hashes were independently archived and their `--help` commands passed. Warnings-denied CI verification subsequently built additional variants in the same active target, explaining its growth beyond the original 8.52 GB.

Preserved 109 distinct historical binary/reference hashes, standalone research examples, referenced libraries/configuration and the original working binaries. Archive: `~/.cache/hp-prime-emulator/research/rust-binaries/<sha256>/<name>`. Archived files are independent inodes (reflinks where available); writing an old target cannot alter the preserved bytes. Every original-to-archive mapping and SHA256 is recorded in the manifest. Historical reports were not rewritten; use that mapping when their old executable paths have been retired.

Kept `target/tmp`, fingerprints, logs/configuration and unclassified files, including versioned native libraries whose removal was not established. Historical target directories therefore intentionally remain. No generic `cargo clean`, cache prune, worktree removal or shared container cleanup was used. The initial safety check stopped on its own held lock descriptors before deletion; the corrected pass excludes only its own PID. Both attempt logs remain under local artifacts.

## Preventing renewed growth

Kept compact dev/test profiles, ARM-only Unicorn and explicit `dev-full`. Cargo configuration and the wrapper now default to the stable space-free `prime-emulator-target-compact` name. On this machine that name is an alias to the existing compact target, which was not moved. No background pruning or global Cargo configuration was installed. Updated the wrapper contract test for the new default and isolated it from caller target overrides.

## Verification and PR readiness

- Rust all-target tests: 1919 passed, 0 failed, 6 existing ignored.
- Locked workspace check, all-target Clippy with warnings denied, formatting and workspace release build: passed, using CI's `RUSTFLAGS=-D warnings`.
- Python: 64 emulator tests and 41 tooling tests passed. The initial tooling failure expected the retired default path; the updated contract passes. No tests were removed or weakened.
- Configured local firmware: 10 tests passed across application, recovery and session suites. Existing fireworks-g1 XFAIL remains explicit; it is not a new pass claim.
- Actual CAS/Function GUI: 24 glyph/frame/lifecycle assertions passed with USB disabled and private profiles.
- FullSystem: self-authored ARM smoke passed with the retained QEMU executable. No proprietary kernel boot or physical calculator parity is claimed.

No PR exists for this branch. The starting branch was 112 commits ahead of remote `main`, so readiness covers the full integrated branch. No firmware/build binaries with the inspected proprietary/build suffixes were found in that diff. Branch-wide whitespace warnings concern existing GPL patch content/context; those identity-pinned patches were not altered to silence warnings. Local gates are green; Windows hosted CI remains unverified until the branch is published and that job runs. No push, PR creation or merge was performed.

## Evidence

`evidence/m19-5-cleanup/summary.json` contains exact totals; `manifest.json` contains preservation mappings and ownership checks; `removed.jsonl.gz` is the first-pass deletion journal; `retired-preserved-copies.json` records subsequent binary-copy retirement. Before/after inventories, original-binary hashes, GUI assertions, PR assessment and the exact one-off cleanup script text are retained alongside them. Full commands, logs, GUI frames, firmware observations and unsuccessful attempts remain under `artifacts/m19-5-cleanup/`. These records should be retained as research evidence.
