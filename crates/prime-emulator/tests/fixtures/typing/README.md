# Firmware Alpha status captures

These 34 × 8 pixel crops come from genuine local HP Prime G1 firmware build
15751 captures on 2026-11-02. They contain only the Shift/Alpha status area.
They are firmware-only verification, not physical calculator measurements.

- `neutral`: Home, no modifiers.
- `home_upper`: Home, Alpha once (A..Z).
- `cas_lower`: CAS, Alpha once (a..z).
- `locked_upper`: Home, Alpha twice (locked AZ).
- `shift`: Home, Shift once.
- `editor_shift_lower`: program editor, Alpha then Shift (a..z + Shift).

Full original captures were inspected under `/tmp/prime-keyboard-probe` and
`/tmp/prime-keyboard-editor-probe`. No firmware files are bundled.
