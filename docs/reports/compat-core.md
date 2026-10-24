> Historical engineering record. Counts, commands and status below describe
> the recorded session, not a current guarantee. See the root README and latest
> milestone reports for supported interfaces and validation.

# Compatibility report: `core`

Hardware evidence: program `ZZT_1A9784, ZZT_3115CB, ZZT_352D68, ZZT_7BB42D, ZZT_82E3D2, ZZT_CB8188, ZZT_D45C33` on HP Prime G1 (firmware build 8151 or later) (firmware build 15157), recorded 2026-10-22T12:35:34Z.

Summary: FAILED: 1, UNVERIFIED: 24, VERIFIED: 47

| case | source | emulator | G1 | status |
|---|---|---|---|---|
| `num_div_one_third` | `1/3` | 0.333333333333 (type 0) | 0.333333333333 (type 0) | VERIFIED |
| `num_one_third_mul_three` | `1/3*3` | 0.999999999999 (type 0) | 0.999999999999 (type 0) | VERIFIED |
| `num_sqrt_two_pow` | `2^0.5` | 1.41421356237 (type 0) | 1.41421356237 (type 0) | VERIFIED |
| `num_sqrt_two_builtin` | `SQRT(2)` | 1.41421356237 (type 0) | 1.41421356237 (type 0) | VERIFIED |
| `num_exp_small` | `1E-5` | 0.00001 (type 0) | 0.00001 (type 0) | VERIFIED |
| `num_exp_max` | `1E499` | 1E499 (type 0) | 1E499 (type 0) | VERIFIED |
| `num_exp_min` | `1E-499` | 1E-499 (type 0) | 1E-499 (type 0) | VERIFIED |
| `num_exp_overflow` | `1E499*10` | error: Error: Overflow | — | UNVERIFIED |
| `num_round_half_pos` | `ROUND(2.5, 0)` | 3 (type 0) | 3 (type 0) | VERIFIED |
| `num_round_half_neg` | `ROUND(-2.5, 0)` | -3 (type 0) | -3 (type 0) | VERIFIED |
| `num_round_half_0_5` | `ROUND(0.5, 0)` | 1 (type 0) | 1 (type 0) | VERIFIED |
| `num_round_half_3_5` | `ROUND(3.5, 0)` | 4 (type 0) | 4 (type 0) | VERIFIED |
| `num_round_1_25` | `ROUND(1.25, 1)` | 1.3 (type 0) | 1.3 (type 0) | VERIFIED |
| `num_round_1_35` | `ROUND(1.35, 1)` | 1.4 (type 0) | 1.4 (type 0) | VERIFIED |
| `num_round_1_15` | `ROUND(1.15, 1)` | 1.2 (type 0) | 1.2 (type 0) | VERIFIED |
| `num_round_places` | `ROUND(3.14159, 2)` | 3.14 (type 0) | 3.14 (type 0) | VERIFIED |
| `num_mod_neg_pos` | `-7 MOD 3` | 2 (type 0) | 2 (type 0) | VERIFIED |
| `num_mod_pos_neg` | `7 MOD -3` | -2 (type 0) | -2 (type 0) | VERIFIED |
| `num_div_neg_pos` | `-7 DIV 3` | -3 (type 0) | — | UNVERIFIED |
| `num_div_pos_neg` | `7 DIV -3` | -3 (type 0) | — | UNVERIFIED |
| `num_12_digits_exact` | `1.23456789012` | 1.23456789012 (type 0) | 1.23456789012 (type 0) | VERIFIED |
| `num_13_digits_round_down` | `1.234567890124` | 1.23456789012 (type 0) | 1.23456789012 (type 0) | VERIFIED |
| `num_13_digits_round_up` | `1.234567890126` | 1.23456789013 (type 0) | 1.23456789013 (type 0) | VERIFIED |
| `num_13_digits_half_way` | `1.234567890125` | 1.23456789013 (type 0) | 1.23456789012 (type 0) | FAILED |
| `num_arith_12_digit_limit` | `1 + 1E-11` | 1.00000000001 (type 0) | 1.00000000001 (type 0) | VERIFIED |
| `num_arith_13_digit_drop` | `1 + 1E-12` | 1 (type 0) | 1 (type 0) | VERIFIED |
| `type_real` | `TYPE(42)` | 0 (type 0) | 0 (type 0) | VERIFIED |
| `type_real_val` | `42` | 42 (type 0) | 42 (type 0) | VERIFIED |
| `type_int` | `TYPE(#FFh)` | 1 (type 0) | 1 (type 0) | VERIFIED |
| `type_int_val` | `#FFh` | #FFh (type 1) | #FFh (type 1) | VERIFIED |
| `type_str` | `TYPE("hello")` | 2 (type 0) | 2 (type 0) | VERIFIED |
| `type_str_val` | `"hello"` | hello (type 2) | hello (type 2) | VERIFIED |
| `type_cplx` | `TYPE((1, 2))` | 3 (type 0) | 3 (type 0) | VERIFIED |
| `type_cplx_val` | `(1, 2)` | 1+2*i (type 3) | 1+2*i (type 3) | VERIFIED |
| `type_mat` | `TYPE([1, 2])` | 4 (type 0) | 4 (type 0) | VERIFIED |
| `type_mat_val` | `[1, 2]` | [1,2] (type 4) | [1,2] (type 4) | VERIFIED |
| `type_list` | `TYPE({1, 2})` | 6 (type 0) | 6 (type 0) | VERIFIED |
| `type_list_val` | `{1, 2}` | {1,2} (type 6) | {1,2} (type 6) | VERIFIED |
| `bool_and_false` | `0 AND 1` | 0 (type 0) | 0 (type 0) | VERIFIED |
| `bool_and_true` | `1 AND 1` | 1 (type 0) | 1 (type 0) | VERIFIED |
| `bool_or_true` | `1 OR 0` | 1 (type 0) | 1 (type 0) | VERIFIED |
| `bool_or_false` | `0 OR 0` | 0 (type 0) | 0 (type 0) | VERIFIED |
| `bool_not_zero` | `NOT 0` | 1 (type 0) | 1 (type 0) | VERIFIED |
| `bool_not_one` | `NOT 1` | 0 (type 0) | 0 (type 0) | VERIFIED |
| `control_ifte_true` | `IFTE(1, 10, 20)` | 10 (type 0) | 10 (type 0) | VERIFIED |
| `control_ifte_false` | `IFTE(0, 10, 20)` | 20 (type 0) | 20 (type 0) | VERIFIED |
| `str_dim` | `DIM("hello")` | 5 (type 0) | 5 (type 0) | VERIFIED |
| `str_indexing` | `LOCAL s:="hello"; RETURN s(2); ` | 101 (type 0) | 101 (type 0) | VERIFIED |
| `str_concat_num` | `"ab"+1` | ab1 (type 2) | ab1 (type 2) | VERIFIED |
| `str_mid` | `MID("hello", 2, 3)` | ell (type 2) | ell (type 2) | VERIFIED |
| `str_asc` | `ASC("A")` | {65} (type 6) | {65} (type 6) | VERIFIED |
| `list_size` | `SIZE({10, 20, 30})` | 3 (type 0) | — | UNVERIFIED |
| `list_indexing` | `LOCAL L:={10, 20, 30}; RETURN L(2); ` | 20 (type 0) | — | UNVERIFIED |
| `list_append_zero` | `LOCAL L:={10, 20}; L(0):=30; RETURN L; ` | {10,20,30} (type 6) | — | UNVERIFIED |
| `mat_create_2x2` | `[[1, 2], [3, 4]]` | [[1,2],[3,4]] (type 4) | — | UNVERIFIED |
| `mat_indexing` | `LOCAL M:=[[1, 2], [3, 4]]; RETURN M(2, 1); ` | 3 (type 0) | — | UNVERIFIED |
| `safe_hangle_deg` | `LOCAL prev, r; prev:=HAngle; HAngle:=1; r:=SIN(90); HAngle:=prev; RETURN r; ` | 1 (type 0) | — | UNVERIFIED |
| `safe_hangle_rad` | `LOCAL prev, r; prev:=HAngle; HAngle:=0; r:=SIN(0); HAngle:=prev; RETURN r; ` | 0 (type 0) | — | UNVERIFIED |
| `scope_uninit_local` | `LOCAL x; RETURN x; ` | 0 (type 0) | — | UNVERIFIED |
| `scope_for_after_loop` | `LOCAL i, s:=0; FOR i FROM 1 TO 3 DO   s:=s+i; END; RETURN i; ` | 4 (type 0) | — | UNVERIFIED |
| `scope_while_loop` | `LOCAL x:=1; WHILE x < 10 DO   x:=x*2; END; RETURN x; ` | 16 (type 0) | — | UNVERIFIED |
| `scope_repeat_loop` | `LOCAL x:=1; REPEAT   x:=x*3; UNTIL x > 20; RETURN x; ` | 27 (type 0) | — | UNVERIFIED |
| `sem_and_short_circuit` | `0 AND (1/0)` | error: Error: Division by 0 | — | UNVERIFIED |
| `sem_or_short_circuit` | `1 OR (1/0)` | error: Error: Division by 0 | — | UNVERIFIED |
| `sem_ifte_lazy` | `IFTE(1, 5, 1/0)` | error: Error: Division by 0 | — | UNVERIFIED |
| `sem_string_quoting` | `STRING("hello")` | hello (type 2) | — | UNVERIFIED |
| `sem_asc_multi` | `ASC("AB")` | {65,66} (type 6) | — | UNVERIFIED |
| `sem_str_index_local` | `LOCAL s:="ab"; RETURN s(2); ` | 98 (type 0) | — | UNVERIFIED |
| `sem_list_append_empty` | `LOCAL L:={}; L(0):=5; L(0):=6; RETURN L; ` | {5,6} (type 6) | — | UNVERIFIED |
| `sem_underflow` | `1E-499/10` | 0 (type 0) | — | UNVERIFIED |
| `sem_list_copy_by_value` | `LOCAL a:={1,2},b; b:=a; b(1):=9; RETURN a; ` | {1,2} (type 6) | — | UNVERIFIED |
| `sem_for_limit_eval` | `LOCAL i,n:=3,c:=0; FOR i FROM 1 TO n DO   n:=1;   c:=c+1; END; RETURN c; ` | 3 (type 0) | — | UNVERIFIED |
