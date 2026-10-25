# Monitor, Project and capture trace — 2026-10-25

Binary identity, address mapping and reproducible evidence are defined in
[the HID trace](HP_CONNECTIVITY_KIT_HID_CALL_TRACE.md). All addresses here
refer to that same PE32 SHA-256 a6ee0762… executable, base 0x400000;
RVA = VA−0x400000, `.text` file offset = VA−0x400c00. Function names below
are reconstructed unless supplied by Qt metadata. Static confidence is high
for the listed instructions and call edges. No Windows session or physical
experiment was performed.

## UI entry and scheduling

Qt metadata resolves `CCalcScreenProjectorWindow` at 111ebe4, static
metacall 4a20a0, and `CCalcScreenMonitorDockWidget` at f17908, static
metacall 4a2320. Monitor's onProject slot is **42aee0**, its double-click
slot **42a7c0**, manual refresh **42a880**, automatic refresh **42a960**,
and visibility change **42b2b0**. Selected-item data supplies an endpoint
identifier; Project initializes through **42ccf0**. That function sets the
window's endpoint +4c, adjusts selection through 44fe90, disables ordinary
monitor routing through 452410, selects the endpoint in global 113a6f4,
and sets endpoint +30=1 to request a subsequent full capture. This routing
change is not evidence of a special USB projection session command.

Automatic refresh sets global 113a6f4 to 1; disabled refresh sets it to 0.
Project sets it to its selected endpoint ID. The shared service **4d02c0**
visits endpoints through **4c7580** (call at 4d060a). Value 1 refreshes
eligible nonlocal clients; an endpoint ID restricts acquisition to that
Project target. Closing Project at 42d090 restores monitor routing.

Eligibility helper **4c66a0** checks endpoint busy flags, large-file state,
existing capture pending state/deadline, an FC message being assembled,
and the outgoing queue. Builder **4cce00** sets pending +28 and a deadline
of now+**10000 ms**. Completion **4cb930**, at **4cbb05**, clears pending,
copies one-shot full-frame flag +30 to next-format selector +2c, and clears
+30. This supports response-driven repeated capture with a pending-request
gate and timeout recovery. **10 seconds is a deadline, not a refresh rate.**
No fixed Monitor/Project capture QTimer frequency has been established.
Transport reads run in their own worker. Scheduling frequency still depends
on service-loop timing, eligibility and device response; static analysis
does not establish FPS or remove the measured firmware wait.

## Exact screenshot request selection

The ten two-byte records at **f5e300** are:

```text
FC05 FC07 FC0D FC15 FC04  FC09 FC0B FC11 FC17 FC08
```

4cce00 chooses index `Project ? 4 : thumbnail_scale_index`, adding 5 when
endpoint +2c requests a full image. Thumbnail scale index is factor−1,
set through 42b3b0. Endpoint initialization 4c91b0 sets +2c=1 (4c929e).

| View | Ordinary request | Full-image variant |
|---|---|---|
| Project, full size RGB565 | FC04 | FC08 |
| Monitor factor 1 | FC05 | FC09 |
| Monitor factor 2 | FC07 | FC0B |
| Monitor factor 3 | FC0D | FC11 |
| Monitor factor 4 | FC15 | FC17 |

Thus Project can return to incremental **04** after initial/full requests;
it is not simply a continuous stream of FC08. Full-image selection can
persist for a follow-up because completion transfers the one-shot flag.
The exact first requests in a Windows session still require capture.

**450320 is now identified:** it is a thiscall-style FC request constructor
with a caller-supplied format and target selection, not a keyboard function.
It allocates 0x26 bytes, initializes internal metadata, sets serialized
length 2, writes `[FC, format]` at allocation +24, then dispatches via
42c220/4648c0 to 4cb220. Manual capture slot **4289e0** passes **08**.
The snapshot path through 431bd0 and other callers is retained in
`capture-builder.asm`. The message reference is transferred/incremented
through the callback and endpoint queue, then released by the worker.
The previous uncertainty about whether this particular function constructs
a screenshot request is superseded by caller, payload and consumer evidence.
It is not itself a PNG decoder or the automatic refresh scheduler.

## Complete physical screenshot software path

