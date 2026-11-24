# Explicit firmware-persistence checkpoints

M16 adds a request-bound `checkpoint` command to the local firmware session.
This is an opt-in transaction through the calculator's real OFF path. It does
not save a running CPU or restore a suspended application.

```json
{"command":"checkpoint","id":42,"wake":true}
```

The matching `checkpoint` event carries the same `id` and a tagged result:
`saved`, `deferred`, or `failed`. A saved receipt identifies the exact generation,
firmware identity, manifest SHA-256, payload bytes and file count. Its kind is
`firmware_persistence`, safe point is `firmware_off`, and `restores_cpu` and
`restores_framebuffer` are explicitly false. `wake:true` requests ON after the
generation commits; wait for the normal Ready lifecycle before issuing work.

The session defers requests during an active program, guest call, input script,
queued keyboard/touch/text input, held keys, boot or shutdown. An admitted
request blocks further calculator mutations, waits for a fresh main-thread wait
entry and the existing measured idle-settlement boundary, then delivers SHIFT
and ON through normal key pacing. It never stops a program or dismisses a dialog
to make a checkpoint succeed. Requesting this operation explicitly accepts the
calculator's own OFF semantics for the currently displayed editor/application.

After `SysPowerOff`, the existing memory gate excludes guest mutations while
VFS durability is checked and drive files are copied. A new generation is
published only after payloads, manifest, manifest checksum and staging directory
have been synced; the final directory rename and parent sync establish the
publication boundary. An already sleeping calculator receives a new bounded
generation rather than an unrelated earlier receipt. Save failure retains prior
generations and leaves the calculator asleep; it never reports successful save.
Normal OFF shutdown behavior is preserved.

The explicit operation uses a bounded policy: 256 MiB guest payload, 1 GiB of
regular-file bytes under `.recovery`, 100,000 entries and four compatible
generations. Byte accounting excludes filesystem block/directory overhead.
Retention runs only after durable publication and always preserves at least two
intact compatible generations. Corrupt/incompatible evidence, interrupted writes
and restore archives are preserved and count toward the byte budget. A profile
that cannot fit another checkpoint fails safely; it is not silently cleaned.
Automatic startup/OFF recovery retains its existing non-pruning behavior.

On a fresh process, existing firmware/resource identity checks and manifest/file
hash validation run before bootstrap. Interrupted current drives are archived,
then an intact compatible generation is installed through the resumable restore
journal. Unsaved RAM, open editors, active-app views, pending input and transient
graphics are outside this format. Persisted variables, lists, matrices and
settings are recovered only to the extent the firmware saved them to files.

Full live snapshots remain unsupported. The current gate permits blocked SVC
callbacks to park while their native continuations and descriptor leases remain
on host stacks. Additional host-only state includes the heap allocator free
list, file offsets and searches, host-thread critical-section owners, timers,
guest-call plans and engine ownership. Raw RAM plus general registers would
produce an inconsistent fresh-process restore. A future live implementation
needs explicit resumable waits and reconstructible typed host resources first.

Implementation: `ppl-engine/src/checkpoint.rs`, `prime-emulator/src/checkpoint.rs`,
`recovery.rs`, `session.rs`, `services/mod.rs` and `tests/firmware_recovery.rs`.
The focused implementation was validated on Linux x86_64, Rust 1.96.0 and
Unicorn 2.1.5, using firmware 15751 (ELF SHA-256
`ad9779ff72db39fe4e9977ce7c4324f39aad1bb00c849c0aab067dc5bfc2fb39`). No physical
calculator was accessed. The serial release recovery suite passed all five
tests in 134.53 s. Its new scenario computes with CAS, saves A/L1/M1, kills the
helper, recovers in a fresh process, saves again, wakes, changes A, kills again
and recovers the second saved value. The strengthened-gate scenario also passed
separately in 37.96 s. A matching second-save comparison using normal PowerDown
and another fresh process passed in the full release suite.
The final protocol-6 repeat passed again in 38.18 s. Emulator library tests
passed 87/87, engine library tests passed 21/21, formatting passed, and Clippy
passed for emulator, engine and GUI across all targets with warnings denied.

A debug-build limitation remains explicit. The same post-CAS repeated-cycle
comparison with unchanged normal PowerDown failed to reach Ready after reboot
(165.08 s total, 120 s boot timeout). The new strict checkpoint instead refused
its second OFF when a fresh settled idle boundary was not observed (74.52 s
total, 30 s checkpoint deadline), preserving the previous generation. An earlier
instantaneous-idle implementation allowed that OFF, after which its next debug
boot also stalled. The fresh-wait/settled-idle gate prevents that admission; it
does not establish a fix for the underlying debug-build firmware behavior.
The optional firmware tests retain their strict success assertions, so this
debug verification failure is not hidden by a skip or a weakened comparison.

Local diagnostic evidence is preserved under
`/home/k9-vortex/.cache/prime-m16-recovery-tmp/failed-third-boot-evidence` with a
captured screenshot/report at `boot-probe-debug.png` and
`boot-probe-debug-report.json` in the same parent directory. This disposable
profile is not distributed or used as a calculator reference fixture.
