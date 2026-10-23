//! Pure, UI-free editing operations on a buffer and a char-index
//! selection. The editor widget routes keystrokes here *before* egui's
//! `TextEdit` sees them, so every operation is applied once, atomically,
//! and can be recorded as a single undo step.

use std::ops::Range;

use ppl_syntax::Diagnostics;
use ppl_syntax::lexer::{Keyword, Tok, lex};

/// One indentation step (PPL convention in the HP Connectivity Kit).
pub const INDENT: &str = "  ";
const INDENT_WIDTH: usize = 2;

fn byte_of(text: &str, c: usize) -> usize {
    text.char_indices().nth(c).map_or(text.len(), |(b, _)| b)
}

fn char_at(text: &str, c: usize) -> Option<char> {
    text.chars().nth(c)
}

/// Char index of the start of the line containing char `c`.
fn line_start(text: &str, c: usize) -> usize {
    let b = byte_of(text, c);
    text[..b]
        .rfind('\n')
        .map_or(0, |i| text[..=i].chars().count())
}

/// Char index of the end (before `\n`) of the line containing char `c`.
fn line_end(text: &str, c: usize) -> usize {
    let b = byte_of(text, c);
    match text[b..].find('\n') {
        Some(i) => c + text[b..b + i].chars().count(),
        None => c + text[b..].chars().count(),
    }
}

/// Char indices of the starts of every line touched by `sel`. A selection
/// ending at column 0 does not include that last line.
fn touched_line_starts(text: &str, sel: &Range<usize>) -> Vec<usize> {
    let first = line_start(text, sel.start);
    let last_char = if sel.end > sel.start && line_start(text, sel.end) == sel.end {
        sel.end - 1
    } else {
        sel.end
    };
    let mut starts = vec![first];
    let mut c = first;
    for ch in text.chars().skip(first) {
        c += 1;
        if c > last_char {
            break;
        }
        if ch == '\n' {
            starts.push(c);
        }
    }
    starts
}

fn leading_ws(line: &str) -> &str {
    let n = line.len() - line.trim_start_matches([' ', '\t']).len();
    &line[..n]
}

fn line_text(text: &str, c: usize) -> &str {
    let (s, e) = (
        byte_of(text, line_start(text, c)),
        byte_of(text, line_end(text, c)),
    );
    &text[s..e]
}

/// Tab: with a multi-line selection (or one spanning a whole line), indent
/// every touched line; otherwise replace the selection by spaces up to the
/// next indentation stop. Returns the new selection.
pub fn indent(text: &mut String, sel: Range<usize>) -> Range<usize> {
    let starts = touched_line_starts(text, &sel);
    if starts.len() == 1 && !(sel.start == starts[0] && sel.end > sel.start) {
        let b0 = byte_of(text, sel.start);
        let b1 = byte_of(text, sel.end);
        let col = sel.start - starts[0];
        let pad = INDENT_WIDTH - col % INDENT_WIDTH;
        text.replace_range(b0..b1, &" ".repeat(pad));
        let c = sel.start + pad;
        return c..c;
    }
    for &s in starts.iter().rev() {
        let b = byte_of(text, s);
        text.insert_str(b, INDENT);
    }
    let added = starts.len() * INDENT_WIDTH;
    let start = if sel.start == starts[0] {
        sel.start
    } else {
        sel.start + INDENT_WIDTH
    };
    start..sel.end + added
}

/// Shift+Tab: remove one indentation step (spaces, or one tab) from every
/// touched line. Returns the new selection.
pub fn dedent(text: &mut String, sel: Range<usize>) -> Range<usize> {
    let starts = touched_line_starts(text, &sel);
    let mut removed_before_start = 0;
    let mut removed_total = 0;
    for (i, &s) in starts.iter().enumerate().rev() {
        let b = byte_of(text, s);
        let rest = &text[b..];
        let n = if rest.starts_with('\t') {
            1
        } else {
            rest.chars()
                .take(INDENT_WIDTH)
                .take_while(|c| *c == ' ')
                .count()
        };
        if n == 0 {
            continue;
        }
        text.replace_range(b..b + n, "");
        removed_total += n;
        if i == 0 {
            removed_before_start = n.min(sel.start - s);
        }
    }
    let start = sel.start - removed_before_start;
    let end = (sel.end.saturating_sub(removed_total)).max(start);
    start..end
}

