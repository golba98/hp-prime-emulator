//! Maps PPL token classes to egui text formats.
//!
//! Tokenization and classification live in the toolkit-independent
//! [`ppl_compiler::highlight`] module (built on the real lexer); this file
//! only turns its spans into a [`LayoutJob`] with the editor palette.

use egui::text::{LayoutJob, TextFormat};
use egui::{FontId, Stroke};

use ppl_compiler::highlight::{SourceAnalysis, TokenKind, analyze};

/// Editor syntax colors. The palette itself lives in
/// [`crate::theme::syntax`]; the short aliases are kept for callers.
pub mod colors {
    use egui::Color32;

    pub use crate::theme::syntax::*;

    /// Documented commands (`PRINT`, `SOLVE`, `MAXREAL`).
    pub const BUILTIN: Color32 = COMMAND;
    /// Functions defined in the file, including `EXPORT` names.
    pub const FUNCTION: Color32 = USER_FUNCTION;
    pub const EXPORT_NAME: Color32 = USER_FUNCTION;
    /// Variables and unclassified identifiers.
    pub const IDENT: Color32 = TEXT;
}

/// Font metrics for the code editor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CodeStyle {
    pub font_size: f32,
    /// Row height as a multiple of the font size.
    pub line_spacing: f32,
}

impl CodeStyle {
    pub const DEFAULT_FONT_SIZE: f32 = 14.0;
    pub const DEFAULT_LINE_SPACING: f32 = 1.45;

    pub fn new(font_size: f32) -> Self {
        CodeStyle {
            font_size,
            line_spacing: Self::DEFAULT_LINE_SPACING,
        }
    }

    pub fn font(&self) -> FontId {
        FontId::monospace(self.font_size)
    }

    /// Row height in points, rounded so rows land on whole pixels at the
    /// common scale factors.
    pub fn row_height(&self) -> f32 {
        (self.font_size * self.line_spacing).round()
    }
}

impl Default for CodeStyle {
    fn default() -> Self {
        CodeStyle::new(Self::DEFAULT_FONT_SIZE)
    }
}

/// The color of a token class.
pub fn color_of(kind: TokenKind) -> egui::Color32 {
    use TokenKind as K;
    match kind {
        K::Plain | K::Variable => colors::TEXT,
        K::Keyword => colors::KEYWORD,
        K::OperatorKeyword => colors::OPERATOR_KEYWORD,
        K::Command => colors::COMMAND,
        K::CasCommand => colors::CAS_COMMAND,
        K::UserFunction => colors::USER_FUNCTION,
        K::Parameter => colors::PARAMETER,
        K::SystemVariable => colors::SYSTEM_VARIABLE,
        K::Constant => colors::CONSTANT,
        K::Number => colors::NUMBER,
        K::String => colors::STRING,
        K::StringEscape => colors::STRING_ESCAPE,
        K::InvalidString | K::Error => colors::INVALID,
        K::Symbolic => colors::SYMBOLIC,
        K::Comment => colors::COMMENT,
        K::Pragma => colors::PRAGMA,
        K::CasBlock => colors::CAS_BLOCK,
        K::Operator => colors::OPERATOR,
        K::Punctuation => colors::PUNCTUATION,
        K::Bracket => colors::BRACKET,
    }
}

/// The text format of a token class. No synthetic italics: skewed glyphs
/// render soft, and color alone separates the classes.
pub fn format_of(kind: TokenKind, style: &CodeStyle) -> TextFormat {
    let mut f = TextFormat {
        font_id: style.font(),
        color: color_of(kind),
        line_height: Some(style.row_height()),
        ..Default::default()
    };
    if matches!(kind, TokenKind::InvalidString | TokenKind::Error) {
        f.background = colors::INVALID_BG;
        f.underline = Stroke::new(1.0, colors::INVALID);
    }
    f
}

