# Experimental incremental screen mirroring

## Official-software correction — 2026-10-25

The screen constructor was missing from the prior color trace. Both official
builds select **QImage::Format_RGB555 (11)** and copy decoded byte pairs as native
little-endian words. Full, fast and incremental PNG decoding now follow that
layout. The earlier big-endian RGB565 interpretation caused the colored glyph
artifacts; the corrected decoder agrees with system Qt5 on all input words and
preserved physical fixtures. This does not establish new FC04 hardware acceptance.
See [the complete pipeline](../reverse-engineering/CONNECTIVITY_KIT_SCREEN_PIPELINE.md).

The acknowledgement-only normal-stream bypass has been removed. `VerifiedIncremental`
can only come from a reviewed production profile, and that profile list remains
empty. `BoundedIncrementalPermit` is a separate diagnostic type with global request
and time budgets. Normal mirror/IPC refusal happens before device access even
with the legacy flag. Existing one-shot and manual-change diagnostics remain.

The separately approved `verify-incremental-stream` command checks prior visual
fixtures, captures exact references, requires multiple actual transitions and
recovery, and cannot enable production streaming. Its raw outputs remain private.
No new physical request, changed-screen acceptance or sustained test was run.
The manual-change verifier now refuses EOF at its confirmation prompt before
sending FC04, rather than continuing without operator confirmation.

Status: **Physical FC04 response verified** on G1 firmware 15157 (`03f0:1541`).
The first authorized hardware probe returned a valid coordinate update in 16.33 ms
(vs 990 ms baseline). **Physical visual correctness under screen changes remains
unverified** until authorized execution of the controlled verification tool
`ppl device verify-incremental`. Sustained incremental streaming remains capability-gated.
Existing full and fast modes remain fully supported and unmodified.

## Protocol evidence and limits

The hash-pinned PE32 Connectivity Kit, version 2.4.2 r15751, base 0x400000,
SHA-256 `a6ee07629a0151a85bfd263bd54c3dfdf79cb57596e36c2e4456087b080b57eb`,
is the source of the static evidence. See the
[Project trace](../reverse-engineering/HP_CONNECTIVITY_KIT_PROJECT_TRACE.md)
and [HID trace](../reverse-engineering/HP_CONNECTIVITY_KIT_HID_CALL_TRACE.md).
These establish software behavior, not G1 firmware acceptance.

| Field/behavior | Evidence | Implemented interpretation |
|---|---|---|
| Logical request | f5e300 table, builder 4cce00, queue 4cb220/4d1560 | Two bytes `FC 04`; full request `FC 08` |
| HID framing | Worker 4c5270/4c5602, wrapper 414820 | Existing legacy sequence framing, no new transport negotiation |
| Linux API request | Existing session framing | `00 00 FC 04`: report ID, sequence 0, two payload bytes |
| Windows boundary | WriteFile 41488f | Descriptor-sized report plus Windows report ID; API length is not logical-message length |
| Response header | 44f7a0, 42d8e0 and producer 5a19e5 onward | `FC 01 length_BE32 CRC_BE16 format x_LE16 y_LE16 PNG` |
| Length | Existing message assembler 4c6d60 | Body length excludes first six bytes; includes CRC, format, coordinates and PNG |
| CRC | 44f7a0/5a3c60 | CRC16 over format/coordinates/PNG, checked before decoding |
| PNG start | Handler's payload offsets | Message offset 13; dimensions come from IHDR |
| Coordinates | 42dae0 | `(FFFF,FFFF)` denotes full placement; otherwise source-pixel x/y |
| Color | 42d8e0/58afc0 | Formats 04/08: grayscale16 PNG samples carry little-endian RGB555 |
| Cache baseline | 5a17b7 → 5a2540; copy at 5a17e3 | Embedded producer sizes and populates comparison cache on full output |
| Incremental output | 5a14c1 → 59c290 | Bounding-rectangle pixels and coordinates |
| Periodic full image | 5a147c onward | Embedded producer forces full output every 32nd count; not an instruction for the host to request FC08 every 32 calls |

The normal API uses its existing short HIDAPI write; OS/driver padding is
separate. No new descriptor assumptions or G2 report sizes were introduced.
G2, negotiated framing, paletted updates and unexplained high-bit format flags
are unsupported by the experimental backend.

There is no recovered frame-level serial number, generation, acknowledgement
or timestamp in this response. HID sequence values validate fragments of one
message (FE wraps to 00), not the chronology of complete images. Local ordinals
are host bookkeeping only. A silent loss of an entire update, a semantically
incorrect but well-formed patch, or an undetectable device-side cache reset
cannot be ruled out from this header alone. A delayed, otherwise valid full-frame
reply also carries no generation identifier proving its freshness after reconnect.

A 1×1 patch is decoded and compared like any other patch, never automatically
ignored. Empty payloads, missing PNGs, mixed sentinel coordinates and unexpected
formats are errors. The 100 preserved format logs contain 08/09/0A/0B/0D,
**not FC04**; the new FC04 fixtures are explicitly synthetic.

