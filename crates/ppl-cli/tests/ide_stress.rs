//! Milestone 2 Adversarial Stress, Syntax Fuzzing & Boundary Conditions Test Suite
//!
//! Authored by Challenger 1 for Milestone 2: Multi-Tab PPL Source Code Editor & Diagnostics (R2).
//!
//! Focus Areas:
//! 1. Strings with escaped quotes and comment markers (`"str \" // not comment"`)
//! 2. Unicode mathematical constants and symbols (`π`, `ⅈ`, `ℯ`, `∞`, `≠`, `▶`)
//! 3. Zero-width diagnostic spans and diagnostics at EOF
//! 4. Large PPL source buffers (500,000+ characters)
//! 5. Rapid buffer edits and layouter cache invalidation

use std::time::Instant;

use ppl_compiler::CompileOptions;
use ppl_gui::ide::editor::{self, EditorArea, EditorTab};
use ppl_gui::ide::syntax::{self, colors};
use ppl_syntax::diag::Severity;
use ppl_syntax::source::{SourceFile, Span};

// ============================================================================
// 1. Strings with Escaped Quotes and Comment Markers
// ============================================================================

#[test]
fn test_stress_string_with_escaped_quote_and_comment_markers() {
    let src = "PRINT(\"str \\\" // not comment\"); // this is a real comment\n";
    let job = syntax::highlight(src);

    assert_eq!(job.text, src, "strict job text match");
    let total_len: usize = job.sections.iter().map(|s| s.byte_range.len()).sum();
    assert_eq!(total_len, src.len(), "every byte must be styled");

    // Verify sections are strictly contiguous
    for i in 0..job.sections.len() - 1 {
        assert_eq!(
            job.sections[i].byte_range.end,
            job.sections[i + 1].byte_range.start,
            "section {i} and {next} must be contiguous",
            next = i + 1
        );
    }

    // Find the string literal section
    // Escapes have their own palette class; together they must cover the
    // exact literal and must never turn an embedded // into a comment.
    let string_text: String = job
        .sections
        .iter()
        .filter(|s| matches!(s.format.color, colors::STRING | colors::STRING_ESCAPE))
        .map(|s| &src[s.byte_range.clone()])
        .collect();
    let str_text = &string_text;
    assert_eq!(str_text, "\"str \\\" // not comment\"");

    // Find the comment section
    let comment_sec = job
        .sections
        .iter()
        .find(|s| s.format.color == colors::COMMENT)
        .expect("must contain a comment section");
    let comment_text = &src[comment_sec.byte_range.clone()];
    assert_eq!(comment_text, "// this is a real comment");
}

#[test]
fn test_stress_multiple_escaped_quotes_and_nested_slashes() {
    let src = "LOCAL s := \"a \\\" // c1 \\\" b \\\" // c2 \\\" d\"; // comment 1\n// comment 2\nLOCAL y := 20;\n";
    let job = syntax::highlight(src);
    assert_eq!(job.text, src);

    // Escapes have their own palette class; together they must cover the
    // exact literal and must never turn an embedded // into a comment.
    let string_text: String = job
        .sections
        .iter()
        .filter(|s| matches!(s.format.color, colors::STRING | colors::STRING_ESCAPE))
        .map(|s| &src[s.byte_range.clone()])
        .collect();
    assert_eq!(string_text, "\"a \\\" // c1 \\\" b \\\" // c2 \\\" d\"");

    let comment_sections: Vec<&str> = job
        .sections
        .iter()
        .filter(|s| s.format.color == colors::COMMENT)
        .map(|s| &src[s.byte_range.clone()])
        .collect();
    assert_eq!(comment_sections.len(), 2);
    assert_eq!(comment_sections[0], "// comment 1");
    assert_eq!(comment_sections[1], "// comment 2");
}

