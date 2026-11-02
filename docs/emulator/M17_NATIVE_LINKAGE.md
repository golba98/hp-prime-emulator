# M17 Native linkage increment

Implemented an independently compiled bytecode linker, not source concatenation
or nested VM instances. Exported calls resolve by their genuine bare function
names. Calculator globals/settings alias one live slot; file variables remain
owned by their original module/revision. Linked calls use one VM stack, Host,
resource budget and IFERR/error unwinding. Linked image Rc values remain local
to the execution thread; retained SessionContext and source catalogs remain Send.

The linker follows only the selected root's external dependency closure.
Unrelated broken modules cannot block a valid root, including an invalid tab
attempting to redefine a reserved builtin. Missing/private/ambiguous external
exports fail explicitly. Caller/callee source identity follows error frames.

A real firmware control established a previous Native discrepancy: firmware
Programs assignment executes file initializers at installation. It does not
wait until the first Run. Native scenario Import/Edit now initializes at install,
retains unchanged module state, and invalidates edited module state. Direct
SDK run_linked still initializes any uninitialized units before their calls.

Real firmware15751 and Native comparisons passed initialization timing (12
assertions) and a three-program chain with arguments, shared A, identical private
variable names, retained calls and edited dependency (22 assertions). Fixtures:
initialization.toml and linkage.toml under tests/differential/m17. No physical
calculator evidence was created. Reports remain in the owner's home cache at
prime-m17-native-tmp/*compared.json.

New VM tests cover chains, arguments, shared/private state, normal cross-module
recursion with a shared depth limit, edited dependency/arity invalidation,
missing/private/ambiguous exports, source-bound traces and IFERR across modules,
initializer timing, preserved Send state and unrelated broken sources.

Current limits: dynamic Programs replacement of a catalog module during an
active call returns Unsupported instead of executing stale code. Cross-module
exported variable references (as distinct from shared calculator variables) and
initialization cycles need further firmware controls. Existing Native initializer
failure retry is retained; exact firmware failure/recursive-initializer semantics
are not claimed verified. Runtime error details remain distinct across backends.
The Native engine and CLI now consume the same linker: `ppl run` and `ppl check`
accept repeatable `--module NAME=PATH` arguments. Only the reachable dependency
closure is compiled; a broken unrelated supplied module remains harmless.
Module warnings and runtime failures retain the original source identity and
position. Ambiguous exported names are diagnosed before candidate arities.
Output/report destinations cannot alias any dependency input file. Three actual
CLI integration tests, ten linkage tests and six engine core tests pass; scoped
Clippy with warnings denied passes. IDE open-tab catalog wiring and new firmware
object decoders remain subsequent increments.
