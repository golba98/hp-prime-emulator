# M17 typed firmware observation increment

The running G1 firmware15751 (ELF SHA-256
`ad9779ff72db39fe4e9977ce7c4324f39aad1bb00c849c0aab067dc5bfc2fb39`)
returned real controls that establish these layouts:

| Value | Observed representation |
| --- | --- |
| Complex | Tag0x13, two inline16-byte decimals; first header shares the type tag. Sign and exponent remain per component. |
| Integer | Tags0x31/0x51/0x71/0x91 encode binary/octal/decimal/hex; subtype is word width1–64; unsigned word at+8. Reserved bytes+4 contain non-value allocation contents and are not decoded. |
| Matrix/vector | Tag0x14(real) or0x94(complex); rank at+4, rows at+8, columns at+12. Rank1 is a vector. Cells are row-major inline8-byte decimals, paired for complex matrices. |
| Packed decimal |12BCD mantissa digits at bits12–59, signed12-bit exponent, high nibble0(positive) or9(negative). Other sign/special encodings are rejected. |

Controls compared components using actual RE/IM and matrix indexing results.
They cover positive/negative components, zero, ±100 exponents,12significant
digits,1D vectors,1×N/N×1/1×1 matrices, complex matrices, all four integer
bases and widths1,8,16,32,63,64. This is local-firmware evidence, with no new
physical-device observation. Retained exact representation graphs live under
`crates/prime-emulator/tests/fixtures/runtime-values/`.

The decoder validates dimensions, rank, width, mantissa digits, exponent,
component tags, lengths and graph identities. Matrix cells share the existing
4096logical-item and1MiB capture/output budgets with enclosing lists. No guessed
pointer is followed. Unknown objects may retain up to4096bytes of their exact
known host allocation for forensics, while remaining explicitly undecoded.

`tests/differential/m17/typed_values.toml` passes17assertions across real
retained-session HLE and Native executions. It includes nested lists, strings,
matrices, vectors, complex numbers, integer variants, complex matrices and a
repeated run. Structured values already agree; no broad normalization was
added. Comparator controls deliberately change vector identity, dimensions,
cell type, imaginary component, integer width/base and nested values and verify
specific mismatch paths. Four new decoder tests cover retained real captures,
corrupt layouts/components, shared budgets and unknown-object capture bounds.
Existing runtime and representation suites also pass. Scoped Clippy with
warnings denied and formatting pass.

Limits: special/nonfinite representations, unobserved object tags, rank>2 and
zero-dimensional matrices remain unsupported. Exact rational output is still
preserved as rational when a finite decimal is unavailable. An additional
compatibility control found that firmware accepts ASCII `i` as an imaginary
constant while Native currently requires the Unicode imaginary unit or complex
pair syntax. The shared typed fixture uses `(real,imaginary)` syntax; this does
not conceal or normalize the remaining ASCII input difference.