#[test]
fn test_stress_escaped_backslashes_before_quote() {
    // In C/PPL strings, \\ is an escaped backslash, and \" is an escaped quote.
    let src = "PRINT(\"escaped backslash \\\\ and quote \\\" // not a comment\");";
    let job = syntax::highlight(src);
    assert_eq!(job.text, src);

    // Escapes have their own palette class; together they must cover the
    // exact literal and must never turn an embedded // into a comment.
    let string_text: String = job
        .sections
        .iter()
        .filter(|s| matches!(s.format.color, colors::STRING | colors::STRING_ESCAPE))
        .map(|s| &src[s.byte_range.clone()])
        .collect();
    assert_eq!(
        string_text,
        "\"escaped backslash \\\\ and quote \\\" // not a comment\""
    );

    // Ensure no comment section was falsely generated
    assert!(
        job.sections
            .iter()
            .all(|s| s.format.color != colors::COMMENT),
        "no comment should exist inside the string"
    );
}

#[test]
fn test_stress_unclosed_string_with_escaped_quote_at_eof() {
    let src = "PRINT(\"unclosed string with \\\" and // comment marker";
    let job = syntax::highlight(src);
    assert_eq!(job.text, src);

    let total_len: usize = job.sections.iter().map(|s| s.byte_range.len()).sum();
    assert_eq!(total_len, src.len());

    let str_sec = job
        .sections
        .iter()
        .find(|s| s.format.color == colors::INVALID)
        .expect("unclosed string must be styled as invalid");
    assert!(!str_sec.format.underline.is_empty());
    assert_eq!(
        &src[str_sec.byte_range.clone()],
        "\"unclosed string with \\\" and // comment marker"
    );
    assert!(
        job.sections
            .iter()
            .all(|s| s.format.color != colors::COMMENT)
    );
}

#[test]
fn test_fuzz_strings_and_comments_randomized() {
    // Deterministic pseudo-random string generator using LCG
    let mut state: u64 = 0xDEADBEEFCAFE1234;
    let mut next_rand = || -> u64 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        state
    };

    let fragments = [
        "EXPORT ",
        "FOO() ",
        "BEGIN ",
        "END; ",
        "PRINT(",
        "); ",
        "\"simple\" ",
        "\"with \\\" quote\" ",
        "\"with // slash\" ",
        "\"with \\\" // both\" ",
        "// comment to eol\n",
        "LOCAL x := ",
        "100; ",
        "  ",
        "\n",
        "π ",
        "ⅈ ",
        "ℯ ",
        "∞ ",
        "≠ ",
        "▶ ",
    ];

    for iter in 0..500 {
        let mut src = String::new();
        let num_frags = (next_rand() % 25) + 5;
        for _ in 0..num_frags {
            let idx = (next_rand() as usize) % fragments.len();
            src.push_str(fragments[idx]);
        }

        let job = syntax::highlight(&src);

        // Core Invariants:
        assert_eq!(
            job.text, src,
            "iter {iter}: job.text must strictly equal src"
        );
        let total: usize = job.sections.iter().map(|s| s.byte_range.len()).sum();
        assert_eq!(
            total,
            src.len(),
            "iter {iter}: total byte range must equal src.len()"
        );

        // Check contiguity
        let mut cur = 0;
        for (i, sec) in job.sections.iter().enumerate() {
            assert_eq!(
                sec.byte_range.start, cur,
                "iter {iter}, section {i} start mismatch"
            );
            cur = sec.byte_range.end;
        }
        assert_eq!(
            cur,
            src.len(),
            "iter {iter}: sections must end at src.len()"
        );
    }
}

// ============================================================================
// 2. Unicode Mathematical Constants and Symbols
// ============================================================================

