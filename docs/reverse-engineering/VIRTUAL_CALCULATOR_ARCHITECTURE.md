# Virtual Calculator architecture — 2026-10-25

This is the separate official **HPPrime.exe** implementation. Its local simulated
LCD is not evidence of physical G1 USB update performance.

## Provenance and packaging

Installer SHA-256 `8653a27d099d78e4fcdd5504dbca46373381d3646391f3364116eada02fbb6a9`,
85,350,392 bytes. Both downloaded copies match. The 84,481,571-byte payload
cabinet and 50,749,074-byte nested application cabinet pass `7z t`; the latter
contains 189 files. Extraction preserves the original installer and records the
MSI/container chain, rather than treating file existence as download completion.

HPPrime.exe is PE32+/AMD64, fixed version **2.4.2.15751**, preferred base
`140000000`, SHA-256
`17b26c0eda9dba07a35383e786c0c9717e196e41f60b0dfefa8f52c7e87742a4`.
All addresses below are preferred hexadecimal VAs; **RVA = VA − 140000000**.
Qt and MSVC C++ metadata establish Qt/C++ UI components. Qt is 5.15.19.0.
Skin files, resources, Qt platform/image plugins and graphics support DLLs are
separate installed assets. A graphics support DLL's presence alone is not its use.

## Local LCD and event path

```text
local simulated engine / framebuffer holder global 14105a2a8
  → source pointer at holder +148
  → CEmuScreen update 1400136f0
  → copy width×height×4 bytes into QImage at view +40
  → conditional local overlay 140029ab0
  → set byte 3 of each pixel to FF
  → QPixmap at view +60, convertFromImage
  → QWidget::repaint
  → CEmuScreen::paintEvent 140015250
  → optional QPainter::scale using +98/+a0
  → drawPixmap at origin
  → Qt Windows platform backing-store path
```

| Function / RVA | Recovered role and observed state | Caller/callee evidence |
|---|---|---|
| 140012620 / 12620 | CEmuScreen constructor | Installs RTTI-correlated vtable 140d2bd78; creates image +40, pixmap +60, worker and QObject |
| 1400136f0 / 136f0 | Display update, view pointer → pixmap/repaint | QImage dimensions, copy from holder +148, overlay, alpha-byte loop, convertFromImage |
| 140015250 / 15250 | Paint handler | QPainter construction; optional scale from view +98/+a0 when +94 set; drawPixmap at origin |
| 140015370 / 15370 | QtScreenThread process loop | Metadata static metacall/slot; local engine service; separate from HID screenshot requests |
| 140016470 / 16470 | Start GUI timer/engine thread | QBasicTimer::start with 0x19=25 ms; QThread::start |
| 1400164a0 / 164a0 | Timer dispatch | Matches timer ID, local engine flags at +2a34, time checks, calls 1400136f0 on embedded view +50 |
| 140029ab0 / 29ab0 | Conditional local image overlay | Checks local animation/time globals; calls drawing helper 14009d810; not a generic screenshot color repair |

Qt metadata independently identifies `QtScreenThread` at `14103a770`, static
metacall `14001de80`, signals `finished`, `error`, and slot `process`.
`CEmuScreen` is at `14103a7a0`; `CapturedCalcFramebuffer` at `141039ac8`;
`QtEmulatorWidget` at `14103a7d0`. MSVC x64 RTTI locators use image-relative
fields and native pointer-sized vtables. The extended inspector verifies the
locator's self RVA and handles Qt's 24-byte string headers on x64.

The GUI image is a **32-bit** local image; initialization passes Qt format 6
(ARGB32 premultiplied) at `140015dd5` for parent image +90. The embedded
view is at parent +50, making this its image +40. Update forces opaque alpha before
conversion. This is different from Connectivity Kit's packed RGB555 USB image.
The 25 ms timer is a local scheduling parameter, not proof of 40 distinct frames/s.
No physical capture occurs in the recovered display-update function.

Local key press/release handlers at `1400137e0` and `1400138a0` construct local
event records (types 1 and 0B) and call the engine event route `14007c9a0`.
Mouse handlers map coordinates through the screen scale and use local event
delivery. Those records are not verified USB key packets. HID imports in this
executable do not turn this particular local image path into a physical screen.

