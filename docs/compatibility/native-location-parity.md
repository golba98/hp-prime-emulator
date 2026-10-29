# Native diagnostic location parity

M2 base: `20fa3dc3a22509fbfcb53e19e9d5913edb08a25f`. Implementation commit: `03df6e5ff978029cbc7dc5f54a8b5b678bbad2f3`. All firmware observations below come from original G1 firmware 2.4.2 running locally, not a physical calculator.

The original 75 sources and authentic firmware reference are unchanged. Verdict agreement remains **75/75**; rejected-program location agreement improves from **16/32 to 32/32**.

## Complete mismatch inventory

Spans are half-open UTF-8 byte ranges. Firmware offsets are zero-based UTF-16 code units; line and scalar column are one-based.

| Case | Native before | Native span before | Firmware / native after | Firmware UTF-16 offset | Native span after |
|---|---:|---:|---:|---:|---:|
| `e01_missing_end` | 2:1 | `[17,22)` | 4:1 | 34 | `[34,34)` |
| `e02_nested_missing_end` | 2:1 | `[19,24)` | 9:1 | 95 | `[95,95)` |
| `e06_blit_zero_args` | 3:2 | `[24,32)` | 3:9 | 31 | `[31,32)` |
| `e07_builtin_arity` | 3:9 | `[31,39)` | 3:14 | 36 | `[36,37)` |
| `e12_unclosed_paren` | 3:9 | `[31,32)` | 3:13 | 35 | `[35,36)` |
| `e13_unclosed_string` | 3:9 | `[29,34)` | 5:1 | 40 | `[40,40)` |
| `e14_error_in_second_function` | 9:5 | `[78,79)` | 9:8 | 81 | `[81,82)` |
| `e18_blit_zero_args` | 3:2 | `[25,31)` | 3:7 | 30 | `[30,31)` |
| `k06_lowercase_mod_div_operators` | 3:19 | `[39,40)` | 3:22 | 42 | `[42,45)` |
| `k07_lowercase_logical_operators` | 2:1 | `[15,20)` | 3:15 | 35 | `[35,36)` |
| `k22_var_upper_mod` | 3:8 | `[28,31)` | 4:12 | 47 | `[47,48)` |
| `k30_lowercase_end_assigned_at_statement_start` | 5:9 | `[57,60)` | 6:4 | 65 | `[65,66)` |
| `k32_lowercase_div_after_variable` | 3:10 | `[30,30)` | 3:11 | 31 | `[31,34)` |
| `k33_title_case_div_operator` | 3:10 | `[30,30)` | 3:11 | 31 | `[31,34)` |
| `k34_title_case_not_operator` | 2:1 | `[15,20)` | 3:9 | 29 | `[29,30)` |
| `k35_lowercase_end_variable_before_semicolon` | 4:9 | `[44,47)` | 5:4 | 52 | `[52,53)` |

The 16 previously matching rejected locations remain unchanged. No current mismatch was caused by UTF-16 conversion: every stored rejected location agrees with its raw UTF-16 offset.

## Sources and root causes

### e01_missing_end

Missing-block diagnostics used the opening BEGIN span instead of the EOF stopping point.

Correction is supported by the unchanged original reference and the adjacent local-firmware probes.

```ppl
EXPORT DPNOEND()
BEGIN
 RETURN 1;
```

### e02_nested_missing_end

Nested IF/FOR blocks consume the available END tokens; the remaining function closure fails at EOF, but native diagnostics pointed at BEGIN.

Correction is supported by the unchanged original reference and the adjacent local-firmware probes.

```ppl
EXPORT DPNESTEND()
BEGIN
 LOCAL i;
 FOR i FROM 1 TO 3 DO
  IF i>1 THEN
   PRINT(i);
 END;
END;
```

### e06_blit_zero_args

The semantic arity error covered the whole call; firmware stops at the closing parenthesis when required arguments are absent.

Correction is supported by the unchanged original reference and the adjacent local-firmware probes.

```ppl
EXPORT DPBLIT0()
BEGIN
 BLIT_P();
END;
```

### e07_builtin_arity

The semantic arity error covered the whole call; firmware stops at the comma introducing the first excess argument.

Correction is supported by the unchanged original reference and the adjacent local-firmware probes.

```ppl
EXPORT DPARITY()
BEGIN
 RETURN SIN(1,2);
END;
```

### e12_unclosed_paren

Missing-delimiter diagnostics used the opening parenthesis instead of the unexpected semicolon.

Correction is supported by the unchanged original reference and the adjacent local-firmware probes.

```ppl
EXPORT DPPAREN()
BEGIN
 RETURN (1+2;
END;
```

### e13_unclosed_string