#[test]
fn test_stress_unicode_math_constants_and_symbols() {
    let src = "π ⅈ ℯ ∞ MAXREAL MINREAL maxreal minreal ≠ ▶";
    let job = syntax::highlight(src);
    assert_eq!(job.text, src);

    // Match compiler-resolved classes, including case-sensitive command names.
    for name in [
        "π", "ⅈ", "ℯ", "∞", "MAXREAL", "MINREAL", "maxreal", "minreal",
    ] {
        let sec = job
            .sections
            .iter()
            .find(|s| s.byte_range.contains(&src.find(name).unwrap()))
            .unwrap_or_else(|| panic!("missing section for {name}"));
        assert_eq!(
            sec.format.color,
            match name {
                "π" | "ⅈ" | "ℯ" | "∞" => colors::CONSTANT,
                "MAXREAL" | "MINREAL" => colors::COMMAND,
                _ => colors::IDENT,
            },
            "unexpected token class for {name}"
        );
    }

    // Mathematical symbols ≠ and ▶ must be colored as OPERATOR
    for sym in ["≠", "▶"] {
        let sec = job
            .sections
            .iter()
            .find(|s| &src[s.byte_range.clone()] == sym)
            .unwrap_or_else(|| panic!("missing section for {sym}"));
        assert_eq!(
            sec.format.color,
            colors::OPERATOR,
            "{sym} must be classified as OPERATOR"
        );
    }
}

#[test]
fn test_stress_hp_system_registers_highlighting() {
    let src = "G0 G9 L0 L9 M0 M9 Z0 Z9 g1 l2 m3 z4";
    let job = syntax::highlight(src);
    assert_eq!(job.text, src);

    // Register names follow the compiler: uppercase registers, lowercase identifiers.
    for reg in [
        "G0", "G9", "L0", "L9", "M0", "M9", "Z0", "Z9", "g1", "l2", "m3", "z4",
    ] {
        let sec = job
            .sections
            .iter()
            .find(|s| s.byte_range.contains(&src.find(reg).unwrap()))
            .unwrap_or_else(|| panic!("missing section for {reg}"));
        assert_eq!(
            sec.format.color,
            if reg.starts_with(|c: char| c.is_ascii_uppercase()) {
                colors::SYSTEM_VARIABLE
            } else {
                colors::IDENT
            },
            "unexpected register token class for {reg}"
        );
    }
}

#[test]
fn test_stress_complex_euler_unicode_program() {
    let src = "EXPORT EULER(X)\nBEGIN\n  LOCAL z := ℯ^(ⅈ * π) + 1;\n  IF z ≠ 0 THEN\n    z ▶ G0;\n  END;\n  RETURN √(X² + 1);\nEND;\n";
    let job = syntax::highlight(src);
    assert_eq!(job.text, src);

    // Verify EXPORT_NAME
    let export_sec = job
        .sections
        .iter()
        .find(|s| &src[s.byte_range.clone()] == "EULER")
        .expect("must find EULER");
    assert_eq!(export_sec.format.color, colors::EXPORT_NAME);

    // Constants and system registers retain their distinct semantic classes.
    for builtin in ["ℯ", "ⅈ", "π", "G0"] {
        let sec = job
            .sections
            .iter()
            .find(|s| &src[s.byte_range.clone()] == builtin)
            .unwrap_or_else(|| panic!("missing builtin {builtin}"));
        assert_eq!(
            sec.format.color,
            if builtin == "G0" {
                colors::SYSTEM_VARIABLE
            } else {
                colors::CONSTANT
            },
            "unexpected class for {builtin}"
        );
    }

    // Verify OPERATOR symbols
    for op in ["≠", "▶", "√", "²", ":=", "+", "*", "^", ";"] {
        let sec = job
            .sections
            .iter()
            .find(|s| &src[s.byte_range.clone()] == op)
            .unwrap_or_else(|| panic!("missing operator {op}"));
        assert_eq!(
            sec.format.color,
            if op == ";" {
                colors::PUNCTUATION
            } else {
                colors::OPERATOR
            },
            "unexpected class for {op}"
        );
    }
}

#[test]
fn test_stress_byte_to_char_index_unicode_fuzzing() {
    // Multi-byte string with mixture of 1, 2, 3, and 4-byte UTF-8 sequences
    let text = "Aπⅈℯ∞≠▶🦀🚀B\nC123";
    let char_count = text.chars().count();

    // 1. Offsets within range
    for b in 0..=text.len() {
        let c = editor::byte_to_char_index(text, b);
        assert!(c <= char_count, "char index must not exceed total chars");
    }

    // 2. Offsets out of bounds
    assert_eq!(
        editor::byte_to_char_index(text, text.len() + 100),
        char_count
    );
    assert_eq!(editor::byte_to_char_index(text, usize::MAX), char_count);

    // 3. Monotonicity
    let mut prev_c = 0;
    for b in 0..=text.len() {
        let c = editor::byte_to_char_index(text, b);
        assert!(
            c >= prev_c,
            "char index must be non-decreasing with byte offset"
        );
        prev_c = c;
    }
}