## Shared rendering components

The x64 Windows plugin is identical in both packages, SHA-256
`221713c6be9d31a8c6cc06b79240423f6492dace0620554296d976ba3ff38101`.
Qt5Core/Gui/Widgets and EGL/GLES libraries have identical mapped sections,
including `.text`, `.rdata`, resources and relocations, despite whole-file hash
differences. Exact section hashes are retained in `analysis/shared-components.json`.

The plugin's `QWindowsBackingStore` RTTI resolves vtable `180109378`, with
flush at `180062340`. That function calls ordinary GDI BitBlt at `1800628cb`
and has a conditional UpdateLayeredWindowIndirect path at `1800627e7`.
This proves a concrete shared raster presentation implementation. Actual branch
selection, compositor latency and GPU fallback need runtime evidence; neither
bundled D3Dcompiler nor libGLESV2 proves an accelerated calculator screen.

Application-specific framebuffer production and acquisition are not shown to
be identical. Rust can reuse the observed pixel semantics and event separation
without embedding HP binaries, importing proprietary code or replacing softbuffer.

## Windows runtime preparation and blocker

The isolated QEMU environment, media hash and retained artifacts are described
in [the central report](HP_OFFICIAL_SOFTWARE_ARCHITECTURE.md). Both ordinary
OVMF and Secure Boot/SMM configurations with a TPM2 reached Windows Setup but
showed its system-requirements error. Captured setup diagnostics show
`0xC1900220`, extended `0x40008`, plus a missing .NET diagnostic component.
The code is a compatibility-report display failure, not sufficient evidence
to name TPM or CPU as the underlying failed requirement.
[Microsoft error definitions](https://learn.microsoft.com/en-us/troubleshoot/windows-client/setup-upgrade-and-drivers/windows-10-upgrade-resolution-procedures).

No official application runtime, USBPcap session or official desktop FPS has
been established by booting Windows Setup. The guest bootstrap is prepared to
record Authenticode status, install the unmodified official installers, capture
module versions, process CPU/working set and bounded desktop samples. Its
presence is a prepared procedure, not a successful execution result.

Private reproducible environment records are under
`scratch/research/official-software-20261025/windows/`: media provenance,
`launch-arguments*.json`, answer file/read-only research ISO, bootstrap,
QMP control scripts, setup screenshots and disk. The VM exposes only Unix
monitor/VNC sockets and no physical device or host repository share. Guest
credentials remain private. Resolve the setup compatibility failure or supply
a working Windows VM before claiming the planned application runtime findings.

The VM was stopped through QMP after retaining setup diagnostics. The disk has
snapshot `setup-blocked-20261025`; no HP runtime result is inferred from it.
To reproduce the retained environment, first confirm no instance is running:

```python
from pathlib import Path
import json, subprocess
root = Path("scratch/research/official-software-20261025/windows").resolve()
runtime = Path(json.loads((root / "runtime.json").read_text())["runtime"])
subprocess.run([
    "swtpm", "socket", "--tpm2", "--tpmstate", f"dir={root / 'tpm-state'}",
    "--ctrl", f"type=unixio,path={runtime / 'tpm.sock'}", "--daemon",
    "--pid", f"file={runtime / 'swtpm.pid'}",
], check=True)
subprocess.run(json.loads(
    (root / "launch-arguments-secureboot.json").read_text()
), check=True)
```

The short Unix socket directory is private; the long research path cannot be
used directly as a Unix socket name. The TPM exits when QEMU disconnects, so
restarting QEMU also requires restarting the TPM. Use retained `vm_control.py
key ret` during the DVD boot prompt and `screen NAME.png` to save a QMP capture.
`quit` stops this guest. The launch configuration contains no `usb-host`, host
serial port, network interface or repository share. Do not add physical USB
passthrough while reproducing this environment failure.

Required runtime follow-up: correlate actual local framebuffer copies and
paint calls, inspect the active Windows backend, observe scale/input changes,
measure changed framebuffer versus desktop submission rates, and compare with
Connectivity Kit only on genuinely equivalent content. Keep VM rendering
measurements separate from Fedora and physical G1 performance.
