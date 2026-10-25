# Agent-driven physical debugging

## What works now

The CLI can preserve physical source, enumerate a verified partial inventory, capture real screenshots, check local source with original firmware, and journal repeated agent-driven local attempts. It cannot yet navigate, initiate physical Check/Run, dismiss a physical dialog, or confirm physical execution. Those interfaces return `unsupported_capability` without mutation.

```sh
ppl device diagnose examples/FIREWORKS.hpprgm --json
ppl device diagnose examples/FIREWORKS.hpprgm --engine firmware --json
ppl device diagnose examples/FIREWORKS.hpprgm --engine physical --json
ppl device run-file examples/FIREWORKS.hpprgm --json
ppl device debug examples/FIREWORKS.hpprgm --json
```

`diagnose` defaults to **local original firmware compilation**, without USB. It calls the same managed `ppl-engine` M6/M8 backend used by `ppl check --engine firmware`, preserving compiler return evidence, actual diagnostics, firmware/source identities, transformations, locations, cancellation, and retained artifacts. There is no native parser fallback. A native parser disagreement is not a physical or original-firmware verdict.

Configure the existing local assets and matching helper through flags or environment:

```sh
export PPL_FIRMWARE="$PWD/scratch/research/firmware/extracted/fat/armfir.elf"
export PPL_OS_ROM="$PWD/scratch/research/firmware/extracted/PRIME_OS.ROM"
export PPL_FACTORY_A="$PWD/scratch/research/m7-private/factory/A"
export PPL_FIRMWARE_INIT_DATA="$PWD/scratch/research/m7-private/initial-ready/data"
export PPL_FIRMWARE_RUNNER="$PWD/../.cache/prime-emulator-target/release/prime-emulator"
ppl device diagnose examples/FIREWORKS.hpprgm --engine firmware --json
```

Equivalent flags are `--firmware`, `--os-rom`, `--factory-a`, `--init-data`, and `--firmware-runner`; `--timeout-secs` defaults to 360. `--artifacts-dir` retains backend evidence. Missing assets, helper, or infrastructure are explicit failures rather than syntax errors. The nested `firmware_report` retains the original backend's result contract. The outer engine is `firmware_emulator` and physical `check_executed` and `execution_confirmed` remain false.

The local pinned firmware is G1 2.4.2, not the physical calculator's build 15157. Passing it does not prove physical acceptance. Unicode/UTF-16 positions and any LF normalization remain described by the backend's source identity and raw positions. Physical screenshot lines are not mapped to an unrelated local revision.

## One attempt, then agent edits

```sh
ppl device debug ./ZZT_TRIAL.hpprgm --max-attempts 5 --json
# Inspect the returned report and session_id; the agent edits the local file.
ppl device debug ./ZZT_TRIAL.hpprgm --resume debug-ZZT_RETURNED_ID --json
```

The example ID is a placeholder; use the exact returned ID. Each invocation takes one immutable snapshot, syncs a journal before checking, and creates an attempt-specific report. The agent does the reasoning and editing. The CLI does not watch files, modify source, generate renamed exports, or retry in the background.

Default budget: five attempts; explicit bounds: 1–100. A resumed session cannot change its budget or device selector. Unchanged source after a terminal attempt is refused. Concurrent access to one debug session is locked. An unresolved future physical submission blocks further attempts; interruption during a local-only check does not imply a calculator invocation. Previous source revisions and reports are retained.

A genuine local syntax error reports `syntax_error`, exit 1, and `agent_action: edit_local_source_then_resume`. A valid local check currently stops at the physical capability gate: `unsupported_capability`, exit 6, with the successful local report under `firmware_check`. It does not upload a program. Infrastructure failures consume a recorded attempt so evidence is not lost; inspect and repair configuration before starting another session.

## Safe physical staging requirements

`run-file` and physical `diagnose` presently decode source and return the unsupported backend result with source identity and staging issues, without opening USB or invoking local execution. Successful physical execution must eventually require all of:

1. A verified input/execution backend for the observed hardware and firmware.
2. Genuine local firmware Check before deployment, clearly distinguished from physical acceptance.
3. Agent-prepared source with a fresh ZZT_ export/catalog name and local helpers, preserving identifiers and original revisions.
4. Existing guarded absence checks, size checks, creation, and exact source read-back on the same owned connection.
5. Screenshot-verified exact selection and invocation; independent evidence that execution occurred.
6. A known terminal result or an explicit uncertain outcome that prevents another invocation.

None of these gates may be bypassed by a mock success or a configuration flag. Ordinary installed programs are never overwritten by this workflow. No automatic cleanup or deletion is added. Existing user-authorized send/delete operations retain their safeguards and permissions.

The current local FIREWORKS example is 19,876 UTF-16 bytes, above the existing 15,928-byte G1 upload ceiling. The separately read physical FIREWORKS was 14,762 bytes. The agent must deliberately prepare an acceptable revision; the CLI does not silently strip comments, shorten source, rename functions, or equate these sources.

## Physical error evidence

`ppl device ui inspect --json` preserves two full-resolution screenshots. When available, conservative local OCR may describe the visible syntax dialog with a line number and `source: physical_screen_ocr`. It does not prove that this command invoked Check or that the source equals a local file. With no OCR, malformed screenshots, uncertain text, or an unsupported layout, the command returns uncertainty and preserves original evidence.

Runtime errors, program completion, current running state, safe Esc recovery, Home, and editor navigation lack verified physical automation. Their absence from an observation is not success. A timeout never becomes `execution_completed`. `execution_unknown` remains distinct from successful completion; no physical operation is currently initiated that could legitimately produce a completion claim.

Read-only hardware evidence, journals, screenshots, and tests live under ignored `artifacts/device-debug/`. Keep them private. Raw firmware, guest drive copies, and arbitrary device backups must not be committed or published. See [control](autonomous-control.md), [inventory](program-inventory.md), and [remote-input investigation](../reverse-engineering/remote-input-investigation.md).

## Implementation-pass validation

All four project-standard commands passed: formatting, all-target workspace check, Clippy with warnings denied, and all-target workspace tests (1,245 passed, 0 failed, 4 existing ignored). The executable-script fixtures in `ppl-engine` are serialized under tests to avoid a reproduced Linux ETXTBSY race between parallel fixture creation and process spawning; production supervision and assertions are unchanged.

Genuine asset-enabled checks were run separately from asset-unset workspace tests:

- The 23-case compiler matrix passed in isolation. The first broader run had one Check-tap settling timeout; its failed evidence was retained, not relabeled as success.
- All five original-firmware CLI integration tests passed, covering checks, runtime results/errors, graphics, entry selection, Unicode containers, limits, cancellation, and independent runs.
- The new device-diagnosis integration test passed both genuine valid-source and syntax-error cases, with `engine: firmware_emulator` and no physical execution claim.
- The cold-boot display suite had two passing tests and two boot-stability timeouts (`m4a_framebuffer_is_written_by_the_firmware`, `m4b_a_digit_key_draws_on_the_home_edit_line`). Those tests and production emulation behavior were not changed to force acceptance. This remains a local firmware-emulator validation limitation.
- The fixture-only Xvfb mirror smoke test passed: zero displayed/resized pixel mismatches, all four scales, pause/resume, snapshot, and window closing. Its input targeted the isolated X server, never the calculator.

Read-only physical acceptance confirmed service identity, exclusive USB locking (a competing command returned busy), shared-connection source export with the independent matching hash, screenshot capture, and clean service shutdown/socket removal. Inventory returned the verified partial dataset described in [inventory](program-inventory.md). No new physical key experiment, upload, deletion, Check invocation, or program execution was performed.

Local evidence indexes: `artifacts/device-debug/final-ac723be10c9b/summary.json`, `artifacts/device-debug/firmware-isolated-4bd1b4d02642/summary.json`, `artifacts/device-debug/service-read-only-623d1735e0b8/summary.json`, and `artifacts/device-debug/gui-xvfb-control-regression/results.json`. These ignored artifacts can contain private code or proprietary firmware work directories and are not publication bundles.