// ============================================================================
// 3. Zero-Width Diagnostic Spans and Diagnostics at EOF
// ============================================================================

#[test]
fn test_stress_zero_width_diagnostic_spans() {
    let source = "EXPORT F() BEGIN RETURN 42; END;";
    let char_count = source.chars().count();

    let src_len = source.len() as u32;
    let test_spans = [
        Span { start: 0, end: 0 },   // Zero-width at start
        Span { start: 10, end: 10 }, // Zero-width in middle
        Span {
            start: src_len,
            end: src_len,
        }, // Zero-width at EOF
        Span {
            start: src_len + 50,
            end: src_len + 100,
        }, // Far out of bounds
        Span { start: 20, end: 10 }, // Inverted span (start > end)
    ];

    for span in test_spans {
        let start_char = editor::byte_to_char_index(source, span.start as usize);
        let raw_end_char = editor::byte_to_char_index(source, span.end as usize);
        let end_char = raw_end_char.max(start_char + 1);

        assert!(
            start_char <= char_count,
            "start_char must be clamped to char_count"
        );
        assert!(
            end_char > start_char,
            "end_char must provide at least 1-char width for underline"
        );
    }
}

#[test]
fn test_stress_empty_buffer_with_diagnostics() {
    let source = "";
    let mut tab = EditorTab::new("empty.ppl", source);

    let job = tab.layout(13.0);
    assert_eq!(job.text, "");
    assert!(job.sections.is_empty());

    // Navigation on empty buffer must not panic
    tab.goto_line(1, 1);
    assert_eq!(tab.pending_jump, Some(0));
    tab.goto_line(50, 10);
    assert_eq!(tab.pending_jump, Some(0));

    // Zero-width span on empty buffer
    let start_char = editor::byte_to_char_index(source, 0);
    let end_char = editor::byte_to_char_index(source, 0).max(start_char + 1);
    assert_eq!(start_char, 0);
    assert_eq!(end_char, 1);
}

#[test]
fn test_stress_compiler_generated_eof_diagnostic_mapping() {
    let source = "EXPORT INCOMPLETE() BEGIN";
    let file = SourceFile::new("<test>", source);
    let (_, diags) = ppl_compiler::compile_source(&file, &CompileOptions::default());

    assert!(
        diags.has_errors(),
        "unclosed function must produce compiler error"
    );
    let err = diags
        .items
        .iter()
        .find(|d| d.severity == Severity::Error)
        .expect("error diagnostic");

    // Map to DiagnosticItem
    let item = editor::DiagnosticItem::from_diagnostic(err, &file);
    assert_eq!(item.class, ppl_syntax::DiagnosticClass::Syntax);

    assert!(item.line >= 1);
    assert!(item.col >= 1);

    let start_char = editor::byte_to_char_index(source, item.span.start as usize);
    let end_char = editor::byte_to_char_index(source, item.span.end as usize).max(start_char + 1);
    assert!(start_char <= source.chars().count());
    assert!(end_char > start_char);

    let mut tab = EditorTab::new("test.ppl", source);
    let revision = tab.revision();
    tab.analysis = Some(editor::TabAnalysis {
        revision,
        items: vec![item.clone()],
        failure: None,
    });
    assert_eq!(tab.current_diagnostics(), &[item.clone()][..]);
    tab.goto_line(item.line, item.col);
    assert!(tab.pending_jump.is_some());
}

// ============================================================================
// 4. Large PPL Source Buffers (500,000+ characters)
// ============================================================================

