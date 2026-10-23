//! Keystrokes the editor handles itself, before egui's `TextEdit`.
//!
//! `TextEdit` would otherwise apply its own generic behavior (Tab replaces
//! the selection with `\t`, Enter inserts a bare newline). Intercepted
//! events are removed from the frame's input, applied through
//! [`super::text_ops`], and recorded on the `TextEdit` undo stack as one
//! step each, so Ctrl+Z reverts exactly one indent, newline or pair.

use std::ops::Range;

use egui::text::{CCursor, CCursorRange};
use egui::text_edit::TextEditState;
use egui::{Event, Id, Key, Modifiers};
use ppl_compiler::highlight::{SourceAnalysis, TokenKind};

use super::completion::{self, CompletionState};
use super::document::LineIndex;
use super::text_ops::{self, TypingContext};

/// What the editor did with this frame's input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Intercepted {
    /// The text changed.
    pub edited: bool,
    /// Completion was explicitly requested (Ctrl+Space).
    pub complete: bool,
}

/// Events that never move the cursor or change the text.
fn is_inert(ev: &Event) -> bool {
    matches!(
        ev,
        Event::PointerMoved(_)
            | Event::MouseMoved(_)
            | Event::Key { pressed: false, .. }
            | Event::WindowFocused(_)
    )
}

fn plain(m: &Modifiers) -> bool {
    !m.command && !m.ctrl && !m.alt && !m.mac_cmd
}

/// Applies an edit: records the state before and after on the undo stack
/// and moves the cursor.
pub(super) fn commit(
    state: &mut TextEditState,
    before: (CCursorRange, String),
    text: &str,
    sel: Range<usize>,
) {
    let after = CCursorRange::two(CCursor::new(sel.start), CCursor::new(sel.end));
    let mut undoer = state.undoer();
    undoer.add_undo(&before);
    undoer.add_undo(&(after, text.to_owned()));
    state.set_undoer(undoer);
    state.cursor.set_char_range(Some(after));
}

/// Context for typing a bracket or quote at the cursor, from the analysis
/// of the text as it was at the start of the frame.
fn typing_context(
    text: &str,
    lines: &LineIndex,
    analysis: &SourceAnalysis,
    cursor: usize,
) -> TypingContext {
    let byte = lines.byte_of_char(text, cursor);
    let kind_before = byte.checked_sub(1).and_then(|b| analysis.kind_at(b));
    let kind_here = analysis.kind_at(byte);
    let stringy = |k: Option<TokenKind>| {
        matches!(
            k,
            Some(TokenKind::String | TokenKind::StringEscape | TokenKind::InvalidString)
        )
    };
    // Inside a literal when the cursor sits between two of its characters;
    // a comment (or an unclosed string) runs to the end of the line.
    let in_string_or_comment = (stringy(kind_before) && stringy(kind_here))
        || matches!(
            kind_before,
            Some(TokenKind::Comment | TokenKind::InvalidString)
        );
    let next_closer_is_matched = analysis
        .bracket_partner(byte)
        .is_some_and(|(_, partner)| partner.is_some_and(|p| p < byte));
    let next_is_closing_quote = text[byte..].starts_with('"')
        && kind_here == Some(TokenKind::String)
        && matches!(
            kind_before,
            Some(TokenKind::String | TokenKind::StringEscape)
        )
        && analysis.kind_at(byte + 1) != Some(TokenKind::String);
    TypingContext {
        in_string_or_comment,
        next_closer_is_matched,
        next_is_closing_quote,
    }
}

/// Accepts the selected completion: replaces the word before the cursor.
pub fn accept_completion(
    ctx: &egui::Context,
    id: Id,
    text: &mut String,
    completion: &mut CompletionState,
) -> bool {
    let Some(item) = completion.items.get(completion.selected).cloned() else {
        completion.close();
        return false;
    };
    let Some(mut state) = TextEditState::load(ctx, id) else {
        return false;
    };
    let Some(range) = state.cursor.char_range() else {
        return false;
    };
    let cursor = range.primary.index;
    let start = completion.word_start.min(cursor);
    let before = (range, text.clone());
    let b0 = text
        .char_indices()
        .nth(start)
        .map_or(text.len(), |(b, _)| b);
    let b1 = text
        .char_indices()
        .nth(cursor)
        .map_or(text.len(), |(b, _)| b);
    text.replace_range(b0..b1, &item.label);
    let end = start + item.label.chars().count();
    commit(&mut state, before, text, end..end);
    state.store(ctx, id);
    completion.close();
    true
}

