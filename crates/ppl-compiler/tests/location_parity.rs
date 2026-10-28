//! Locations captured from original G1 2.4.2 firmware, never native snapshots.
//! The shared corpus owns the exact source and firmware reference; these tests
//! additionally lock byte spans and stability after source edits/decoding.

use ppl_compiler::{CompileOptions, compile_source};
use ppl_syntax::{SourceFile, Span, source::decode_source};

fn assert_error(src: &str, code: &str, span: Span, line: u32, column: u32) {
    let file = SourceFile::new("location-regression", src);
    let (program, diagnostics) = compile_source(&file, &CompileOptions::default());
    assert!(program.is_none(), "unexpected acceptance: {src}");
    let first = diagnostics
        .errors()
        .next()
        .expect("rejected source has an error");
    assert_eq!(first.code, code, "{src}: {diagnostics:?}");
    assert_eq!(first.span, span, "{src}");
    let lc = file.line_col(first.span.start);
    assert_eq!((lc.line, lc.col), (line, column), "{src}");
    assert!(file.text.is_char_boundary(first.span.start as usize));
    assert!(file.text.is_char_boundary(first.span.end as usize));
}

macro_rules! firmware_case {
    ($name:ident, $code:literal, $start:literal, $end:literal, $line:literal, $col:literal) => {
        #[test]
        fn $name() {
            let source = include_str!(concat!(
                "../../../tests/diagnostics/cases/",
                stringify!($name),
                ".ppl"
            ));
            let span = Span {
                start: $start,
                end: $end,
            };
            assert_error(source, $code, span, $line, $col);
            // A source edit with both BMP and supplementary Unicode shifts
            // byte and UTF-16 offsets by different amounts, but not columns.
            let prefix = "// π 😀\n";
            let edited = format!("{prefix}{source}");
            assert_error(
                &edited,
                $code,
                Span {
                    start: span.start + prefix.len() as u32,
                    end: span.end + prefix.len() as u32,
                },
                $line + 1,
                $col,
            );
            assert_eq!(
                edited[..span.start as usize + prefix.len()]
                    .encode_utf16()
                    .count(),
                source[..span.start as usize].encode_utf16().count()
                    + prefix.encode_utf16().count()
            );

            // Container/plain-text decoding normalizes newlines before parsing.
            let crlf = source.replace('\n', "\r\n");
            let (decoded, _) = decode_source(crlf.as_bytes());
            assert_eq!(decoded, source);
            assert_error(&decoded, $code, span, $line, $col);
            let utf16: Vec<u8> = std::iter::once(0xfeffu16)
                .chain(edited.encode_utf16())
                .flat_map(u16::to_le_bytes)
                .collect();
            let (decoded, _) = decode_source(&utf16);
            assert_eq!(decoded, edited);
        }
    };
}

firmware_case!(e01_missing_end, "unclosed-block", 34, 34, 4, 1);
firmware_case!(e02_nested_missing_end, "unclosed-block", 95, 95, 9, 1);
firmware_case!(e06_blit_zero_args, "wrong-arg-count", 31, 32, 3, 9);
firmware_case!(e07_builtin_arity, "wrong-arg-count", 36, 37, 3, 14);
firmware_case!(e12_unclosed_paren, "unclosed-delimiter", 35, 36, 3, 13);
firmware_case!(e13_unclosed_string, "unclosed-string", 40, 40, 5, 1);
firmware_case!(
    e14_error_in_second_function,
    "expected-expression",
    81,
    82,
    9,
    8
);
firmware_case!(e18_blit_zero_args, "wrong-arg-count", 30, 31, 3, 7);
firmware_case!(
    k06_lowercase_mod_div_operators,
    "unclosed-delimiter",
    42,
    45,
    3,
    22
);
firmware_case!(
    k07_lowercase_logical_operators,
    "expected-then",
    35,
    36,
    3,
    15
);
firmware_case!(k22_var_upper_mod, "expected-expression", 47, 48, 4, 12);
firmware_case!(
    k30_lowercase_end_assigned_at_statement_start,
    "invalid-top-level",
    65,
    66,
    6,
    4
);
firmware_case!(
    k32_lowercase_div_after_variable,
    "missing-semicolon",
    31,
    34,
    3,
    11
);
firmware_case!(
    k33_title_case_div_operator,
    "missing-semicolon",
    31,
    34,
    3,
    11
);
firmware_case!(k34_title_case_not_operator, "expected-then", 29, 30, 3, 9);
firmware_case!(
    k35_lowercase_end_variable_before_semicolon,
    "invalid-top-level",
    52,
    53,
    5,
    4
);

#[test]
fn syntax_errors_precede_resolution_even_on_earlier_lines() {
    let source =
        include_str!("../../../tests/diagnostics/location-probes/cases/c06_earlier_unknown.ppl");
    let (_, diagnostics) = compile_source(
        &SourceFile::new("syntax-first", source),
        &CompileOptions::default(),
    );
    assert!(diagnostics.has_errors());
    assert_eq!(diagnostics.errors().next().unwrap().code, "expected-then");
    assert!(
        diagnostics
            .items
            .iter()
            .all(|d| d.code != "undeclared-identifier")
    );
}

#[test]
fn excess_arity_skips_nested_commas_and_grouped_callees() {
    let source = "EXPORT LPARGS()\nBEGIN\n RETURN ((SIN)((MIN(1,2)),3));\nEND;\n";
    let comma = source.find(")),3").unwrap() + 2;
    let file = SourceFile::new("nested-call", source);
    let lc = file.line_col(comma as u32);
    assert_error(
        source,
        "wrong-arg-count",
        Span::new(comma, comma + 1),
        lc.line,
        lc.col,
    );
}