## Architecture and pixel correctness

`ppl-device::screenshot::update` validates envelopes and exposes separate
one-shot and verified-stream permissions. Its capture path reuses the exclusive
ScreenshotSession, bounded assembly, deadline and failure poisoning. The
ordinary full-frame capture method still refuses partial images.

`ppl-graphics::incremental::IncrementalFramebuffer` owns a fixed 320×240 canvas
and a reusable PNG decoder. It accepts only grayscale16 RGB555 patches. Palette
colors remain supported by the general decoder, but are not silently treated
as packed RGB555 in this backend. No claim is made that paletted modes preserve
RGB555 source colors.

For every update the compositor:

1. Checks its host-local ordinal, encoded extent, RGB555 IHDR and complete IEND.
2. Checks nonzero dimensions and checked rectangle bounds before allocating.
3. Requires a complete baseline unless already synchronized. Full-origin
   responses must be exactly 320×240; a rectangle at (0,0) of that size also
   constitutes a complete image.
4. Decodes the entire PNG into scratch pixels, including PNG checksum and
   end-of-stream validation. Animated images are rejected.
5. Compares covered rows and commits only after all fallible validation succeeds.
   Pixels outside the patch remain unchanged. RGB555 expansion uses five-bit-per-channel
   replication, with no composition scaling, gamma or palette conversion.
6. Publishes an immutable Arc-backed Grob to the existing renderer. Unchanged
   updates reuse the Arc. Changed updates use copy-on-write when a displayed
   snapshot is still held; this can require a canvas copy to prevent tearing.

The existing PNG decoder now finishes validation through trailing chunks/IEND;
the packed pixel conversion now follows the official little-endian RGB555 consumer. Compressed
update storage is transferred from the transport allocation rather than cloned.
Nearest/linear display filtering, DPI, 4:3 viewport fitting, saving and resize
behavior remain separate from composition.

## State and failure behavior

States are `NeedsBaseline`, `Synchronized`, and `Stale`. Startup requests a
complete FC08 baseline, then FC04 updates, with one request in flight. This
sequence is supported by the recovered host producer, but remains hardware
gated. There is no speculative projection-session command.

Invalid updates never change active pixels. Failure marks the canvas stale,
drops the session and prevents subsequent commands on that connection. A new
connection must match the original exact serial; incremental reconnection also
checks firmware/boot identity and the reviewed capability profile. It restarts
with a full baseline. The last valid image remains available during recovery.
At most three consecutive recovery attempts occur; only a successful incremental
update resets that budget. A baseline alone does not reset repeated FC04 failures. Exhaustion leaves the stale view open until the user closes
it, without further USB attempts. Existing full-backend reconnect policy remains.

Pause stops new requests. An in-flight request receives the existing bounded
cleanup; cancellation prevents composition/publication. Restart after a failed
exchange requires a baseline. Ordinary incremental updates never reinitialize
the canvas. No automatic same-session FC08 recovery follows corrupted data.

The reviewed profile predicate is initially empty. Neither a mock transport,
user-edited evidence file nor a successful one-shot response grants a continuous
stream permit. Enabling a profile requires reviewed genuine changing-screen and
recovery evidence in a later change.

## CLI

```sh
ppl device mirror --capture full --quality full
ppl device mirror --capture full --quality fast
ppl device mirror --capture incremental
```

Full remains the default. Incremental currently returns JSON
`unsupported_capability`, with `usb_commands_sent: 0`, before opening a USB
connection or contacting an IPC owner. Incremental requires full quality;
`--quality fast` does not silently reduce its logical resolution. Optional
`--capture auto` is deliberately not implemented.

### First physical test results (executed)

The first authorized one-shot probe was run on the physical G1 (`03f0:1541`, build 15157):

```sh
scripts/emulator/run.sh run --bin ppl -- device probe-incremental \
  --device 5f3d8dbdb38ae2a0 \
  --acknowledge-experimental-fc04 \
  --out-dir artifacts/live-mirror/fc04-probe-test1
```

The device returned:
- Baseline FC08: 990 ms total (950 ms first response), 9,771 PNG bytes, 156 reports.
- Incremental FC04: 16.33 ms total (15.88 ms first response), 79 PNG bytes, 2 reports.
- Envelope decoded as valid `coordinate_update`, format 4, origin (0, 0), dimensions 1×1, `changed: false`.
- Status: `valid_response_unverified_visual_result`.

### Visual-change verification harness: explicit authorization required

The visual-change verification harness is implemented in `ppl device verify-incremental`:

```sh
scripts/emulator/run.sh run --bin ppl -- device verify-incremental \
  --device 5f3d8dbdb38ae2a0 \
  --acknowledge-experimental-fc04 \
  --out-dir artifacts/live-mirror/fc04-visual-test1 \
  --updates 5
```

