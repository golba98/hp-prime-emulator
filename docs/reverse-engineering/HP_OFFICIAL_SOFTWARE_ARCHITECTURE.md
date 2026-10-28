# Official HP software investigation — 2026-10-25

The official Connectivity Kit establishes a concrete Rust decoding defect:
**the screenshot PNG's decoded byte pairs are little-endian RGB555, not
big-endian RGB565.** Correcting both properties removes the colored glyph
artifacts in preserved physical screenshots. The corrected Rust output matches
an independent system Qt5 `QImage::Format_RGB555` conversion exactly, and actual
X11 window captures match the corrected renderer. No new physical USB request
was made. Full-frame physical throughput has not improved or been remeasured.

Implementation is in the original `3-HP Prime Emulator` directory on
`rust-rewrite`. The temporary separate worktree was consolidated here at the
user's request: all 32 changed/new files were hash-verified before removing the
duplicate directory. Research assets and captures remain in their original local
ignored directories. No commit or push occurred.

## Provenance and evidence boundaries

The named Virtual Calculator download and its `(1)` copy are identical. Both
payload and nested application cabinets validate, rather than merely the small
bootstrap cabinet. All three installers were extracted as data, never executed
on Fedora. `official_archives.py` records each cabinet offset, hash, MSI member,
component hash, architecture, imports and fixed version. It refuses existing
output directories, truncated cabinets and unsafe archive paths.

| Installer | Bytes | SHA-256 |
|---|---:|---|
| Connectivity Kit, original | 56,639,784 | `43a16de1a126001e0ee2a796eebb4291a47faa384730e902d80f4fea32c8a493` |
| Connectivity Kit x64 | 73,154,688 | `ebd00e76de63dc00c31b19a93720a0459c8edfdcd80b00df6dba7e9e12a649f9` |
| Virtual Calculator x64 | 85,350,392 | `8653a27d099d78e4fcdd5504dbca46373381d3646391f3364116eada02fbb6a9` |

| Application | Architecture / preferred base | SHA-256 |
|---|---|---|
| ConnectivityKit.exe, original | PE32/i386 / `400000` | `a6ee07629a0151a85bfd263bd54c3dfdf79cb57596e36c2e4456087b080b57eb` |
| ConnectivityKit.exe, x64 | PE32+/AMD64 / `140000000` | `477e75bd1040f3387a1174d6a8c9be4ffb07cb0408bf4bd810142146420ab6d1` |
| HPPrime.exe | PE32+/AMD64 / `140000000` | `17b26c0eda9dba07a35383e786c0c9717e196e41f60b0dfefa8f52c7e87742a4` |

All application fixed file versions are **2.4.2.15751**. The wrappers are PE32
WiX-style Burn bundles with CAB/MSI payloads; wrapper bitness does not identify
application bitness. C++ member imports, MSVC RTTI/vtables and C++ runtime imports
establish C++ components. Qt5Core/Gui/Widgets, Qt metadata and call sites establish
Qt UI use. These observations do not identify every source language in either
executable. Bundled Qt reports **5.15.19.0**. Authenticode trust is a separate
Windows verification step, not inferred from these hashes or archive checks.

The prior PE32 binary and all 22 retained trace files still match their manifest.
All 100 historical format logs validate offline. None is an official Windows USB
capture, and none proves changed-screen FC04 reconstruction.

## Architecture and newly resolved differences

- Connectivity Kit: endpoint discovery and persistent HID worker; request queue;
  framed-message assembly; asynchronous Qt delivery; PNG decode into a persistent
  coordinate-aware screen image; Qt pixmap/widget rendering. Project selects
  FC04/FC08; Monitor uses scaled paletted variants. See the
  [complete screen trace](CONNECTIVITY_KIT_SCREEN_PIPELINE.md).
- Virtual Calculator: local simulated engine, a local 32-bit framebuffer,
  `CEmuScreen` image/pixmap, GUI timer and local input event delivery. This path
  does not acquire its displayed screen through FC04/FC08. See the
  [separate architecture](VIRTUAL_CALCULATOR_ARCHITECTURE.md).
- Shared: the x64 `qwindows.dll` files are byte-identical. All mapped sections
  of Qt5Core, Qt5Gui, Qt5Widgets, libEGL and libGLESV2 also match exactly despite
  different whole-file hashes. Thus their executable code and mapped resources
  are shared; file-hash differences alone would have understated that result.
  This does not make the local framebuffer and physical acquisition pipelines
  interchangeable.

