use ppl_syntax::{SourceFile, parse};

#[test]
fn nested_call_punctuation_survives_grouping_and_trivia() {
    let source = "EXPORT T()\nBEGIN\n RETURN (SIN(SIN(1,2) , // note\n 3));\nEND;\n";
    let (program, diagnostics) = parse(source);
    assert!(!diagnostics.has_errors(), "{diagnostics:?}");
    use ppl_syntax::ast::PunctuationKind;
    assert!(
        program
            .punctuation
            .windows(2)
            .all(|tokens| tokens[0].span.end <= tokens[1].span.start)
    );
    let commas: Vec<_> = program
        .punctuation
        .iter()
        .filter(|token| token.kind == PunctuationKind::Comma)
        .map(|token| token.span.start as usize)
        .collect();
    assert_eq!(
        commas,
        source
            .match_indices(',')
            .map(|(i, _)| i)
            .collect::<Vec<_>>()
    );
    for token in &program.punctuation {
        assert!(source.is_char_boundary(token.span.start as usize));
        assert!(source.is_char_boundary(token.span.end as usize));
    }
}

#[test]
fn truncated_unicode_delimiters_use_token_boundaries() {
    for tail in ["(1+2", "SIN(1", "{1,2", "[1,2", "(1,2"] {
        let source = format!("// π 😀\nEXPORT T()\nBEGIN\n RETURN {tail}");
        let (_, diagnostics) = parse(&source);
        assert!(diagnostics.has_errors());
        let file = SourceFile::new("truncated", &source);
        for diagnostic in diagnostics.errors() {
            assert!(source.is_char_boundary(diagnostic.span.start as usize));
            assert!(source.is_char_boundary(diagnostic.span.end as usize));
            assert_eq!(diagnostic.span.start as usize, source.len());
            assert_eq!(diagnostic.span.start, diagnostic.span.end);
            assert_eq!(file.line_col(diagnostic.span.start).line, 4);
        }
    }
}
