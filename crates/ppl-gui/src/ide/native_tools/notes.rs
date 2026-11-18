//! Paragraph formatting for standalone calculator Notes.
use egui::{TextBuffer, Vec2};
use std::{collections::BTreeSet, ops::Range};

fn editor_id() -> egui::Id {
    egui::Id::new("native_catalog_content")
}

impl super::NativeTools {
    pub(crate) fn load_note_bullets(
        &mut self,
        ctx: &egui::Context,
        bullets: Option<&BTreeSet<usize>>,
    ) {
        self.note_bullets = Some(bullets.cloned().unwrap_or_default());
        egui::text_edit::TextEditState::default().store(ctx, editor_id());
    }

    pub(crate) fn clear_note_bullets(&mut self) {
        self.note_bullets = None;
    }
}

fn paragraph_at(text: &str, byte: usize) -> usize {
    text[..byte].bytes().filter(|&byte| byte == b'\n').count()
}

struct NoteBuffer<'a> {
    text: &'a mut String,
    bullets: &'a mut BTreeSet<usize>,
}

impl TextBuffer for NoteBuffer<'_> {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<NoteBuffer<'static>>()
    }

    fn is_mutable(&self) -> bool {
        true
    }

    fn as_str(&self) -> &str {
        self.text
    }

    fn insert_text(&mut self, text: &str, char_index: usize) -> usize {
        let inserted_lines = paragraph_at(text, text.len());
        if inserted_lines > 0 {
            let index = paragraph_at(self.text, self.byte_index_from_char_index(char_index));
            let inherit = self.bullets.contains(&index);
            *self.bullets = self
                .bullets
                .iter()
                .map(|&line| {
                    if line > index {
                        line + inserted_lines
                    } else {
                        line
                    }
                })
                .collect();
            if inherit {
                self.bullets.extend(index + 1..=index + inserted_lines);
            }
        }
        self.text.insert_text(text, char_index)
    }

    fn delete_char_range(&mut self, range: Range<usize>) {
        let start = self.byte_index_from_char_index(range.start);
        let end = self.byte_index_from_char_index(range.end);
        let first = paragraph_at(self.text, start);
        let last = paragraph_at(self.text, end);
        if first < last {
            let starts_paragraph = start == 0 || self.text.as_bytes()[start - 1] == b'\n';
            *self.bullets = self
                .bullets
                .iter()
                .filter_map(|&line| {
                    if line < first || (line == first && !starts_paragraph) {
                        Some(line)
                    } else if line > last || (line == last && starts_paragraph) {
                        Some(line - (last - first))
                    } else {
                        None
                    }
                })
                .collect();
        }
        self.text.delete_char_range(range);
    }

    fn replace_with(&mut self, text: &str) {
        // TextEdit restores a string during undo/redo. Limit the replacement
        // to the changed range so unrelated paragraphs retain their style.
        let prefix = self
            .text
            .chars()
            .zip(text.chars())
            .take_while(|(left, right)| left == right)
            .count();
        let old_start = self.byte_index_from_char_index(prefix);
        let new_start = text
            .char_indices()
            .nth(prefix)
            .map_or(text.len(), |(index, _)| index);
        let suffix_bytes: usize = self.text[old_start..]
            .chars()
            .rev()
            .zip(text[new_start..].chars().rev())
            .take_while(|(left, right)| left == right)
            .map(|(left, _)| left.len_utf8())
            .sum();
        let old_end = self.char_index_from_byte_index(self.text.len() - suffix_bytes);
        let new_end = text.len() - suffix_bytes;
        self.delete_char_range(prefix..old_end);
        self.insert_text(&text[new_start..new_end], prefix);
    }
}

pub(super) fn show(
    ui: &mut egui::Ui,
    scale: f32,
    text: &mut String,
    bullets: &mut BTreeSet<usize>,
) {
    let mut buffer = NoteBuffer { text, bullets };
    let gutter = (12. * scale).round().clamp(6., 96.);
    let output = egui::TextEdit::multiline(&mut buffer)
        .id(editor_id())
        .desired_rows(4)
        .desired_width(300. * scale - gutter)
        .margin(egui::Margin {
            left: gutter as i8,
            right: 4,
            top: 2,
            bottom: 2,
        })
        .font(egui::TextStyle::Monospace)
        .show(ui);
    let (first, last) = if let Some(range) = output.state.cursor.range(&output.galley) {
        let [start, end] = range.sorted_cursors();
        let end = if end.index > start.index {
            end.index - 1
        } else {
            end.index
        };
        (
            paragraph_at(buffer.text, buffer.byte_index_from_char_index(start.index)),
            paragraph_at(buffer.text, buffer.byte_index_from_char_index(end)),
        )
    } else {
        (0, 0)
    };
    let marked = (first..=last).all(|line| buffer.bullets.contains(&line));
    if ui.selectable_label(marked, "Bullets").clicked() {
        if marked {
            buffer.bullets.retain(|line| !(first..=last).contains(line));
        } else {
            buffer.bullets.extend(first..=last);
        }
        output.response.request_focus();
    }

    let painter = ui
        .painter()
        .with_clip_rect(ui.clip_rect().intersect(output.response.rect));
    let color = ui.visuals().text_color();
    let mut paragraph = 0;
    let mut starts_paragraph = true;
    for row in &output.galley.rows {
        if starts_paragraph && buffer.bullets.contains(&paragraph) {
            let at = output.galley_pos + Vec2::new(-gutter / 2., row.pos.y + row.size.y / 2.);
            painter.circle_filled(at, (2. * scale).max(1.), color);
        }
        starts_paragraph = row.ends_with_newline;
        paragraph += usize::from(starts_paragraph);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitting_and_joining_note_paragraphs_preserves_visual_formatting() {
        let mut text = String::from("α\nβ\nγ");
        let mut bullets = BTreeSet::from([1]);
        let mut buffer = NoteBuffer {
            text: &mut text,
            bullets: &mut bullets,
        };
        assert_eq!(buffer.insert_text("\nδ", 3), 2);
        assert_eq!(buffer.as_str(), "α\nβ\nδ\nγ");
        assert_eq!(*buffer.bullets, BTreeSet::from([1, 2]));

        buffer.delete_char_range(0..2);
        assert_eq!(buffer.as_str(), "β\nδ\nγ");
        assert_eq!(*buffer.bullets, BTreeSet::from([0, 1]));

        buffer.delete_char_range(1..2);
        assert_eq!(buffer.as_str(), "βδ\nγ");
        assert_eq!(*buffer.bullets, BTreeSet::from([0]));

        buffer.replace_with("βδ!\nγ");
        assert_eq!(buffer.as_str(), "βδ!\nγ");
        assert_eq!(*buffer.bullets, BTreeSet::from([0]));
    }
}
