//! Per-buffer derived data, recomputed only when the text changes:
//! language analysis, the line index and the laid-out galley.

use std::ops::Range;
use std::sync::Arc;

use egui::Galley;
use egui::text::LayoutJob;
use ppl_compiler::highlight::{SourceAnalysis, analyze};

use crate::ide::syntax::{self, CodeStyle};

/// Line starts of a text, in bytes and in characters, for O(log n)
/// conversions between egui's char cursors, byte spans and line/column.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineIndex {
    start_bytes: Vec<usize>,
    start_chars: Vec<usize>,
    len_bytes: usize,
    len_chars: usize,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut start_bytes = vec![0];
        let mut start_chars = vec![0];
        let mut chars = 0;
        for (b, c) in text.char_indices() {
            chars += 1;
            if c == '\n' {
                start_bytes.push(b + 1);
                start_chars.push(chars);
            }
        }
        LineIndex {
            start_bytes,
            start_chars,
            len_bytes: text.len(),
            len_chars: chars,
        }
    }

    /// Number of lines (an empty text and a trailing newline both count).
    pub fn line_count(&self) -> usize {
        self.start_chars.len()
    }

    pub fn len_chars(&self) -> usize {
        self.len_chars
    }

    /// 0-based line containing char index `c` (clamped).
    pub fn line_of_char(&self, c: usize) -> usize {
        let c = c.min(self.len_chars);
        self.start_chars.partition_point(|&s| s <= c) - 1
    }

    /// 0-based line containing byte offset `b` (clamped).
    pub fn line_of_byte(&self, b: usize) -> usize {
        let b = b.min(self.len_bytes);
        self.start_bytes.partition_point(|&s| s <= b) - 1
    }

    /// 1-based (line, column) of char index `c`; columns count characters.
    pub fn line_col(&self, c: usize) -> (usize, usize) {
        let c = c.min(self.len_chars);
        let line = self.line_of_char(c);
        (line + 1, c - self.start_chars[line] + 1)
    }

    /// Char range of 0-based `line`, excluding its newline.
    pub fn line_chars(&self, line: usize) -> Range<usize> {
        let line = line.min(self.line_count() - 1);
        let start = self.start_chars[line];
        let end = self
            .start_chars
            .get(line + 1)
            .map_or(self.len_chars, |&next| next - 1);
        start..end
    }

    /// Byte range of 0-based `line`, excluding its newline.
    pub fn line_bytes(&self, line: usize) -> Range<usize> {
        let line = line.min(self.line_count() - 1);
        let start = self.start_bytes[line];
        let end = self
            .start_bytes
            .get(line + 1)
            .map_or(self.len_bytes, |&next| next - 1);
        start..end
    }

    /// Char index of 1-based `line` / `col`, clamped to the text and to the
    /// end of the line.
    pub fn char_of_line_col(&self, line: usize, col: usize) -> usize {
        let range = self.line_chars(line.max(1).min(self.line_count()) - 1);
        range
            .start
            .saturating_add(col.saturating_sub(1))
            .min(range.end)
    }

    /// Char index of byte offset `b` in `text` (which must be the indexed
    /// text). Mid-character offsets clamp down to the character start.
    pub fn char_of_byte(&self, text: &str, b: usize) -> usize {
        let mut b = b.min(self.len_bytes);
        while b > 0 && !text.is_char_boundary(b) {
            b -= 1;
        }
        let line = self.line_of_byte(b);
        self.start_chars[line] + text[self.start_bytes[line]..b].chars().count()
    }

    /// Byte offset of char index `c` in `text` (the indexed text).
    pub fn byte_of_char(&self, text: &str, c: usize) -> usize {
        let c = c.min(self.len_chars);
        let line = self.line_of_char(c);
        let start = self.start_bytes[line];
        text[start..]
            .char_indices()
            .nth(c - self.start_chars[line])
            .map_or(self.len_bytes, |(i, _)| start + i)
    }
}

#[derive(Clone)]
struct GalleyEntry {
    revision: u64,
    style: CodeStyle,
    pixels_per_point: f32,
    galley: Arc<Galley>,
}

/// Counters that let tests and benchmarks prove caching works.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub analyses: u64,
    pub layouts: u64,
}

/// Derived data for one buffer, keyed by a revision counter that advances
/// whenever the text actually differs from the last analyzed snapshot.
#[derive(Clone, Default)]
pub struct DocumentCache {
    text: String,
    revision: u64,
    synced: bool,
    analysis: Arc<SourceAnalysis>,
    lines: Arc<LineIndex>,
    job: Option<(u64, CodeStyle, Arc<LayoutJob>)>,
    galley: Option<GalleyEntry>,
    pub stats: CacheStats,
}

impl std::fmt::Debug for DocumentCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocumentCache")
            .field("revision", &self.revision)
            .field("len", &self.text.len())
            .field("stats", &self.stats)
            .finish()
    }
}

