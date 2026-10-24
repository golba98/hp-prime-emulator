> Historical engineering record. Counts, commands and status below describe
> the recorded session, not a current guarantee. See the root README and latest
> milestone reports for supported interfaces and validation.

# Compatibility report: `g1_smoke`

Hardware evidence: program `ZZT_998C5E` on HP Prime G1 (firmware build 8151 or later) (firmware build 15157), recorded 2026-10-22T12:12:45Z.

Summary: VERIFIED: 5

| case | source | emulator | G1 | status |
|---|---|---|---|---|
| `smoke_add` | `1+2` | 3 (type 0) | 3 (type 0) | VERIFIED |
| `smoke_mul` | `6*7` | 42 (type 0) | 42 (type 0) | VERIFIED |
| `smoke_third` | `1/3` | 0.333333333333 (type 0) | 0.333333333333 (type 0) | VERIFIED |
| `smoke_round` | `ROUND(2.5,0)` | 3 (type 0) | 3 (type 0) | VERIFIED |
| `smoke_strcat` | `"AB"+"C"` | ABC (type 2) | ABC (type 2) | VERIFIED |
