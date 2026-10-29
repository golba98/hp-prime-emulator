# Native location probes

These 34 sources extend the original 75-case corpus without changing it.
The original G1 2.4.2 firmware runs only in local emulator profiles. Results
are authentic firmware observations, not physical-calculator verification.

- `s01`–`s04`: unterminated/closed strings, EOF and Unicode.
- `c01`–`c09`: malformed conditions, contextual NOT and syntax before semantics.
- `m01`–`m06`: uppercase MOD declarations and operator interpretation.
- `r01`–`r06`: empty RETURN, contextual END and unclaimed top-level END.
- `a01`–`a04`: insufficient/excess builtin arguments, nesting and whitespace.
- `d01`–`d05`: missing delimiters, nested blocks and stopping-token positions.

Each source was checked twice using the existing fresh firmware checker,
with separate emulator profiles. Both captures agreed on verdict, status,
message, line, column and raw UTF-16 offset: 34 cases, zero differences.
The two existing-test arity controls in `../location-arity-controls` were
also captured twice and agreed exactly. Raw reports remain under ignored
`artifacts/location-parity/`; firmware assets and runtime state are not bundled.

With the firmware environment configured as described in
`docs/compatibility/diagnostic-parity.md`, repeat into a new file:

```sh
ppl diag-parity capture --corpus tests/diagnostics/location-probes \
  --output artifacts/location-parity/new-reference.toml \
  --artifacts-dir artifacts/location-parity/new-capture
ppl diag-parity diff tests/diagnostics/location-probes/reference.toml \
  artifacts/location-parity/new-reference.toml
cargo test -p ppl-compat --test diagnostic_parity
```

The compatibility test checks source hashes, firmware provenance, verdicts
and every rejected location. Focused syntax/compiler tests additionally lock
original byte spans and their behavior after Unicode edits and source decoding.
See `docs/compatibility/native-location-parity.md` for the full mismatch inventory.