```text
Monitor slots 42a960/42b2b0; Project 42aee0 → 42ccf0
  → service 4d02c0 → endpoint visit 4c7580 → eligibility 4c66a0
  → FC format builder 4cce00
  → 4cb220 → 4d1560 → outgoing queue
  → worker 4c5270 → HID write 414820 → WriteFile 41488f
  → [physical calculator: no new observation]
  → ReadFile 41495b → 414900 → worker → 4c9b00
  → assembler 4c6d60 → completion 4cb930
  → application listener 462000 → post event 44ef70
  → QCoreApplication::postEvent at 44efde, event type 13ef
  → customEvent 44e7c0 → protocol dispatch 44f110
  → FC branch 44f1f9 → image handler 44f7a0
  → model routing 431230 → selected Project 42ced0/42d660
  → decoder 42d8e0 → patch compositor 42dae0
  → renderer 42ddc0 (pixmap/repaint)
```

Transport edges are detailed in the HID report. CConKitApp listener is its
secondary vtable eef798 (+24 subobject), slot +10 = 462000. Confusing it
with slot +14's progress callback would break this trace. Event ownership
retains the message for asynchronous Qt dispatch and later releases it.

44f7a0 computes CRC over message +8 (format, coordinates, PNG), comparing
the BE field at +6/+7 using 5a3c60. Its mismatch warning branches rejoin the
processing path; this is not justification to weaken Rust's CRC rejection.

## Decoder, patch coordinates and cached image

For incoming FC replies, message layout is:

```text
FC version body_length_BE32 CRC_BE16 format x_LE16 y_LE16 PNG…
```

42d8e0 uses the format to select scale and PNG color conversion. Even
formats **04/08** use 16-bit grayscale PNG decoding via **58afc0** with
color type 0/depth 16, interpreted as packed RGB565. Odd formats **05/09**
use RGBA8 decoding followed by RGB565 conversion. Factors are 1 for
04/05/08/09, 2 for 06/07/0A/0B, 3 for 0C/0D/10/11, and 4 for
14/15/16/17. Not every value between these ranges is accepted.
The observed decoder is embedded PNG code, not Qt's loadFromData path
used elsewhere for UI icons.

Coordinates are payload +1/+3 (message +9/+11). **FFFF,FFFF** means a
full image at origin. Otherwise **42dae0** places the decoded image at the
returned location and replicates by the scale factor into a cached image;
it does not replace the entire display with every PNG. The UI image also
has a five-pixel border. Resize handlers alter display sizing, not the
physical capture resolution.

The embedded host calculator's FC receive branch **59e050/59e128** wakes
worker **5a12f0**, whose implementation **5a1330** operates on host global
framebuffer 113a65c. It provides a second, independent explanation of the
formats: 04/05/06/07/0C/0D/14/15 compute a changed rectangle using
**59c290**; 08/09/0A/0B/10/11/16/17 force a full image. Dimension changes
and every 32nd frame also force full output. The rectangle helper compares
cached 16-bit pixels, extracts a bounding rectangle and updates the cache.
An identical-image branch returns a 1×1 rectangle at origin. Producer
5a19e5 writes packed coordinates into the response; 5a1a16 adds its CRC.

This producer is **host embedded calculator code**, not a dump of physical
G1 firmware 15157. It establishes intended format semantics and compatibility
with the desktop consumer, not physical support or timing for FC04.

## Preserved physical evidence replay

No new calculator requests were issued. Offline parser
`scripts/research/ck_screen_replay.py` inspected all 100 retained logs under
`artifacts/live-mirror/planning-baseline-20261024T213653Z/format-*.hid.log`.
All assembled messages passed their declared-length/sequence/CRC checks:
20 each for 08,09,0A,0B,0D. The 0D series contains two 106×80 full-image
responses, then eighteen 1×1 images at (0,0). This is consistent with the
recovered patch consumer, rather than interpreting each PNG as a whole screen.

The optional Pillow replay applies the recovered scale/coordinate rules.
The first tiny patch changes nine composed pixels; the following seventeen
leave the composed hash unchanged. Do not call all eighteen perfect
no-change responses: the first can blacken a 3×3 origin region. Third-scale
106-pixel width also leaves two canvas columns outside the scaled image.
Windows rendering, active LCD transitions, and FC04 were not tested.
Replay proves parser/compositor behavior on saved data, not firmware intent.
Metadata/hash outputs are retained in continuation-20261025 as
`preserved-format-metadata.json` and `preserved-0d-replay.json`.

