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

The current Rust adapter requests `PPLCAS04`: after the three setting bytes it
sends an operation byte (0 evaluates, 1 parses without evaluating), then the
expression and CAS bindings, a Home binding count (zero when absent), and Home
name/expression pairs. Its
reply includes `home_updates` and preserves Giac's explicit `list` and ordinary
`vector` subtypes and structural `unit` nodes. The Rust adapter performs the
calculator's numeric/list/matrix conversion at the Home boundary. Rebuild the
helper with this adapter; older helpers report a protocol error with rebuild
instructions rather than silently losing those types.

The helper still accepts `PPLCAS01` and the Home extension `PPLCAS02`, with their
original reply shapes and generic list/symbolic tags. Existing clients therefore
retain their protocol. `PPL_CAS_HELPER` overrides discovery; a matching helper
beside the application takes precedence over the separately installed user helper.
Version4 additionally preserves algebraic undefined and signed/unsigned infinity
values, separately from parse and evaluation failures. Saved undefined bindings
are restored as values; malformed expressions still fail without committing their
environment. `PPLCAS03` retains its prior typed-unit/vector reply shapes and
legacy undefined-error behavior. Rebuild the helper for the version4 adapter.

`PPLCAS05` is a separate, isolated Home/PPL container projection. It accepts
only a parse operation with no saved CAS or Home bindings and a container root.
Giac determines which array cells have numeric values; unresolved symbols and
strings remain data. The VM retains the original exact CAS answer, then converts
the projected numeric array to a Home matrix or mixed list. This operation does
not execute the original CAS source again. Symbolic Home arrays require a helper
built with protocol5; an older helper produces an explicit rebuild error.
Ordinary evaluation and saved-profile parse inspection still use protocol4.

Parse-only requests recover older profiles' missing unit metadata from Giac's
actual parsed root, without executing saved source or committing bindings/answers.

Home identifiers are renamed only at the compatibility boundary to avoid Giac
reserved names; string literals remain strings. Default uppercase calculator
variables and supplied Home values are visible without becoming CAS variables.
Only actual Home writes are returned, and private aliases are removed from all
exported CAS expressions. Callers commit the resulting CAS session and Home
updates together under their calculator operation lease. CAS `Ans` and settings
remain independent of their Home counterparts.

Giac compatibility is not a blanket claim of physical HP Prime parity. The HP
adapter and differential command tests must account for firmware differences.
