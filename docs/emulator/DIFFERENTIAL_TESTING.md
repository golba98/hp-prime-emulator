# Unified PPL differential testing

Implemented schema 1 and stateful schema 2. This extends `ppl-compat`; existing `ppl compare`
suites, hardware evidence, interpretation history and the 77 physical references
are unchanged. Live HLE execution uses isolated profiles; physical evidence collection remains separately guarded. No full-system emulator is implemented.

## Running scenarios

```sh
ppl differential tests/differential/arithmetic.toml --backend native
ppl differential tests/differential/arithmetic.toml --backend native,firmware-hle --output comparison.json
ppl differential tests/differential/state.toml --backend native
ppl differential tests/differential/graphics.toml --backend native,firmware-hle
```

Firmware uses the existing user-supplied asset resolution (flags, PPL variables,
then configuration); `--firmware`, `--os-rom`, `--factory-a`, `--init-data` and
`--firmware-runner` may be specified. Execution uses disposable profiles created
by the existing engine, never an existing calculator save or a USB device.

The output is structured JSON. Exit 0 means all required backends completed and
all comparisons passed; 1 means disagreement, unavailable required backend,
unsupported capability or missing observations; 2 means invalid input or report
I/O failure. No comparisons is a failure. Optional unavailable backends remain
explicit in the report and cannot produce an otherwise empty passing run.

## Fixture contract

Schema 1 retains its single `[program]` source/file contract and original hashes.
Schema 2 adds `[programs.ZZT_NAME]` source/file entries and the actions `import`,
`run_program`, `probe`, `edit`, `start` and `await`. Existing key/touch,
capture and power actions remain available according to backend capability.
Initial settings/setup execute through PPL in HLE; unsupported RNG seeding is
refused. Fixtures under `tests/differential/m16` are executable examples.

Schema 2 records a sorted source manifest for every initial and edited revision.
Its `source_sha256` is the manifest digest, and provenance additionally exposes
individual source hashes. Changing a second program or an edit invalidates
identity even when the default program is empty.

| Adapter | Implemented behavior | Explicit limits |
|---|---|---|
| Native | Retained named modules, sequential execution, authored probes, edits, settings, framebuffer | Cross-program linkage, asynchronous physical input and firmware power semantics unsupported |
| Firmware HLE | Genuine retained named programs; source-bound execution; Start/Await/key/touch; program-output graphics; OFF/fresh restart | Known observer profile required; numeric arguments; arbitrary in-place Check refused |
| Physical capture | Qualified historical replay and sealed stateful capture bundles | New live USB collector unimplemented; manual input confirmations and retained artifacts required |
| Full system | Explicit Unavailable result | No CPU/kernel/SoC execution backend |

Named HLE loading assigns source directly to its catalog program through firmware.
RunStored never silently reimports or invokes Check. Fresh-process dispatch
rebinds by checking the actual firmware context's source before body entry.
That execution proof is distinct from a compiler verdict.

Start acknowledges real body entry. Key/touch acknowledgments explicitly mean
`delivery_level=queued`; terminal fixture results must prove semantic consumption.
Await correlates the terminal result. Cleanup preserves pending releases and
waits for consumed input and idle before another invocation. Program-output
captures refer to the return frame, not an arbitrary later Home screen.

Authored probes return a typed list with one element per declared variable name.
They are executions, not pure memory inspections. Matrix scenarios currently
compare explicit dimensions/cell projections; undecoded direct matrix/complex/
integer returns remain unobserved.

Native Run reuses the existing VM `SessionContext`, preserving module globals,
calculator globals, settings and GROB state by exact source identity. It does
not introduce a second VM or interpreter. Initial HAngle/HFormat/HDigits are
validated; an optional zero-argument `setup_entry` executes authored PPL before
the steps. Other initial effects belong in that setup routine, not host-memory
injection. Native observations distinguish unknown variables from actual zero.

Arguments use the existing tagged JSON values, for example
`args = [{type="real",value="1.23456789012"}]`. Decimal strings preserve precision.
Expected results use tagged values too: a real is not a string, integer width
and matrix dimensions remain meaningful, and list ordering is exact.

`[[expect]]` selects a step id and JSON pointer into that observation:

```toml
[[expect]]
step = "run"
path = "/return_value"
value = { type="real", value="42" }
```

Backend constraints may name a required firmware build or ELF hash. Native
cannot satisfy firmware identity constraints. The current one-shot HLE report
identifies the ELF hash but has no verified build-number field: build-number
constraints are explicitly unsupported. `firmware_sha256` constraints validate
the configured ELF before execution. Never infer a firmware version from its
filename. `optional=true` permits an unavailable backend without counting it as
executed or matched.