impl DocumentCache {
    /// Brings the cache up to date with `text`. Returns true when the text
    /// changed (and the revision advanced). Cost when unchanged: one
    /// memory comparison.
    pub fn sync(&mut self, text: &str) -> bool {
        if self.synced && self.text == text {
            return false;
        }
        self.text.clear();
        self.text.push_str(text);
        self.revision += 1;
        self.synced = true;
        self.analysis = Arc::new(analyze(text));
        self.lines = Arc::new(LineIndex::new(text));
        self.stats.analyses += 1;
        true
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Language analysis of the last synced text.
    pub fn analysis(&self) -> &Arc<SourceAnalysis> {
        &self.analysis
    }

    /// Line index of the last synced text.
    pub fn lines(&self) -> &Arc<LineIndex> {
        &self.lines
    }

    /// The highlighted, laid-out text, reusing the previous galley when the
    /// text, style and scale are unchanged (window resizes never invalidate
    /// it: code is laid out without wrapping).
    /// The highlighted layout job of `text`, cached per revision and style.
    pub fn job(&mut self, text: &str, style: CodeStyle) -> Arc<LayoutJob> {
        self.sync(text);
        if let Some((rev, st, job)) = &self.job {
            if *rev == self.revision && *st == style {
                return job.clone();
            }
        }
        let job = Arc::new(syntax::layout_job(text, &self.analysis, &style));
        self.job = Some((self.revision, style, job.clone()));
        job
    }

    pub fn galley(&mut self, ctx: &egui::Context, text: &str, style: CodeStyle) -> Arc<Galley> {
        self.sync(text);
        let ppp = ctx.pixels_per_point();
        if let Some(g) = &self.galley {
            if g.revision == self.revision && g.style == style && g.pixels_per_point == ppp {
                return g.galley.clone();
            }
        }
        let job = (*self.job(text, style)).clone();
        let galley = ctx.fonts(|f| f.layout_job(job));
        self.stats.layouts += 1;
        self.galley = Some(GalleyEntry {
            revision: self.revision,
            style,
            pixels_per_point: ppp,
            galley: galley.clone(),
        });
        galley
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_index_conversions() {
        // "aπ\n\nbc" : chars a π \n \n b c ; bytes a(1) π(2) \n \n b c
        let text = "aπ\n\nbc";
        let ix = LineIndex::new(text);
        assert_eq!(ix.line_count(), 3);
        assert_eq!(ix.len_chars(), 6);
        assert_eq!(ix.line_col(0), (1, 1));
        assert_eq!(ix.line_col(2), (1, 3)); // on the newline
        assert_eq!(ix.line_col(3), (2, 1));
        assert_eq!(ix.line_col(4), (3, 1));
        assert_eq!(ix.line_col(99), (3, 3));
        assert_eq!(ix.line_chars(0), 0..2);
        assert_eq!(ix.line_chars(1), 3..3);
        assert_eq!(ix.line_chars(2), 4..6);
        assert_eq!(ix.line_bytes(0), 0..3);
        assert_eq!(ix.char_of_line_col(3, 2), 5);
        assert_eq!(ix.char_of_line_col(1, 999), 2);
        assert_eq!(ix.char_of_line_col(0, 0), 0);
        assert_eq!(ix.char_of_line_col(usize::MAX, usize::MAX), 6);
        assert_eq!(ix.char_of_byte(text, 2), 1); // inside π
        assert_eq!(ix.char_of_byte(text, 3), 2);
        assert_eq!(ix.char_of_byte(text, 5), 4);
        assert_eq!(ix.byte_of_char(text, 2), 3);
        assert_eq!(ix.byte_of_char(text, 6), 7);
    }

    #[test]
    fn line_index_trailing_newline_and_empty() {
        let ix = LineIndex::new("");
        assert_eq!((ix.line_count(), ix.line_col(0)), (1, (1, 1)));
        let ix = LineIndex::new("x\n");
        assert_eq!(ix.line_count(), 2);
        assert_eq!(ix.line_col(2), (2, 1));
    }

    #[test]
    fn cache_reanalyzes_only_on_change() {
        let mut d = DocumentCache::default();
        assert!(d.sync("EXPORT F() BEGIN END;"));
        assert!(!d.sync("EXPORT F() BEGIN END;"));
        assert_eq!(d.stats.analyses, 1);
        assert!(d.sync("EXPORT F() BEGIN END; "));
        assert_eq!((d.stats.analyses, d.revision()), (2, 2));
    }

    #[test]
    fn galley_is_reused_until_text_style_or_scale_changes() {
        let ctx = egui::Context::default();
        // Fonts are only available inside a frame.
        let _ = ctx.run(Default::default(), |_| {});
        let mut d = DocumentCache::default();
        let style = CodeStyle::new(14.0);
        let g1 = d.galley(&ctx, "A\nB", style);
        let g2 = d.galley(&ctx, "A\nB", style);
        assert!(Arc::ptr_eq(&g1, &g2));
        assert_eq!(d.stats.layouts, 1);
        d.galley(&ctx, "A\nBC", style);
        d.galley(&ctx, "A\nBC", CodeStyle::new(16.0));
        assert_eq!(d.stats.layouts, 3);
        assert_eq!(g1.rows.len(), 2);
    }
}
