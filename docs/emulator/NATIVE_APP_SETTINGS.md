# Native application Symbolic Setup

The calculator's active app owns independent `AppSettings`, persisted alongside
other versioned Native calculator memory. `SessionContext` and `SessionSnapshot`
carry the per-app map. Existing version 1 profiles without this map migrate to
System defaults; malformed present maps fail without changing current memory.

The compiler resolves all four settings as system variables, rather than module
variables. PPL assignments, calculator variable updates and subsequent runs share
the active app's values. App switching retains other apps' overrides. Home
settings remain independent. The documented integer codes are:

| Variable | Codes |
|---|---|
| AAngle | 0 System, 1 Radians, 2 Degrees, 3 Gradians |
| AComplex | 0 System, 1 ON, 2 OFF |
| AFormat | 0 System, 1 Standard, 2 Fixed, 3 Scientific, 4 Engineering |
| ADigits | 1 through 10 decimal places, as documented `0 < n < 11` |

`effective_app_settings()` returns a Settings clone for graph/numeric work.
System fields inherit their corresponding current Home setting. An explicit app
format uses ADigits; System format inherits Home format and Home digits. The
clone does not modify Home settings or the independent CAS settings.

Reference: HP Prime User Guide, chapter 28, pages 637–638:
https://h10032.www1.hp.com/ctg/Manual/c05332710.pdf.

`cargo test -p ppl-vm --test app_settings` checks exact codes, Home isolation,
independent Function/Parametric overrides, invalid assignments, transactional
restore, migration, and serialization/restoration in a new operating-system
process. Desktop Symbolic Setup controls consume the same state through the
controller; these VM tests do not substitute for desktop interaction tests.
