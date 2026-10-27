# Tested file compatibility

50 authored files; 26 cases opened and saved by original G1 firmware 2.4.2
on both the import boot and the reboot. Every firmware-saved source matched
the expected source exactly. Binary imports were copied byte for byte; text
imports required explicit source mode and reported newline/encoding changes.
All rejected cases left isolated profiles unchanged.

`host policy` means no firmware verdict is claimed for that file. File validity
and PPL compiler validity are separate; host acceptance is not a firmware
compiler verdict. Program source is pinned separately under `sources/`.

| Case | Input | Host import | Firmware save/reopen, two boots |
|---|---|---|---|
| MINIMAL | binary | accepted | passed |
| NESTED | binary | accepted | passed |
| UNICODE | binary | accepted | passed |
| SUPPLEMENT | binary | accepted | passed |
| GRAPHICS | binary | accepted | passed |
| WHITESPACE | binary | accepted | passed |
| ESCAPES | binary | accepted | host policy; unobserved |
| MULTI | binary | accepted | passed |
| ENTRIES | binary | accepted | passed |
| LONGIDENT | binary | accepted | passed |
| LOWER | binary | accepted | passed |
| UPPEREXT | binary | accepted | passed |
| SPACEFILE | binary | accepted | passed |
| UNICODEPATH | binary | accepted | passed |
| BADFILENAME | binary | rejected: invalid-name | host policy; unobserved |
| BADUNICODENAME | binary | rejected: invalid-name | host policy; unobserved |
| LEADINGDIGIT | binary | rejected: invalid-name | host policy; unobserved |
| EMPTYNAME | binary | rejected: invalid-name | host policy; unobserved |
| NAME31 | binary | accepted | passed |
| NAME32 | binary | rejected: invalid-name | host policy; unobserved |
| UNEDITED | binary | accepted | passed |
| CACHED | binary | accepted | passed |
| UNKNOWN | binary | accepted | host policy; unobserved |
| CONTAINER_CRLF | binary | accepted | passed |
| LARGE | binary | accepted | passed |
| LARGE256 | binary | accepted | passed |
| LF | text utf-8 | accepted | passed |
| CRLF | text utf-8 | accepted | passed |
| CR | text utf-8 | accepted | passed |
| UTF8BOM | text utf-8 | accepted | passed |
| UTF16LE | text utf-16le | accepted | passed |
| UTF16BE | text utf-16be | accepted | passed |
| MIXED | text utf-8 | accepted | passed |
| TEXTDEFAULT | text utf-8 | rejected: plain-text | host policy; unobserved |
| EMPTY | text utf-8 | rejected: source | host policy; unobserved |
| SPACEONLY | text utf-8 | rejected: source | host policy; unobserved |
| BADUTF8 | malformed bytes | rejected: source | host policy; unobserved |
| ODDUTF16 | malformed bytes | rejected: source | host policy; unobserved |
| SURROGATE | malformed bytes | rejected: source | host policy; unobserved |
| NUL | malformed bytes | rejected: source | host policy; unobserved |
| BOMLESS16 | malformed bytes | rejected: source | host policy; unobserved |
| TRUNCATED | malformed bytes | rejected: container | host policy; unobserved |
| HEADERONLY | malformed bytes | rejected: container | host policy; unobserved |
| BADPREAMBLE | malformed bytes | rejected: container | host policy; unobserved |
| TRAILING | malformed bytes | rejected: container | host policy; unobserved |
| BADLENGTH | malformed bytes | rejected: container | host policy; unobserved |
| PARTIALGROUP | malformed bytes | rejected: container | host policy; unobserved |
| BADSOURCE16 | malformed bytes | rejected: container | host policy; unobserved |
| DUPSECTION | malformed bytes | rejected: container | host policy; unobserved |
| DEEP | malformed bytes | rejected: container | host policy; unobserved |

Additional scenarios: duplicate refusal, case-variant duplicate refusal, explicit
replacement, ambiguous alias refusal, concurrent threads/processes, stale temp
symlinks and interrupted-lock recovery pass host storage tests. Earlier firmware
program tests also confirm duplicate-name refusal in the authentic catalog.

The genuine editor modification case changed RETURN 1 to RETURN 2 and retained
exact source through its save and two subsequent reboot/reopen/save cycles.
An unrelated sentinel program stayed byte-identical.

Live observations: firmware Programs(name):=source created NEWLIVE in a running
session using ordinary Editor Check and Home invocation keys. NEWLIVE appeared
in the real catalog, reopened after reboot, and retained exact assigned source.
Copying a host file while running left catalog pixels unchanged. The existing
managed Run command failed catalog-order attribution before execution; that
runtime-owned interface was not modified. General live import is not exposed
by this milestone.

Raw saved hashes may vary with allocator padding; source hashes and observed
firmware outcomes are the stable evidence. The JSON reference retains actual
saved hashes, sizes, export caches and per-session proof. No physical calculator
was accessed.
