//! Original Help books from a user's legitimately installed G1 resources.
//!
//! This module supplies data, not command implementation claims. Topic names
//! can describe firmware commands unavailable in the Native VM. Original text
//! formatting and grouped interface strings remain explicit so a consumer can render them
//! faithfully rather than silently replacing their content. No vendor data is
//! bundled or downloaded.
mod huffman;
use std::{collections::BTreeMap, fmt, fs::File, io::Read, path::Path, sync::Arc};

const MAX_ARCHIVE: usize = 32 * 1024 * 1024;
const MAX_BOOK: usize = 4 * 1024 * 1024;
const MAX_TEXT_UNITS: usize = 8 * 1024 * 1024;
const BASE: usize = 28;
const HEADER: usize = 332;
type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(String);
impl Error {
    fn new(message: &str) -> Self {
        Self(message.into())
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Language {
    English = 1,
    Chinese = 2,
    French = 3,
    German = 4,
    Spanish = 5,
    Dutch = 6,
    Portuguese = 7,
    Japanese = 8,
}
impl Language {
    pub const ALL: [Self; 8] = [
        Self::English,
        Self::Chinese,
        Self::French,
        Self::German,
        Self::Spanish,
        Self::Dutch,
        Self::Portuguese,
        Self::Japanese,
    ];
    fn resource(self) -> usize {
        match self {
            Self::German => 71,
            Self::English => 72,
            Self::Spanish => 73,
            Self::French => 74,
            Self::Japanese => 75,
            Self::Dutch => 76,
            Self::Portuguese => 77,
            Self::Chinese => 78,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Topic {
    /// Zero marks an explicit vacant index entry in the installed book.
    pub id: u32,
    pub name: String,
    /// Original category/format metadata, not a Native support classification.
    pub kind: u8,
    pub mode: u16,
    /// Raw UTF16 strings: title, optional formatted body, optional examples.
    /// A vacant entry has no spans; some localized entries have no index name.
    /// Markup and private calculator glyphs are retained without substitution.
    pub spans: Vec<Vec<u16>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringGroup {
    /// Original low23-bit group identity; gaps between groups are valid.
    pub id: u32,
    /// Exact original interface strings, indexed from zero. CR is discarded by
    /// the original decoder; private glyphs and isolated units are preserved.
    pub strings: Vec<Vec<u16>>,
}

#[derive(Debug)]
pub struct Book {
    pub language: Language,
    pub topics: Vec<Topic>,
    pub groups: Vec<StringGroup>,
    /// Complete demos stored separately from the Help book's launcher references.
    pub demo_programs: BTreeMap<String, String>,
}

fn bytes(data: &[u8], at: usize, len: usize) -> Result<&[u8]> {
    data.get(
        at..at
            .checked_add(len)
            .ok_or_else(|| Error::new("Help offset overflow"))?,
    )
    .ok_or_else(|| Error::new("Help resource range is out of bounds"))
}
fn word(data: &[u8], at: usize) -> Result<usize> {
    Ok(u32::from_le_bytes(bytes(data, at, 4)?.try_into().unwrap()) as usize)
}
fn short(data: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(bytes(data, at, 2)?.try_into().unwrap()))
}

/// Resolve the same installed archive used for the calculator's fonts. An
/// explicit Help resource overrides it; unreadable selected files are errors.
pub fn configured(language: Language) -> Result<Option<Arc<Book>>> {
    let path = std::env::var_os("PPL_PRIME_HELP_RESOURCE")
        .filter(|p| !p.is_empty())
        .or_else(|| std::env::var_os("PPL_PRIME_FONT_RESOURCE").filter(|p| !p.is_empty()))
        .map(std::path::PathBuf::from);
    let path = match path {
        Some(path) => Some(path),
        None => crate::fonts::resource(
            &crate::firmware_options::AssetPaths::default()
                .resolve()
                .map_err(Error)?,
        ),
    };
    path.map(|path| Book::load_resource(&path, language))
        .transpose()
}

impl Book {
    pub fn load_resource(path: &Path, language: Language) -> Result<Arc<Self>> {
        let file = File::open(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
        let mut data = Vec::new();
        file.take((MAX_ARCHIVE + 1) as u64)
            .read_to_end(&mut data)
            .map_err(|e| Error(format!("{}: {e}", path.display())))?;
        Self::from_resource(&data, language).map(Arc::new)
    }

    pub fn from_resource(data: &[u8], language: Language) -> Result<Self> {
        if data.len() > MAX_ARCHIVE {
            return Err(Error::new("Help resource archive exceeds its size limit"));
        }
        for (index, value) in [u32::MAX as usize, 5, 1, 0, 3, 2, 6]
            .into_iter()
            .enumerate()
        {
            if word(data, index * 4)? != value {
                return Err(Error::new("Unsupported Help resource archive"));
            }
        }
        let directory_len = word(data, BASE)?;
        if directory_len % 8 != 0 || directory_len / 8 <= language.resource() {
            return Err(Error::new("Invalid Help resource directory"));
        }
        bytes(data, BASE, directory_len)?;
        let entry = BASE + language.resource() * 8;
        let offset = word(data, entry)?;
        let len = word(data, entry + 4)?;
        if offset < directory_len || !(HEADER..=MAX_BOOK).contains(&len) {
            return Err(Error::new("Invalid Help book resource extent"));
        }
        let start = BASE
            .checked_add(offset)
            .ok_or_else(|| Error::new("Help book offset overflow"))?;
        let mut book = Self::from_book(bytes(data, start, len)?, language)?;
        // A demo is an optional UTF-8 program in its own archive entry. The
        // localized Help book only holds the reference used by its picker.
        book.demo_programs = (0..directory_len / 8)
            .filter_map(|index| {
                let entry = BASE + index * 8;
                let offset = word(data, entry).ok()?;
                let len = word(data, entry + 4).ok()?;
                if offset < directory_len || len > 64 * 1024 {
                    return None;
                }
                let section = bytes(data, BASE.checked_add(offset)?, len).ok()?;
                Self::decode_demo(section)
            })
            .collect();
        Ok(book)
    }

    fn decode_demo(section: &[u8]) -> Option<(String, String)> {
        let text = std::str::from_utf8(section).ok()?;
        let name = text.lines().find_map(|line| {
            let (name, _) = line.trim_start().strip_prefix("EXPORT ")?.split_once('(')?;
            let suffix = name.strip_prefix("Demo_")?;
            (!suffix.is_empty()
                && suffix
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_'))
            .then_some(name)
        })?;
        let source = ppl_syntax::source::decode_text(section).ok()?.text;
        Some((name.into(), source))
    }

    pub fn topic_demo_programs(&self, topic: &Topic) -> Vec<(&str, &str)> {
        let Some(examples) = topic.spans.get(2) else {
            return Vec::new();
        };
        let Ok(examples) = String::from_utf16(examples) else {
            return Vec::new();
        };
        examples
            .lines()
            .filter_map(|line| {
                let name = line.trim().strip_prefix("__$")?;
                let (name, source) = self.demo_programs.get_key_value(name)?;
                Some((name.as_str(), source.as_str()))
            })
            .collect()
    }

    fn from_book(data: &[u8], language: Language) -> Result<Self> {
        if word(data, 0)? != 0xb28a617c
            || word(data, 4)? != 0xfe84125d
            || word(data, 8)? != HEADER
            || word(data, 12)? != language as usize
        {
            return Err(Error::new("Unsupported Help book header or language"));
        }
        let count = word(data, 28)?;
        if !(1..=10000).contains(&count) {
            return Err(Error::new("Invalid Help topic count"));
        }
        let index_start = word(data, 32)?;
        let index_len = word(data, 36)?;
        let dict_start = word(data, 40)?;
        let dict_len = word(data, 44)?;
        let text_start = word(data, 48)?;
        let text_len = word(data, 52)?;
        let group_count = word(data, 56)?;
        let groups_index = word(data, 60)?;
        let groups_start = word(data, 64)?;
        let groups_len = word(data, 68)?;
        if dict_start != HEADER
            || dict_start.checked_add(dict_len) != Some(index_start)
            || index_start.checked_add(index_len) != Some(text_start)
            || text_start.checked_add(text_len) != Some(groups_index)
            || group_count > 10000
            || groups_index.checked_add(group_count * 12) != Some(groups_start)
            || groups_start.checked_add(groups_len) != Some(data.len())
        {
            return Err(Error::new("Overlapping or inconsistent Help book sections"));
        }
        let decoder = huffman::Decoder::new(bytes(data, dict_start, dict_len)?)?;
        let index = bytes(data, index_start, index_len)?;
        let texts = bytes(data, text_start, text_len)?;
        let mut at = 0;
        let mut text_end = 0;
        let mut remaining = MAX_TEXT_UNITS;
        let mut topics = Vec::with_capacity(count);
        for id in 1..=count {
            let record = bytes(index, at, 12)?;
            let ident = word(record, 0)?;
            let offset = word(record, 4)?;
            let len = usize::from(short(record, 8)?);
            let name_len = usize::from(record[11]) * 2;
            let name_units = bytes(index, at + 12, name_len)?
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes([v[0], v[1]]))
                .collect::<Vec<_>>();
            let name = String::from_utf16(&name_units)
                .map_err(|_| Error::new("Invalid Help topic name"))?;
            if short(index, at + 12 + name_len)? != 0 || offset != text_end || len < 2 {
                return Err(Error::new("Invalid Help topic index record"));
            }
            let encoded = bytes(texts, offset, len)?;
            let mode = short(encoded, 0)?;
            if mode > 2 {
                return Err(Error::new("Unknown Help topic mode"));
            }
            let vacant =
                ident == 0 && name.is_empty() && len == 2 && record[10] == 128 && mode == 0;
            if ident != id && !vacant {
                return Err(Error::new("Invalid Help topic identity"));
            }
            let spans = if vacant {
                Vec::new()
            } else {
                decoder.spans(&encoded[2..], &mut remaining)?
            };
            topics.push(Topic {
                id: ident as u32,
                name,
                kind: record[10],
                mode,
                spans,
            });
            at += 14 + name_len;
            text_end += len;
        }
        if at != index.len() || text_end != texts.len() {
            return Err(Error::new("Trailing Help topic data"));
        }
        let mut groups = Vec::with_capacity(group_count);
        let encoded_groups = bytes(data, groups_start, groups_len)?;
        let mut group_end = 0;
        let mut previous = None;
        for index in 0..group_count {
            let record = groups_index + index * 12;
            let descriptor = word(data, record)? as u32;
            let id = descriptor & 0x7fffff;
            let count = (descriptor >> 23) as usize;
            let offset = word(data, record + 4)?;
            // The original reader masks the high length bit before admission.
            let len = word(data, record + 8)? & 0x7fffffff;
            if offset != group_end
                || len == 0
                || count == 0
                || previous.is_some_and(|before| id <= before)
            {
                return Err(Error::new("Invalid Help string group extent or identity"));
            }
            let strings =
                decoder.strings(bytes(encoded_groups, offset, len)?, count, &mut remaining)?;
            groups.push(StringGroup { id, strings });
            previous = Some(id);
            group_end += len;
        }
        if group_end != groups_len {
            return Err(Error::new("Trailing Help string group data"));
        }
        Ok(Self {
            language,
            topics,
            groups,
            demo_programs: BTreeMap::new(),
        })
    }

    /// Group and string identities are firmware metadata, not Native command
    /// identifiers. An absent group or index remains absent, without fallback.
    pub fn string(&self, group: u32, index: usize) -> Option<&[u16]> {
        self.groups
            .binary_search_by_key(&group, |group| group.id)
            .ok()
            .and_then(|position| self.groups[position].strings.get(index))
            .map(Vec::as_slice)
    }

    pub fn topic(&self, id: u32) -> Option<&Topic> {
        id.checked_sub(1)
            .and_then(|index| self.topics.get(index as usize))
            .filter(|topic| topic.id == id)
    }

    /// Names are not unique in the original index (e.g. each app has its own
    /// Symbolic Variables topic). Keep all matches with their stable identities.
    pub fn named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Topic> {
        self.topics
            .iter()
            .filter(move |topic| !topic.name.is_empty() && topic.name.eq_ignore_ascii_case(name))
    }
}