/// Whether the editor itself handles `ev` (rather than `TextEdit`).
fn is_editor_event(ev: &Event, completion_open: bool) -> bool {
    match ev {
        Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } => match key {
            Key::Tab | Key::Backspace => plain(modifiers),
            Key::Enter => plain(modifiers) && !modifiers.shift,
            Key::Slash | Key::Space => modifiers.command,
            Key::ArrowUp | Key::ArrowDown | Key::Escape => completion_open,
            _ => false,
        },
        Event::Text(s) => {
            let mut chars = s.chars();
            matches!(
                (chars.next(), chars.next()),
                (Some('(' | '[' | '{' | ')' | ']' | '}' | '"'), None)
            )
        }
        _ => false,
    }
}

/// Replaces the selection with `s`; returns the cursor range after it.
fn insert(text: &mut String, sel: Range<usize>, s: &str) -> Range<usize> {
    let s = s.replace("\r\n", "\n");
    let byte = |c: usize| text.char_indices().nth(c).map_or(text.len(), |(b, _)| b);
    let (b0, b1) = (byte(sel.start), byte(sel.end));
    text.replace_range(b0..b1, &s);
    let c = sel.start + s.chars().count();
    c..c
}

/// Deletes the selection, or the character before (`back`) or after the
/// cursor. Returns the new cursor.
fn delete(text: &mut String, sel: Range<usize>, back: bool) -> usize {
    let byte = |c: usize| text.char_indices().nth(c).map_or(text.len(), |(b, _)| b);
    let (from, to) = if !sel.is_empty() {
        (sel.start, sel.end)
    } else if back {
        (sel.start.saturating_sub(1), sel.start)
    } else {
        (sel.start, (sel.start + 1).min(text.chars().count()))
    };
    let (b0, b1) = (byte(from), byte(to));
    text.replace_range(b0..b1, "");
    from
}

