# HP Prime G1 FC04 Physical Visual Verification & Incremental Mirroring

## 1. Physical FC04 Probe Evidence

The first physical `FC04` probe was executed against an authorized HP Prime G1 running firmware build 15157:

```bash
scripts/emulator/run.sh run --bin ppl -- device probe-incremental \
  --device 5f3d8dbdb38ae2a0 \
  --acknowledge-experimental-fc04 \
  --out-dir artifacts/live-mirror/fc04-probe-test1
```

### Result Record

```json
{
  "operation": "probe_incremental",
  "status": "valid_response_unverified_visual_result",
  "fc04_attempts": 1,
  "hardware_verified": false,
  "streaming_enabled": false,
  "request_hidapi_hex": "0000fc04",
  "response_class": "coordinate_update",
  "response_format": 4,
  "update": {
    "origin": [0, 0],
    "width": 1,
    "height": 1,
    "changed": false
  }
}
```

### Measured Timings

| Stage / Metric | FC08 Full Baseline | FC04 Incremental Update | Ratio / Speedup |
|---|---:|---:|---:|
| First response latency | 950.03 ms | 15.88 ms | ~60× faster |
| Total request duration | 990.00 ms | 16.33 ms | ~60× faster |
| HID reports transferred | 156 | 2 | ~78× fewer |
| PNG payload bytes | 9,771 B | 79 B | ~123× smaller |
| Total report bytes | 9,984 B | 128 B | ~78× smaller |

This test proves that physical G1 firmware build 15157 accepts logical request `FC 04`, returns format byte `0x04` with coordinate framing, and completes transfer in ~16.33 ms.

However, because the display was static during this probe, the resulting 1×1 patch did not change any pixels. Visual correctness under real screen changes remained unverified.

---

## 2. Request and Response Wire Protocol

### Logical Framing

- **Request:** 2 bytes `[0xFC, 0x04]` sent via standard USB HID report:
  `00 00 FC 04` (Report ID 0, sequence 0, payload bytes).
- **Full-frame alternative:** `00 00 FC 08` (320×240 RGB565).

### Response Header

```text
Offset  Bytes  Field         Description
0..1    2      [0xFC, 0x01]  Screenshot response marker
2..5    4      length_BE32   Body length (bytes after offset 5)
6..7    2      CRC_BE16      CRC16 over format + coordinates + PNG
8       1      format        0x04 (RGB565 patch) or 0x08 (RGB565 full)
9..10   2      x_LE16        Patch X origin (or 0xFFFF for full canvas)
11..12  2      y_LE16        Patch Y origin (or 0xFFFF for full canvas)
13..N   M      PNG bytes     Grayscale 16-bit PNG carrying RGB565 samples
```

---

## 3. Analysis & Interpretation of the 1×1 Unchanged Update

Reverse engineering of the official Connectivity Kit (embedded producer routines at `0x5a17b7`, `0x5a14c1`, `0x59c290`) reveals the calculator's internal image difference pipeline:

1. **Internal Baseline Cache:**
   When an `FC 08` full-screen screenshot is requested, the calculator sizes its internal cache buffer (object `113c6f4`) and copies `320 × 240 × 2` bytes from the current display buffer.
2. **Delta Calculation:**
   When `FC 04` is requested, routine `0x59c290` compares the current display buffer against that cached baseline to identify the bounding rectangle of changed pixels: `[min_x, min_y, max_x, max_y]`.
3. **Empty Change Set Handling:**
   When no pixels have changed since the previous screenshot, the bounding box routine falls back to copying the origin pixel `(0, 0)`: a 1×1 rectangle. The PNG encoder then emits a minimal 79-byte grayscale16 PNG with sample depth 16.
4. **Compositor Impact:**
   The 1×1 update is **not** a special out-of-band wire opcode or sentinel; it is a valid 1×1 pixel patch. When decoded by `IncrementalFramebuffer`, the pixel value matches the existing canvas at `(0, 0)`, producing `applied.changed == false`.

---

## 4. Visual Verification Tooling (`device verify-incremental`)

To prove that `FC 04` correctly captures genuine physical screen updates, the new command `ppl device verify-incremental` executes a controlled, bounded, interactive workflow.

### Verification Sequence

```text
              Physical HP Prime G1 Screen A
                            |
                            v
               Capture FC08 Full Baseline
              (baseline.raw.png, baseline.png)
                            |
                            v
           Prompt Operator: "Change screen now..."
           (Manual keypress on physical calculator)
                            |
                            v
              Physical HP Prime G1 Screen B
                            |
                            v
           Operator presses [Enter] to confirm
                            |
                            v
           Request Bounded FC04 Updates (1..=N)
        (update-01.raw.png, update-01-patch.png, ...)
                            |
                            v
             Apply Validated Patches to Canvas
                            |
                            v
               Save Composed Reconstructed B
                       (composed.png)
                            |
                            v
           Capture Independent FC08 Full Reference
             (reference.raw.png, reference.png)
                            |
                            v
             Three-Way Framebuffer Comparison
               (composed vs reference: target 0)
                            |
                            v
                  Emit report.json
```

### Verification Scenarios

1. **Test A — Static Screen (`--static`)**
   - Skips operator prompt.
   - Captures baseline and bounded FC04 updates while display remains undisturbed.
   - Expected: 0 pixel changes, `static_verification_passed`.