/// Builds the highlighted layout of `source` from an existing analysis.
pub fn layout_job(source: &str, analysis: &SourceAnalysis, style: &CodeStyle) -> LayoutJob {
    let mut job = LayoutJob {
        text: source.to_owned(),
        // Code never wraps: a resize must not re-layout the text.
        wrap: egui::text::TextWrapping::no_max_width(),
        break_on_newline: true,
        ..Default::default()
    };
    job.sections.reserve(analysis.spans.len());
    let mut formats: Vec<(TokenKind, TextFormat)> = Vec::new();
    for span in &analysis.spans {
        let format = match formats.iter().find(|(k, _)| *k == span.kind) {
            Some((_, f)) => f.clone(),
            None => {
                let f = format_of(span.kind, style);
                formats.push((span.kind, f.clone()));
                f
            }
        };
        // Merge runs of the same class into one section.
        if let Some(last) = job.sections.last_mut() {
            if last.byte_range.end == span.start && last.format == format {
                last.byte_range.end = span.end;
                continue;
            }
        }
        job.sections.push(egui::text::LayoutSection {
            leading_space: 0.0,
            byte_range: span.start..span.end,
            format,
        });
    }
    if job.sections.is_empty() && !source.is_empty() {
        job.sections.push(egui::text::LayoutSection {
            leading_space: 0.0,
            byte_range: 0..source.len(),
            format: format_of(TokenKind::Plain, style),
        });
    }
    debug_assert_eq!(
        job.sections.last().map_or(0, |s| s.byte_range.end),
        source.len()
    );
    job
}

/// Builds a syntax-highlighted layout job for `source` (default style).
pub fn highlight(source: &str) -> LayoutJob {
    highlight_with_style(source, &CodeStyle::default())
}

/// [`highlight`] at a specific font size (zoom).
pub fn highlight_with_font(source: &str, font: FontId) -> LayoutJob {
    highlight_with_style(source, &CodeStyle::new(font.size))
}

pub fn highlight_with_style(source: &str, style: &CodeStyle) -> LayoutJob {
    layout_job(source, &analyze(source), style)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Color32;

    fn color_of_text(job: &LayoutJob, src: &str, text: &str) -> Color32 {
        // Sections merge same-class neighbours, so look for the section
        // containing the first occurrence of `text`.
        let at = src
            .find(text)
            .unwrap_or_else(|| panic!("{text:?} not in source"));
        job.sections
            .iter()
            .find(|s| s.byte_range.contains(&at))
            .map(|s| s.format.color)
            .unwrap()
    }

    #[test]
    fn job_covers_source_exactly() {
        for src in [
            "",
            "BEGIN",
            "EXPORT TEST()\nBEGIN\n  // comment\n  PRINT(30);\nEND;\n",
            "// ünïcode comment 🦀\nPRINT(1);",
            "LOCAL x; // EOF comment without newline",
            "PRINT(\"never closed\n2;",
        ] {
            let job = highlight(src);
            assert_eq!(job.text, src);
            let mut at = 0;
            for s in &job.sections {
                assert_eq!(s.byte_range.start, at, "{src:?}");
                at = s.byte_range.end;
            }
            assert_eq!(at, src.len(), "{src:?}");
        }
    }

    #[test]
    fn classes_get_their_palette_colors() {
        let src =
            "EXPORT F(n)\nBEGIN\n  PRINT(\"a\\tb\" + n + 2); // note\n  RETURN solve(x, x);\nEND;";
        let job = highlight(src);
        assert_eq!(color_of_text(&job, src, "EXPORT"), colors::KEYWORD);
        assert_eq!(color_of_text(&job, src, "F("), colors::USER_FUNCTION);
        assert_eq!(color_of_text(&job, src, "PRINT"), colors::COMMAND);
        assert_eq!(color_of_text(&job, src, "\\t"), colors::STRING_ESCAPE);
        assert_eq!(color_of_text(&job, src, "\"a"), colors::STRING);
        assert_eq!(color_of_text(&job, src, " n +"), colors::TEXT);
        assert_eq!(color_of_text(&job, src, "2)"), colors::NUMBER);
        assert_eq!(color_of_text(&job, src, "// note"), colors::COMMENT);
        assert_eq!(color_of_text(&job, src, "solve"), colors::CAS_COMMAND);
        assert!(job.sections.iter().all(|s| !s.format.italics));
    }

    #[test]
    fn unclosed_string_is_marked_invalid_on_its_line_only() {
        let src = "PRINT(\"oops);\nPRINT(2);\n";
        let job = highlight(src);
        let bad = job
            .sections
            .iter()
            .find(|s| s.format.color == colors::INVALID)
            .expect("invalid section");
        assert_eq!(&src[bad.byte_range.clone()], "\"oops);");
        assert!(!bad.format.underline.is_empty());
        assert_eq!(color_of_text(&job, src, "2)"), colors::NUMBER);
    }

    #[test]
    fn rows_have_the_configured_height() {
        let style = CodeStyle::new(14.0);
        let job = highlight_with_style("A\nB", &style);
        assert!(
            job.sections
                .iter()
                .all(|s| s.format.line_height == Some(style.row_height()))
        );
        assert_eq!(style.row_height(), 20.0);
    }
}
