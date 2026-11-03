# Saved G1 milestone PR stack

Review and merge in this order. Each PR targets the preceding milestone branch, so the diff isolates that milestone. Do not merge until its required checks pass.

| Order | PR | Base | Source checkpoint at creation |
|---|---|---|---|
| 1 | [Improve G1 editing, recovery, Native dialogs and firmware run performance](https://github.com/golba98/hp-prime-emulator/pull/24) | `main` | `4adcc4620c3a` |
| 2 | [Add source-bound diagnostics, stateful differential scenarios and checkpoints (M16)](https://github.com/golba98/hp-prime-emulator/pull/25) | `pr/g1-01-usability-foundations` | `7e33a624622c` |
| 3 | [Bring up ARM926 research execution, Native linkage and typed observations (M17)](https://github.com/golba98/hp-prime-emulator/pull/26) | `pr/g1-02-m16-stateful-diagnostics` | `31cfa64aa285` |
| 4 | [Add hardware guest controls, linked debugging and constrained continuation (M18)](https://github.com/golba98/hp-prime-emulator/pull/27) | `pr/g1-03-m17-runtime-linkage` | `e0d1e6eb39dc` |
| 5 | [Add NAND, clocks, LCD/input and synthetic boot foundations (M19)](https://github.com/golba98/hp-prime-emulator/pull/28) | `pr/g1-04-m18-hardware-debugging` | `0a259072fdc8` |
| 6 | [Reduce accumulated Rust build storage and preserve research artifacts (M19.5)](https://github.com/golba98/hp-prime-emulator/pull/29) | `pr/g1-05-m19-boot-foundations` | `ea74009dd467` |

The complete integrated source, committed evidence and milestone histories are pushed. A verified Git bundle containing every local ref is retained under the directory recorded in `evidence/publication/preservation.json`. Ten unique untracked M16 worker evidence files were copied into a lossless local archive and verified against SHA256; their original files were left untouched. Raw private data, firmware, calculator saves and preserved compiled research binaries remain local and are not uploaded to GitHub. Ignored historical evidence remains saved in its existing local artifact directories. This is preservation in the current environment, not a claim of offsite backup for those private resources.

Canonical working-tree status was clean before publication. Other extant worker source checkouts were clean; the archived M16 evidence was the only non-ignored untracked work found. No worker checkout was changed and no branches were deleted.

GitHub CI runs Linux and Windows on each published milestone. Results must be read from the PR checks; local passing tests alone do not establish hosted CI success. The latest integrated source passed 1,919 Rust tests, 105 Python tests, 10 configured firmware tests, actual GUI controls and an ARM guest smoke; existing ignored tests and the Fireworks expected failure remain documented in the storage cleanup report. No proprietary firmware is available to hosted CI.
