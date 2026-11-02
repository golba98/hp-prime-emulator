# Desktop typing in G1 Firmware mode

The calculator uses different Alpha case defaults in Home and CAS. The
firmware program editor also inserts paired parentheses rather than a single
literal character. Desktop input now goes through physical calculator key
sequences, with the firmware's visible Shift/Alpha indicator observed between
consumed key events. The emulator does not inject text into guest memory or
infer the current mode from the last Home/CAS key.

## Everyday controls

- Type letters normally, including mixed case. Desktop Shift controls the
  typed character; it does not independently latch calculator Shift.
- Ctrl+V pastes one expression, up to 2048 characters. A complete packet is
  checked before any of its characters are queued. Unsupported characters,
  multiline text, and pasted `^` expressions are rejected with an explanation.
- Paste multiline PPL source into the IDE editor and use Import or Run.
- `(`, `[` and `{` open the firmware's paired template. A matching desktop
  closing delimiter moves Right out of the pair. An unmatched closing
  delimiter is refused; use Right to leave a pair already on the calculator.
- `^` or F11 intentionally enters the physical power template. Use Right to
  leave the exponent. Paste rejects `^` because a linear expression such as
  `2^3+4` would otherwise enter `2^(3+4)` in a template editor.
- Tab sends physical Alpha. Shift+Tab sends physical calculator Shift. The
  virtual keypad retains its physical calculator semantics. Shift with a
  navigation/function key still sends a calculator Shift shortcut.
- Escape cancels queued typing. Window/calculator focus loss clears queued
  input and releases delivered keys. Held arrows and Backspace retain the
  session's consumption-paced repetition.

Desktop Ctrl is no longer automatically sent as Alpha, so clipboard commands
do not change calculator case. Copying the firmware's selection into the
system clipboard is not implemented. Calculator Copy/Paste remain available
through the virtual Shift+View/Shift+Menu keys.

## Implementation and safety boundaries

`ppl-engine/src/session.rs` protocol version 4 carries atomic `Text` packets
with a paste flag. `prime-emulator/src/session/typing.rs` validates and queues
characters, observes a 34 × 8 pixel status region under the guest snapshot
gate, and sends normal down/up key transitions only after preceding events
were consumed. `ppl-gui/src/app_firmware.rs` coalesces adjacent desktop text
and routes paste through that protocol; it never floods the transport with
one command per generated key. The ordinary physical-key route stays intact.

The observer recognises the captured neutral, one-shot upper/lower Alpha,
locked uppercase Alpha, and Shift indicators. Unknown screens/indicators
stop typing with an actionable status message. A firmware screen without
these indicators, including a program's custom graphics/input UI, is not
claimed to support desktop text. Use the physical keypad there. Modal and
catalog search behavior beyond the observed indicator mapping has not been
exhaustively verified.

Typing changes the physical Shift/Alpha latches as needed; the observed
firmware state remains authoritative. Existing Alpha lock works without
silently inverting requested case. Cursor moves and touch invalidate the
record of desktop-created pairs. This is intentionally conservative when
mixing physical edits with desktop delimiters.

Prevalidation prevents partial delivery caused by unsupported clipboard
characters. It does not promise transactional rollback if the user changes
context, cancels, or the firmware stops accepting input halfway through a
valid paste. No arbitrary delay substitutes for firmware event consumption.

## Reproducible verification

Local build 15751 was used to capture status fixtures under
`crates/prime-emulator/tests/fixtures/typing/`. These are firmware-only
observations; no physical device was accessed.

Run the actual GUI scenario with an isolated profile:

```sh
tests/ide_xvfb_smoke/run_m12.sh m16_typing 180
python3 tests/ide_xvfb_smoke/assert_m16_typing.py <evidence>/m16_typing
```

The scenario compares desktop `aA` with independent physical key sequences
in Home, CAS and the firmware editor; repeats the editor test with Alpha
already locked; compares pasted parentheses and brackets with physical
controls; and verifies that rejected multiline/power paste neither types nor
executes. The pasted `(2+3)*4` was also visually inspected and evaluates to
`20`. Header masks exclude the host-driven clock. Tests of queue limits,
modifier convergence, unknown screens, cancellation state, transport batching
and unsupported text run without proprietary firmware in the Rust suites.

Validation of this increment: 65 emulator unit tests and 302 GUI unit tests
passed (2 ignored); Clippy with warnings denied and formatting passed.
The real GUI scenario passed all 10 assertions on build 15751, with evidence
under `/tmp/prime-keyboard-final-15751/m16_typing`.

A 15157 GUI run was attempted with its own supplied ELF, ROM and initialized
resources. The current live-session compiler observer refuses that build
(`unsupported firmware for compiler observation`), before any input can be
sent. Its typing behavior is therefore unverified here; no observer bypass
was introduced. The attempt's actual GUI was captured at
`/tmp/prime-keyboard-15157-startup.png`.
