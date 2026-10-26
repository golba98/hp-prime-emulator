# Program checking and diagnostic provenance

## Operations are distinct

| Operation | Backend / evidence | Physical Check or Run? |
|---|---|---|
| Native parser/compiler | Rust syntax, semantic and compatibility diagnostics | No |
| Original-firmware Check | Genuine firmware under the local emulator; observed UI Check, compiler entry, source identity and return | No |
| Native runtime | Rust VM execution | No |
| Local firmware runtime | Emulated calculator execution | No |
| Physical source readback | F8/F7 exact name/type, CRC and source comparison | No |
| Physical calculator Check | Capability unverified; refused | Unsupported |
| Physical calculator Run | Capability unverified; refused | Unsupported |

The physical G1 build 15157 and locally emulated original firmware are separate
profiles. A configured runner or successful local compile is never physical
acceptance. Existing EC/input and execution gates stay closed.

## Shared check service

`ppl-engine::checking` returns a versioned `CheckReport` with independent native
and optional firmware reports, source hash, firmware availability/status, and
`physical_check_executed: false`. Native rejection does not suppress an
independent firmware verdict. Original guest diagnostics retain their provenance,
message, raw position and mapped line/column. Native compatibility classification
is retained separately in diagnostic context.

```sh
ppl check examples/FIREWORKS.hpprgm --all-backends --json
ppl check example.hpprgm --engine native --json
ppl check example.hpprgm --engine firmware --json \
  --firmware PATH/armfir.elf --os-rom PATH/PRIME_OS.ROM --factory-a PATH/A
```

`--all-backends` preserves explicit single-engine interfaces. When configured
through `PPL_FIRMWARE`, `PPL_OS_ROM`, `PPL_FACTORY_A`, `PPL_FIRMWARE_RUNNER`, and
optional `PPL_FIRMWARE_INIT_DATA`, it invokes the managed original-firmware
backend. Missing/broken configuration produces an explicit backend failure;
it does not fall back to a native success. With no configuration it explicitly
reports `firmware_status: not_configured`, with no firmware verdict. This is a
native-only check, not an equivalent firmware result.

Replacement preparation uses this service before deletion authorization. Failed
configured firmware checks block preparation. Both verdicts remain reviewable
in the prepared replacement report. No source compaction or splitting occurs.

## IDE integration

Background diagnostics remain fast native checks. Explicit Check runs the shared
check service off the UI thread, including configured original firmware, even
when the live calculator session is selected. It uses the managed checker for
structured evidence rather than interpreting live-session console text as
compiler diagnostics. Run behavior is unchanged.

Firmware diagnostic items carry `guest-firmware` provenance into the Problems
panel and are bound to the originating tab and revision. Edited or closed tabs
discard stale results. Unknown locations remain unavailable rather than being
invented as line 1; navigation is suppressed without a known line. Native syntax,
semantic, compatibility and runtime diagnostics retain their classes. Backend
availability failures are reported explicitly in the console; they are not
presented as source-language errors. Physical upload/deletion status is separate
from compiler and execution status.

Tools → Physical calculator programs uses `ppl-programs` on a worker, either
through the shared owner socket or the exclusive HID connection. Source sizes,
blocked uploads, preparation, exact confirmation and operation evidence are
shown without duplicating USB framing in the GUI. New uploads require an absent
destination. Replacement and deletion each require a separately clicked,
content-bound authorization; pending confirmations cannot migrate to another
device selection. Physical Check and Run remain visibly unsupported.

## Validation boundaries

Focused offline tests cover valid source, exact error-line provenance, missing
firmware, JSON, strict decoding, differing native/firmware results, retained
source identities, stale GUI revisions, and pending confirmation behavior.
Mock runner results validate adapter contracts, not authentic compilation.

Asset-dependent original-firmware integration tests require explicit
`PRIME_TEST_FIRMWARE`, `PRIME_TEST_FACTORY_A`, and optionally `PRIME_TEST_OS_ROM` /
`PRIME_TEST_INIT_DATA`. A default workspace run may report success after these
tests return early; such cases must be counted as asset-skipped in acceptance
reports. Genuine separately configured runs must be reported separately.

