# Official HP versus Rust screen comparison

## Findings and difference matrix

The demonstrated image-quality difference was **decoding**, not scaling or USB
bandwidth. Official constructors and word copies require little-endian RGB555.
Rust and its earlier independent oracle both used big-endian RGB565. The latter
could reproduce saved bytes consistently while inventing incorrect display colors.

| Area | Official HP evidence | Rust before → after | Verified difference / benefit |
|---|---|---|---|
| USB connection | Persistent exclusive OVERLAPPED HID worker; descriptor-sized reports | Persistent exclusive HIDAPI screenshot session retained | Equivalent broad ownership; actual Windows report sizes still need capture |
| Screenshot request | Project FC04, full FC08; Monitor scaled paletted formats | Full FC08 / fast FC0A retained; normal incremental still gated | Official uses partial acquisition; no new physical throughput comparison |
| Screen decoding | QImage format 11; native little-endian RGB555 words | BE RGB565 → **LE RGB555** | Concrete color/glyph correction on preserved physical full and fast fixtures |
| Incremental state | Persistent image; rectangle placement; full-selection flags | Transactional persistent canvas retained, corrected decoder | Synthetic multi-patch exactness; changing-screen physical correctness remains unverified |
| Polling | Regular service thread sleeps 200 ms; outstanding request gate | Response-aware worker and existing idle pacing retained | No justified new scheduler constant; actual official cadence unmeasured |
| Error recovery | Pending deadlines, full flags, assembly discard, handle teardown | Strict CRC rejection and bounded recovery retained | HP's continue-after-CRC-warning behavior is not copied |
| Desktop rendering | Qt QImage/QPixmap/QWidget; shared Windows raster backing store | winit/softbuffer retained | No demonstrated need for renderer replacement |
| Scaling | Format-dependent source reduction; pixmap/widget presentation | Integer nearest default and optional linear retained | Color fix visible at native size; resampling cannot fix a packed-format error |
| Metrics | Local Virtual Calculator timer is 25 ms; not physical FPS | Capture/changed/unique submission rates remain separate | No GUI-timer-to-USB-FPS inference |

The Virtual Calculator's local four-byte framebuffer is a different source from
the physical PNG stream. Identical Qt platform code does not make their
acquisition mechanisms equivalent. See the [physical pipeline](CONNECTIVITY_KIT_SCREEN_PIPELINE.md)
and [Virtual Calculator report](VIRTUAL_CALCULATOR_ARCHITECTURE.md).

## Before/after evidence and limits

All inputs below were preserved hardware captures; no calculator was opened.
The decoder correction changes **76,761 of 76,800** pixels in the full fixture.
This number is a count of changed output values, not a subjective quality score.
The corrected text is visually clean and the error icon red, with native detail
preserved. Both byte order and 5/5/5 channel interpretation are required.

- [Prior interpretation, same physical bytes](../../artifacts/live-mirror/official-software-20261025/interpretation-565.png)
- [Corrected Rust rendering](../../artifacts/live-mirror/official-software-20261025/full-corrected/after.png)
- [Independent corrected source](../../artifacts/live-mirror/official-software-20261025/full-corrected/independent-source.png)
- [Captured X11 viewer](../../artifacts/live-mirror/official-software-20261025/xvfb-corrected/displayed.png)
- [Captured resized viewer](../../artifacts/live-mirror/official-software-20261025/xvfb-corrected/resized.png)

These links need ignored local artifacts and can contain private calculator
content. No images or raw USB captures are added to Git. The prior interpretation
is an offline decoder reproduction, not an official-application screenshot.

| Independent check | Full | Fast |
|---|---:|---:|
| Source dimensions | 320×240 | 160×120 |
| Pillow source interpretation versus Rust | 0 mismatches | 0 mismatches |
| Actual system Qt5 RGB555 conversion versus Rust | 0 / 76,800 | 0 / 19,200 |
| Integer-expanded expected versus surface | 0 mismatches | 0 mismatches |

The full actual X11 window and 1000×600 resize match the submitted pixels exactly
at 133⅓% logical DPI. Unit tests cover native, 125%, 133⅓%, 150% physical sizes
and resize/letterbox behavior. This is not a native Wayland compositor result.
System Qt5 also agrees with the conversion for all 65,536 input words, including
the unused high bit. Its code is Fedora's Qt, not vendor code loaded on the host.

An official Windows acquisition of the same physical screen and a simultaneous
LCD photograph are still absent. Consequently this proves faithful reproduction
of the recovered official consumer and visibly improved saved physical pixels;
it does not establish LCD photometric calibration or identical acquisition timing.

## Performance and ranked changes

Historical physical full capture remains approximately **0.950 responses/s**:
median first response wait 1008.965 ms, transfer 42.916 ms, total 1052.186 ms.
Fast capture was **2.579 responses/s**, at reduced source detail. Both static
runs had zero changed frames/s. The unchanged FC04 probe's total was 16.33 ms;
changed-patch latency, sustained visual FPS and missed-update recovery are unknown.

No physical performance claim is updated by replay, a 25 ms virtual timer or
a smaller patch payload. USB attempts/s, valid responses/s, changed images/s,
unique presented images/s and actual physical-change-to-display latency remain
distinct. The headless stream diagnostic records no presented FPS or display
latency; those fields are null rather than invented zero measurements.

1. **Implemented — correct packed source interpretation.** High visible benefit,
   small decoder change, direct official constructor/copy evidence and independent
   Qt verification. Full/fast and incremental decoding share the fix.
2. **Implemented — close acknowledgement-only streaming bypass.** Acceptance
   remains mandatory; diagnostic permissions have separate budgets. This corrects
   a capability-boundary discrepancy, not a capture-speed defect.
