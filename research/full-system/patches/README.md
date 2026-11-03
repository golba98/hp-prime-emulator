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
