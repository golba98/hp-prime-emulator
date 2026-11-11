# Native Cartesian graphics: implementation and firmware evidence

Drawing uses current app plot bounds Xmin/Xmax/Ymin/Ymax. The projection is
fixed to the calculator plot viewport, independently of the destination GROB's
allocated dimensions. With bounds 0..10 on both axes, genuine local G1 firmware
build 15751 returns these C→PX coordinates:

| Cartesian point | Pixel point |
|---|---|
| (0,10) | (1,0) |
| (5,5) | (160,109) |
| (10,0) | (319,218) |
| (2.5,7.5) | (81,55) |
| (7.5,2.5) | (240,164) |
| (-1,11) | (-31,-22) |

The implementation uses x = round(1 + 318*(x-Xmin)/(Xmax-Xmin)) and
pixel y = round(218*(Ymax-y)/(Ymax-Ymin)). A 641x481 destination GROB still
receives PIXON(5,5) at (160,109). The previous screen-size-minus-one transform
incorrectly used 0..319 and 0..239.

ARC uses a Cartesian center and a **pixel radius**, including in rectangular
plot windows. This mixed coordinate behavior is documented in the User Guide,
page 593, and confirmed by actual firmware: ARC(G1,5,5,10,0,π,green) draws
points (170,109), (160,99), (150,109). Scaling its radius into an ellipse would
introduce a defect. CIRCLE is a retained native extension; this G1 firmware
rejects it at compilation, and the official drawing table does not list it.
No HP firmware parity is claimed for that extension.

TEXTOUT uses the projected Cartesian starting point; width and returned ending
x coordinate remain pixel quantities. Font rasterization remains a separate
approximation: these tests do not claim identical HP font glyphs.

BLIT now transforms explicitly provided source and destination Cartesian
coordinates. Omitted bounds remain the physical bitmap edges. At unit plot
scale, copied pixel locations agree with actual firmware for explicit source
regions, whole sources, positioned copies and stretched destination syntax.
The existing nearest-neighbor rasterizer still differs from actual firmware
for some resized Cartesian regions: BLIT_PROBE places a marked region at
(8,12), whereas the current native nearest-neighbor path maps it to (9,12).
Exact firmware raster alignment/resampling is unresolved; this fix does not
mark BLIT raster parity complete.

SUBGROB supports whole copies, optional start points, omitted source G0 with
explicit coordinates, and Cartesian regions. End bounds are exclusive. With a
320x240 source, a full Cartesian region (0,10)..(10,0) yields 318x218;
start-only (5,5) yields 160x131. Reversed rectangles and destination G0 are
rejected. The source-omitted single-GROB form is rejected by genuine firmware;
compiler arity remains at least two arguments.

## Documentation discrepancy: RGB alpha

The User Guide's claim that RGB's alpha byte only flags transparency does not
match this firmware build. Actual PIXON_P and RECT_P tests with RGB(red,A)
show partially changed pixels for A=64/128/129, opaque red for A=0, and unchanged
blue background for A=255. Continuous transparency remains implemented; changing
it to a threshold flag would regress these observed operations. Explicit BLIT
and TRIANGLE alpha blending remains independent.

A further exact read gives PIXON red over blue at A=0/64/128/129/255 as
FF0000/BE003F/7E007F/7D0080/0000FF. RECT interior gives BF003F at A=64 and
7E0080 at A=129. These are eight-bit integer blend-rounding differences, not
RGB555 quantization. The current shared /255 blend differs by one channel unit;
precise per-primitive firmware rounding remains unresolved. The additional
`rgb_transparency` regression verifies actual continuous behavior without
claiming these exact low-level coefficients have been corrected.

## Verification

`cargo test -p ppl-vm --test graphics_cartesian --test parity` passes seven new
behavior tests and 41 existing parity tests. New tests check actual drawing
locations, independent GROB size, unchanged pixel-radius ARC behavior, text
clipping/return equivalence, Cartesian and pixel bitmap operations, optional
source/start-point forms, and invalid regions.

Primary documentation: HP Prime User Guide, chapter 28, pages 592–599,
https://h10032.www1.hp.com/ctg/Manual/c05332710.pdf.

Genuine local firmware evidence is retained under `/tmp/prime-cartesian-probes`:
`PIXEL_PROBE.json`, `ARC2_PROBE.json`, `SUB2_PROBE.json`, `BLIT3_PROBE.json`,
`RGBFLAG_PROBE.json`, their exact `.ppl` sources, raw runtime reports and frames.
The firmware SHA-256 is
`ad9779ff72db39fe4e9977ce7c4324f39aad1bb00c849c0aab067dc5bfc2fb39`.
All probes run through the existing original-firmware compiler/runtime with
independent disposable storage profiles; no physical calculator is contacted.
