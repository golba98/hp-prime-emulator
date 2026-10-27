# Live viewer quality and performance — 2026-10-25

## Official-reference investigation and corrected result

**The colored glyph artifacts in the preserved physical screenshots are fixed.**
The official Connectivity Kit constructor selects QImage format 11 (RGB555),
and its compositor copies decoded PNG byte pairs as little-endian words. The
previous Rust decoder read those bytes as big-endian RGB565. Both byte order
and channel layout were wrong; smoothing was unrelated to the cause.

The proof is present independently in the official PE32 and PE64 builds. The
embedded producer's 5/5/5 packing corroborates it, but no producer routine is
misrepresented as firmware 15157. Full native source detail remains 320×240;
fast remains 160×120. Nearest/linear scaling and the softbuffer renderer are
retained. Generic GROB RGB565 comparison arithmetic is unchanged.

Earlier independent Pillow tests shared the incorrect format assumption. Their
zero-mismatch result demonstrated that two decoders agreed and that the renderer
reproduced those decoded values. It did **not** establish correct colors. The
historical conclusion below that these artifacts necessarily originated upstream
of decoding is superseded for these fixtures.

### Independent verification and visible evidence

- System Qt5 `QImage::Format_RGB555` agrees with the conversion for all 65,536
  possible input words, including the unused high bit.
- Preserved physical full and fast fixtures match independent Pillow and actual
  system Qt5 conversions with zero mismatches across 76,800 / 19,200 pixels.
- Independently expanded expected images match Rust surfaces exactly in both
  modes. The actual corrected X11 window and its 1000×600 resize have zero pixel
  mismatches. The isolated GUI also passes pause/resume, snapshot and close.
- A literal byte fixture distinguishes LE RGB555 from all three incorrect
  combinations of byte order and RGB565/RGB555 layout. Transactional patch tests
  cover changed, overlapping, full, origin and unchanged updates.
- The preserved full fixture changes 76,761 output pixel values under the correct
  interpretation. This is not a performance score or a new physical capture.

Local private evidence:

- [Before: former decoder, same bytes](../../artifacts/live-mirror/official-software-20261025/interpretation-565.png)
- [After: corrected Rust surface](../../artifacts/live-mirror/official-software-20261025/full-corrected/after.png)
- [Independent source](../../artifacts/live-mirror/official-software-20261025/full-corrected/independent-source.png)
- [Actual X11 viewer](../../artifacts/live-mirror/official-software-20261025/xvfb-corrected/displayed.png)
- [Actual resized viewer](../../artifacts/live-mirror/official-software-20261025/xvfb-corrected/resized.png)

The after images show clean neutral glyphs and the red error icon, without
invented color substitutions or discarded native pixels. The before image is
an offline reproduction, not an official Windows screenshot. Private source
images remain ignored and were not rewritten.

### Physical performance and capability boundary

No new hardware requests were authorized or performed. The historical full
baseline remains approximately 0.950 responses/s and fast 2.579 responses/s.
The source-format correction does not reduce the approximately one-second
full-frame pre-response wait. The regular official service loop sleeps 200 ms;
this is static scheduling evidence, not a measured Windows capture rate.
Virtual Calculator's local 25 ms timer does not imply any physical USB FPS.

The saved FC04 visual report is still `screen_did_not_change`, so physical
changed-screen reconstruction and sustained acceptance remain unverified.
The legacy acknowledgement flag can no longer bypass the normal streaming gate.
A separate bounded stream diagnostic checks reviewed visual fixtures, limits
FC04 attempts/time, validates references, and checks exact-identity recovery.
It cannot grant production permission, including after synthetic tests pass.

### Validation and remaining work

Hardware-isolated workspace all-target tests: **1,282 passed, zero failed,
four ignored**. Workspace Clippy with warnings denied passes. Python research,
script-contract and independent-oracle tests pass. The complete original pinned
trace remains reproducible, and the new four-binary trace reproduces exactly.
Tests needing proprietary firmware still do not establish asset-enabled boot
acceptance; historical m4a/m4b failures remain unchanged.