The new pinned trace includes constructors that prior investigations omitted.
PE32 `42d5b7` passes format **11** to QImage at `42d5d3`; the Project constructor
passes its true/16-bit selector at `42caa6`. PE64 independently passes **11** at
`14003250e`. Qt's public enum identifies 11 as RGB555, and its converter uses
five bits per channel at shifts 10/5/0. The consumer copies native 16-bit words
from the decoded PNG bytes. This is a verified conversion requirement, not a
visual heuristic. [Qt enum](https://github.com/qt/qtbase/blob/5.15/src/gui/image/qimage.h),
[Qt pixel layout](https://github.com/qt/qtbase/blob/5.15/src/gui/painting/qdrawhelper.cpp).

The earlier “independent” Pillow oracle used the same wrong RGB565 assumption as
Rust. Its zero-mismatch result established agreement and faithful rendering, but
could not establish correct color interpretation. Claims that these particular
artifacts necessarily originated upstream of decoding are superseded.

## Implemented and demonstrated

The packed screenshot decoder now reads little-endian RGB555; the full/fast
capture formats and transport are unchanged. Incremental patches use the same
corrected decoder. Generic graphics RGB565 quantization remains separate.
The old synthetic fixture is preserved; a new independently generated RGB555-LE
fixture and literal primary-color byte test replace it as the screenshot oracle.

The acknowledgement-only production-stream bypass was removed. Normal
incremental mirroring remains disabled even with its legacy flag. A separate
`verify-incremental-stream` experiment has a shared 200-attempt/120-second maximum,
reviewed changed-screen prerequisite, exact reference checks, fresh baselines
after recovery and three total reconnect attempts. It never promotes a profile.
The actual physical changed-screen and sustained gates remain unverified.

| Verification | Result |
|---|---|
| Native system Qt5 conversion, every 16-bit word | 65,536 / 65,536 agree |
| Preserved physical full source → independent Qt5 → Rust | 76,800 pixels, zero mismatches |
| Preserved physical fast source → independent Qt5 → Rust | 19,200 pixels, zero mismatches |
| Corrected renderer → actual X11 window and resize | zero pixel mismatches |
| X11 shortcuts, pause/resume, snapshot and close | pass; close approximately 8.35 ms |
| Hardware-isolated workspace all-target tests | 1,282 passed; zero failed; four ignored |
| Workspace Clippy, warnings denied | pass |

Private before/after images, independent reports and test logs are under
`artifacts/live-mirror/official-software-20261025/`. They show visibly repaired
saved source pixels and a faithful desktop display, not a new live hardware
measurement. Firmware tests that return early without proprietary assets are
not firmware boot acceptance. Historical m4a/m4b failures remain separate.

## Runtime work and remaining acceptance

A disposable QEMU/KVM Windows evaluation environment was prepared with four
vCPUs, 8 GiB RAM, an 80 GiB sparse disk, TPM2, read-only installer media, no network
and **no physical USB passthrough**. Microsoft media is 5,112,850,432 bytes,
SHA-256 `67cec5865eaa037a72ddc633a717a10a2bed50778862267223ddb9c60ef5da68`.
The initial and Secure Boot-enabled hardware profiles reached Windows Setup but
its requirements check refused installation. This is environment evidence, not
an HP runtime experiment. Preserved commands, screenshots, private answer file,
guest bootstrap and media hashes are under the ignored research `windows/` root.
Runtime status and any subsequent progress are recorded in the supporting
Virtual Calculator report.

Official physical USB scheduling, actual Windows HID report lengths, physical
FC08 cache resets, silent update loss, changed-screen FC04 correctness, sustained
streaming and LCD-to-desktop latency still require explicitly approved hardware
experiments. Native Wayland compositor output and a simultaneous LCD photograph
are also unverified. No official-versus-Rust physical speed advantage is claimed.

## Reproduction and next experiments

```sh
python3 scripts/research/official_archives.py INSTALLER NEW_IGNORED_DIRECTORY
python3 scripts/research/official_screen_trace.py \
  scratch/research/official-software-20261025 NEW_TRACE_DIRECTORY
python3 scripts/research/qt555_oracle.py RAW_PHYSICAL_PNG RUST_DECODED_PNG
python3 -m unittest discover -s scripts/tests -p 'test_*.py'
```

The new trace checks all four input hashes before writing. Its address records
contain VA, RVA, file offset and evidence hashes; a second generation reproduced
the output byte for byte. It extends rather than rewrites the original trace.
The Qt oracle loads Fedora's system Qt5Gui, not any vendor library; this host has
`qt5-qtbase-gui-5.15.18-3.fc44.x86_64`. Proprietary binaries and captures stay ignored.

The exact bounded physical procedures and improvement priorities are in
[the comparison](HP_VS_RUST_SCREEN_COMPARISON.md). Neither hardware testing nor
physical remote input, Check, Run, uploads or firmware operations is authorized
by this report or by passing an offline test.
