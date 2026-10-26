# HP Prime G1 live screen mirror

`ppl device mirror` captures the physical calculator over read-only USB HID and displays its actual screenshot pixels in a native winit/softbuffer window. Full quality remains 320×240 RGB555. No keys, programs, settings, firmware writes, or machine-learning upscaling are involved.

## Experimental incremental backend

`--capture full` preserves the default full/fast polling implementation.
`--capture incremental` provides low-latency rectangular patch capture.
The first physical `FC04` probe against G1 build 15157 returned a valid
1×1 response in **16.33 ms** (vs 990 ms for full FC08).
Actual visual correctness under physical screen changes is verified using
`ppl device verify-incremental`. Normal incremental streaming remains disabled even with the legacy acknowledgement
flag. A separately approved bounded `verify-incremental-stream` diagnostic is available;
it requires reviewed changed-screen evidence and cannot grant production permission.
See [FC04 Visual Verification](FC04_PHYSICAL_VISUAL_VERIFICATION.md) and
[incremental mirroring](INCREMENTAL_SCREEN_MIRROR.md) for details.

## Usage

```bash
ppl device mirror
ppl device mirror --scale 3 --filter nearest --quality full
ppl device mirror --filter linear
ppl device mirror --quality fast
ppl device mirror --diagnostics artifacts/live-mirror/my-new-run
```

| Option | Default | Behavior |
|---|---|---|
| `--scale 1…4` | `2` | Logical client size: 320×240, 640×480, 960×720, or 1280×960 |
| `--filter nearest\|linear` | `nearest` | Exact source colors or center-aligned bilinear interpolation |
| `--capture full\|incremental` | `full` | Existing polling or hardware-gated experimental patch capture |
| `--quality full\|fast` | `full` | 320×240 RGB555 (`FC 08`) or 160×120 RGB555 (`FC 0A`) |
| `--fps N` | `10` | Positive finite capture ceiling, not a guaranteed rate |
| `--fullscreen` | off | Borderless fullscreen with 4:3 fitting |
| `--snapshot PATH` | none | Save first received image; S also saves to this path |
| `--diagnostics DIR` | none | Streaming measurements and first-frame evidence; use a fresh directory |
| `--device SERIAL_ID` | none | Existing CLI cryptographic selector; multiple devices require selection |

Fast mode halves source width and height. Enlargement cannot recover the missing detail. Its color-preserving format was verified on G1 firmware build 15157. Other firmware is unverified; malformed or unexpected-size replies are errors, never silent substitutes for full quality. The paletted screenshot formats can change colors substantially and are not exposed as quality modes.

Keyboard shortcuts operate only on the desktop viewer:

| Key | Action |
|---|---|
| Esc | Close |
| Space | Pause/resume capture |
| S / F12 | Save the current source image as PNG |
| 1–4 | Change logical window size and leave fullscreen |

See the [official software comparison](../reverse-engineering/HP_VS_RUST_SCREEN_COMPARISON.md)
for the constructor evidence, corrected independent oracle and before/after images.

## Pixels and DPI

The screenshot's unusual 16-bit grayscale PNG samples are little-endian RGB555, not grayscale luminance. Conversion uses bit replication into `0x00RRGGBB`, exactly the softbuffer representation. The PNG container declares big-endian numeric samples, but HP's screen consumer copies the raw decoded bytes into a little-endian RGB555 QImage. The decoder preserves those bytes, then interprets the native words once. The decoder validates dimensions, output lengths, and row strides before indexing; the mirror also checks the expected dimensions of its selected quality.

The window requests a **logical client size**, excluding decorations. Rendering uses the actual **physical client size** on X11 and Wayland. Default nearest filtering centers the largest integer multiple of 320×240 that fits, with letterboxing; fractional desktop DPI therefore does not create uneven source-pixel widths. Windows smaller than native resolution use aspect fitting. Fast 160×120 pixels occupy twice the width and height of full-quality pixels. Linear filtering remains an optional aspect-filling mode. The compositor can independently resample windows; real Wayland compositor validation remains outstanding.

## Capture and performance

One dedicated worker owns one exclusive USB connection. It queries identity on the worker, sends one screenshot request at a time, verifies sequence/header/size/format/CRC, and publishes only the latest changed image. Notifications are coalesced, frame memory is bounded, and unchanged captures update health/FPS without redundant redraws. Continuous mirroring does not retain an unbounded raw-report log.

The screenshot stream drains normally on initialization. Fully completed, validated sequential requests do not insert the ordinary 50 ms quiet wait between frames. Ordinary screenshot, file-transfer, deletion, and safety/evidence APIs retain their existing behavior. A failed stream cannot issue another command; reconnection establishes the original calculator's exact serial identity again. Raw serials stay in memory and are never written to mirror diagnostics.

