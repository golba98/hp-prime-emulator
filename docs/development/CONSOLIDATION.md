# Canonical repository consolidation

Canonical checkout: `/home/k9-vortex/Development/4-Rust/3-HP Prime Emulator`. Branch: `feat/g1-compat-integration`.
Main remains at 413251ebb9e8340d5d381a218416db174d7332d6; no push occurred.

## Audit and integration

Original main was clean; Claude explicitly released it. Completed file,
location and integration worktrees were clean and explicitly released.
Runtime remained dirty at 06d4531 on feat/g1-runtime-parity, with Claude's
active GUI test processes. Its uncommitted work was not integrated or altered.

Main and 54b3b686b3e8520ffd5433d5312d032a2783d6b0 had identical trees
b190cc6ee5ca224d31290ddc27a6bd91fd59e0ec. Merge 7da88a7 connects those
histories without file changes. Merge bf4b762 integrates completed feature
history through 0e94e514093d520e72742e10c506d90d645516c1 without conflicts.
Before policy edits its tree equaled that feature tip exactly.

Integrated milestones: diagnostic M2 20fa3dc; direct compiler M3 851642b;
location equivalents de4015a and 9c560e3; integration correction 06d4531;
file compatibility 8578fcc, eeeb4f8 and 0e94e51. Original location commits
03df6e5 and 2d5f14e are patch-equivalent (range-diff confirmed), not reapplied.
Committed prerequisite engine, GUI and device histories are preserved too.

## Validation in the original checkout

Workspace: 1458 passed, 0 failed, 6 ignored. Formatting and Clippy
with warnings denied passed. Existing enabled firmware suites: 21 passed,
including eight committed runtime tests. File firmware tests: 2 passed,
covering 26 exact-source save/reboot/reopen cases and editor modification
through two reboots. Diagnostic baseline: 75/75 verdicts and 32/32 rejected
locations; fresh direct-engine capture differed from stored reference on no
cases. Direct regression passed; warm median 12.497648 ms, cold 10.686 s.

Fourteen GUI scenarios exited successfully on a private Xvfb with isolated
profiles and remapped output paths. All six assertion groups passed after
correcting one stale editor assertion: unclosed strings now require firmware's
EOF position (line 6, column 1), not the old opening-string line 3. This is the
already-verified location-parity behavior, not a relaxed baseline. Original
failure and corrected result are retained. Firmware references and diagnostic
baseline files were not changed. No physical calculator was accessed.

Raw validation logs, scripts, fresh references and per-command results reside
under ignored `artifacts/consolidation/`; proprietary profiles remain local.

## Cleanup and remaining ownership

Released worktree evidence is preserved under ignored
`artifacts/consolidation-preserved/<worktree>/`, with original relative paths.
The preservation inventory under artifacts/consolidation records original
heads, sizes and SHA-256 hashes. Every moved file was hash-verified; open
checkout paths/descriptors and Git cleanliness were checked before removal.
Source branches are retained. No force removal, destructive reset or clean
was used. See the final Git worktree list for actual removal completion.

Claude retains hp-prime-runtime-parity. Runtime completion is not claimed.
Integration requires Claude's committed checkpoint, validation and explicit
handoff. Canonical ownership remains assigned to Codex; shared claim metadata
and the ownership register require explicit release before another writer.
