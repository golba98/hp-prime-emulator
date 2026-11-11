# Optional Giac CAS helper

This separately built executable uses the system Giac library. The Rust
`ppl-cas` crate communicates with it over bounded stdin/stdout pipes; it does
not link Giac. No firmware assets or calculator access are involved.

Install the development package supplied by your operating system (`giac-devel`
on Fedora, `libgiac-dev` on Debian), then run `bash tools/giac-helper/build.sh`.
Set `PPL_CAS_HELPER` to the resulting absolute executable path. `CPPFLAGS`,
`LDFLAGS`, `CXX`, and `PPL_CAS_OUTPUT` allow a user-owned installation prefix.

Giac is GPLv3 software with commercial licensing available from its authors.
This helper's source is GPL-3.0-or-later; its executable and dependencies must
be distributed with their required licenses and source. It is an optional
separate program, not an MIT-licensed dependency embedded in the Rust binary.
See the [upstream Giac project](https://www-fourier.univ-grenoble-alpes.fr/~parisse/giac.html).

Each invocation evaluates one request in a fresh Giac context with explicit
settings and bindings. The response exports typed results and the updated
binding table. Rust commits the environment only after a valid correlated
success reply. ON/cancellation and timeouts kill the helper process group.
Giac's secure mode disables filesystem/process operations; calculator object
operations belong to the Native object store, not the algebra engine.

The protocol starts with `PPLCAS01`, a little-endian u64 request ID, u8 angle
(0 radians, 1 degrees, 2 gradians), u8 approximate flag, u8 precision, a u32
length-prefixed UTF-8 expression, then a u32 binding count and length-prefixed
name/expression pairs. EOF terminates the request. The helper returns one JSON
reply containing protocol version, request ID, Giac version, typed result,
error and updated bindings. Both sides enforce size and depth bounds.

`CasSession::evaluate_with_home` uses the optional `PPLCAS02` request extension:
after the CAS bindings it sends a Home binding count and name/expression pairs.
The response adds typed `home_updates`. Existing `PPLCAS01` requests still use
the original protocol and reply shape. A rebuilt helper supports both versions.
Home identifiers are renamed only at the compatibility boundary to avoid Giac
reserved names; string literals remain strings. Default uppercase calculator
variables and supplied Home values are visible without becoming CAS variables.
Only actual Home writes are returned, and private aliases are removed from all
exported CAS expressions. Callers commit the resulting CAS session and Home
updates together under their calculator operation lease. CAS `Ans` and settings
remain independent of their Home counterparts.

Giac compatibility is not a blanket claim of physical HP Prime parity. The HP
adapter and differential command tests must account for firmware differences.
