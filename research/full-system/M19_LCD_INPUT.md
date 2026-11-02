# M19 LCD and input guest execution

The isolated GPL overlay `patches/0006-lcd-input.patch` repairs the inherited
qemuPrime display/input foundation. Production Rust HLE is unchanged. No
proprietary boot/kernel image or physical calculator is used.

The S3C2443/S3C2416 variant is explicitly identified in the primary Linux
[s3c-fb driver](https://raw.githubusercontent.com/torvalds/linux/v5.15/drivers/video/fbdev/s3c-fb.c):
two windows, OSD base0x28/stride12, timing base0x8, window base0x14,
framebuffer start/end/size bases0x64/0x7c/0x94. The shared
[register definitions](https://raw.githubusercontent.com/torvalds/linux/v5.15/include/video/samsung_fimd.h)
define RGB565, halfword swap, enable and buffer select/status bits. The fork
had adapted many offsets but retained the wrong OSD stride, failed to persist
inactive buffer ends and set the request bit instead of the status bit.
The overlay repairs those operations. It adds framebuffer rectangle/row/RAM
bounds, bounded partial pixel groups, releases mappings before reset clears
ownership records, and lowers display IRQ outputs on reset. These safety
checks define refusal behavior for malformed research input, not undocumented
hardware behavior.

Original `display.S`/`display.c`/`display.ld` execute as ARM926 instructions at
ELF entry0x30000000. Guest C initializes LCD0x4c800000, writes both RAM buffers,
selects RGB565 with halfword swap, configures320x240 with a672byte padded stride,
updates pixel(17,23) to magenta, flips buffers and disables/re-enables display.
QMP `screendump` captures the actual display surface as PPM. The independent
host oracle defines every RGB pixel from coordinates and compares all76,800
pixels for each frame. PNGs are converted from those verified capture bytes;
no host framebuffer is injected. Frame0 and frame1 previews were inspected.

The input guest scans GPD output rows and GPG input columns and receives
scripted QMP key transitions. This mapping is independently corroborated by
the retained ripem BSP `lib/libhpbsp/keypad.c`, but no new physical measurement
is claimed. Per-host-key held state prevents release of one Enter alias from
clearing another held Enter alias. GPIO reset now reapplies inherited defaults
and clears matrix state and IRQ outputs. Bounce/debounce/electrical behavior
and complete external-interrupt trigger semantics remain unverified.

The guest accesses NT11002 through modeled I2C0x54000000 at slave address1.
Tests cover press/move/release, coordinates, out-of-range host-coordinate
clamping,20byte point reads,4byte version reads and reset. The model previously
read past2byte/12byte arrays and shifted negatively on overlong address writes.
Reads now return0xff after the bounded payload; extra address bytes NACK;
transaction counters saturate; reset cancels the pending conversion timer,
clears packet/pressure/address state and lowers IRQ. Raw coordinate extents
450x320, packet9/10/11 and16ms conversion delay are **inherited research-model
assumptions**, not validated G1 touch calibration/protocol/timing. The tests
prove polling transactions; they do not prove accurate physical touch IRQs.

Host-side qtest independently verifies window1 register isolation/stride,
inactive buffer retention, read-only status, invalid MMIO framebuffer refusal,
reset of a mapped display and overlong touch-address NACK. Those controls
make no guest-execution claim. The same unchanged controls fail the M18 binary:
missing inactive buffer end, failed guest buffer status and unsafe touch-read
bounds. Failed experiments remain retained. The first input-reset harness
mistook an old ready marker for reset completion; a guest-owned boot counter
now establishes fresh reset initialization before the next command.

Reproduce using a new output directory each time:

```sh
python3 scripts/emulator/full_system_display_model.py --build-root BUILD --output NEW_MODEL
python3 scripts/emulator/full_system_display.py --build-root BUILD --case lcd --output NEW_LCD
python3 scripts/emulator/full_system_display.py --build-root BUILD --case input --output NEW_INPUT
python3 -m unittest discover -s scripts/emulator -p 'test_full_system*.py'
```

Build with the pinned source/DTC/Xenial toolchain and ordered M18 overlays,
then0006. Source is exported to an owned build; the reference checkout is never
edited. Final independent display-build binary SHA256:
`3adddb7273c77dff9ab3108c923e1dac84be52e698cac224dcbdf4457d19d5a9`.
Final guest ELF SHA256:
`de4ef6e6563859a4794c350e0efc688a8a7cfad3eae6eef4155143f619e69e63`.
The exact executable/image/patch/source identities, QEMU command, GDB entry,
single step, terminal breakpoint, registers, mailbox, UART`DI`, QMP records,
frame hashes and raw artifact locations are retained in
[evidence/m19-display-input.json](evidence/m19-display-input.json).
Each guest case passed three fresh processes; host model checks also passed
three fresh processes. The boot-counter guest revision is used for final runs.
Kernel boot remains unattempted here; synthetic LCD/input execution is distinct.
