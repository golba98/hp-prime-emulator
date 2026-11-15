//! Native navigation for optionally installed original Help books.
//! Loading stays off the UI thread; command support is still shown by the
//! Native catalog, independently of the original book's contents.
use super::ToolMemory;
use ppl_engine::help::{Book, Language, Topic};
use std::sync::{Arc, mpsc};

type Loaded = Result<Option<Arc<Book>>, String>;
#[derive(Default)]
pub(super) struct Browser {
    requested: Option<usize>,
    pending: Option<mpsc::Receiver<Loaded>>,
    book: Option<Arc<Book>>,
    error: Option<String>,
}
const LANGUAGES: [&str; 8] = [
    "English",
    "Chinese",
    "French",
    "German",
    "Spanish",
    "Dutch",
    "Portuguese",
    "Japanese",
];

fn title(topic: &Topic) -> String {
    if !topic.name.is_empty() {
        topic.name.clone()
    } else {
        topic.spans.first().map(|s| text(s)).unwrap_or_default()
    }
}
// Keep isolated UTF16 units visible rather than silently replacing original
// calculator data with U+FFFD. No original prose or glyphs are bundled.
fn text(units: &[u16]) -> String {
    char::decode_utf16(units.iter().copied())
        .map(|unit| match unit {
            Ok(c) => c.to_string(),
            Err(error) => format!("\\u{:04X}", error.unpaired_surrogate()),
        })
        .collect()
}
fn matches(topic: &Topic, query: &str) -> bool {
    topic.id != 0 && title(topic).to_lowercase().contains(&query.to_lowercase())
}
impl Browser {
    fn request(&mut self, language: usize) {
        let (sender, receiver) = mpsc::channel();
        self.requested = Some(language);
        self.pending = Some(receiver);
        self.book = None;
        self.error = None;
        std::thread::spawn(move || {
            let result = ppl_engine::help::configured(Language::ALL[language])
                .map_err(|error| error.to_string());
            let _ = sender.send(result);
        });
    }
    fn poll(&mut self) {
        let Some(receiver) = &self.pending else {
            return;
        };
        match receiver.try_recv() {
            Ok(result) => {
                self.pending = None;
                match result {
                    Ok(book) => self.book = book,
                    Err(error) => self.error = Some(error),
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                self.error = Some("Help loader ended before returning the book".into());
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }
    pub(super) fn show(&mut self, ui: &mut egui::Ui, memory: &mut ToolMemory) {
        memory.help_language = memory.help_language.min(LANGUAGES.len() - 1);
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("original_help_language")
                .selected_text(LANGUAGES[memory.help_language])
                .show_ui(ui, |ui| {
                    for (index, name) in LANGUAGES.iter().enumerate() {
                        if ui
                            .selectable_value(&mut memory.help_language, index, *name)
                            .changed()
                        {
                            memory.help_topic_id = None;
                        }
                    }
                });
            if ui.button("Reload Help").clicked() {
                self.requested = None;
            }
        });
        // A rapid language change must not start unbounded decoders. Finish
        // the current bounded load, then discard it and load the latest choice
        // before exposing any book from the previous language.
        self.poll();
        if self.requested != Some(memory.help_language) && self.pending.is_none() {
            self.request(memory.help_language);
        }
        if self.pending.is_some() {
            ui.label("Loading installed Help…");
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(30));
        } else if let Some(error) = &self.error {
            ui.label(format!("Installed Help could not be read: {error}"));
        } else if self.book.is_none() {
            ui.label(
                "Original Help is not installed. The Native command catalog is available below.",
            );
        }
        let Some(book) = &self.book else { return };
        ui.strong("Installed calculator Help");
        ui.small("Original documentation does not establish Native command support. Original formatting codes are preserved in the text.");
        ui.text_edit_singleline(&mut memory.help_query);
        let selected = memory
            .help_topic_id
            .and_then(|id| book.topic(id))
            .filter(|topic| title(topic) == memory.help_topic)
            .or_else(|| book.named(&memory.help_topic).next());
        if let Some(topic) = selected {
            ui.strong(title(topic));
            ui.small(format!(
                "Topic {} · category {} · mode {}",
                topic.id, topic.kind, topic.mode
            ));
            for span in &topic.spans {
                ui.add(egui::Label::new(text(span)).wrap().selectable(true));
            }
        }
        let topics: Vec<_> = book
            .topics
            .iter()
            .filter(|topic| matches(topic, &memory.help_query))
            .collect();
        let row_height = ui.text_style_height(&egui::TextStyle::Body);
        egui::ScrollArea::vertical()
            .id_salt("original_help_topics")
            .max_height(140.)
            .show_rows(ui, row_height, topics.len(), |ui, rows| {
                for row in rows {
                    let topic = topics[row];
                    if ui
                        .selectable_label(
                            memory.help_topic_id == Some(topic.id),
                            format!("{} · {}", title(topic), topic.id),
                        )
                        .clicked()
                    {
                        memory.help_topic = title(topic);
                        memory.help_topic_id = Some(topic.id);
                    }
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn topic(id: u32, name: &str) -> Topic {
        Topic {
            id,
            name: name.into(),
            kind: 0,
            mode: 0,
            spans: vec!["Authored test title".encode_utf16().collect()],
        }
    }
    #[test]
    fn searching_keeps_duplicate_topic_identities_and_localized_titles() {
        let topics = [
            topic(0, "Variables"),
            topic(1, "Variables"),
            topic(2, "Variables"),
            topic(3, "Équations"),
            topic(4, ""),
        ];
        let found: Vec<_> = topics
            .iter()
            .filter(|t| matches(t, "variables"))
            .map(|t| t.id)
            .collect();
        assert_eq!(found, [1, 2]);
        assert!(matches(&topics[3], "ÉQU"));
        assert!(matches(&topics[4], "authored"));
        assert_eq!(text(&[0xd800, 65, 0xdc00]), "\\uD800A\\uDC00");
    }
    #[test]
    fn asynchronous_errors_and_missing_books_remain_distinct() {
        let (sender, receiver) = mpsc::channel();
        let mut browser = Browser {
            pending: Some(receiver),
            ..Default::default()
        };
        sender.send(Ok(None)).unwrap();
        browser.poll();
        assert!(browser.book.is_none() && browser.error.is_none() && browser.pending.is_none());
        let (sender, receiver) = mpsc::channel();
        browser.pending = Some(receiver);
        sender.send(Err("authored corruption".into())).unwrap();
        browser.poll();
        assert_eq!(browser.error.as_deref(), Some("authored corruption"));
    }
    #[test]
    fn persisted_duplicate_selection_is_exact() {
        let book = Book {
            language: Language::English,
            topics: vec![topic(1, "Variables"), topic(2, "Variables")],
            groups: vec![],
        };
        assert_eq!(book.topic(2).unwrap().id, 2);
        let memory = ToolMemory {
            help_language: 3,
            help_topic: "Variables".into(),
            help_topic_id: Some(2),
            ..Default::default()
        };
        let roundtrip: ToolMemory =
            serde_json::from_str(&serde_json::to_string(&memory).unwrap()).unwrap();
        assert_eq!(roundtrip.help_topic_id, Some(2));
        assert_eq!(roundtrip.help_language, 3);
    }
    #[test]
    fn real_help_ui_renders_the_selected_duplicate_original_topic() {
        let mut second = topic(2, "Variables");
        second
            .spans
            .push("Second app authored help body".encode_utf16().collect());
        let book = Book {
            language: Language::English,
            topics: vec![topic(1, "Variables"), second],
            groups: vec![],
        };
        let mut browser = Browser {
            requested: Some(0),
            book: Some(Arc::new(book)),
            ..Default::default()
        };
        let mut memory = ToolMemory {
            help_topic: "Variables".into(),
            help_topic_id: Some(2),
            ..Default::default()
        };
        let context = egui::Context::default();
        let output = context.run(egui::RawInput::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| browser.show(ui, &mut memory));
        });
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    Some(text.galley.text())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            labels.iter().any(|text| text.contains("Topic 2")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|text| text.contains("Second app authored help body")),
            "{labels:?}"
        );
        assert!(
            !labels
                .iter()
                .any(|text| text.contains("Native implementation available"))
        );
    }
}
