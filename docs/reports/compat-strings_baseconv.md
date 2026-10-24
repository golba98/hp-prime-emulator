> Historical engineering record. Counts, commands and status below describe
> the recorded session, not a current guarantee. See the root README and latest
> milestone reports for supported interfaces and validation.

# Compatibility report: `strings_baseconv`

No hardware evidence recorded: every case is UNVERIFIED.

Summary: UNVERIFIED: 20

| case | source | emulator | G1 | status |
|---|---|---|---|---|
| `bc_mid_past_end` | `"\|"+MID("ABC",4)+"\|"` | "\|\|" (type 2) | — | UNVERIFIED |
| `bc_mid_len_past_end` | `MID("ABC",2,9)` | "BC" (type 2) | — | UNVERIFIED |
| `bc_left_zero` | `"\|"+LEFT("A",0)+"\|"` | "\|\|" (type 2) | — | UNVERIFIED |
| `bc_left_past_end` | `LEFT("AB",9)` | "AB" (type 2) | — | UNVERIFIED |
| `bc_instring_miss` | `INSTRING("0123456789ABCDEF","G")` | 0 (type 0) | — | UNVERIFIED |
| `bc_instring_hit` | `INSTRING("0123456789ABCDEF","A")` | 11 (type 0) | — | UNVERIFIED |
| `bc_instring_empty_needle` | `INSTRING("abc","")` | 0 (type 0) | — | UNVERIFIED |
| `bc_dim_empty` | `DIM("")` | 0 (type 0) | — | UNVERIFIED |
| `bc_upper_mixed` | `UPPER("7b.8")` | "7B.8" (type 2) | — | UNVERIFIED |
| `bc_upper_non_ascii` | `UPPER("äb")` | "ÄB" (type 2) | — | UNVERIFIED |
| `bc_dim_unicode` | `DIM("₂é")` | 2 (type 0) | — | UNVERIFIED |
| `bc_string_real` | `STRING(10.5)` | "10.5" (type 2) | — | UNVERIFIED |
| `bc_string_large` | `STRING(1.84467440738E19)` | "1.84467440738E19" (type 2) | — | UNVERIFIED |
| `bc_mod_16` | `255 MOD 16` | 15 (type 0) | — | UNVERIFIED |
| `bc_ip_div` | `IP(255/16)` | 15 (type 0) | — | UNVERIFIED |
| `bc_ip_negative` | `IP(-2.5)` | -2 (type 0) | — | UNVERIFIED |
| `bc_round_13_digits` | `9999999999999` | 1E13 (type 0) | — | UNVERIFIED |
| `bc_large_mod` | `1.84467440738E19 MOD 10` | 0 (type 0) | — | UNVERIFIED |
| `bc_large_ip_div` | `IP(1.84467440738E19/16)` | 1.15292150461E18 (type 0) | — | UNVERIFIED |
| `bc_frac_times_base` | `IP(0.1*2*2*2*2)` | 1 (type 0) | — | UNVERIFIED |