Official Windows acquisition of the same physical screen, a simultaneous LCD
photograph, real changed-screen FC04, sustained recovery, end-to-end physical
latency and native Wayland output remain unverified. A disposable Windows VM was
prepared and booted without physical USB; Setup compatibility failures prevented
claiming an official application runtime experiment. All evidence and exact
bounded next steps are in the [central investigation](../reverse-engineering/HP_OFFICIAL_SOFTWARE_ARCHITECTURE.md)
and [comparison](../reverse-engineering/HP_VS_RUST_SCREEN_COMPARISON.md).

## Preserved historical report — color conclusions superseded above


## Result and scope

The default nearest viewer now uses centered integer scaling in physical pixels.
Native 320×240 detail is preserved; fractional desktop DPI no longer creates
unequal source-pixel widths. Linear filtering remains an optional aspect fit.
This improves presentation consistency, but does **not** remove the colored
speckling already present in the saved physical source image. No color correction,
sharpening, palette substitution, or invented motion was introduced.

Work was performed on `rust-rewrite`, preserving the existing dirty tree and
scripts refactor. No new physical requests, keys, programs, deletion, commits,
pushes, or changes to hardware capability gates were performed. Historical
screenshot logs and evidence were retained. All new local evidence is under
`artifacts/live-mirror/quality-20261025/` (private calculator content, ignored).

## Source → decoder → renderer evidence

Inputs are the preserved physical `final-full-dialog/source.raw.png` and
`final-fast-dialog/source.raw.png`. These are grayscale16 PNG containers whose
big-endian samples encode RGB565, rather than ordinary grayscale brightness.
An independent Pillow PNG decode and explicit RGB565 bit extraction are compared
against the Rust output by `scripts/device/viewer_fixture_compare.py`.

| Stage | Full | Fast |
|---|---|---|
| A: physical source dimensions | 320×240 | 160×120 |
| A→B: independent RGB565 vs Rust decoded pixels | 0 mismatches | 0 mismatches |
| B→C: independent integer-expanded expected vs rendered surface | 0 mismatches | 0 mismatches |
| C: 853×640 client viewport | 640×480 at (106,80) | same viewport |
| Displayed X11 vs submitted surface; resized 1000×600 | 0 mismatches | 0 mismatches |

The comparison checks every pixel, including letterbox borders. At 640×480,
full pixels occupy 2×2 squares; fast pixels occupy 4×4 squares. Lost fast-mode
text detail first occurs in A. Color speckling around the dialog text is also
present in independently decoded A, including full capture. Its upstream origin
(calculator framebuffer, screenshot producer, or earlier transport behavior)
is **not established** by these fixtures. This report does not claim that it is
normal LCD appearance. An authorized simultaneous physical-screen photograph
and source capture would help distinguish those causes.

No decoder/channel/stride/order/palette defect was found in these RGB565 inputs.
The decoder validates expected dimensions, stride, lengths, PNG completion and
uses big-endian samples with bit-replicated channels. Nearest presentation adds
no colors. Immutable Arc frame snapshots and one bounded latest-frame slot prevent
partially overwritten frames and unbounded queues.

Before/after images, generated from identical source bytes:

- [Full before: fractional fill](../../artifacts/live-mirror/quality-20261025/full/before.png)
- [Full after: uniform integer pixels](../../artifacts/live-mirror/quality-20261025/full/after.png)
- [Fast after: visibly lower detail](../../artifacts/live-mirror/quality-20261025/fast/after.png)
- [Independent full source](../../artifacts/live-mirror/quality-20261025/full/independent-source.png)
- [Actual X11 window](../../artifacts/live-mirror/quality-20261025/xvfb-instrumented/displayed.png)
- [Actual resized window](../../artifacts/live-mirror/quality-20261025/xvfb-instrumented/resized.png)

These local links require the preserved artifacts directory. The before image
uses the former fractional viewport; it is a renderer reproduction, not a
historical desktop screenshot. The actual X11 captures independently verify
softbuffer output. The original screenshot mentioned in the task was not
attached, so its exact desktop state cannot be reconstructed.

## Demonstrated defects and changes

- `mirror.rs`: nearest default previously selected fractional fit. For example,
  853×640 mapped 320 source columns into unequal 2/3-pixel widths. Default integer
  scaling now selects 640×480, centered, with complete border repaint on resize.
  Linear retains fractional fit. Smaller-than-native windows must downsample.
- `mirror.rs`: F12 had no keyboard handler. It now saves the current native source
  PNG using the existing S path, independent of display scaling.