Compiler lexing recovered at the opening line despite multiline strings being legal; firmware consumes the unterminated literal through EOF. Highlighting recovery is intentionally preserved.

Correction is supported by the unchanged original reference and the adjacent local-firmware probes.

```ppl
EXPORT DPSTR()
BEGIN
 RETURN "abc;
END;
```

### e14_error_in_second_function

An opener-anchored missing-delimiter error sorted ahead of the actual missing-expression error. Both now point at the unexpected semicolon, retaining the primary expression error.

Correction is supported by the unchanged original reference and the adjacent local-firmware probes.

```ppl
EXPORT DPFIRST()
BEGIN
 RETURN 1;
END;

EXPORT DPSECOND()
BEGIN
 LOCAL x;
 x:=(3+;
 RETURN x;
END;
```

### e18_blit_zero_args

Same insufficient-arity punctuation rule as BLIT_P.

Correction is supported by the unchanged original reference and the adjacent local-firmware probes.

```ppl
EXPORT DPBLITC0()
BEGIN
 BLIT();
END;
```

### k06_lowercase_mod_div_operators

The parser correctly rejects lowercase div as an infix operator, but its group-delimiter diagnostic pointed at the opening parenthesis.

Correction is supported by the unchanged original reference and the adjacent local-firmware probes.

```ppl
EXPORT DPK06()
BEGIN
 RETURN (7 mod 3)+(7 div 2);
END;
```

### k07_lowercase_logical_operators

Opener-anchored recovery errors and semantic analysis of an invalid AST obscured the expected-THEN error. Firmware performs syntax checking before name resolution, including when an unknown name occurs earlier in the source.

Correction is supported by the unchanged original reference and the adjacent local-firmware probes.

```ppl
EXPORT DPK07()
BEGIN
 IF 1 and not 0 or 0 xor 1 THEN RETURN 1; END;
 RETURN 0;
END;
```

### k22_var_upper_mod

Firmware permits uppercase MOD in LOCAL declarations but consumes it as an operator in expressions. Native rejected the declaration prematurely. Prefix MOD now diagnoses the next token where parsing stops.

Correction is supported by the unchanged original reference and the adjacent local-firmware probes.

```ppl
EXPORT DPK22()
BEGIN
 LOCAL MOD:=3;
 RETURN MOD+1;
END;
```

### k30_lowercase_end_assigned_at_statement_start

Firmware accepts empty RETURN before contextual end. Native imposed the value-RETURN semicolon rule on empty RETURN, then located the extra top-level END before rather than after consuming it.

Correction is supported by the unchanged original reference and the adjacent local-firmware probes.

```ppl
EXPORT DPK30()
BEGIN
 LOCAL end:=4;
 end:=end+1;
 RETURN end;
END;
```

### k32_lowercase_div_after_variable

Missing-semicolon diagnostics pointed at the preceding expression end, before whitespace, rather than the unexpected div token.

Correction is supported by the unchanged original reference and the adjacent local-firmware probes.

```ppl
EXPORT DPK32()
BEGIN
 RETURN 7 div 2;
END;
```

### k33_title_case_div_operator

Same stopping-token rule as lowercase div.

Correction is supported by the unchanged original reference and the adjacent local-firmware probes.

```ppl
EXPORT DPK33()
BEGIN
 RETURN 7 Div 2;
END;
```

### k34_title_case_not_operator

Same syntax-before-semantics and recovery-location issue as k07; Not remains a contextual name rather than becoming uppercase NOT.

Correction is supported by the unchanged original reference and the adjacent local-firmware probes.

```ppl
EXPORT DPK34()
BEGIN
 IF Not 0 THEN RETURN 1; END;
 RETURN 0;
END;
```

### k35_lowercase_end_variable_before_semicolon

Same empty-RETURN and unclaimed top-level END behavior as k30.

Correction is supported by the unchanged original reference and the adjacent local-firmware probes.

```ppl
EXPORT DPK35()
BEGIN
 LOCAL end:=4;
 RETURN end;
END;
```

## Implementation and evidence

- Block, delimiter and inter-statement diagnostics use the actual stopping token; open-block hints retain opener information.
- `Program.punctuation` retains compact open/close/comma spans while releasing lexer tokens. Incorrect builtin arities binary-search to their call and inspect its punctuation, respecting nested delimiters. Valid calls need no lookup or per-call allocation. Expression variants, executable bytecode spans and compiler entrypoint signatures stay unchanged. External Program struct literals must now supply the punctuation vector (empty preserves fallback locations).
- Builtin arity diagnostics use those exact token punctuation spans. Existing arity rules and user-function diagnostics are unchanged.
- `compile_source` returns syntax errors before semantic analysis. Probe `c06_earlier_unknown` demonstrates that firmware likewise prioritizes a later syntax error over an earlier undefined variable. Well-formed sources still receive normal semantic analysis.
- Compiler lexing lets unterminated multiline strings reach EOF. `lex_with_trivia` retains opening-line recovery for editor highlighting.
- LOCAL permits uppercase MOD declarations; uppercase MOD remains an operator in expressions. Empty RETURN may precede END without a semicolon; value RETURN keeps its semicolon requirement.
- No firmware dependency, source rewriting, case-specific offset, or additional whole-source pass was added to native analysis.

