# Native graphics transforms and packed data

Native and Firmware remain separate backends. Native renders calculator-owned
GROBs on its VM worker; it does not execute firmware drawing routines.

The documented interfaces are in the [HP Prime User Guide](https://h10032.www1.hp.com/ctg/Manual/c05332710.pdf),
printed pages 594–601. Numeric LINE/LINE_P and TRIANGLE/TRIANGLE_P accept indexed
point and primitive records, matrix transforms, optional projection, source-space
clipping, normalized depth and a writable depth string. Primitive definitions can
override point colors and opacity. Native uses one shared implementation for
these forms and reusable transformed points. Cartesian commands read calculator
Xmin/Xmax/Ymin/Ymax, shared with the native Function application.

## Execution and validation

The compiler preserves candidate optional z-string references, including argument
position and variable identity. The runtime determines the selected overload and
returns updated strings through the existing BuiltinRef/store path. This works
for local variables, global variables, scalar coordinate triangles and cached
primitive batches. Other argument values retain their original contents.
Expressions that cannot receive writeback are rejected before rendering.

The renderer clips geometry in its original coordinate space, multiplies the
matching prefix of `[x,y,z,1]` by the supplied matrix, projects x/y using the
specified eye, and keeps normalized z independent of the projection denominator.
A 2x3 matrix multiplies actual z; translation requires the fourth matrix column.
Line and polygon clipping preserve interpolated vertex color and depth. Both
winding orders are rendered. Drawing has interruption checks within primitives
and raster rows. A drawing operation stages its GROB and depth buffer so invalid
later primitives and interruptions cannot publish partial output or fake success.

## Native point-cache format

The string prefix `U+E000 HP-PRIME-NATIVE-POINTS:1:` identifies the Native cache
schema. Its JSON object contains:

- `points`: records `[transformed_xyz, original_xyz, depth, color_u32]`.
- `eye`: a three-number projection vector or null.
- `clip`: six source-space clipping bounds or null.

The schema is deterministic and portable through Native variables and saved
program state. Both line and triangle commands consume these caches. Invalid
versions, malformed numbers and unknown strings are rejected before drawing.
This format is not an HP firmware cache ABI. Genuine firmware cache strings may
contain heap addresses; Native rejects them explicitly, and Native cache strings
must not be imported into a firmware calculation expecting its opaque cache.

## Depth strings and UTF16 units

`TRIANGLE_P([G])` and `TRIANGLE([G])` initialize two depth bytes per character at
255, including an unused high byte for odd pixel counts. Depth comparisons and
writes are per pixel, and both line and triangle commands update the same buffer.
Values from 0 through 255 are supported, with optional normalization of mesh z.

Packed depth values now retain arbitrary UTF16 units, including isolated
surrogates, in the VM's `RawString` value and version3 Native memory image.
DIM, ASC, indexing, copies, parameters and string edits operate on those units.
Ordinary Unicode strings retain their existing representation. The old tagged
Native depth format is accepted for migration; new writeback uses the genuine
packed units. Unicode-only boundaries report an explicit error for an unpaired
surrogate rather than substituting a replacement character or rendering escape
text as calculator output. This does not make heap-bearing firmware point caches
portable between backends.

## Packed DIMGROB data

Local firmware controls establish the supported data overload:

```ppl
DIMGROB_P(G1, 2, 2, {#000003E07C00001F:64h});
```

Data is the fourth argument, after explicit dimensions. Each entry contributes
64 bits; least-significant pixels come first. If enough entries fill a 32-bit
plane, entries contain two raw ARGB32 pixels. Otherwise they contain four
A1R5G5B5 pixels. The high RGB555 bit makes a pixel transparent/white; channels
expand from five to eight bits. Missing tail pixels retain white, empty data
creates white, and excess words beyond the plane are ignored. DIMGROB shares
this data decoder. A two-argument `{width,height}` list still selects dimensions;
`{width,height,data}` was rejected by the local firmware controls.

## Evidence and checks

`tests/fixtures/native-graphics-controls.json` contains self-authored probe
sources, source hashes and actual returned values from the user's original
local firmware. It excludes firmware binaries, opaque caches and heap dumps.
The controls establish packed pixel order/plane selection, source-space clipping,
projection independent of normalization, rectangular matrices and scalar depth
writeback. These are genuine local firmware results, not physical-device
verification.

`crates/ppl-vm/tests/graphics3d.rs` exercises actual VM compilation/execution,
pixel results, cache reuse, depth mutation/occlusion, alpha/color interpolation,
Cartesian state, malformed arguments and transactional errors. The old
unsupported test now specifically rejects an opaque firmware cache; numerical
point geometry has separate concrete pixel assertions.

## Fonts from installed calculator resources

The Native calculator can use the Prime Sans faces in a configured, legitimately
installed G1 `armfir.dat`. No vendor font data is bundled. A fresh installation
continues to provide the bundled DejaVu font; its shapes differ from the G1.
The selected Prime renderer uses the current FreeType font parser and the
standalone FreeType 2.8 gray scan converter. Separate per-call work areas and
bounded copied outlines keep rendering independent across workers. Selected
font failures invalidate the operation instead of substituting another font.

TEXTOUT regular-font baselines, coverage and color composition are checked
against complete independent firmware framebuffers for all seven font codes.
The original G1 ignores line-feed characters inside TEXTOUT strings; PRINT and
general UI text retain multiline layout. These text controls do not establish
complete graphics or GUI raster parity.

This software is based in part on the work of the FreeType Team. Portions are
copyright 2017 The FreeType Project (https://freetype.org/). All rights reserved.
Unmodified upstream scan-converter files, their provenance and the FreeType
License are retained in `crates/ppl-graphics/src/text/legacy_gray/`. The surrounding
adapter is separate; no older font parser is included.

## Visible-work bounds

Native circle and filled-circle operations clip their work to the target GROB
before iterating. Large finite centers and radii no longer overflow integer
squares or walk every offscreen radius step. The outline reconstructs the
existing midpoint octants, retaining endpoint/diagonal transparency writes;
filled circles use exact integer pixel-center inclusion. Generic UI lines and
rectangle borders also iterate only visible coordinates while retaining their
existing Bresenham pixels. These bounds protect the calculator worker and
Native app rendering from extreme user coordinates.

The graphics tests compare bounded rendering against independent small-shape
reference rasterizers and exercise full-i64 centers and radii. A VM regression
executes CIRCLE_P and FILLCIRCLE_P with a radius of 1E18. These are implementation
and regression checks, not new firmware or physical-device parity evidence.