- The title formerly said `FPS (visual)` without identifying the counted events.
  It now explicitly displays capture/s, changed/s, and unique presented/s, with
  `160×120 low detail` in fast mode. Presentation means successful softbuffer
  submission, not compositor scanout.
- The rate estimator previously ignored elapsed time after the last change until
  suddenly expiring. Rates now decay across gaps. Static content correctly reaches
  zero changed/s; duplicate captures are never counted as movement.
- Resume formerly discarded the comparison baseline and could count an unchanged
  screen as a new visual frame. It now preserves pixels and frame identity while
  republishing when necessary. Resize/expose/resume do not inflate unique counts.
- Disconnect/reconnect explicitly marks the last image stale. A one-second health
  timer labels missing captures after three seconds without redrawing static pixels.
- Diagnostics now record publication-to-GUI-notification latency. Existing USB,
  decompression, conversion, publication, scaling and submission timings remain.

Tests add complete expected-image expansion for full and fast inputs at native,
125%, 133%, 150% DPI physical sizes and arbitrary resize sizes. Frame replacement
checks every destination pixel and border to detect stale data. Controlled
A,A,B,B input checks four captures, two changes, two duplicates and latest B.
The Xvfb test checks actual window pixels, pause, resume, F12, resizing, closing,
and duplicate/unique statistics. Existing cancellation, bounded shutdown,
disconnect/recovery and malformed protocol tests remain active.

## Performance: measured evidence and limits

Re-analysis of saved physical diagnostics (20 full and 20 fast captures):

| Stage, median ms | Full | Fast |
|---|---:|---:|
| USB write | 0.2301 | 0.1773 |
| First response wait | 1008.9647 | 371.7600 |
| Subsequent payload read waits | 42.9157 | 15.4906 |
| Host message assembly | 0.0069 | 0.0041 |
| Extraction/CRC | 0.0197 | 0.0073 |
| PNG decompression | 0.1569 | 0.0490 |
| RGB565 conversion | 0.0624 | 0.0160 |
| Publication | 0.0004 | 0.0003 |
| Total | 1052.1856 | 387.5148 |
| Capture responses/s | 0.950 | 2.579 |
| Changed frames/s over static run | 0 | 0 |

Approximately 96% of time is before the first response. Host evidence cannot
separate calculator screenshot generation from firmware scheduling. This is not
a one-second renderer cost. Saved GUI linear measurements have median scaling
3.9871 ms and submission 0.0118 ms at 960×720; these are historical, not new
physical measurements. New Xvfb median publication-to-notification is 0.7702 ms (max 1.4138 ms),
nearest scaling 0.3997 ms, and submission 0.0019 ms. These include resize redraws
at several window sizes. Full notification/render timings are in
`xvfb-instrumented/diagnostics/presentations.jsonl`, measured on synthetic replay
of saved physical pixels. They do not establish physical streaming performance.
The historical report prints zero request rate for old rows without request
counters; that field is unavailable, not a measured zero USB request rate.

The static physical runs each publish only the initial frame. This explains a
healthy capture rate alongside zero visual FPS for those runs; it does not prove
that the user's unprovided screenshot showed a static screen. Saved GUI evidence
also contains two changed publications, so the path can publish different frames.
Neither offline replay nor the unchanged FC04 probes establishes sustained
changed-screen physical FPS. End-to-end LCD-to-eye latency remains unmeasured.

The event loop continues handling resize/input during slow USB reads. USB work
owns an exclusive session on its worker, with no image-state lock held across
reads, one request in flight, coalesced notifications and bounded storage.
Closing hides the window promptly and retains transport ownership until a bounded
in-flight exchange settles (up to five seconds); it never detaches the USB worker.
Xvfb close measurements are approximately 8 ms with the fixture source.

