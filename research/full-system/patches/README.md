# qemuPrime GPL research overlays

These patches modify the pinned QEMU research component and follow its GPL-2.0
COPYING/LICENSE terms. Keep source notices and license files with any derivative
component. They are not copied into or linked with the production Rust HLE.
The build helper applies them only to an owned export of the exact pinned tree.
The canonical scratch reference is never modified.

0001-intc-reset-fiq.patch corrects two reproduced model bugs:

- System reset retained pending/mode/offset register state and asserted CPU lines.
- FIQ lookup used the entire mode-selection mask instead of the pending source;
  selecting UART0 and timer0 while only UART0 was pending aborted the emulator.

Unknown source/subsource lookups are bounds checked. Simultaneous pending FIQ
priority remains explicitly unsupported with a LOG_UNIMP diagnostic; no hardware
priority policy is invented. Reset defaults for masks/priority mode stay as in
the existing fork. These are machine-model consistency/safety fixes, not new
physical-calculator verification.

The zero-context patch format avoids preserving pre-existing trailing whitespace
as newly introduced repository whitespace. The source revision is pinned before
application, and the exact published overlay was rebuilt and execution-tested.

M18 adds ordered overlays:

- `0002-s3c2416-timer.patch` implements the board's dedicated five-channel
  S3C2416 16-bit countdown/interrupt subset and selects it in the board. It does
  not modify the shared Exynos model or introduce the incompatible TINT_CSTAT
  register. Static PCLK is an explicit research input. Unsupported external
  clock/DMA modes stop counting and diagnose the gap. Whole-machine migration is
  blocked until timer reconstruction is supported.
- `0003-armv5-alignment.patch` honors SCTLR.A for ordinary A32 integer loads/stores
  on pre-ARMv6 system CPUs, including control in translated-block identity.
  Actual old-binary fault omission and new-binary guest abort/return controls are
  retained. ARMv6+,64-bit helpers and user-only translation are unchanged;
  multiword edge cases and cache fidelity remain unverified.

Apply all three in order with repeated `--overlay`. The M18 focused report and
actual guest/device evidence live in `../M18_HARDWARE_EXECUTION.md` and
`../evidence/m18-hardware.json`. New patches contain original self-authored GPL
code or small changes to GPL QEMU files. Keep original notices and QEMU license
files with any derivative research distribution.

M19 adds ordered GPL overlays0004 NAND,0005 selectable clocks/reset and0006 LCD/input. Apply after0001–0003. These modify only owned pinned-source exports. See the M19 NAND, clock and LCD/input execution reports for exact supported subsets and genuine guest evidence. None boots proprietary calculator firmware.