/// Handles this frame's editor keys. When a frame holds any key the
/// editor handles, the plain insertions and deletions in that frame are
/// applied here too, in order, so a burst of input (fast typing, key
/// repeat, scripted input) is never reordered and never loses the smart
/// behavior of the keys in it. Anything else stops the run and is left,
/// with all later events, to `TextEdit`.
pub fn intercept(
    ui: &mut egui::Ui,
    id: Id,
    text: &mut String,
    lines: &LineIndex,
    analysis: &SourceAnalysis,
    completion: &mut CompletionState,
) -> Intercepted {
    let ctx = ui.ctx().clone();
    let mut result = Intercepted::default();
    let Some(mut state) = TextEditState::load(&ctx, id) else {
        return result;
    };
    let Some(mut range) = state.cursor.char_range() else {
        return result;
    };
    let events = ui.input(|i| i.events.clone());
    if !events.iter().any(|e| is_editor_event(e, completion.open)) {
        return result;
    }
    let mut consumed = vec![false; events.len()];
    // The analysis describes the text at the start of the frame; after an
    // edit, typing context falls back to the conservative default.
    let mut pristine = true;

    for (ix, ev) in events.iter().enumerate() {
        if is_inert(ev) {
            continue;
        }
        let [a, b] = range.sorted_cursors();
        let sel = a.index..b.index;
        let before = (range, text.clone());

        // Completion popup navigation.
        if completion.open {
            if let Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = ev
            {
                match key {
                    Key::ArrowDown if plain(modifiers) => {
                        completion.selected =
                            (completion.selected + 1) % completion.items.len().max(1);
                        consumed[ix] = true;
                        continue;
                    }
                    Key::ArrowUp if plain(modifiers) => {
                        let n = completion.items.len().max(1);
                        completion.selected = (completion.selected + n - 1) % n;
                        consumed[ix] = true;
                        continue;
                    }
                    Key::Enter | Key::Tab if plain(modifiers) && !modifiers.shift => {
                        state.clone().store(&ctx, id);
                        if accept_completion(&ctx, id, text, completion) {
                            result.edited = true;
                            pristine = false;
                        }
                        consumed[ix] = true;
                        match TextEditState::load(&ctx, id) {
                            Some(s) => state = s,
                            None => break,
                        }
                        match state.cursor.char_range() {
                            Some(r) => range = r,
                            None => break,
                        }
                        continue;
                    }
                    Key::Escape => {
                        completion.close();
                        consumed[ix] = true;
                        continue;
                    }
                    _ => {}
                }
            }
        }

        let new_sel: Option<Range<usize>> = match ev {
            Event::Key {
                key: Key::Tab,
                pressed: true,
                modifiers,
                ..
            } if plain(modifiers) => Some(if modifiers.shift {
                text_ops::dedent(text, sel)
            } else {
                text_ops::indent(text, sel)
            }),
            Event::Key {
                key: Key::Enter,
                pressed: true,
                modifiers,
                ..
            } if plain(modifiers) && !modifiers.shift => {
                let c = text_ops::newline(text, sel);
                Some(c..c)
            }
            Event::Key {
                key: key @ (Key::Backspace | Key::Delete),
                pressed: true,
                modifiers,
                ..
            } if modifiers.is_none() => {
                let pair = (*key == Key::Backspace && sel.is_empty())
                    .then(|| text_ops::backspace_pair(text, sel.start))
                    .flatten();
                pair.or_else(|| Some(delete(text, sel, *key == Key::Backspace)))
                    .map(|c| c..c)
            }
            Event::Key {
                key: Key::Slash,
                pressed: true,
                modifiers,
                ..
            } if modifiers.command && !modifiers.shift => {
                Some(text_ops::toggle_line_comment(text, sel))
            }
            Event::Key {
                key: Key::Space,
                pressed: true,
                modifiers,
                ..
            } if modifiers.command => {
                result.complete = true;
                consumed[ix] = true;
                continue;
            }
            Event::Text(s) if s.chars().count() == 1 => {
                let ch = s.chars().next().unwrap_or(' ');
                if matches!(ch, '(' | '[' | '{' | ')' | ']' | '}' | '"') {
                    let ctx_here = if pristine {
                        typing_context(text, lines, analysis, sel.start)
                    } else {
                        TypingContext {
                            in_string_or_comment: true,
                            ..TypingContext::default()
                        }
                    };
                    text_ops::type_char(text, sel.clone(), ch, ctx_here)
                } else {
                    None
                }
                .or_else(|| Some(insert(text, sel, s)))
            }
            Event::Text(s) | Event::Paste(s) if !s.is_empty() => Some(insert(text, sel, s)),
            _ => None,
        };

        match new_sel {
            Some(new_sel) => {
                consumed[ix] = true;
                if before.1 != *text {
                    result.edited = true;
                    pristine = false;
                    commit(&mut state, before, text, new_sel.clone());
                } else {
                    // Cursor-only (stepping over a closer).
                    state
                        .cursor
                        .set_char_range(Some(CCursorRange::one(CCursor::new(new_sel.end))));
                }
                range = CCursorRange::two(CCursor::new(new_sel.start), CCursor::new(new_sel.end));
            }
            // Anything else ends the run we may reorder safely: TextEdit
            // will apply it, and later events must follow it.
            None => break,
        }
    }

    if consumed.iter().any(|c| *c) {
        state.store(&ctx, id);
        ui.input_mut(|i| {
            let mut k = 0;
            i.events.retain(|_| {
                let keep = !consumed.get(k).copied().unwrap_or(false);
                k += 1;
                keep
            });
        });
    }
    result
}

/// Recomputes the completion list for the word ending at `cursor`.
pub fn update_completion(
    completion: &mut CompletionState,
    candidates: &[completion::CompletionItem],
    text: &str,
    cursor: usize,
    explicit: bool,
) {
    let start = completion::word_start(text, cursor);
    let prefix: String = text.chars().skip(start).take(cursor - start).collect();
    let min_len = if explicit { 1 } else { 2 };
    if prefix.chars().count() < min_len || prefix.starts_with(|c: char| c.is_ascii_digit()) {
        completion.close();
        return;
    }
    let items = completion::filter(candidates, &prefix, 12);
    if items.is_empty() {
        completion.close();
        return;
    }
    completion.open = true;
    completion.word_start = start;
    completion.selected = completion.selected.min(items.len() - 1);
    completion.items = items;
}