Thirty-four adjacent probes live in `tests/diagnostics/location-probes`; two controls in `tests/diagnostics/location-arity-controls` cover the pre-existing PRINT and INPUT arity assertions. Each has an independently captured authentic firmware reference. Proprietary runtime artifacts remain under ignored `artifacts/location-parity/`.

## Validation

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo test --workspace`: **1,444 passed, 0 failed, 4 ignored**.
- `ppl diag-parity report --check-baseline`: passed, **75/75 verdicts and 32/32 rejected locations**.
- Both adjacent reference re-capture diffs: passed, **36 cases and zero differences**. Across two captures, 72 authentic local firmware checks ran; none of these required probes was skipped.
- Additional offline probe parity: **36/36 verdicts and 26/26 rejected locations**. Combined with the unchanged original corpus: **111/111 verdicts, 58/58 rejected locations**.
- All original corpus sources and authentic reference snapshots are unchanged from M2 (`git diff --exit-code 20fa3dc -- tests/diagnostics/cases tests/diagnostics/reference`).

The four explicitly ignored tests cover Xvfb GUI smoke, analysis latency,
a private preserved hardware screenshot, and capture/encoding measurement.
Default workspace execution also leaves the older opt-in firmware suites
unconfigured (`PRIME_TEST_FIRMWARE` is unset); those tests return early.
Firmware assets were available and explicitly configured through `PPL_*`
for all new diagnostic probes. No physical device was accessed.

Twenty-one focused test functions were added: eighteen compiler tests, two
syntax tracking tests and one independent-reference parity test. The compiler
tests preserve every corrected source, verdict, exact byte span and line/column,
then exercise a Unicode source edit and source decoding. Existing accepted
programs and the sixteen originally matching rejected locations stay covered.
Three GUI native-diagnostic expectations changed solely to follow EOF locations
and syntax-before-semantics behavior; no GUI production behavior changed.

### Performance

Optimized M2 and updated libraries were compared with the same synthetic
workloads on the same machine. Four alternating before/after process pairs
were measured; each result below is the median of four medians, each based
on nine 150 ms batches. Both source and raw measurements remain in ignored
`artifacts/location-parity/bench.rs` and `performance-paired.json`.

| Source lines | Parse M2 / updated | Change | Full analysis M2 / updated | Change |
|---:|---:|---:|---:|---:|
| 520 | 115.28 / 116.46 µs | +1.03% | 221.12 / 225.03 µs | +1.77% |
| 2,600 | 818.45 / 843.56 µs | +3.07% | 1151.47 / 1156.64 µs | +0.45% |
| 10,400 | 2261.28 / 2245.80 µs | -0.68% | 3568.09 / 3566.61 µs | -0.04% |

Full-analysis changes range from -0.04% to +1.77%; the largest workload remains about 3.57 ms. Parser changes range from -0.68% to +3.07%. Compact punctuation is collected as existing lexer tokens are released; no text re-lexing, firmware dependency or per-call allocation is added.

The existing Criterion parser, compile and editor-diagnostics benchmarks also
completed successfully (10 samples, 0.5 s warm-up, 1 s measurement). Current
editor analysis estimates are 645.61 µs for 1,252 lines, 3.9163 ms for 6,220
lines and 18.894 ms for 24,772 lines. Frontend synthetic-100KB estimates are
2.3330 ms parsing and 2.4937 ms full compilation. Logs are retained under
`artifacts/location-parity/criterion-{frontend,editor}.log`.

No diagnostic disagreements remain in the original or adjacent measured corpora.

## Integration

Production changes are confined to ppl-syntax and ppl-compiler. Tests additionally tighten the compatibility ratchet from 59 to 75 total matches and update only native baseline outcomes. Existing corpus sources and shared authentic firmware snapshots are untouched.

M3 integration should require no firmware, session or GUI conflict resolution. Coordinate the native baseline and ratchet test if M3 also edits those files. Three GUI native-diagnostic test expectations also changed in app.rs/app_compile_tests.rs; those edits are isolated to tests and should be applied carefully if M3 touches the same sections. This branch does not contain or depend on M3 working-tree changes.
