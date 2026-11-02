# Unified PPL differential testing

Implemented foundation, schema 1. This extends `ppl-compat`; existing `ppl compare`
suites, hardware evidence, interpretation history and the 77 physical references
are unchanged. It does not add hardware access or a full-system emulator.

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

A fixture has `schema_version = 1`, an id, exactly one `[program] source` or
`file`, ordered `[[steps]]`, optional `[initial]`, `[[expect]]` and backend
constraints. File paths resolve against the fixture directory. Original fixture
bytes and decoded program source receive separate SHA-256 identities.

Supported step names are `check`, `run`, `observe`, `capture`, `key`, `touch`,
`power_off` and `restart`. Their capabilities depend on the actual adapter:

| Adapter | Implemented behavior | Explicit limits |
|---|---|---|
| Native | Check, retained-state Run, selected variable/settings observations, framebuffer capture | Physical key/touch and OFF/restart scenarios are refused before execution |
| Firmware HLE | Check and one Run with typed engine Report and optional capture | Multiple Runs, initial setup/settings, variables, physical input and power steps require a future structured live-session adapter |
| Physical capture | Qualified immutable legacy TYPE/STRING replay | No live operations; no inferred graphics, variables or persistent state |
| Full system | Unavailable result with explanation | No CPU/kernel/hardware execution is implemented |

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
Partial/transcription classifications without changing that evidence. Physical
capture adaptation of new scenario observations remains future work.

## Tests and next integration

Regression tests cover retained Native globals, setup/settings, scalar/list type
and order distinctions, exact error differences, source binding, missing or
duplicate observations, unknown variables, unavailable/unsupported backends,
actual drawing coordinates and image encoding/dimensions. All committed
scenarios execute their declared Native expectations. Existing compatibility
and physical-evidence tests remain part of the required regression suite.

Next: adapt structured live-session runtime observations (not human status
strings), then support genuine HLE retained state, key/touch and OFF/restart
steps. Existing protocol/result capabilities must be used and version checked.
Extend the Native host's scripted event seam only for semantics it implements.
Add new physical scenario captures through the existing guarded manual workflow.
A future full-system runner can implement `Backend` without changing fixtures
or relaxing provenance and comparison rules.

## First actual firmware comparison

A real local-firmware smoke of the rational-difference fixture found Native's
12-digit decimal `0.333333333333` versus firmware's exact rational `1/3`.
The structured report identifies `/return_value/value/0`; it keeps the rational
numerator/denominator and does not equate it to a decimal merely because a
calculator display can round them similarly. This is a known representation /
compatibility distinction, not a failed harness test or physical verification.
The common arithmetic fixture uses exact integer-valued arithmetic so it can
also serve as a positive cross-backend control.