/// Net block effect of one line of PPL: `opens` is how many blocks remain
/// open after the line relative to its start, and `leads_with_closer`
/// tells whether the line starts with `END`, `UNTIL`, `ELSE` or `DEFAULT`
/// (which belong one level out).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlockEffect {
    pub opens: i32,
    pub leads_with_closer: bool,
}

/// Uses the real lexer, so keywords inside strings, comments or longer
/// identifiers (`BEGINNER`) never count.
pub fn block_effect(line: &str) -> BlockEffect {
    let mut d = Diagnostics::default();
    let toks = lex(line, &mut d);
    let mut depth = 0i32;
    let mut min_depth = 0i32;
    let mut leads_with_closer = false;
    for (i, t) in toks.iter().enumerate() {
        let Tok::Kw(k) = t.tok else { continue };
        match k {
            Keyword::Begin | Keyword::Do | Keyword::Repeat | Keyword::Case | Keyword::Iferr => {
                depth += 1
            }
            // IF ... THEN opens the body; a THEN that starts a line closes
            // the IFERR part and opens the handler.
            Keyword::Then if i == 0 => {
                depth -= 1;
                min_depth = min_depth.min(depth);
                depth += 1;
            }
            Keyword::Then => depth += 1,
            Keyword::Else | Keyword::Default => {
                depth -= 1;
                min_depth = min_depth.min(depth);
                depth += 1;
                leads_with_closer |= i == 0;
            }
            Keyword::End | Keyword::Until => {
                depth -= 1;
                min_depth = min_depth.min(depth);
                leads_with_closer |= i == 0;
            }
            _ => {}
        }
    }
    BlockEffect {
        opens: depth - min_depth,
        leads_with_closer,
    }
}

/// Enter: replaces the selection with a newline plus the right indentation.
/// A line that starts with a block closer (`END`, `ELSE`, ...) is first
/// re-indented one level out, so typing `END;` + Enter lines it up with its
/// opener. Returns the new cursor.
pub fn newline(text: &mut String, sel: Range<usize>) -> usize {
    let b0 = byte_of(text, sel.start);
    let b1 = byte_of(text, sel.end);
    text.replace_range(b0..b1, "");
    let mut cursor = sel.start;

    // Electric dedent of the line being finished.
    let ls = line_start(text, cursor);
    let current = line_text(text, cursor).to_string();
    let effect = block_effect(&current[..byte_of(&current, cursor - ls)]);
    if effect.leads_with_closer {
        let target = closer_indent(text, ls);
        let have = leading_ws(&current).chars().count();
        if have > target.chars().count() {
            let b = byte_of(text, ls);
            let ws_bytes = leading_ws(&current).len();
            text.replace_range(b..b + ws_bytes, &target);
            cursor = cursor + target.chars().count() - have;
        }
    }

    let ls = line_start(text, cursor);
    let current = line_text(text, cursor).to_string();
    let before_cursor = &current[..byte_of(&current, cursor - ls)];
    let mut indent = leading_ws(&current).to_string();
    if block_effect(before_cursor).opens > 0 {
        indent.push_str(INDENT);
    }
    // Do not carry trailing whitespace of the left part into the new line.
    let b = byte_of(text, cursor);
    let mut insert = String::with_capacity(indent.len() + 1);
    insert.push('\n');
    insert.push_str(&indent);
    text.insert_str(b, &insert);
    cursor + insert.chars().count()
}

/// Indentation a closer line at char `ls` (its start) should have: that of
/// the previous non-blank line if it opened a block, else one level less.
fn closer_indent(text: &str, ls: usize) -> String {
    let mut c = ls;
    while c > 0 {
        let prev_start = line_start(text, c - 1);
        let prev = line_text(text, prev_start);
        if !prev.trim().is_empty() {
            let ws = leading_ws(prev);
            if block_effect(prev).opens > 0 {
                return ws.to_string();
            }
            let keep = ws.chars().count().saturating_sub(INDENT_WIDTH);
            return ws.chars().take(keep).collect();
        }
        c = prev_start;
    }
    String::new()
}

