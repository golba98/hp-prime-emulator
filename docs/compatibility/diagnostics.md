# Diagnostics: Classes, Codes and Evidence

Every diagnostic has a **class**, `ppl_syntax::DiagnosticClass`, derived
from the stage that produced it. The editor, the Problems panel and the
CLI report the class next to the severity.

| Class | Meaning | Blocks compilation? |
|---|---|---|
| **Syntax** (lexer/parser) | The source is malformed | Errors do |
| **Semantic** (compiler) | Well-formed source that breaks a language rule | Errors do |
| **Compatibility** (compiler) | Valid HP Prime code that this emulator cannot (fully) run | **Never** |
| **Runtime** (VM) | A failure while executing the program | — |

**Rule:** a missing emulator feature is never a Syntax or Semantic error.
A documented command the emulator lacks:

- compiles,
- produces a *Compatibility* warning,
- and raises an "unsupported" runtime error **only if that call actually
  executes**.

The program still runs on every other path (see
`crates/ppl-vm/tests/unsupported.rs`).

## Sources of truth

- **HP Prime Graphing Calculator User Guide, Third Edition (December 2017),
  HP part number 813269-003**, chapters 23–29 ("Functions and commands"
  through "Programming in HP PPL"). The Edition 1 guide (2013) predates
  firmware features such as INPUT's sixth argument, so it is not used to
  reject code.
- `ppl_compiler::catalog`: names taken from the guide's command headings
  and syntax lines, minus everything the emulator implements.
- Rules not checked against a physical calculator carry `unverified`. The
  editor shows "(unverified rule)".

## Codes

### Syntax (lexer and parser)

| Code | Severity | Notes |
|---|---|---|
| `unclosed-string`, `unclosed-quote` | Error | The error spans the rest of the opening line only. Lexing resumes on the next line, so later statements highlight and parse normally (no whole-file "string" cascade). |
| `multiline-string` | Note, unverified | A closed string spanning lines is legal; the note helps spot a missing quote. |
| `unknown-escape` | Warning, unverified | |
| `invalid-character`, `malformed-integer`, `invalid-identifier` | Error | `%` followed by `(` or a capital letter is an identifier (`%(x,y)`, `%CHANGE`, `%TOTAL` are documented). |
| `expected-*`, `unexpected-token`, `unexpected-keyword`, `unclosed-delimiter`, `unclosed-block`, `missing-semicolon`, `trailing-comma`, `invalid-top-level`, `nested-function`, `export-header-parens`, `export-header-semicolon`, `local-missing-comma`, `return-outside-function` | Error | |
| `equals-statement`, `for-assign-form`, `missing-semicolon-before-end` | Warning | `missing-semicolon-before-end`: the original firmware accepts these forms (diagnostic corpus h04–h06, h12). |
| `missing-semicolon` after RETURN before END, and after a function's END | Error | Firmware-backed: original G1 2.4.2 Check rejects both (diagnostic corpus h03, h09, h10, h13). |

### Semantic (compiler)

| Code | Severity |
|---|---|
| `undeclared-identifier`, `unknown-function`, `wrong-arg-count`, `invalid-assignment-target`, `expected-variable`, `read-only`, `reserved-name`, `duplicate-function`, `duplicate-parameter`, `invalid-dimension`, `number-out-of-range`, `div-requires-integers`, `outside-loop`, `return-outside-function` | Error |
| `call-before-definition`, `forward-without-definition`, `duplicate-global`, `duplicate-local`, `shadows-builtin`, `unused-local`, `unreachable-code`, `break-levels`, `input-unknown-type` (unverified: an INPUT type code outside -1, 0–4, 6, 8, 9) | Warning |

- `unknown-function` and `undeclared-identifier` are reported only for
  names that are neither declared nor a documented HP Prime command. An
  undeclared name *inside the arguments* of an unsupported CAS call (the
  `x` in `solve(x^2-4=0, x)`) is a CAS symbol and gets no diagnostic.
- `wrong-arg-count` for builtins uses `ppl_compiler::builtins`. Every range
  there was cross-checked against the guide's syntax lines; documented
  forms the emulator only partly implements are accepted (see below).

### Compatibility (compiler)

| Code | Severity | Raised for |
|---|---|---|
| `unsupported-command` | Warning | A documented command the emulator lacks: `SOLVE`, `solve`, `EVAL`, `%CHANGE`, `MAXREAL`, … (builtin status *Unsupported*, or a catalog name) |
| `cas-unsupported` | Warning | Qualified `CAS.name(...)` / `App.name(...)` calls |
| `cas-block-unsupported` | Warning | `#cas … #end` sections |
| `symbolic-unsupported` | Warning | `'quoted'` symbolic expressions |
| `unsupported-value` | Warning | `∞` |
| `lowercase-command` | Note, unverified | `append(...)`: on the calculator a lower-case name calls the CAS command; the emulator runs the Home command (`APPEND`) |
| `input-defaults-ignored` | Note | `INPUT` with `reset_value` / `initial_value`: accepted, but the emulator's dialog does not apply them yet |
| `input-field-position-ignored` | Note | `INPUT({{var,[types],{x,width,line}}})`: the field position is accepted but the emulator lays fields out in order |

