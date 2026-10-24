> Historical engineering record. Counts, commands and status below describe
> the recorded session, not a current guarantee. See the root README and latest
> milestone reports for supported interfaces and validation.

# Front-End Validation Hardening Summary

## Bugs Fixed

1. Missing semicolon now emits an error (not warning) for statement contexts.
   - Covered by: `missing_semicolon`
2. Invalid `EXPORT` identifier (digit start / hyphen) is rejected during front-end validation.
   - Covered by: `invalid_identifier_starts_with_digit`, `invalid_identifier_contains_hyphen`
3. Undeclared variable assignment/use now emits semantic errors.
   - Covered by: `undeclared_variable_use`
4. `EXPORT Name();` with extra semicolon in header is rejected.
   - Covered by: `extra_semicolon_after_export_header`
5. `EXPORT Name` without parentheses is rejected.
   - Covered by: `missing_parentheses_in_export_header`
6. Single-quoted strings are rejected with a targeted message.
   - Covered by: `single_quoted_string_literal`
7. `=` in statement assignment context is rejected with a `:=` hint.
   - Covered by: `invalid_assignment_equals_instead_of_colon_equals`
8. Malformed `LOCAL` declarations missing commas are rejected.
   - Covered by: `malformed_local_missing_comma`
9. Trailing commas in function call argument lists are rejected.
   - Covered by: `trailing_comma_in_function_call`
10. Invalid top-level tokens are rejected before transpile.
    - Covered by: `invalid_top_level_tokens_before_after_program`
11. Front-end diagnostics now carry file, line, optional column, and optional hint.
    - Covered across all regression failures through lint + CLI assertions.
12. Compiler pipeline now always gates on front-end errors before transpilation/execution (even with `--no-lint`).
    - Covered across all invalid regression cases (assert no transpile/execution markers in CLI output).

## Explicit Syntax Rules Formalized

- Program header must be `EXPORT Name()` with no trailing semicolon in the header line.
- Statements require trailing `;` unless the statement is a block/header keyword context.
- Assignment statements must use `:=`; `=` is comparison-only.
- String literals must use double quotes.
- `LOCAL` declaration lists must be comma-separated identifiers.
- Function call argument lists may not have trailing commas.
- Identifiers must start with a letter/underscore and cannot contain `-`.