Pacing waits only for the remaining time before the next eligible capture. A slow capture receives no additional frame interval. Space stops new requests; a request already in flight may finish. Closing hides the window, keeps processing events, and joins the worker after its bounded in-flight reply settles (up to the five-second request deadline on an unresponsive device). Threads are never detached with a live handle or lock. Unplugging errors release the failed session before reconnect backoff.

On the tested static dialog, full capture measured about **0.94–0.95 FPS**, with a roughly **1.01 s wait before the first response** and **43 ms transfer**. Fast capture measured **2.58 FPS** at 160×120. The connected G1 enumerated at **480 Mb/s**, with 64-byte interrupt reports and a 125 µs descriptor interval. Connector type or theoretical link bandwidth does not explain its one-second pre-response wait. Firmware scheduling and screenshot generation cannot be separated from each other using host-side timing alone.

These are measured results for one screen/build, not guaranteed rates. See [the performance report](LIVE_MIRROR_PERFORMANCE_REPORT.md) for distributions, limitations, and remaining manual tests.

## Diagnostics and reproduction

Build with the space-safe workspace wrapper:

```bash
scripts/emulator/run.sh build --release -p ppl-cli --bin ppl -p ppl-gui --example mirror_diagnostics
scripts/emulator/run.sh run --release --bin ppl -- device mirror --diagnostics artifacts/live-mirror/gui-new
```

Diagnostics contain `captures.jsonl` and, for the GUI, `presentations.jsonl`. The first capture preserves `source.raw.png` and `decoded.png`; the first rendered frame preserves `renderer-source.png` and `surface.png`. Surface evidence is captured before presentation, not represented as a desktop screenshot. No PNGs are encoded on subsequent frames unless the user saves one.

Capture records separate setup, write, first-response wait, subsequent HID read waits, host assembly, extraction/CRC, PNG decompression, RGB555 conversion, and publication. Raw USB read waits are distinct from host CPU work. Reported assembly excludes read waits; minor uninstrumented bookkeeping remains inside total time. First-image evidence I/O is separately reported as `evidence_ms` and is excluded from stage timing but can affect the next arrival interval.

```bash
# Persistent connection, exactly 20 successful captures, no GUI metrics claimed:
scripts/emulator/run.sh run --release -p ppl-gui --example mirror_diagnostics -- \
  --quality full --frames 20 --out-dir artifacts/live-mirror/full-new
scripts/emulator/run.sh run --release -p ppl-gui --example mirror_diagnostics -- \
  --quality fast --frames 20 --out-dir artifacts/live-mirror/fast-new

# Bounded GUI measurement (operate the calculator physically if desired):
scripts/emulator/run.sh run --release -p ppl-gui --example mirror_diagnostics -- \
  --quality full --filter nearest --scale 2 --gui-seconds 25 \
  --out-dir artifacts/live-mirror/gui-new-25s
python3 scripts/device/mirror_report.py artifacts/live-mirror/full-new artifacts/live-mirror/gui-new-25s
```

The title shows **capture FPS**. Publication FPS counts changed captures. Displayed submission FPS counts unique frame IDs successfully submitted to the surface; expose/resize redraws do not inflate it. A static screen can have zero changed-frame FPS while USB capture remains healthy. Request age is measured from screenshot request start to presentation submission; it is not the unknown physical LCD sampling time, compositor scanout time, or LCD-to-eye latency. `last_success` records successful unchanged captures as well.

Same-source regression and isolated desktop verification:

```bash
# Substitute the absolute path of a preserved full-resolution raw capture:
PPL_MIRROR_FIXTURE="$PWD/artifacts/live-mirror/full-new/source.raw.png" \
  scripts/emulator/run.sh test -p ppl-gui --lib \
  private_hardware_fixture_matches_the_standalone_decoder -- --ignored

# No calculator access: synthetic fixture, Xvfb, XTest, ImageMagick and Pillow:
python3 scripts/device/mirror_xvfb_smoke.py \
  --fixture tests/fixtures/prime-rgb555-le.png --out-dir artifacts/live-mirror/xvfb-new

# Reproduce the old integer viewport and the new fit from identical bytes:
scripts/emulator/run.sh run --release -p ppl-gui --example mirror_diagnostics -- \
  --fixture artifacts/live-mirror/full-new/source.raw.png --size 853x640 \
  --out-dir artifacts/live-mirror/comparison-new
```

Keep physical screenshots and logs under ignored `artifacts/`; they may show private calculator content. The tracked synthetic fixture contains only generated color patterns. A diagnostic directory is not overwritten silently.

Diagnostic capture and presentation JSONL files now stop at approximately 16 MiB each.