### Runtime (VM)

`runtime-error` (Error) is located at the innermost frame of the VM trace.
Runtime problems are cleared when the next run starts.

## Rulings on the reported diagnostics

| Reported diagnostic | Finding | Evidence | Resolution |
|---|---|---|---|
| `INPUT expects 1–5 argument(s), got 6` | **Wrong rule.** | `INPUT(var,["title"],["label"],["help"],[reset_value],[initial_value])`, and the list form with six list arguments: User Guide 3rd ed., p.652. The legacy Python spec also allowed 6. | Arity is 1–6. Arguments 5–6 add a Compatibility note. |
| `append resolves to APPEND` | Valid PPL. On the calculator `append` is the documented CAS command (p.461). | Behavior difference not hardware-verified. | Compatibility **note**, marked unverified. |
| `SOLVE is a CAS/app function and is not emulated` | Valid PPL. `SOLVE(En,var,guess)` is the Solve app function (p.428); `solve` is CAS (p.404). Bare `SOLVE`/`solve` used to be *unknown-function errors* that blocked compilation. | — | Compatibility **warning**; compiles; runtime "unsupported" only when executed. |
| `EVAL is not emulated yet` | Valid PPL (catalog, p.469). | — | Compatibility **warning**. |

## Other argument-count corrections from the guide audit

These documented forms were rejected as `wrong-arg-count` before:

| Command | Was | Now | Emulator behavior |
|---|---|---|---|
| `RANDINT` | 2–3 | 0–3 | `RANDINT()` returns 0 or 1; `RANDINT(a)` returns 0…a (implemented) |
| `MAKEMAT` | 3 | 2–3 | `MAKEMAT(expr, n)` returns a vector (implemented) |
| `SUBGROB_P` / `SUBGROB` | 6 | 2–6 | `SUBGROB_P(src, trgt)` copies the whole graphic (implemented) |
| `DIMGROB_P` / `DIMGROB` | 3–4 | 2–4 | List form raises "unsupported" |
| `LINE_P` / `LINE` | 4–6 | 2–6 | 3D list form raises "unsupported" |
| `TRIANGLE_P` | 6–12 | 0–15 | Only the 7-argument coordinate form is drawn; other forms raise "unsupported" |
| `STRING` | 1–3 | 1–5 | Separator and size-limit arguments raise "unsupported" |
| `EDITMAT` | 1–2 | 1–3 | Unsupported command |

## Firmware-backed rulings

`docs/compatibility/diagnostic-parity.md` compares these rules with the
original G1 firmware's Program Editor Check (local emulation) over the
`tests/diagnostics` corpus. Corrections made from that evidence:

| Rule | Was | Now | Evidence |
|---|---|---|---|
| `BLIT_P` / `BLIT` arity | 0–11 | 1–11 (srcG is required) | Firmware rejects `BLIT_P()` and `BLIT()` (e06, e18) |
| RETURN before END | warning (unverified) | `missing-semicolon` error at the END | h03, h13 rejected; h04, h12 (other statements) accepted |
| Function END without `;` | accepted | `missing-semicolon` error at the next token | h09, h10 rejected |
| Keyword letter case | upper case only; `begin`, `End` were names | any ASCII case where the grammar expects the keyword; other than upper case it is a *soft* keyword that reads as a name in name and operand positions | h01, h07, k01–k05, k24 accepted; `LOCAL begin/end/do/case/step/key/…` and `end:=5` accepted (k08–k20, k23, k36) |
| Operator keywords | upper case only | `MOD`, `AND`, `OR`, `XOR` in any case; `DIV` and `NOT` in upper case only (`div`, `Not` are names) | k06, k21, k25–k29 accepted; k07, k32–k34 rejected |
| Upper-case keyword as a name | rejected | unchanged (upper-case keywords are reserved) | k22 rejected |

Soft keywords are resolved by the parser (`Token::soft`,
`parser::resolve_soft_keywords`), and the highlighter and auto-indent apply
the same resolution, so `LOCAL begin:=4` colours `begin` as a variable. The
source is never rewritten: tokens keep their original spelling and byte
spans. A lower-case `end` before `;` still closes its block (k35), and a
soft keyword followed by `:=` or an operator is a name (k30, k36).

## Verification status

None of the rules above have been checked on the physical G1 in this
change. A hardware check would be an opt-in `ppl compare` run with
`ZZT_*` programs; until then, rulings rest on the User Guide.