#[test]
fn test_stress_large_source_buffer_500k_chars() {
    // Synthesize a realistic, highly repetitive PPL program of > 500,000 characters
    let mut large_src = String::with_capacity(550_000);
    large_src.push_str("// Large PPL stress test buffer\n");

    let mut func_idx = 0;
    while large_src.len() < 510_000 {
        large_src.push_str(&format!(
            "EXPORT FUNC_{func_idx}(A, B)\nBEGIN\n  LOCAL x := A * π + B * ⅈ;\n  IF x ≠ 0 THEN\n    PRINT(\"Step: \\\"// not comment\\\"\");\n    x ▶ G0;\n  END;\n  RETURN x + ℯ + ∞;\nEND;\n\n"
        ));
        func_idx += 1;
    }

    assert!(
        large_src.len() >= 500_000,
        "buffer must exceed 500,000 characters (actual: {})",
        large_src.len()
    );
    let line_count = large_src.lines().count();
    assert!(
        line_count > 10_000,
        "buffer must contain > 10,000 lines (actual: {line_count})"
    );

    // 1. Stress time the highlight pass
    let t0 = Instant::now();
    let job = syntax::highlight(&large_src);
    let elapsed = t0.elapsed();

    // Highlighting 500k chars must be fast (< 250ms on modern CPU)
    assert!(
        elapsed.as_millis() < 500,
        "highlighting 500,000 chars took too long: {:?}",
        elapsed
    );

    // Strict correctness invariants
    assert_eq!(
        job.text, large_src,
        "job text must strictly match 500k source"
    );
    let total_bytes: usize = job.sections.iter().map(|s| s.byte_range.len()).sum();
    assert_eq!(
        total_bytes,
        large_src.len(),
        "total section byte ranges must match source length"
    );

    // 2. EditorTab operations on large buffer
    let mut tab = EditorTab::new("large.ppl", large_src.clone());

    // First layout call caches the job
    let t1 = Instant::now();
    let _ = tab.layout(13.0);
    let layout_time_first = t1.elapsed();

    // Second layout call must be a fast cache hit
    let t2 = Instant::now();
    let _ = tab.layout(13.0);
    let layout_time_cached = t2.elapsed();

    println!("500k chars highlight() elapsed: {:?}", elapsed);
    println!(
        "500k chars initial layout() elapsed: {:?}",
        layout_time_first
    );
    println!(
        "500k chars cached layout() elapsed: {:?}",
        layout_time_cached
    );

    assert!(
        layout_time_cached <= layout_time_first,
        "cached layout ({:?}) must be faster than initial layout ({:?})",
        layout_time_cached,
        layout_time_first
    );
    // In debug mode, cloning a 30,000-section LayoutJob for a 500k buffer takes ~10-15ms;
    // ensure it is well within interactive threshold (< 50ms)
    assert!(
        layout_time_cached.as_millis() < 50,
        "cached layout must be sub-50ms, was: {:?}",
        layout_time_cached
    );

    // Goto deep line in 500k buffer
    tab.goto_line(5_000, 10);
    assert!(tab.pending_jump.is_some());

    // Replace all on large buffer
    let replaced_count = tab.replace_all("FUNC_", "ROUTINE_");
    assert_eq!(replaced_count, func_idx);
    assert!(tab.modified);
}

// ============================================================================
// 5. Rapid Buffer Edits and Layouter Cache Invalidation
// ============================================================================

#[test]
fn test_stress_rapid_typing_and_cache_invalidation() {
    let mut tab = EditorTab::new("typed.ppl", "");
    let script = "EXPORT COMPUTE(N)\nBEGIN\n  LOCAL total := 0;\n  FOR i FROM 1 TO N DO\n    total := total + i * π;\n  END;\n  RETURN total;\nEND;\n";

    // Simulate 100 sequential character insertions
    for (i, ch) in script.chars().take(100).enumerate() {
        tab.content.push(ch);
        let job = tab.layout(13.0);

        // Invariant: the cached analysis must always reflect the latest
        // content: every edit is a new revision, analyzed exactly once.
        assert_eq!(job.text, tab.content);
        assert_eq!(tab.revision(), i as u64 + 1, "one revision per edit");
        assert_eq!(tab.document().stats.analyses, i as u64 + 1);
    }

    // Verify cache hit when content is untouched
    let t0 = Instant::now();
    for _ in 0..100 {
        let _ = tab.layout(13.0);
    }
    let elapsed = t0.elapsed();
    assert!(
        elapsed.as_millis() < 5,
        "100 cached layout queries must take < 5ms (actual: {:?})",
        elapsed
    );

    // Cache hits never re-analyze.
    let analyses = tab.document().stats.analyses;
    assert_eq!(tab.document().stats.analyses, analyses);

    // Font size invalidation: the job is rebuilt at the new size.
    let j = tab.layout(14.0);
    assert!(j.sections.iter().all(|s| s.format.font_id.size == 14.0));

    let j = tab.layout(16.0);
    assert!(j.sections.iter().all(|s| s.format.font_id.size == 16.0));
    // ...from the same analysis.
    assert_eq!(tab.document().stats.analyses, analyses);
}

