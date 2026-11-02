# M17 snapshot component prerequisites

These components are implemented and independently restored in fresh child
processes. They are not a live calculator snapshot. They do not resume Unicorn,
blocked native callbacks, scheduler state or a calculator application.
`SessionInspection.readiness.live_restore_ready` remains false. The normal OFF
and explicit firmware-persistence checkpoint mechanisms are unchanged.

## Heap and allocator

`prime_emulator::memory::component::HeapComponent::{capture,save,load,restore}`
captures the designated heap block and its host allocator layout. Capture
requires the existing memory gate and holds the allocator lock. It never uses
an expiring CPU-owner pause as permission to copy memory. Host callers must
exclude any other memory writer outside that gate. This is a controlled component
API, not an asynchronous snapshot command.

The version 1 binary container consists of an 8-byte kind header, a bounded JSON
manifest, and raw heap bytes. The manifest binds the expected resource identity,
base and size, ordered allocation/free intervals, allocated-byte accounting and
payload SHA-256. A second digest covers all manifest fields. Validation rejects
gaps, overlaps, duplicate/empty intervals, out-of-range addresses, inconsistent
counts, corrupt payloads, unknown versions and fabricated live-resume claims.
The heap is capped at 256 MiB, layout at 100,000 intervals and manifest at 8 MiB.
The file length must match exactly; malformed length fields are rejected before
allocating a payload buffer.

Restore requires exclusive access to a newly configured, empty allocator with
the same heap mapping and resource identity. Validation completes before bytes
or allocation metadata change. The remaining guest mappings, CPU registers,
firmware globals outside this heap and host resources are absent. Restoring this
component into a booted calculator is unsupported.

`save` validates the component, writes a private temporary sibling, synchronizes
it, atomically replaces the destination and synchronizes its parent directory.
This does not implement a rotating machine-checkpoint history or prove that the
component can resume a full firmware session.

## Read-only filesystem handles

`prime_emulator::services::fs::component::ReadOnlyVfsComponent` supports
`capture`, `save`, `load` and `restore`. Capture and restore require exclusive
`&mut Vfs`. An owner with open writers, directory searches or a prior durability
error is refused. The source and target storage trees must be privately owned
and remain stable during the operation; this API is not a hostile-filesystem
race defense.

The checksummed version 1 manifest records guest handle IDs, jailed relative
paths, byte positions, file sizes/content SHA-256, next-handle/search counters,
current drive and per-drive current directories. It records no raw file
handles, descriptors, host pointers or synchronization primitives. The original
open descriptor is compared with the reopened path, with its position preserved,
so an already replaced file does not silently become another resource.

Restore requires a fresh resource owner. It rejects path traversal, symlink
resources, malformed/duplicate handles, changed resources and identity/version
mismatches. It reopens read-only files, verifies their contents and seeks them to
the recorded positions. All provisional descriptors are dropped on failure;
target resources are installed only after every check completes. Bounds are 512
handles, 256 MiB total referenced content, 4096-byte paths and a 2 MiB manifest.
Writable files and active directory searches still need separate reconstruction.

## Reproduce the component validation

```sh
TMPDIR="$HOME/.cache/prime-component-tmp" \
CARGO_TARGET_DIR="$HOME/.cache/prime-component-target" \
cargo test -p prime-emulator --lib component::tests -- --nocapture
```

Create the selected TMPDIR first. Tests use synthetic, private data. They do not
load proprietary firmware or open USB. The parent tests serialize components and
start the current Rust test executable as a separate process. The heap child
loads RAM bytes and allocator metadata, verifies a marker and reuses a freed
48-byte interval without overlapping the retained allocation. The VFS child
reopens the original guest handle, reads `cde` from saved offset 2, preserves the
next handle, and repeats reconstruction 32 times. Linux additionally compares
its file-descriptor count before and after those cycles.

Twelve component tests passed after coordinator review. A Linux FIFO regression
first failed at admission; the path validator now rejects nonregular resources
before descriptor open, preventing capture/restore from waiting for a FIFO
writer. The test uses the standard Linux `mkfifo` utility.
Negative cases include changed file contents,
replaced descriptors, corrupt data/metadata, incompatible resource identity,
occupied targets, active memory writers, unsupported writers/searches, symlinks,
path traversal, fabricated live-resume claims, malformed/truncated files and an
abandoned partial staging file that leaves the valid component intact.

These are synthetic component restore tests, not fresh-process firmware live
restoration, physical-calculator parity or a newly captured hardware result.
Actual live inspection still observes parked `OSWaitForEvent`/`GetEvent` native
continuations. CPU context export, stack-safe SVC continuation replacement,
complete host resource reconstruction and a coherent whole-machine commit remain
required before live calculator restoration can be enabled.
