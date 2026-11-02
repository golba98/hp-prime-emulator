# M18 — Linked Native debugging

Native linked programs can now be debugged in the CLI and IDE using the existing single VM and linker. This increment changes debugger metadata and navigation; it does not change PPL module initialization or variable-linkage semantics.

## Implemented

- `Machine::call_stack_frames()` exposes each original module/revision and span; `globals_for_frame(depth)` inspects the selected owner's file variables plus shared calculator slots. Existing `call_stack()` and `globals()` remain adapters.
- `ppl debug ROOT.ppl --module LEAF=LEAF.ppl` shares the Run/Check dependency loader. `--break LEAF:5` identifies a source module; numeric command-line breakpoints still target the root. Interactive unqualified breakpoints target the stopped module. Listing, backtraces, frame selection, locals/globals and runtime errors use original sources. Existing single-file output tests remain passing.
- IDE debug workers consume the exact immutable open-source catalog used for compilation. Breakpoints include the module/revision; identical line numbers in other modules do not alias. Step into/over/out use the existing VM frame depth, and explicit breakpoints can interrupt step-over in a dependency.
- Pauses activate the correct source tab and line. Call-stack selection navigates to the selected frame; its private variables remain distinct from identically named caller variables. Only the actual paused tab displays the execution marker.
- Editing or closing a running source opens a captured read-only source tab instead of drawing old locations on changed text. Captured tabs are excluded from linking, cannot be executed or saved over the original, and retain scoped breakpoint controls. They can be selected/copied and closed; they are not persistent source files.
- Dependency initializers are observable by the debugger when they execute. Retained initialized modules remain initialized according to the existing session policy.
- An actual GUI run exposed a previous resume race: Step/Continue left the preceding pause published until the worker consumed the command. Resume now invalidates the old pause while holding the command mutex before publishing the command. This prevents an immediate UI pause query from reporting the previous statement as completion of the requested step. A deterministic bridge regression covers the ordering.
- `PPL_IDE_NO_DEVICE=1` disables IDE USB enumeration and device-job admission. This optional offline mode is used by the native GUI test; ordinary interactive defaults are unchanged. Unit tests retain their unconditional physical-device-open refusal.

## Validation

The isolated native-only Xvfb runner `tests/ide_xvfb_smoke/run_m18_native_debug.sh` executes a three-module program, with real window rendering and 40 assertions. It verifies root and callee breakpoints, step into, ordinary step-over, a dependency breakpoint interrupting step-over, two step-outs, source/frame navigation, private globals, a local variable, editing while paused, immutable captured text, the final result 113 and stopping a subsequent debug session. No wait timed out. The captured screenshot was visually inspected: LEAF is active at line 5 with its execution marker, local x=3 and USB disabled.

Focused validation: 317 GUI library tests passed (2 existing ignored), 14 CLI debugger tests, 3 CLI Native module tests, 11 VM linkage tests and 4 VM library tests passed. Scoped all-target Clippy with warnings denied and Rust formatting pass. The coordinator runs the broad integrated M18 regression suite separately.

Owner evidence is preserved under `/home/k9-vortex/.cache/prime-m18-native-gui-final/` and `/home/k9-vortex/.cache/prime-m18-native-debug-final-tests.log`. Initial failed GUI traces are retained separately: the first exposed the old-pause race; the second identified a scenario expecting step-over to skip an ordinary intermediate statement. Assertions were corrected to require both intermediate stops, not weakened to accept stale locations.

## Explicit remaining limits

This increment does not claim verified exported cross-module variable semantics, initialization-cycle ordering, initializer failure/retry equivalence, or dynamic source replacement during an active linked call. Their M17 Unsupported/limited behavior remains. It does not implement ASCII i or special/unknown firmware object representations. These require controlled real-firmware observations before changing language semantics. No physical calculator observations or writes occurred. Native debugging is separate from firmware CPU debugging and FullSystem hardware diagnostics.
