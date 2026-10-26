# Connectivity Kit physical-screen pipeline

This supplements the preserved [HID trace](HP_CONNECTIVITY_KIT_HID_CALL_TRACE.md)
and [Project trace](HP_CONNECTIVITY_KIT_PROJECT_TRACE.md). **Their RGB565 color
interpretation is superseded:** the actual screen constructor selects RGB555.
Transport, queue, coordinate and CRC observations remain valid. Function names
below are descriptions unless supplied by RTTI, Qt metadata or imported APIs.

## Binary identity and address convention

PE32 ConnectivityKit.exe: version 2.4.2 r15751, SHA-256
`a6ee07629a0151a85bfd263bd54c3dfdf79cb57596e36c2e4456087b080b57eb`,
preferred base `400000`. All unprefixed addresses in this report are hexadecimal
preferred VAs. **RVA = VA − 400000**. Code file offsets are VA − 400c00.
The new evidence generator records all mappings in its manifest.

Independent PE64 constructor corroboration uses SHA-256
`477e75bd1040f3387a1174d6a8c9be4ffb07cb0408bf4bd810142146420ab6d1`,
base `140000000`; `140032490` is its screen-view constructor. Do not apply PE32
addresses to this image. The x64 Windows platform plugin has hash
`221713c6be9d31a8c6cc06b79240423f6492dace0620554296d976ba3ff38101`,
base `180000000`.

## Complete recovered application route

```text
Monitor slots 42a960/42b2b0; Project 42aee0 → 42ccf0
  → recurring service 462900 → 4d02c0
  → endpoint visit 4c7580 → eligibility 4c66a0
  → format selection/builder 4cce00
  → endpoint send 4cb220 → HID enqueue 4d1560
  → worker 4c5270 → transport writer 414820
  → WriteFile call 41488f / OVERLAPPED completion
  → [physical G1: historical captures only, no new Windows trace]
  → ReadFile call 41495b → report-ID removal 414900
  → worker → received-report handler 4c9b00
  → fragment assembly 4c6d60 → complete message 4cb930
  → application listener 462000 → queued event 44ef70
  → QCoreApplication::postEvent 44efde, event 13ef
  → customEvent 44e7c0 → dispatch 44f110 → FC handler 44f7a0
  → model route 431230 → Project 42ced0/42d660
  → PNG decode 42d8e0 → word/rectangle copy 42dae0
  → persistent QImage, pixmap preparation 42ddc0
  → QPixmap::fromImage → QLabel::setPixmap → QWidget::repaint
  → Qt widget raster/backing-store path → Windows platform flush
  → ordinary GDI BitBlt or conditional layered-window update
```

This is static control/data-flow evidence through the OS API boundaries. The
physical box and actual Windows timings are intentionally not represented as
new observations. Qt's plugin contains alternate graphics paths; the recovered
software backing-store path does not prove which branch every window uses at
runtime or when the desktop compositor scans out a submitted image.

| Function / RVA | Inputs, output and relevant state | Evidence / edges |
|---|---|---|
| 45c9bd / 5c9bd, 462900 / 62900 | Launches recurring worker; loops while thread handle global 1135804 is nonzero | Constructor calls 415b10 with entry 462900; service call at 462915, `Sleep(200)` at 46291a–1f |
| 4c66a0 / c66a0 | Endpoint eligibility → boolean | Checks busy/queued/current-assembly/pending/deadline conditions; called by endpoint visitor |
| 4cce00 / cce00 | Endpoint + view mode → two serialized FC bytes | Table f5e300; marks pending +28 and deadline now+10000 ms |
| 414820 / 14820 | Transport `this`, fragment pointer/length → write success | Descriptor-sized zero report-ID wrapper → WriteFile; outgoing queue consumer supplies fragment |
| 414900 / 14900 | Transport, destination/length/wait → count/status | ReadFile, asynchronous completion, strips Windows report-ID byte |
| 4c6d60 / c6d60 | Ordered fragments → complete declared-length logical message | FE→00 sequence wrap; completion callback 4cb930 |
| 44f7a0 / 4f7a0 | FC message → routed image payload | CRC BE field +6, coverage +8 onward; model callback |
| 42d530 / 2d530 | Boolean 16-bit/native-screen selector → screen view | True selects QImage format 11; false format 4. 330×250 includes border |
| 42d8e0 / 2d8e0 | Format/coordinates/PNG payload → decoded patch | Even formats request grayscale16; odd formats decode RGBA8 then pack RGB555; calls 58afc0 and 42dae0 |
| 42dae0 / 2dae0 | x,y,width,height,pixel bytes,scale,ownership → image mutation | Native word copy; persistent QImage; row offset and five-pixel border |
| 42ddc0 / 2ddc0 | Cached image + label → pixmap/widget repaint | Border writes include 7fff; imported Qt pixmap, painter and widget calls |

