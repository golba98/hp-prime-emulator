# Standalone Native catalog operations

The calculator Programs, Notes, Variables, Lists and Matrices catalogs share the
persistent Native session. Objects are independent of IDE tabs and files.

## Collection and variable operations

`app_catalog.rs` owns completion of catalog variable stores and collection
editors. These use the existing controller worker and GuiHost. The pending task
remembers the catalog page and underlying calculator view. It consumes its own
success, error or interruption before ordinary program completion handling.

This prevents a previous operation's `outcome_handled` flag from hiding a failed
store, and prevents successful standalone edits from creating program-output
transitions or Home history. Actual saved values refresh the catalog preview.
Failures preserve the draft and appear in the catalog. ON interrupts the actual
worker, cleans up modal input and restores the catalog. Split-view starts transfer
keyboard focus from the IDE to the calculator's modal input owner.

The **Open editor / Create** action creates a named blank collection immediately.
Cancel then discards changes made in the editor, retaining the initial created
object. Existing collection values are unchanged by cancellation. This explicit
creation policy is covered by tests; the editor does not silently delete names.

## Object selection after CRUD

Successful note/program saves select the saved name. Successful renames update
`original_name`, allowing subsequent renames to target the actual object. Failed
renames retain both existing objects, their values and the original selection.
Deleting the selected object clears its editor draft. Metadata changes occur only
after the underlying shared-memory mutation succeeds. Busy or rejected-profile
operations leave the objects unchanged.

## Program execution

Programs launched from the Tools catalog execute exact source snapshots directly
on the existing linked Native worker. Their stable namespace is the calculator
object name. Repeated runs retain file-local values, installed dependencies remain
available after IDE tabs close, and launching a program creates no IDE tabs.
Opening, saving, running and debugging normal IDE tabs retain their original paths.

Default entry selection uses `CompiledProgram::pick_entry_with_stem`, preferring
an export matching the object name. An explicit entry and argument expression
string can be supplied through `run_native_catalog_program_configured`. The
controller and worker support both paths. A parameterized default entry opens
actual calculator INPUT fields named from its compiled parameters. Fields accept
PPL expressions evaluated against the real calculator memory. Invalid expressions
remain editable with an error. Escape cancels before the entry executes; ON
interrupts parameter entry or running code. Explicit arguments use the existing
session argument evaluator. Neither launch origin promotes its return value to
Home Ans or adds Home calculation history.

Program output remains visible until dismissed. Escape returns to the originating
Programs page, and interruption restores that page immediately. A cancelled
argument form is recorded as cancelled, without a fabricated program result.

## Verification

`app_catalog_tests.rs` drives real egui button shapes and pointer events, the
production calculator keyboard route, GuiHost modal events, controller workers
and VM memory. Its seventeen tests cover save/rename/delete, collection dimensions
and cells, list editing, cancellation, busy/type/profile failures, ON during
editing/waits/program loops, repeated program-local counters, dependency calls,
entry/argument selection, split-view focus, and all object types after a profile
reopen. No physical calculator connection is opened.

The original coordinator routes hid async store errors, failed to select saved
rename targets, and reset a program-local counter on each catalog run. Separate
failing-then-passing focus coverage established that split-view dialogs retained
IDE keyboard ownership before the focus transfer was added. The corrected routes
pass these assertions.