The command:
1. Verifies physical G1 identity (VID:PID 03f0:1541, build 15157).
2. Captures full FC08 baseline and records baseline provenance.
3. Prompts the operator to manually change the screen on the physical calculator (e.g. open Program Catalog or press Symb).
4. Captures bounded FC04 updates (1..=N) and applies validated patches.
5. Saves the composed framebuffer (`composed.png`).
6. Captures an independent FC08 full-frame reference (`reference.png`).
7. Performs three-way per-pixel comparisons (composed vs reference, baseline vs reference, baseline vs composed) with exact mismatch counts and bounding rectangles.
8. Writes machine-readable evidence to `report.json`.

Static screen verification can also be run with `--static` without manual prompts:

```sh
scripts/emulator/run.sh run --bin ppl -- device verify-incremental \
  --device 5f3d8dbdb38ae2a0 \
  --acknowledge-experimental-fc04 \
  --out-dir artifacts/live-mirror/fc04-static-test1 \
  --updates 5 \
  --static
```

### Capability status

```text
FC04 protocol parsing:             VERIFIED
Physical FC04 response:             VERIFIED
Physical visual correctness:        UNVERIFIED (pending operator visual verification)
Sustained incremental streaming:    UNVERIFIED
Production enablement:              DISABLED
```

Normal `ppl device mirror --capture incremental` refuses with `unsupported_capability`. The legacy `--acknowledge-experimental-fc04` flag does not enable ordinary mirroring.
Use the separately approved bounded `verify-incremental-stream` diagnostic after a
reviewed changed-screen pass; see the official-software comparison for exact bounds.

## Diagnostics and performance

The existing capture/presentation records now include incremental format,
origin, patch dimensions, baseline flag and composition time. Counters separate
screenshot write attempts, valid captures, changed/published images and unique
presentation submissions. Attempt counts include ambiguous failed writes and
are not proof that the calculator received every request. USB timing retains
setup, write, first response, transfer, assembly and extraction/CRC stages;
decode and publication remain separate. Resynchronizations, errors and frame
age are recorded in worker metrics. Each streamed diagnostic file stops at
approximately 16 MiB, and only initial image evidence is automatically saved.

Repeated unchanged responses increase valid-response throughput, not changed
frame or unique-presentation throughput. Frame age is host request-to-presentation
age, not physical LCD sampling latency. Completion cadence is not physical
screen-change FPS. Historical full/fast measurements remain 0.950/2.579 FPS;
**incremental hardware FPS and improvement are unknown**.

## Offline verification

Synthetic tests independently construct expected framebuffers and compare all
76,800 pixels after each commit: full initialization, disjoint and overlapping
patches, bottom-right/origin pixels, full replacement, unchanged updates and
immutable snapshots. Baseline samples cover every RGB555 value. Negative tests
cover malformed envelopes, length/CRC/format errors, fragment ordering,
truncation, PNG CRC/IEND, palette rejection, excessive dimensions, mixed sentinel
coordinates, out-of-bounds rectangles and missing/local-out-of-order updates.

Mock transport tests assert exact FC08/FC04 writes, probe budget exhaustion,
failed-session poisoning and no production continuous-stream permission. A
synthetic end-to-end CLI probe composes a bottom-right red pixel, compares every
pixel, and checks baseline retention on corruption. Worker tests bound failed
recovery and preserve the stale view; existing shutdown, pacing, pause, rendering
and full/fast regressions also run.

Workspace checks are run using the configured shared target directory. Tests
run inside `bwrap --bind / / --dev /dev --unshare-net --die-with-parent` to prevent
existing integration tests from reaching real hidraw devices. No firmware assets
are enabled by this validation; the previously documented two asset-enabled
cold-boot timeouts remain unresolved and their thresholds are unchanged.

### Validation results (2026-10-25)

- `cargo fmt --all -- --check`: passed.
- `scripts/emulator/run.sh check --workspace --all-targets`: passed.
- Workspace clippy with `-D warnings`: passed.
- Hardware-isolated `scripts/emulator/run.sh test --workspace --all-targets`:
  **1,258 passed, zero failed, four ignored**, across 55 test binaries.
  Some proprietary-firmware cases return early without assets; these numbers
  are not an asset-enabled firmware acceptance result.
- The separately invoked ignored test
  `private_hardware_fixture_matches_the_standalone_decoder`, using the saved
  `final-full-dialog/source.raw.png`, passed without any USB access.
- Six offline Python research/report tests passed. The diagnostic summary tool
  also parsed the existing historical full-mode report successfully.
- `git diff --check`: passed.

Full/fast transport, decoder and rendering regressions passed offline. No new
physical full/fast or incremental benchmark was run. In particular the earlier
asset-enabled m4a/m4b cold-boot display timeouts were not rerun with assets and
are not relabeled as passing. No tests or timeout thresholds were weakened.