## Comparison and provenance

The `Backend` trait is the extension seam. Each adapter returns observations,
source/fixture identity, evidence origin, firmware hash when available and raw
engine reports. The coordinator checks identity and unique step observations,
evaluates expectations, and compares semantic observations between completed
backends. Mismatches identify backend, reference backend, step, JSON pointer,
expected value, actual value and reason. Timing and backend metadata are not
semantic observations.

There is no generic error-equals-error rule. Different error text/kinds,
missing fields, undeclared observation gaps and changed source hashes fail.
Expected runtime errors must be explicit status expectations. A successful
firmware execution with an undecoded result remains `unobserved`; the adapter
retains its raw representation and does not parse the display message.

Images default to exact RGB pixels, with dimensions and pixel counts checked.
The library's `compare_images` can explicitly compare RGB565 display encoding;
this is not a visual similarity tolerance. It reports changed count and first
coordinate. Pairwise image discrepancies produce a bounded summary rather than
one mismatch per pixel. There are no implicit clock/status-bar masks or text
normalizations. Per-fixture image masks and reference-PNG expectations are not
implemented yet.

A backend match is not labelled physical `VERIFIED`. Local firmware and Native
results retain their own origin. This framework does not promote HLE execution
to full-system emulation or infer physical coverage from passing mocks.

## Qualified physical replay

```sh
ppl differential \
  --replay-suite tests/compat/cases/core.toml \
  --replay-evidence tests/compat/evidence/core/ZZT_NAME.json \
  --replay-manifest tests/compat/runs/ZZT_NAME.json
```

Use actual existing capture names; the placeholder above is illustrative.
Replay reuses `assessment::qualify`: source and generator identity, manifest,
G1 build 15157 identity, raw artifact hash, transcript and captured settings must
all agree. The validated capture type has private construction fields. A locally
produced report or an arbitrary `physical` label cannot enter this path.

Interpretation revisions and source observations are applied through the existing
append-only provenance registry. Disputed interpretations fail rather than being
counted as matching; original observations remain visible alongside corrections.

The report's scope is **legacy TYPE/STRING only**. IFERR recordings establish
only an error category, so replay does not claim error-detail parity for them.
Values compare exactly. It does not use legacy display normalization or modify
the historical classifications. This stricter result can differ from existing
Partial/transcription classifications without changing that evidence. New scenario captures use the separately sealed bundle workflow below; live USB collection remains unimplemented.

## Tests and next integration

Regression tests cover retained Native globals, setup/settings, scalar/list type
and order distinctions, exact error differences, source binding, missing or
duplicate observations, unknown variables, unavailable/unsupported backends,
actual drawing coordinates and image encoding/dimensions. All committed
scenarios execute their declared Native expectations. Existing compatibility
and physical-evidence tests remain part of the required regression suite.

M16 adds actual-firmware scenarios for alternating modules and edits,
cross-program calls, CAS, errors followed by successful execution, graphics,
OFF/restart persistence and sequential key/touch waits. Their fixture durations
include boot/import/shutdown and are not launch-latency measurements.
See `M16_STATEFUL_EXECUTION.md` and the integrated M16 report.

## New physical scenario bundles

`ppl capture prepare/seal/validate/compare` works offline and never opens USB.
Bundles freeze all source revisions, planned actions, confirmed delivery,
observations and checksummed raw artifacts. Manual physical attestations require
matching retained raw identity replies; controlled mocks cannot become physical
verification. Manual attestation is not cryptographic proof of authenticity.
See `PHYSICAL_SCENARIO_CAPTURE.md` for commands and trust limits.
Legacy TYPE/STRING replay retains its narrower independent provenance.

## First actual firmware comparison

A real local-firmware smoke of the rational-difference fixture found Native's
12-digit decimal `0.333333333333` versus firmware's exact rational `1/3`.
The structured report identifies `/return_value/value/0`; it keeps the rational
numerator/denominator and does not equate it to a decimal merely because a
calculator display can round them similarly. This is a known representation /
compatibility distinction, not a failed harness test or physical verification.
The common arithmetic fixture uses exact integer-valued arithmetic so it can
also serve as a positive cross-backend control.

## M16 live stateful adapter

Schema 2 and the genuine named-program HLE session adapter are implemented. See
[M16 stateful execution](M16_STATEFUL_EXECUTION.md) for source-manifest identity,
authored probes, asynchronous input, restart semantics, actual firmware fixtures
and explicit unsupported capabilities. Schema 1 and historical physical replay
retain their original scope.