fn closer_of(open: char) -> Option<char> {
    match open {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '"' => Some('"'),
        _ => None,
    }
}

/// Context the widget derives from the language analysis.
#[derive(Clone, Copy, Debug, Default)]
pub struct TypingContext {
    /// The cursor is inside a string literal or comment: no auto-pairing.
    pub in_string_or_comment: bool,
    /// The character after the cursor is a closing bracket whose opener is
    /// matched, so typing the same closer should step over it.
    pub next_closer_is_matched: bool,
    /// The character after the cursor is the closing quote of the string
    /// the cursor is in.
    pub next_is_closing_quote: bool,
}

/// Typing one of `( [ { ) ] } "`: wraps a selection, inserts a pair, or
/// steps over an existing closer. Returns the new selection, or `None` to
/// let the character be inserted normally.
pub fn type_char(
    text: &mut String,
    sel: Range<usize>,
    ch: char,
    ctx: TypingContext,
) -> Option<Range<usize>> {
    let next = char_at(text, sel.end);
    // Step over a closer that is already there.
    if sel.is_empty()
        && next == Some(ch)
        && ((matches!(ch, ')' | ']' | '}') && ctx.next_closer_is_matched)
            || (ch == '"' && ctx.next_is_closing_quote))
    {
        return Some(sel.start + 1..sel.start + 1);
    }
    let close = closer_of(ch)?;
    if ctx.in_string_or_comment {
        return None;
    }
    let b0 = byte_of(text, sel.start);
    let b1 = byte_of(text, sel.end);
    if !sel.is_empty() {
        // Wrap a single-line selection.
        if text[b0..b1].contains('\n') {
            return None;
        }
        text.insert(b1, close);
        text.insert(b0, ch);
        return Some(sel.start + 1..sel.end + 1);
    }
    // Only pair when the next character cannot continue an expression.
    let pair_ok =
        next.is_none_or(|n| n.is_whitespace() || matches!(n, ')' | ']' | '}' | ';' | ','));
    if !pair_ok {
        return None;
    }
    if ch == '"' {
        let prev = sel.start.checked_sub(1).and_then(|p| char_at(text, p));
        if prev.is_some_and(|p| p.is_alphanumeric() || p == '"' || p == '\\') {
            return None;
        }
    }
    text.insert(b0, close);
    text.insert(b0, ch);
    Some(sel.start + 1..sel.start + 1)
}

/// Backspace between an empty auto-pair (`(|)`, `"|"`) deletes both.
pub fn backspace_pair(text: &mut String, cursor: usize) -> Option<usize> {
    let prev = char_at(text, cursor.checked_sub(1)?)?;
    let next = char_at(text, cursor)?;
    if closer_of(prev) != Some(next) {
        return None;
    }
    let b = byte_of(text, cursor - 1);
    text.replace_range(b..b + prev.len_utf8() + next.len_utf8(), "");
    Some(cursor - 1)
}