```sh
python3 scripts/research/ck_screen_replay.py --replay \
  artifacts/live-mirror/planning-baseline-20261024T213653Z/format-0d-*.hid.log
```

## UI input and limits

Project metadata exposes size/context-menu/double-click actions; the latter
42d490 resizes locally. CCalcScreenView overrides double-click at 42ddb0
and emits its Qt signal. Its key press/release and mouse press/release
slots are inherited QLabel handlers; Project inherits QMdiSubWindow key
handling. The inspected installEventFilter call at 46b66c installs a
FileBrowserKeyHandler on the file browser. No physical-key producer was
recovered from these screen widgets. Monitor's copy/save, sort, selection
and send-message actions do not establish calculator-key forwarding.
This bounds the investigated routes, not every possible computed callback
in every release. There is no demonstrated UI→physical key→response chain.

## Comparison and recommendation

Rust's measured FC08 full RGB565 mirror remains **0.950 FPS**, FC0A fast
RGB565 **2.579 FPS**, with roughly 1009 ms before full-quality response and
43 ms transfer. Those are historical hardware measurements, not measurements
of this official build. Official software has verified incremental request
selection and patch compositing, with full-frame recovery and outstanding
request gating. No distinct physical push stream or delta codec was found
in this traced path; the observed mechanism is repeated FC screenshots with
rectangular image patches. Negotiated HID framing is a separate question.

FC04 plus a persistent RGB565 canvas is the strongest new performance
candidate. First obtain an official Windows trace and explicitly approved,
bounded read-only G1 comparison including movement, unchanged screen,
full-frame recovery and timing. Current full-frame Rust capture stays intact.
Host decode optimization cannot explain away the measured pre-response wait.

## Incremental implementation follow-up

The Rust implementation and remaining hardware boundary are documented in
[INCREMENTAL_SCREEN_MIRROR.md](../device/INCREMENTAL_SCREEN_MIRROR.md).
A focused follow-up checked 5a17b7–5a17f1 and 5a2540–5a25a4: the full-output
branch sizes cache object 113c6f4 and copies width×height×2 bytes from source
113c708 at 5a17e3. 59c290 subsequently compares that cache with the current
source. This supports FC08 cache initialization in the embedded producer;
it does not establish physical firmware-15157 synchronization behavior.

The branch producing 1×1 output still copies an origin pixel through the
ordinary rectangle extraction path (59c2ee onward). No special wire-level
no-change marker was recovered; the compositor must validate and apply it.
No frame-level ordinal was recovered. The high format bit produced at
5a19f7 is not interpreted by the new backend: unexplained extensions fail
closed. G2 and negotiated transport remain outside this implementation.

The prepared one-shot physical probe remains unexecuted. Ordinary incremental
mirroring is gated, regardless of offline test success.

Reproduce the additional cache-helper inspection without loading vendor code:

```sh
python3 scripts/research/ck_inspect.py \
  scratch/research/autonomous-control-investigation/ck/ConnectivityKit.exe \
  --disasm 0x5a17b7 0x5a17f7 --disasm 0x5a2540 0x5a25a5
```

The relevant full branch loads dimensions, calls the cache-sizing member,
then copies current source pixels; the existing trace's common PNG encoder
path follows it:

```text
5a17b7 push [113c714]           ; height
5a17bd push ecx                ; width
5a17be mov ecx,113c6f4          ; cached image object
5a17c3 call 5a2540              ; ensures allocation, stores dimensions
5a17c8 mov eax,[113c710]
5a17cd imul eax,[113c714]
5a17d4 add eax,eax              ; two bytes per source pixel
5a17d6 push eax                 ; byte count
5a17d7 push [113c708]           ; current source buffer
5a17dd push [113c6f4]           ; cached buffer
5a17e3 call e7931a              ; byte-copy helper
```

Use the retained disassembly for exact instruction bytes. RVA = VA−400000;
these code file offsets are VA−400c00, as elsewhere in this report.
