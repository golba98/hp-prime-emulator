# M19 clock/reset execution

The GPL overlay `0005-s3c2416-clocks-reset.patch` adds an explicitly selectable
S3C2416 clock-tree research profile. Production Rust HLE is unaffected. No
proprietary kernel or physical calculator was used.

## Contract and sources

The default profile retains M18\'s explicit 66 MHz PWM and 24 MHz UART inputs.
Select the modeled subset with `-global s3c2416-sysc.clock-tree=on`; configure
`xti-hz` (default 12000000) and `ext-hz` (default zero). These are synthetic
research inputs, not measured G1 oscillator rates. Supported CLKSRC selection,
MPLL6552/EPLL6553 integer rate calculation, main predivider/HCLK/PCLK divisors,
PWM/UART peripheral gates and UART SCLK divider affect the connected devices.
Active PWM changes preserve remaining counter state. Reserved HCLK division or
zero/overflowing PLL rates stop dependent clocks safely. External UART clock
selection remains unsupported. ARM CPU execution rate, PLL analog lock delays,
DRAM training and unrelated peripheral clock consumers are not modeled.

UART baud parameter calculation uses the documented divisor plus one. Output
remains an immediate chardev transaction; this does not verify wire baud timing.
Selecting a gated/unsupported clock suppresses transmission with a diagnostic.

SYSC now resets deterministically. Exact S3C2416 software-reset key
`0x533c2416` requests real machine reset; wrong keys are ignored. Software reset
retains documented SYSC fields while RSTSTAT becomes SWRST bit5 (`0x20`).
WKUPSTAT is write-one-to-clear. External QMP reset restores this model\'s cold
defaults. CPU/peripheral reset executes normally; no host register/PC injection
stands in for reset. Sleep/wakeup and RSTCON pulse timing remain unsupported.

Primary definitions:

- [Samsung S3C2416X REV1.10, October2008](https://www.armdesigner.com/uploadfile/download/Samsung_ARM9_S3C2416X_datasheet.pdf), printed2-5,2-33,2-35: reset retention, SWRST, RSTSTAT/WKUPSTAT. PDF SHA256 `a45058dc61e1c90f226da236dcd1412b38dbbe3b02c4cd4271f92d469e2122fe`. The PDF was inspected in memory and is not distributed here.
- [Linux v5.15 exact S3C2416 clock driver](https://raw.githubusercontent.com/torvalds/linux/v5.15/drivers/clk/samsung/clk-s3c2443.c) and [PLL formulas](https://raw.githubusercontent.com/torvalds/linux/v5.15/drivers/clk/samsung/clk-pll.c).
- [S3C2416 device-tree clocks](https://raw.githubusercontent.com/torvalds/linux/v5.15/arch/arm/boot/dts/s3c2416.dtsi) and [UART clock selection](https://raw.githubusercontent.com/torvalds/linux/v5.15/include/linux/serial_s3c.h).

## Independent device and guest controls

`full_system_clock_model.py --case clock_model` uses host-side qtest with virtual
clock stepping. It verifies 12/6 MHz PCLK, active divisor changes, independent
PWM/UART gates, a 400 MHz configured MPLL divided to25 MHz PCLK, reserved clock
configurations, wrong reset key, software retention and a subsequent cold reset.
These observations are not ARM CPU execution.

`full_system_clock.py --case clock_reset` compiles self-authored `clock.S` and
executes it on hp-prime ARM926. Under QEMU `-icount shift=3,sleep=off`, each guest
instruction advances virtual time8ns. Identical6000-iteration SUBS/BNE loops
contain12000 instructions, independently specifying96 microseconds. Timer
observations are96 counts at1MHz,48 at500kHz and zero when gated. The guest
changes CLKDIV0 while counting, tests peripheral isolation and emits exact UART
`CSRYZ`; the gated `X` is absent. It writes SWRST, re-enters its ELF entry after
real reset, verifies retained INFORM0, default clock registers and stopped PWM,
then reaches its terminal breakpoint. GDB only observes entry, single-step, halt,
mailbox and registers. The reset loop cannot fall through into the success path.

Both tools require a NEW output directory, validate binary/container identities,
retain raw logs/results, and only remove their owned container. FullSystem
results stay `kernel_boot_verified=false`, `physical_hardware_verified=false`.

## Reproduction

Build the pinned source with ordered0001,0002,0003,0005 overlays using
`full_system_build.py`; add0004 when integrating NAND. Keep the scratch reference
read-only. The coordinator records final cumulative build identity separately.

```sh
python3 scripts/emulator/full_system_clock_model.py --build-root "$BUILD" --case clock_model --output "$NEW_MODEL_OUTPUT" --timeout 30
python3 scripts/emulator/full_system_clock.py --build-root "$BUILD" --case clock_reset --output "$NEW_GUEST_OUTPUT" --timeout 30
python3 -m unittest discover -s scripts/emulator -p 'test_full_system*.py' -v
```

## Preserved negative evidence

Initial outputs remain under `~/.cache/prime-m19-clock-*`: a missing DTC object
failed before building; the first C compilation exposed a missing log header;
an initial board wiring omission left the fixed66MHz input active and the
independent qtest correctly rejected5499 versus1000 expected counts; an initial
`-no-reboot` harness flag prevented the guest reset continuation. The wiring and
harness were corrected. The executed test was not weakened.

## Final standalone evidence

Fresh clean published-overlay build: `~/.cache/prime-m19-clock-published-final-build-02`.
QEMU2.10.1 source pin`32e891a9fc568de3110343c3e35b9ec0d23f6f44`, DTC pin
`558cd81bdd432769b59bff01240c44f82cfb1a9d`, isolated Xenial GCC5/Python2.7.
Executable SHA256`efec839347c9b9f1e313383ceac88f42a0b32f35ddb02afc9eeee6c78526fa27`.
Overlay SHA256`f182c05f3b2a9db350e0f8c60374744ec9c4168ed5211e5a877cac0217a6f202`.
Guest ELF SHA256`0ae029f43942daaa48fc7e68af6093e2bf03e0b64f71d46fb2b72b4d49c17d63`.

Three independent fresh processes pass each new case under this final build.
Raw evidence lives at `~/.cache/prime-m19-clock-published-final-clock_reset-{0,1,2}` and
`~/.cache/prime-m19-clock-published-final-clock_model-{0,1,2}`. Exact commands, runtime
identities, mailbox/register observations and log hashes are retained in
`evidence/m19-clocks.json`; reading that file is replay, not a new execution.
The original ARM/RAM smoke, timerIRQ, timerFIQ, UART/GPIO, MMU and unchanged
five-channel timer qtest also pass against this final build in
`~/.cache/prime-m19-clock-published-final-regression-*`.

All25 offline FullSystem Python tests pass (including5 new clock contracts).
No retained unsuccessful experiment was removed.

The old unselected SYSC register-storage source remains intact. The board now
selects the separately authored `s3c2416_clock.o` implementation. The published
overlay preserves the original Makefile\'s missing final newline explicitly.
The initial object-selection patch failed safely when its generated diff omitted
those EOF markers; its failed build and the coordinator\'s independent rejection
were retained. The corrected published overlay was rebuilt from the pin and all
new and existing hardware controls above were repeated successfully.