3. **Implemented — bounded sustained-test harness.** Preserves raw updates,
   timing records, exact references, reconnection baselines and identity checks.
   It enables reviewable evidence gathering, not production streaming.
4. **Pending — enable FC04 for the reviewed G1/build profile.** Potentially large
   acquisition benefit, but requires genuine changed-screen and sustained recovery
   acceptance. Risk is stale or incomplete reconstruction, not just malformed PNGs.
5. **Pending — negotiated transport or further pacing changes.** Require official
   traces showing actual use and a measurable benefit. No speculative handshake,
   request pipelining, color substitution or renderer replacement is implemented.

## Exact bounded physical procedures — not executed

Each procedure needs separate explicit authorization. Only one selected physical
G1/build 15157 may be accessed. Never defeat an existing owner lock. Leave programs,
settings, firmware and time untouched; use manual physical input only.

### A. Official reference capture

First resolve the Windows environment blocker and inspect startup without hardware.
Confirm automatic firmware update, clock-setting and synchronization operations
are disabled; set debugger/API interception before attachment. Abort on attempted
object writes, execution, clock/firmware commands or unexplained outgoing traffic.
Produce the exact startup/negotiation allowlist from the recovered code before
requesting passthrough approval; this report does not authorize FF EC negotiation.

After approval, capture only this device with USBPcap plus API tracing. Record
descriptor lengths, report IDs, logical versus API lengths and OVERLAPPED
completion counts. Use independently bounded sessions: connection/selection,
10 seconds static Monitor, up to 30 seconds Project with two manual stable-screen
changes, then close each view. Impose a total 120-second/250-FC-request ceiling,
whichever comes first, with a debugger stop before an over-budget write.
Collect an uninstrumented bounded timing run separately if breakpoint overhead
affects cadence. Never identify repaint count as fresh physical-frame count.

Use Project/native full capture for color comparisons; Monitor's paletted and
reduced formats are not equivalent source pixels. Preserve raw pcapng and API
buffers, full replies, decoded native words, reconstructed frames, desktop
screenshots and timestamps privately. Match each raw packet to its receiving
handler and presented image before asserting a reference-software difference.

### B. Rust changed-screen acceptance

After separate approval, from the `rust-rewrite` worktree:

```sh
scripts/emulator/run.sh run --bin ppl -- device \
  --device 5f3d8dbdb38ae2a0 verify-incremental \
  --acknowledge-experimental-fc04 --updates 5 \
  --out-dir artifacts/live-mirror/fc04-changed-NEW
```

Budget: one identity exchange through the selected exclusive session, one FC08
baseline, at most five FC04 updates, one FC08 reference. Device selection can also
perform the existing read-only enumeration/identity workflow. At the baseline
prompt, manually choose a genuinely different stable screen and then press Enter
on the host. Preserve the screen until reference capture finishes. Request/reply
timeouts are bounded; no automatic repetition of the experiment is authorized.

Pass requires nonzero baseline/reference and baseline/composed differences,
changed patches, and **zero** composed/reference mismatches across 76,800 pixels.
Static equality is inconclusive. Any CRC, extent, timeout or mismatch fails;
review raw fixtures and timing before proceeding. Optional operator photographs
must cover the same stable screen and avoid interpreting camera white balance
or LCD viewing angle as a protocol conversion.

### C. Separately authorized sustained stream and recovery

Only after reviewing a genuine B pass, authorize one bounded invocation:

```sh
scripts/emulator/run.sh run --bin ppl -- device \
  --device 5f3d8dbdb38ae2a0 verify-incremental-stream \
  --acknowledge-experimental-fc04 \
  --reviewed-visual-report artifacts/live-mirror/fc04-changed-NEW/report.json \
  --seconds 120 --max-updates 200 \
  --out-dir artifacts/live-mirror/fc04-sustained-NEW
```

The tool verifies the prior report's device/build, changed-screen result, fixture
hashes and current-decoder equality **before opening hardware or creating output**.
An edited capability flag cannot grant production permission. The operator still
must review provenance; a local report is not cryptographic proof of physical origin.

It caps FC04 attempts globally across reconnects, spaces starts by at least 100 ms,
and captures FC08 references every 20 attempts and at the final checkpoint.
The spacing is an experiment setting, not an acceptance FPS. Keep each screen
stable across announced reference captures; make at least two genuine transitions
between checkpoint intervals. A constant screen differing from the initial screen
does not count as repeated transitions. Do not operate active programs or dialogs
that modify stored data.

After one exact checkpoint, perform one authorized manual cable disconnect and
reconnect promptly. The failed session is dropped before reopening; at most three
reconnection attempts occur, with exact serial/build/boot matching in memory.
Recovery requires a new FC08 baseline followed by valid FC04 and an exact reference.
Prior attempts are never refunded. Deadline expiry prevents further screenshot
requests; bounded in-flight identity/I/O cleanup can finish after the ceiling.

FC08 may refresh the device cache: the tool adopts each validated reference as
the next baseline, then verifies the following FC04 sequence independently. All
reference mismatches fail before replacement can hide them. It preserves raw PNGs,
checksums, native comparisons and separate USB stage timings. Filesystem failures
abort immediately rather than initiating further USB recovery.

A complete result needs at least three exact checkpoints, two patch-driven
checkpoint transitions and verified recovery. The final attempted interval must
end at an exact checkpoint; deadline expiry or a final unverified recovery cannot
reuse earlier successful checkpoints. Short/static/uninterrupted runs
remain inconclusive. Even `sustained_verification_passed` does not enable normal
streaming: review the evidence, then separately change only the exact accepted
hardware/firmware profile. Remote navigation, Check and Run stay independently gated.