/// Ctrl+/: comment out every touched line with `// `, or uncomment them
/// all when every non-blank one is already commented.
pub fn toggle_line_comment(text: &mut String, sel: Range<usize>) -> Range<usize> {
    let starts = touched_line_starts(text, &sel);
    let lines: Vec<String> = starts
        .iter()
        .map(|&s| line_text(text, s).to_string())
        .collect();
    let all_commented = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .all(|l| l.trim_start().starts_with("//"));
    let col = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| leading_ws(l).chars().count())
        .min()
        .unwrap_or(0);
    let mut delta_first = 0isize;
    let mut delta_total = 0isize;
    for (i, &s) in starts.iter().enumerate().rev() {
        let line = &lines[i];
        if line.trim().is_empty() {
            continue;
        }
        let b = byte_of(text, s);
        let d: isize = if all_commented {
            let ws = leading_ws(line).len();
            let after = &line[ws..];
            let n = if after.starts_with("// ") { 3 } else { 2 };
            text.replace_range(b + ws..b + ws + n, "");
            -(n as isize)
        } else {
            let at = b + line.char_indices().nth(col).map_or(line.len(), |(i, _)| i);
            text.insert_str(at, "// ");
            3
        };
        delta_total += d;
        if i == 0 {
            delta_first = d;
        }
    }
    let start = if sel.start == starts[0] {
        sel.start
    } else {
        (sel.start as isize + delta_first).max(starts[0] as isize) as usize
    };
    start..((sel.end as isize + delta_total).max(start as isize) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply_indent(src: &str, sel: Range<usize>) -> (String, Range<usize>) {
        let mut t = src.to_string();
        let r = indent(&mut t, sel);
        (t, r)
    }

    #[test]
    fn tab_without_selection_inserts_to_next_stop() {
        assert_eq!(apply_indent("AB", 0..0), ("  AB".into(), 2..2));
        assert_eq!(apply_indent("AB", 1..1), ("A B".into(), 2..2));
        // A selection inside one line is replaced by the indentation.
        assert_eq!(apply_indent("ABCD", 1..3), ("A D".into(), 2..2));
    }

    #[test]
    fn tab_indents_every_selected_line_without_losing_text() {
        // Lines "AAA" "BBB" "CCC"; selection from A(1) to B(5).
        let (t, r) = apply_indent("AAA\nBBB\nCCC", 1..5);
        assert_eq!(t, "  AAA\n  BBB\nCCC");
        assert_eq!(r, 3..9);
        // Selection ending at the start of a line excludes that line.
        let (t, _) = apply_indent("AAA\nBBB\nCCC", 0..8);
        assert_eq!(t, "  AAA\n  BBB\nCCC");
        // A full single line selected from its start is indented.
        let (t, r) = apply_indent("AAA\nBBB", 0..3);
        assert_eq!((t.as_str(), r), ("  AAA\nBBB", 0..5));
    }

    #[test]
    fn shift_tab_dedents_and_round_trips() {
        let mut t = "  AAA\n    BBB\nCCC\n\tDDD".to_string();
        let n = t.chars().count();
        let r = dedent(&mut t, 0..n);
        assert_eq!(t, "AAA\n  BBB\nCCC\nDDD");
        assert_eq!(r.start, 0);
        let mut t = "AAA\nBBB".to_string();
        let r = indent(&mut t, 0..7);
        let r = dedent(&mut t, r);
        assert_eq!((t.as_str(), r), ("AAA\nBBB", 0..7));
    }

    #[test]
    fn block_effect_uses_tokens_not_substrings() {
        assert_eq!(block_effect("BEGIN").opens, 1);
        assert_eq!(block_effect("  IF x THEN").opens, 1);
        assert_eq!(block_effect("IF x THEN y; END;").opens, 0);
        assert_eq!(block_effect("FOR i FROM 1 TO 3 DO").opens, 1);
        assert_eq!(block_effect("WHILE k < 3 DO").opens, 1);
        assert_eq!(block_effect("REPEAT").opens, 1);
        assert_eq!(block_effect("ELSE").opens, 1);
        assert!(block_effect("ELSE").leads_with_closer);
        assert_eq!(block_effect("END;").opens, 0);
        assert!(block_effect("END;").leads_with_closer);
        // Keywords in strings, comments and longer names are ignored.
        assert_eq!(block_effect("PRINT(\"BEGIN THEN\"); // DO").opens, 0);
        assert_eq!(block_effect("BEGINNER := THENCE;").opens, 0);
    }

    fn enter(src: &str, at: usize) -> (String, usize) {
        let mut t = src.to_string();
        let c = newline(&mut t, at..at);
        (t, c)
    }

    #[test]
    fn enter_keeps_and_extends_indentation() {
        let (t, c) = enter("EXPORT F()\nBEGIN", 16);
        assert_eq!((t.as_str(), c), ("EXPORT F()\nBEGIN\n  ", 19));
        let (t, c) = enter("  x := 1;", 9);
        assert_eq!((t.as_str(), c), ("  x := 1;\n  ", 12));
        // Splitting a line keeps the right part.
        let (t, c) = enter("  IF a THEN b;", 11);
        assert_eq!((t.as_str(), c), ("  IF a THEN\n     b;", 16));
    }

    #[test]
    fn enter_after_end_lines_it_up_with_its_block() {
        let src = "BEGIN\n  IF a THEN\n    x := 1;\n    END;";
        let at = src.chars().count();
        let (t, _) = enter(src, at);
        assert_eq!(t, "BEGIN\n  IF a THEN\n    x := 1;\n  END;\n  ");
        // Already lined up: unchanged.
        let src = "BEGIN\n  x;\nEND;";
        let (t, _) = enter(src, src.chars().count());
        assert_eq!(t, "BEGIN\n  x;\nEND;\n");
    }

    #[test]
    fn typing_brackets_pairs_wraps_and_steps_over() {
        let ctx = TypingContext::default();
        let mut t = "x := ".to_string();
        let r = type_char(&mut t, 5..5, '(', ctx).unwrap();
        assert_eq!((t.as_str(), r), ("x := ()", 6..6));
        // Step over the matched closer.
        let over = TypingContext {
            next_closer_is_matched: true,
            ..ctx
        };
        assert_eq!(type_char(&mut t, 6..6, ')', over), Some(7..7));
        assert_eq!(t, "x := ()");
        // Unmatched closer is inserted normally.
        assert_eq!(type_char(&mut t, 6..6, ')', ctx), None);
        // Wrap a selection.
        let mut t = "a+b".to_string();
        assert_eq!(type_char(&mut t, 0..3, '(', ctx), Some(1..4));
        assert_eq!(t, "(a+b)");
        // Do not pair before an identifier.
        let mut t = "abc".to_string();
        assert_eq!(type_char(&mut t, 0..0, '(', ctx), None);
    }

    #[test]
    fn quotes_pair_outside_strings_only() {
        let ctx = TypingContext::default();
        let mut t = "PRINT()".to_string();
        assert_eq!(type_char(&mut t, 6..6, '"', ctx), Some(7..7));
        assert_eq!(t, "PRINT(\"\")");
        let closing = TypingContext {
            next_is_closing_quote: true,
            in_string_or_comment: true,
            ..ctx
        };
        assert_eq!(type_char(&mut t, 7..7, '"', closing), Some(8..8));
        let inside = TypingContext {
            in_string_or_comment: true,
            ..ctx
        };
        let mut t = "\"ab\"".to_string();
        assert_eq!(type_char(&mut t, 2..2, '"', inside), None);
        // After an identifier character a quote is not paired.
        let mut t = "a ".to_string();
        assert_eq!(type_char(&mut t, 1..1, '"', ctx), None);
    }

    #[test]
    fn backspace_removes_empty_pairs() {
        let mut t = "f()".to_string();
        assert_eq!(backspace_pair(&mut t, 2), Some(1));
        assert_eq!(t, "f");
        let mut t = "f(x)".to_string();
        assert_eq!(backspace_pair(&mut t, 2), None);
    }

    #[test]
    fn toggle_comment_round_trips() {
        let src = "BEGIN\n  x := 1;\n\n  y := 2;\nEND;";
        let mut t = src.to_string();
        let n = t.chars().count();
        let r = toggle_line_comment(&mut t, 6..n - 5);
        assert_eq!(t, "BEGIN\n  // x := 1;\n\n  // y := 2;\nEND;");
        toggle_line_comment(&mut t, r);
        assert_eq!(t, src);
    }

    #[test]
    fn operations_are_unicode_safe() {
        let mut t = "π := 1;\n→B := 2;".to_string();
        let r = indent(&mut t, 0..10);
        assert_eq!(t, "  π := 1;\n  →B := 2;");
        dedent(&mut t, r);
        assert_eq!(t, "π := 1;\n→B := 2;");
        let c = newline(&mut t, 7..7);
        assert_eq!((t.as_str(), c), ("π := 1;\n\n→B := 2;", 8));
    }
}