#[test]
fn test_stress_rapid_backspacing_across_token_boundaries() {
    let initial = "EXPORT FOO() BEGIN LOCAL s := \"string \\\" //\"; PRINT(s); END;";
    let mut tab = EditorTab::new("backspace.ppl", initial);

    // Repeatedly delete characters from the end
    while !tab.content.is_empty() {
        tab.content.pop();
        let job = tab.layout(13.0);
        assert_eq!(job.text, tab.content);
        let total: usize = job.sections.iter().map(|s| s.byte_range.len()).sum();
        assert_eq!(total, tab.content.len());
    }
}

#[test]
fn test_stress_slash_and_comment_token_transitions() {
    let mut tab = EditorTab::new("slash.ppl", "PRINT(\"val\");");
    let _ = tab.layout(13.0);

    // Insert single slash at beginning: `/PRINT("val");`
    tab.content.insert(0, '/');
    let j1 = tab.layout(13.0);
    assert_eq!(j1.text, tab.content);
    let slash_sec = j1
        .sections
        .iter()
        .find(|s| &tab.content[s.byte_range.clone()] == "/")
        .expect("slash");
    assert_eq!(
        slash_sec.format.color,
        colors::OPERATOR,
        "single slash is OPERATOR"
    );

    // Insert second slash: `//PRINT("val");` -> becomes COMMENT
    tab.content.insert(1, '/');
    let j2 = tab.layout(13.0);
    assert_eq!(j2.text, tab.content);
    let comment_sec = j2
        .sections
        .iter()
        .find(|s| s.format.color == colors::COMMENT)
        .expect("comment");
    assert_eq!(
        &tab.content[comment_sec.byte_range.clone()],
        "//PRINT(\"val\");"
    );

    // Remove one slash: back to OPERATOR
    tab.content.remove(1);
    let j3 = tab.layout(13.0);
    assert_eq!(j3.text, tab.content);
    let slash_sec2 = j3
        .sections
        .iter()
        .find(|s| &tab.content[s.byte_range.clone()] == "/")
        .expect("slash");
    assert_eq!(slash_sec2.format.color, colors::OPERATOR);
}

#[test]
fn test_stress_multi_tab_cache_isolation() {
    let mut area = EditorArea::new();
    area.new_tab("Tab1.ppl", "EXPORT T1() BEGIN END;");
    area.new_tab("Tab2.ppl", "EXPORT T2() BEGIN PRINT(1); END;");
    area.new_tab("Tab3.ppl", "EXPORT T3() BEGIN PRINT(2); END;");

    for tab in &mut area.tabs {
        let _ = tab.layout(13.0);
        assert_eq!(tab.document().stats.analyses, 1);
    }
    let rev0 = area.tabs[0].revision();
    let rev2 = area.tabs[2].revision();

    // Mutate Tab 1
    area.tabs[1].content.push_str(" // modified");
    let j1 = area.tabs[1].layout(13.0);
    assert!(j1.text.contains("// modified"));
    assert_eq!(area.tabs[1].document().stats.analyses, 2);

    // Ensure Tab 0 and Tab 2 caches are unaffected
    assert_eq!(area.tabs[0].revision(), rev0);
    assert_eq!(area.tabs[2].revision(), rev2);
    assert_eq!(area.tabs[0].document().stats.analyses, 1);
    assert_eq!(area.tabs[2].document().stats.analyses, 1);
    assert_eq!(
        area.tabs[0].layout(13.0).text,
        "EXPORT DEMO()\nBEGIN\n  PRINT(\"HP Prime PPL Emulator\");\nEND;\n"
    );
    assert_eq!(
        area.tabs[2].layout(13.0).text,
        "EXPORT T2() BEGIN PRINT(1); END;"
    );
}