2. **Test B — Changed Screen (Default interactive mode)**
   - Captures baseline.
   - Prompts operator: `"Please change the physical calculator screen now (e.g. press Apps, Symb, or open Program Catalog). Press Enter when ready..."`.
   - Captures bounded updates, reconstructs display, and compares against independent full-frame reference.
   - Expected: baseline vs reference shows changes, composed vs reference has 0 mismatches: `visual_verification_passed`.

---

## 5. Three-Way Framebuffer Comparison Methodology

`ppl-graphics::compare_screens_detailed` computes:

```text
- Total compared pixels (76,800 for 320×240)
- Matching pixel count
- Mismatching pixel count
- Mismatch percentage (mismatch / total * 100.0)
- First mismatch coordinate [x, y]
- Mismatch bounding rectangle: [min_x, min_y, max_x, max_y] with width and height
```

### Comparison Matrix

1. **Composed vs Reference:**
   Verifies whether incremental patch sequence completely and accurately reconstructed the physical screen. (Target: 0 mismatches).
2. **Baseline vs Reference:**
   Verifies whether the physical screen actually changed between start and end. (Ensures test was not trivial/static).
3. **Baseline vs Composed:**
   Verifies that incremental updates modified the canvas in alignment with the physical change.

### Automated Diagnostic Logic

- If `baseline_vs_reference.mismatching_pixels == 0`: Status `screen_did_not_change`.
- If `screen_actually_changed && composed_vs_reference.mismatching_pixels == 0`: Status `visual_verification_passed`.
- If `composed_vs_reference.mismatching_pixels > 0`: Status `reconstruction_mismatch` with exact mismatch coordinates, bounds, and failure analysis.

---

## 6. Sustained Mirroring Implementation

The streaming desktop mirror (`ppl device mirror --capture incremental`) integrates:

1. **Persistent Copy-on-Write Framebuffer:**
   Owns a 320×240 RGB565 canvas. Unchanged updates return the existing `Arc<Grob>` without reallocating. Changed updates use `Arc::make_mut` only when a prior frame snapshot is held by the renderer.
2. **Adaptive Capture Pacing:**
   - When updates change the screen (`changed == true`): polls at full user-specified frame rate (e.g. 10 to 60 FPS).
   - When updates leave the screen unchanged (`consecutive_unchanged >= 2`): gradually relaxes polling from full rate to 50 ms (20 FPS), 100 ms (10 FPS), up to a bounded 200 ms (5 FPS) ceiling.
   - Instantly resets to full polling rate on the first changed frame.
   - Reduces CPU and USB host/device bandwidth by over 90% during static display while retaining immediate responsiveness to user input.
3. **Distinguished Performance Accounting:**
   - **Capture rate:** Valid USB responses received per second (e.g. ~30–60/s on fast loops).
   - **Visual FPS:** Updates that actually modified pixel content per second.
   - **Submission FPS:** Unique frame IDs displayed on the window surface.
   - Title bar displays `[Title] | 10.0 FPS (2.5 visual)` when static frames are present.
4. **State Machine & Recovery:**
   - `NeedsBaseline`: Sends `FC 08`.
   - `Synchronized`: Sends `FC 04` updates with monotonic host ordinals.
   - `Stale`: Any CRC error, dimension mismatch, or timeout marks the canvas stale and releases the session. Recovery reconnects to the exact verified serial and re-establishes an FC08 baseline, bounded to 3 attempts.

---

## 7. Capability Gates

```text
FC04 protocol parsing:             VERIFIED
Physical FC04 response:             VERIFIED
Physical visual correctness:        UNVERIFIED (awaiting authorized visual test)
Sustained incremental streaming:    UNVERIFIED
Production enablement:              DISABLED (requires explicit opt-in flag)
```

Normal `ppl device mirror --capture incremental` refuses before USB or IPC access with structured `unsupported_capability`.

Experimental execution requires the explicit flag `--acknowledge-experimental-fc04`.

---

## 8. Exact Commands for Authorized Testing

### Diagnostic Visual-Change Verification Command

```bash
scripts/emulator/run.sh run --bin ppl -- device verify-incremental \
  --device 5f3d8dbdb38ae2a0 \
  --acknowledge-experimental-fc04 \
  --out-dir artifacts/live-mirror/fc04-visual-test1 \
  --updates 5
```

- **Packets sent:**
  - 1× `FC 08` (baseline screenshot)
  - Up to 5× `FC 04` (incremental updates after manual screen change)
  - 1× `FC 08` (reference screenshot)
- **Data recorded:**
  - `baseline.raw.png`, `baseline.png`
  - `update-01.raw.png`, `update-01-patch.png`, ...
  - `composed.png`
  - `reference.raw.png`, `reference.png`
  - Detailed `report.json` with three-way diffs and timings.

### Static Verification Command

```bash
scripts/emulator/run.sh run --bin ppl -- device verify-incremental \
  --device 5f3d8dbdb38ae2a0 \
  --acknowledge-experimental-fc04 \
  --out-dir artifacts/live-mirror/fc04-static-test1 \
  --updates 5 \
  --static
```

### Separately Authorized Experimental Mirroring Command

```bash
scripts/emulator/run.sh run --bin ppl -- device mirror \
  --device 5f3d8dbdb38ae2a0 \
  --capture incremental \
  --acknowledge-experimental-fc04 \
  --diagnostics artifacts/live-mirror/incremental-stream-test1
```