Incremental adaptive idle pacing is unchanged: after eight unchanged updates it
can wait up to 200 ms between request starts (or the user's slower interval).
Thus a real change during idle can wait nearly 200 ms to be detected; resetting
pacing after detection does not eliminate that delay. It is bounded, not instant.
Full/fast capture does not apply this incremental idle backoff or append an extra
frame interval after a slow capture. No pipelining was introduced.

## Physical FC04 acceptance: pending, not passed

All saved `verify_incremental` reports were searched. The available
`fc04-visual-test1/report.json` has five valid 1×1 unchanged patches, identical
baseline/reference, `screen_did_not_change`, `hardware_verified: false`.
The 16–20 ms responses prove protocol response latency only.

The existing bounded interactive tool already implements the necessary baseline,
manual-change prompt, patch reconstruction, independent reference and three-way
pixel comparison. **After separate explicit authorization**, the operator can run:

```bash
scripts/emulator/run.sh run --bin ppl -- device --device 5f3d8dbdb38ae2a0 \
  verify-incremental --acknowledge-experimental-fc04 --updates 5 \
  --out-dir artifacts/live-mirror/fc04-changed-NEW
```

Wait for the prompt after baseline capture, manually open a visibly different
stable screen on the physical calculator, then press Enter on the computer.
Keep that screen stable until reference capture finishes. No remote keys or PPL
are needed. Use a fresh output directory. Pass requires all responses validated,
nonzero baseline/reference differences, changed patches, and **zero**
composed/reference mismatches across all 76,800 pixels. Static equality is
inconclusive. Any mismatch fails; retain raw payloads and the report.

After reviewing a genuine pass, separately authorize a bounded sustained stream
validation: fixed request/time limit, multiple manual stable-screen changes,
independent FC08 references at checkpoints, exact pixel comparison, statistics
separating duplicate/changed/submitted frames, and disconnect/reconnect recovery.
Review any FC08 cache effects when designing checkpoints. Only after both visual
and sustained acceptance pass should the exact G1 firmware capability profile
be approved. No generic G1/G2 gate should be bypassed. No streaming test was run.

## Validation and reproduction

All commands use the existing target directory through the wrapper. No cargo
clean or additional build trees. Outputs are retained under the new evidence root.

```bash
cargo fmt --all -- --check
scripts/emulator/run.sh check --workspace --all-targets
scripts/emulator/run.sh clippy --workspace --all-targets -- -D warnings
scripts/emulator/run.sh test --workspace --all-targets
git diff --check
python3 -m unittest discover -s scripts/tests -p 'test_mirror_report.py'
```

Formatting, workspace check, Clippy with warnings denied, workspace all-target tests,
and diff whitespace checks passed. Workspace totals: 1271 passed, zero failed, 4 ignored
(in addition to firmware tests that return early without assets).
The opt-in preserved full hardware fixture test was explicitly run and passed.
Workspace tests include graphics, screenshot decoder/session, GUI, incremental
protocol, and CLI mirror integration tests. Python mirror-report tests: 5 passed.
The private fixture test remains opt-in; the independent script above checks
physical bytes without requiring USB. Reproduce visuals with new directories:

```bash
scripts/emulator/run.sh build --release -p ppl-gui --example mirror_diagnostics
scripts/emulator/run.sh run --release -p ppl-gui --example mirror_diagnostics -- \
  --fixture artifacts/live-mirror/final-full-dialog/source.raw.png \
  --size 853x640 --out-dir artifacts/live-mirror/quality-replay-NEW
python3 scripts/device/viewer_fixture_compare.py \
  artifacts/live-mirror/final-full-dialog/source.raw.png \
  artifacts/live-mirror/quality-replay-NEW
python3 scripts/device/mirror_xvfb_smoke.py \
  --fixture artifacts/live-mirror/final-full-dialog/source.raw.png \
  --out-dir artifacts/live-mirror/xvfb-quality-NEW
```

Repeat with `final-fast-dialog/source.raw.png` for fast evidence. Test changes
include a corrected fast-source expectation in Xvfb: an intermediate attempt used
320×240 indexing for a 160×120 source and failed in the test oracle. That failed
output is retained as `xvfb-fast-final`; `xvfb-fast-verified` is the successful run.

Known firmware cold-boot failures (`m4a`/`m4b`) are separate unresolved work.
Asset-dependent firmware tests early-return when proprietary asset environment
variables are absent; their Cargo `ok` output is **not firmware acceptance**.
No asset-enabled cold-boot acceptance was performed or claimed here.

Real Wayland compositor display capture, fractional compositor resampling,
new physical full/fast viewing, changed-screen FC04 and sustained incremental
streaming still need physical/desktop validation. Source speckling remains
visible and unexplained upstream; this work demonstrates faithful reproduction,
not repaired calculator-generated pixels or higher physical visual FPS.