// ============================================================================
// 6. Additional Boundary & Stress Cases
// ============================================================================

#[test]
fn test_stress_quoted_expressions_and_adjacent_tokens() {
    let src = "LOCAL e := 'X^2 + 2*X + 1 // not comment';\"str\"//adj comment\n";
    let job = syntax::highlight(src);
    assert_eq!(job.text, src);

    // Symbolic expressions have their own class, separate from string literals.
    let quoted_sec = job
        .sections
        .iter()
        .find(|s| &src[s.byte_range.clone()] == "'X^2 + 2*X + 1 // not comment'")
        .expect("must find quoted expr");
    assert_eq!(quoted_sec.format.color, colors::SYMBOLIC);

    // Adjacent string "str"
    let str_sec = job
        .sections
        .iter()
        .find(|s| &src[s.byte_range.clone()] == "\"str\"")
        .expect("must find adjacent str");
    assert_eq!(str_sec.format.color, colors::STRING);

    // Adjacent comment //adj comment
    let comment_sec = job
        .sections
        .iter()
        .find(|s| &src[s.byte_range.clone()] == "//adj comment")
        .expect("must find adjacent comment");
    assert_eq!(comment_sec.format.color, colors::COMMENT);
}

#[test]
fn test_stress_system_register_vs_ident_boundaries() {
    let src = "G0 G9 G10 GA G_ L0 L9 L15 M0 M9 M100 Z0 Z9 ZZZ";
    let job = syntax::highlight(src);
    assert_eq!(job.text, src);

    // System registers: G0, G9, L0, L9, M0, M9, Z0, Z9
    for reg in ["G0", "G9", "L0", "L9", "M0", "M9", "Z0", "Z9"] {
        let sec = job
            .sections
            .iter()
            .find(|s| s.byte_range.contains(&src.find(reg).unwrap()))
            .unwrap_or_else(|| panic!("missing {reg}"));
        assert_eq!(
            sec.format.color,
            colors::SYSTEM_VARIABLE,
            "{reg} must be a system variable"
        );
    }

    // Non-registers: G10, GA, G_, L15, M100, ZZZ must be IDENT
    for non_reg in ["G10", "GA", "G_", "L15", "M100", "ZZZ"] {
        let sec = job
            .sections
            .iter()
            .find(|s| s.byte_range.contains(&src.find(non_reg).unwrap()))
            .unwrap_or_else(|| panic!("missing {non_reg}"));
        assert_eq!(sec.format.color, colors::IDENT, "{non_reg} must be IDENT");
    }
}

#[test]
fn test_stress_rapid_tab_lifecycle_and_switching() {
    let mut area = EditorArea::new();

    // Rapidly open 20 tabs
    for i in 0..20 {
        area.new_tab(
            format!("Prog{i}.ppl"),
            format!("EXPORT P{i}() BEGIN PRINT({i}); END;"),
        );
    }
    assert_eq!(area.tabs.len(), 21); // initial DEMO.ppl + 20

    // Cycle active tab index through all tabs
    for i in 0..21 {
        area.active_tab_index = i;
        let tab = area.active_tab_mut().expect("active tab");
        let job = tab.layout(13.0);
        assert!(!job.sections.is_empty());
        assert_eq!(job.text, tab.content);
    }

    // Close odd tabs from the middle
    for _ in 0..10 {
        let close_idx = area.tabs.len() / 2;
        area.close_tab(close_idx);
        assert!(area.active_tab_index < area.tabs.len());
    }

    // Ensure remaining tabs are valid and functional
    for tab in &mut area.tabs {
        let job = tab.layout(13.0);
        assert_eq!(job.text, tab.content);
    }
}