The 200 ms sleep is a newly recovered **regular service-loop delay**, not a
measurement of request spacing or a proof of a universal five-frame/s limit.
Wrapper 4628f0 also reaches the service. Indirect entry points, other clients,
eligibility, transfer time and pending flags matter. A ten-second pending
deadline is neither the service interval nor a guaranteed full-frame recovery.

## Connection, selection and lifetime

SetupDi/HidD enumeration obtains report lengths from HIDP_CAPS; selection opens
a persistent exclusive OVERLAPPED read/write handle. The worker starts with FA.
Project selects an endpoint and requests a subsequent full image; closing it
restores Monitor routing. No separate mandatory projection-session opcode has
been established by this path.

Legacy reports carry a sequence byte and up to 63 logical bytes for the historical
64-byte G1 protocol report. The Windows wrapper adds its report-ID byte, giving
65 bytes **for that descriptor**. This is distinct from Linux HIDAPI write length.
Alternative negotiated framing exists: conditional advertised-version/config
checks can send `FF EC 00 00 00 00`. It is transport negotiation, not remote keys.
Its use and benefit on physical build 15157 remain unmeasured.

Worker failures cancel/close the handle and discard partial assembly. Application
queues retain references across asynchronous Qt delivery. Pending capture prevents
overlap; completion clears it and transfers full-image selection flags. No recovered
wire frame generation proves freshness after reconnect. Rust's strict CRC rejection
is retained even though HP's warning branch can continue processing a mismatch.

## FC04, FC08, image layout and compositing

Project's ordinary request is `FC 04`; its full variant is `FC 08`. Monitor uses
05/07/0D/15 with full variants 09/0B/11/17. Manual capture uses FC08.

```text
FC 01 length_BE32 CRC_BE16 format x_LE16 y_LE16 PNG...
  body length excludes the first six bytes
  CRC covers format, coordinates and PNG
  PNG begins at message offset 13
  FFFF,FFFF places a complete image at origin
```

The decisive missing constructor proof is `42d5b7: push 0xb`, consumed by the
QImage constructor at `42d5d3`. Project construction passes true at `42caa6`.
The PE64 constructor independently supplies `r8d=0xb` at `14003250e`.
Qt enum **11 = Format_RGB555**, while RGB565 is enum 7.
[Qt definition](https://github.com/qt/qtbase/blob/5.15/src/gui/image/qimage.h).

For 04/08/0A grayscale16 PNGs, preserve decompressed byte order and read each
pair as **little-endian**. Then R=(word>>10)&31, G=(word>>5)&31, B=word&31;
expand each five-bit channel with `(v<<3)|(v>>2)`. Bit 15 does not become alpha.
The compositor's 16-bit load/store at 42dbb2/42dbb5 performs no byte swap.
Odd-format conversion also constructs five-bit green, corroborating RGB555.
The embedded producer independently packs 5/5/5 at 5a165d–5a1683.

PNG declares big-endian numeric samples, but HP uses that container to transport
native byte pairs. A standards-compliant PNG decompressor does not identify the
calculator's packed pixel layout. For example, decoded bytes `63 0c` mean native
word `0c63`, neutral `(24,24,24)`; the old decoder interpreted them as `630c` RGB565.

Patches update only their rectangle. Coordinates are source-space values and
scaled Monitor images replicate pixels by their format factor. A 1×1 image is
an ordinary pixel patch, not an ignorable wire sentinel. The cached image persists
between messages; presentation scaling and the UI border are separate operations.

## Physical versus embedded-producer conclusions

| Question | Established evidence | Remaining physical question |
|---|---|---|
| Initial baseline | Desktop selects full; embedded producer FC08 copies comparison cache | Exact first Windows request sequence and G1 cache reset behavior |
| Rectangle generation | Embedded 59c290 compares cached native words and extracts a bounding box | Whether build 15157 uses exactly this policy |
| Empty difference | Embedded producer returns an origin pixel through ordinary extraction | G1 origin-pixel equality cannot be assumed without applying it |
| Multiple changes | Desktop accepts repeated rectangular responses | Queue, coalescing, overwriting and missed-update behavior on G1 |
| Periodic full | Embedded producer forces every 32nd count/dimension changes | Physical periodic behavior and cache sharing between clients |
| Rate | Regular service sleeps 200 ms; historical unchanged FC04 ~16–20 ms | Actual changed-patch and official request timing |
| Recovery | Full-selection flags, pending deadline and transport teardown | Sustained G1 synchronization after loss/disconnect |

The preserved 0D replay still changes nine pixels on its first tiny patch and
zero on the next seventeen. It is paletted, so the RGB555 correction does not
change that historical replay. All 100 logs remain preserved and CRC-valid.
No dropped-update detector or device queue structure is invented from them.

Reproduce the new functions with `official_screen_trace.py`; reproduce the older
HID/queue functions with `ck_trace_evidence.py`. Both refuse mismatched binary
hashes. The [comparison report](HP_VS_RUST_SCREEN_COMPARISON.md) specifies the
unperformed physical experiments required to close these questions.
