//! Text search for the editor's find/replace bar. All offsets are byte
//! offsets into the buffer; the widget converts them to char cursors.

use std::ops::Range;

/// What to search for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindQuery {
    pub needle: String,
    pub match_case: bool,
    pub whole_word: bool,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl FindQuery {
    /// Every match in `text`, in order, non-overlapping.
    pub fn matches(&self, text: &str) -> Vec<Range<usize>> {
        if self.needle.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        if self.match_case {
            for (i, m) in text.match_indices(self.needle.as_str()) {
                out.push(i..i + m.len());
            }
        } else {
            // Case-insensitive without changing byte offsets: compare
            // char by char with simple case folding.
            let needle: Vec<char> = self.needle.chars().flat_map(char::to_lowercase).collect();
            let chars: Vec<(usize, char)> = text.char_indices().collect();
            let mut i = 0;
            while i < chars.len() {
                let mut k = 0;
                let mut j = i;
                while k < needle.len() && j < chars.len() {
                    let mut lc = chars[j].1.to_lowercase();
                    match (lc.next(), lc.next()) {
                        (Some(c), None) if c == needle[k] => {
                            k += 1;
                            j += 1;
                        }
                        _ => break,
                    }
                }
                if k == needle.len() {
                    let end = chars.get(j).map_or(text.len(), |&(b, _)| b);
                    out.push(chars[i].0..end);
                    i = j.max(i + 1);
                } else {
                    i += 1;
                }
            }
        }
        if self.whole_word {
            out.retain(|r| {
                let before = text[..r.start].chars().next_back();
                let after = text[r.end..].chars().next();
                !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char)
            });
        }
        out
    }

    /// The next match starting at or after `from` (wrapping around), or
    /// the previous one ending before `from` when `backwards`.
    pub fn find_from(&self, text: &str, from: usize, backwards: bool) -> Option<Range<usize>> {
        let all = self.matches(text);
        if backwards {
            all.iter()
                .rev()
                .find(|r| r.end <= from && r.start < from)
                .or(all.last())
                .cloned()
        } else {
            all.iter()
                .find(|r| r.start >= from)
                .or(all.first())
                .cloned()
        }
    }

    /// Replaces every match; returns the new text and the count.
    pub fn replace_all(&self, text: &str, replacement: &str) -> (String, usize) {
        let all = self.matches(text);
        let mut out = String::with_capacity(text.len());
        let mut at = 0;
        for r in &all {
            out.push_str(&text[at..r.start]);
            out.push_str(replacement);
            at = r.end;
        }
        out.push_str(&text[at..]);
        (out, all.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(needle: &str, match_case: bool, whole_word: bool) -> FindQuery {
        FindQuery {
            needle: needle.into(),
            match_case,
            whole_word,
        }
    }

    #[test]
    fn case_and_word_options() {
        let text = "PRINT(x); print(X); PRINTER;";
        assert_eq!(q("PRINT", true, false).matches(text).len(), 2);
        assert_eq!(q("print", false, false).matches(text).len(), 3);
        assert_eq!(q("print", false, true).matches(text), vec![0..5, 10..15]);
        assert!(q("", false, false).matches(text).is_empty());
    }

    #[test]
    fn unicode_offsets_are_byte_exact() {
        let text = "π := Π + π;";
        let m = q("π", false, false).matches(text);
        assert_eq!(m.len(), 3);
        for r in m {
            assert_eq!(text[r].to_lowercase(), "π");
        }
    }

    #[test]
    fn find_wraps_in_both_directions() {
        let text = "a b a b a";
        let query = q("a", true, false);
        assert_eq!(query.find_from(text, 1, false), Some(4..5));
        assert_eq!(query.find_from(text, 9, false), Some(0..1));
        assert_eq!(query.find_from(text, 4, true), Some(0..1));
        assert_eq!(query.find_from(text, 0, true), Some(8..9));
        assert_eq!(q("z", true, false).find_from(text, 0, false), None);
    }

    #[test]
    fn replace_all_counts() {
        let (out, n) = q("print", false, true).replace_all("PRINT(1); Print(2); PRINTS;", "DISP");
        assert_eq!((out.as_str(), n), ("DISP(1); DISP(2); PRINTS;", 2));
    }
}