Previously recorded cold-boot display timeouts
`m4a_framebuffer_is_written_by_the_firmware` and
`m4b_a_digit_key_draws_on_the_home_edit_line` are not fixed or suppressed by this
work. A warmed local firmware compiler result does not verify cold boot, remote
physical Check, physical execution, or the G1 large-object transfer path.

## Actual validation results

The requested formatting, workspace check, Clippy, all-target tests and whitespace
checks passed. The default workspace reported 1,380 passed and four ignored;
21 asset-skip messages were separately confirmed across `firmware_boot`,
`firmware_compiler`, `firmware_display`, `firmware_programs`, and `firmware_runtime`.
Those early returns must not be counted as exercised guest firmware behavior.
The 21-message count is a measured skip report, not a count of every conditional
path elsewhere in the workspace. Logs are under
`artifacts/program-management/validation-final-9097a6d7/` and
`artifacts/program-management/default-firmware-asset-skips.log`.

With explicitly configured genuine firmware and a preserved program-free local
initialization profile, the release test
`m6_hello_and_broken_have_authentic_independent_results` passed: HELLO was Valid
and BROKEN was Invalid. The new combined CLI service was also exercised through
the release helper: valid source produced native and firmware `success`; broken
source produced firmware `invalid_source`, provenance `guest-firmware`, message
`Error: Syntax Error`, line 3, column 8, raw UTF-16 offset 27. Both reports assert
`physical_check_executed: false`. Artifacts are retained under
`artifacts/program-management/firmware-acceptance-release/` and `firmware-cli/`.

The debug-profile authentic integration run timed out while settling key 2
(`TAP:26:229`) within the existing 20-second budget. This failure was not hidden
or converted to a native result; the release run was a separate measurement.
An earlier attempt with a non-program-free initialization profile was rejected
as InfrastructureError before compiler acceptance; that profile was not cleaned
or modified. These setup/debug limitations and the historical cold-boot display
timeouts remain separate from the passing release compiler result.

The full, unchanged FIREWORKS file was then checked through the same genuine
local release helper. Native checking returned `success`; original-firmware
checking returned `invalid_source`, `Error: Syntax Error`, **line 345, column 12**,
at the `BLIT_P()` call (raw UTF-16 offset 9,778). The overall command exited 1.
The retained report is `artifacts/program-management/firmware-cli/fireworks.json`.
This is an authentic disagreement between backends, not a physical G1 Check or
an instruction to rewrite the file. At that stage the source remained unchanged. Even if a
larger USB transport is established later, this local-firmware rejection must
be reviewed before a replacement is described as compilation-verified.

## FIREWORKS source repair and genuine recheck

Both executable no-argument `BLIT_P()` calls (original lines 345 and 352) were
invalid under the original firmware. The HP manual documents a required source
grob; it also warns that overlapping self-blits may be unpredictable. All
FIREWORKS drawing already targets the visible G0. The two invalid calls are
therefore replaced by explanatory comments, preserving drawing and WAIT timing,
rather than substituting a potentially unsafe self-blit.

The entire corrected file then passed both native and genuine original-firmware
checks with zero diagnostics and `physical_check_executed: false`. Evidence:
`artifacts/fireworks-repair/corrected-3019f4dc/report.json`. Original source is
retained in `artifacts/fireworks-repair/compiler-probes-04e2d19e/`. The native
regression in `ppl-engine/tests/fireworks.rs` compares 61 splash/animation frames
and final exit pixels against reconstructed original native behavior with
identical random seed and scripted input. This verifies native behavioral
preservation, not physical rendering or execution. The corrected source has
20,048 UTF-16 bytes; no compaction was applied.

Manual: https://h10032.www1.hp.com/ctg/Manual/c05332710.pdf
(BLIT_P, printed page 594).
