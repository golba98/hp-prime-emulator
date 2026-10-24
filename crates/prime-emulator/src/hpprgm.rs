//! HP Prime `.hpprgm` program container: strict parser and serializer.
//!
//! Layout established in M5 from three independent sources that agree on
//! every byte of structure (docs/emulator/M5_PROGRAM_CATALOG_REPORT.md):
//! files the emulated 2.4.2 firmware wrote itself (`C:\DATA\HELLO.hpprgm`
//! on creation and on leaving its Program Editor), and two genuine device
//! files (ADVPMT, Leasing20).
//!
//! ```text
//! u32 magic 0xB28A617C, u32 0xFFFFFFFE, u32 0
//! object*                       until end of file
//! object    = u32 len (bytes after this field), u32 type, payload[len - 4]
//! list      = object whose kind (type & 0x3F) is 0x0E or 0x3E;
//!             payload = group*, group = u32 len, object*[len]
//! ```
//!
//! A program saved by the firmware holds three header ints, an optional
//! export cache ([`SECTION_EXPORTS`], [`SECTION_VARIABLES`]: names and
//! values the firmware derives when it checks a program), the source
//! section ([`SECTION_FILES`]: one group `name64, int, string`) and a
//! fixed 10-slot table ([`SECTION_TABLE`]). The source string is UTF-16LE
//! with one NUL terminator and LF line endings.
//!
//! [`build_program`] emits only the structure the firmware itself wrote
//! for a program it had not yet checked (no export cache), plus the
//! source; it never fabricates compiled data.

use std::fmt;

pub const MAGIC: u32 = 0xB28A_617C;
const HEADER_WORD1: u32 = 0xFFFF_FFFE;
const HEADER_WORD2: u32 = 0;

/// Header ints, in file order (values: 0, 0, 0 in firmware-made files).
pub const HEADER_INTS: [u32; 3] = [0x007F_FF05, 0x023F_FF05, 0x00BF_FF05];
/// Exported functions: groups of `(name64, int flags)`.
pub const SECTION_EXPORTS: u32 = 0x0100_023E;
/// Exported variables: groups of `(name64, int flags, value)`.
pub const SECTION_VARIABLES: u32 = 0x00C0_01BE;
/// Source files: groups of `(name64, int, string)`.
pub const SECTION_FILES: u32 = 0x0140_00BE;
/// Ten `(name64, value)` slots; all names empty in every sample.
pub const SECTION_TABLE: u32 = 0x0240_028E;

const FILE_NAME: u32 = 0x0040_008B;
const FILE_INT: u32 = 0x0080_0085;
const FILE_SOURCE: u32 = 0x00C0_009B;
const TABLE_NAME: u32 = 0x0040_028B;
const TABLE_VALUE: u32 = 0x0080_0284;
const TABLE_SLOTS: usize = 10;

/// Payload size of a `name64` object (UTF-16LE, NUL-padded).
pub const NAME64_BYTES: usize = 64;
/// Name the 2.4.2 firmware gives a program's single source file.
pub const MAIN_FILE: &str = "Main";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Truncated { offset: usize, need: usize },
    BadMagic(u32),
    BadHeader { offset: usize, detail: String },
    BadLength { offset: usize, len: u32 },
    GroupOverrun { offset: usize },
    MissingSource,
    DuplicateSource,
    BadSource { offset: usize, detail: String },
    InvalidUtf16 { offset: usize },
    MissingNul { offset: usize },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Truncated { offset, need } => {
                write!(f, "truncated at {offset:#x}: {need} more byte(s) needed")
            }
            Error::BadMagic(m) => write!(
                f,
                "not an HP Prime program container (magic {m:#010x}, expected {MAGIC:#010x})"
            ),
            Error::BadHeader { offset, detail } => write!(f, "bad header at {offset:#x}: {detail}"),
            Error::BadLength { offset, len } => {
                write!(
                    f,
                    "object at {offset:#x}: length {len:#x} exceeds its container"
                )
            }
            Error::GroupOverrun { offset } => {
                write!(f, "group at {offset:#x}: objects overrun the group")
            }
            Error::MissingSource => write!(f, "no source section (type {SECTION_FILES:#010x})"),
            Error::DuplicateSource => write!(f, "more than one source section"),
            Error::BadSource { offset, detail } => {
                write!(f, "source section at {offset:#x}: {detail}")
            }
            Error::InvalidUtf16 { offset } => write!(f, "invalid UTF-16 at {offset:#x}"),
            Error::MissingNul { offset } => {
                write!(f, "string at {offset:#x} is not NUL-terminated")
            }
        }
    }
}

impl std::error::Error for Error {}

/// One object: its type word and either a raw payload or list groups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub ty: u32,
    pub body: Body,
    /// File offset of the object's length field (0 for built objects).
    pub offset: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Data(Vec<u8>),
    List(Vec<Vec<Object>>),
}

/// `type & 0x3F`.
pub fn kind(ty: u32) -> u32 {
    ty & 0x3F
}

fn is_list(ty: u32) -> bool {
    matches!(kind(ty), 0x0E | 0x3E)
}

impl Object {
    pub fn data(ty: u32, payload: Vec<u8>) -> Object {
        Object {
            ty,
            body: Body::Data(payload),
            offset: 0,
        }
    }

    pub fn list(ty: u32, groups: Vec<Vec<Object>>) -> Object {
        Object {
            ty,
            body: Body::List(groups),
            offset: 0,
        }
    }

    pub fn payload(&self) -> Option<&[u8]> {
        match &self.body {
            Body::Data(d) => Some(d),
            Body::List(_) => None,
        }
    }

    pub fn groups(&self) -> &[Vec<Object>] {
        match &self.body {
            Body::List(g) => g,
            Body::Data(_) => &[],
        }
    }

    fn encode(&self, out: &mut Vec<u8>) {
        let len_at = out.len();
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&self.ty.to_le_bytes());
        match &self.body {
            Body::Data(d) => out.extend_from_slice(d),
            Body::List(groups) => {
                for g in groups {
                    let glen_at = out.len();
                    out.extend_from_slice(&[0; 4]);
                    for o in g {
                        o.encode(out);
                    }
                    let glen = (out.len() - glen_at - 4) as u32;
                    out[glen_at..glen_at + 4].copy_from_slice(&glen.to_le_bytes());
                }
            }
        }
        let len = (out.len() - len_at - 4) as u32;
        out[len_at..len_at + 4].copy_from_slice(&len.to_le_bytes());
    }
}

/// A parsed container. `objects` covers the whole file after the 12-byte
/// preamble (header ints included), so [`Container::to_bytes`] reproduces
/// the input exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    pub objects: Vec<Object>,
}

/// The source section's single file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    /// `name64` of the file (`"Main"` on 2.4.2, empty in older files).
    pub name: String,
    pub int: u32,
    pub source: String,
    /// False for a program created but never edited (no string object).
    pub has_text: bool,
    /// Offset of the source string object (of the section without one).
    pub offset: usize,
}

fn u32_at(d: &[u8], at: usize) -> Result<u32, Error> {
    let b = d.get(at..at + 4).ok_or(Error::Truncated {
        offset: at,
        need: at + 4 - d.len().min(at + 4),
    })?;
    Ok(u32::from_le_bytes(b.try_into().unwrap()))
}

fn parse_objects(d: &[u8], mut at: usize, end: usize) -> Result<Vec<Object>, Error> {
    let mut out = Vec::new();
    while at < end {
        if end - at < 8 {
            return Err(Error::Truncated {
                offset: at,
                need: 8 - (end - at),
            });
        }
        let len = u32_at(d, at)?;
        if len < 4 || (len as usize) > end - at - 4 {
            return Err(Error::BadLength { offset: at, len });
        }
        let ty = u32_at(d, at + 4)?;
        let obj_end = at + 4 + len as usize;
        let body = if is_list(ty) {
            let mut groups = Vec::new();
            let mut g = at + 8;
            while g < obj_end {
                let glen = u32_at(d, g)? as usize;
                if glen > obj_end - g - 4 {
                    return Err(Error::GroupOverrun { offset: g });
                }
                groups.push(parse_objects(d, g + 4, g + 4 + glen)?);
                g += 4 + glen;
            }
            Body::List(groups)
        } else {
            Body::Data(d[at + 8..obj_end].to_vec())
        };
        out.push(Object {
            ty,
            body,
            offset: at,
        });
        at = obj_end;
    }
    Ok(out)
}

/// Strict parse: magic, preamble, the three header ints, and an exact
/// object tree to the last byte.
pub fn parse(d: &[u8]) -> Result<Container, Error> {
    let magic = u32_at(d, 0)?;
    if magic != MAGIC {
        return Err(Error::BadMagic(magic));
    }
    for (at, want) in [(4, HEADER_WORD1), (8, HEADER_WORD2)] {
        let got = u32_at(d, at)?;
        if got != want {
            return Err(Error::BadHeader {
                offset: at,
                detail: format!("{got:#010x}, expected {want:#010x}"),
            });
        }
    }
    let objects = parse_objects(d, 12, d.len())?;
    for (i, want) in HEADER_INTS.iter().enumerate() {
        match objects.get(i) {
            Some(o) if o.ty == *want && o.payload().is_some_and(|p| p.len() == 4) => {}
            other => {
                return Err(Error::BadHeader {
                    offset: other.map_or(d.len(), |o| o.offset),
                    detail: format!("header int {i} must be type {want:#010x} with 4 bytes"),
                });
            }
        }
    }
    Ok(Container { objects })
}

/// UTF-16LE up to the first NUL; `strict` also requires nothing but the
/// terminator after it.
fn utf16_string(p: &[u8], offset: usize, strict: bool) -> Result<String, Error> {
    if p.len() % 2 != 0 {
        return Err(Error::InvalidUtf16 { offset });
    }
    let units: Vec<u16> = p
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let nul = units
        .iter()
        .position(|&u| u == 0)
        .ok_or(Error::MissingNul { offset })?;
    if strict && nul + 1 != units.len() {
        return Err(Error::BadSource {
            offset,
            detail: format!("{} unit(s) after the NUL terminator", units.len() - nul - 1),
        });
    }
    String::from_utf16(&units[..nul]).map_err(|_| Error::InvalidUtf16 { offset })
}

/// A `name64` payload as text (bytes after the NUL are not significant:
/// zero in device files, uninitialised memory in emulated saves).
pub fn name64(p: &[u8]) -> Option<String> {
    (p.len() == NAME64_BYTES)
        .then(|| utf16_string(p, 0, false).ok())
        .flatten()
}

impl Container {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC.to_le_bytes());
        out.extend_from_slice(&HEADER_WORD1.to_le_bytes());
        out.extend_from_slice(&HEADER_WORD2.to_le_bytes());
        for o in &self.objects {
            o.encode(&mut out);
        }
        out
    }

    pub fn section(&self, ty: u32) -> Option<&Object> {
        self.objects.iter().find(|o| o.ty == ty)
    }

    /// Header int values.
    pub fn header_ints(&self) -> [u32; 3] {
        let v = |i: usize| {
            let p = self.objects[i].payload().unwrap();
            u32::from_le_bytes(p.try_into().unwrap())
        };
        [v(0), v(1), v(2)]
    }

    /// Export cache: `(name, flags)` per exported function, as the firmware
    /// last derived it (empty when the program has not been checked).
    pub fn exports(&self) -> Vec<(String, u32)> {
        let Some(sec) = self.section(SECTION_EXPORTS) else {
            return Vec::new();
        };
        sec.groups()
            .iter()
            .filter_map(|g| {
                let name = name64(g.first()?.payload()?)?;
                let flags = g.get(1)?.payload()?;
                Some((name, u32::from_le_bytes(flags.try_into().ok()?)))
            })
            .collect()
    }

    /// The program source. Strict: exactly one source section with one
    /// `(name64, int, string)` group and a single NUL terminator.
    pub fn source(&self) -> Result<SourceFile, Error> {
        let mut sections = self.objects.iter().filter(|o| o.ty == SECTION_FILES);
        let sec = sections.next().ok_or(Error::MissingSource)?;
        if sections.next().is_some() {
            return Err(Error::DuplicateSource);
        }
        let bad = |detail: &str| Error::BadSource {
            offset: sec.offset,
            detail: detail.to_owned(),
        };
        let [group] = sec.groups() else {
            return Err(bad("expected exactly one file group"));
        };
        // A program the firmware created but nobody edited yet has no
        // string object (observed: the Program Catalog's New writes
        // `(name64, int)` only).
        let (name, int, text) = match group.as_slice() {
            [name, int] => (name, int, None),
            [name, int, text] => (name, int, Some(text)),
            _ => return Err(bad("expected (name64, int[, string])")),
        };
        let (Some(name_p), Some(int_p)) = (name.payload(), int.payload()) else {
            return Err(bad("unexpected list in file group"));
        };
        if name.ty != FILE_NAME || int.ty != FILE_INT || text.is_some_and(|t| t.ty != FILE_SOURCE) {
            return Err(bad(&format!(
                "object types {:#010x}/{:#010x}/{:#010x}",
                name.ty,
                int.ty,
                text.map_or(0, |t| t.ty)
            )));
        }
        let name_s =
            name64(name_p).ok_or_else(|| bad("file name is not a NUL-terminated name64"))?;
        let int_v = u32::from_le_bytes(int_p.try_into().map_err(|_| bad("int is not 4 bytes"))?);
        let (source, offset) = match text {
            None => (String::new(), sec.offset),
            Some(t) => {
                let p = t.payload().ok_or_else(|| bad("source is a list"))?;
                (utf16_string(p, t.offset, true)?, t.offset)
            }
        };
        Ok(SourceFile {
            name: name_s,
            int: int_v,
            has_text: text.is_some(),
            source,
            offset,
        })
    }
}

fn name64_payload(name: &str) -> Vec<u8> {
    let mut p: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
    p.resize(NAME64_BYTES, 0);
    p
}

/// Source payload: UTF-16LE plus one NUL unit.
pub fn source_payload(source: &str) -> Vec<u8> {
    source
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildError {
    /// The firmware stores NUL-terminated text.
    SourceContainsNul,
    /// The firmware stores LF line endings; normalise first.
    SourceContainsCr,
    FileNameTooLong,
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            BuildError::SourceContainsNul => "source contains a NUL character",
            BuildError::SourceContainsCr => {
                "source contains CR; the firmware stores LF line endings"
            }
            BuildError::FileNameTooLong => "file name does not fit a name64 field",
        })
    }
}

impl std::error::Error for BuildError {}

/// A container with the structure the firmware writes for a program it has
/// not checked yet (header ints 0, no export cache), holding `source` as
/// its `Main` file. Name64 padding and table names are zero, as in device
/// files.
pub fn build_program(source: &str) -> Result<Container, BuildError> {
    if source.contains('\0') {
        return Err(BuildError::SourceContainsNul);
    }
    if source.contains('\r') {
        return Err(BuildError::SourceContainsCr);
    }
    if MAIN_FILE.encode_utf16().count() * 2 >= NAME64_BYTES {
        return Err(BuildError::FileNameTooLong);
    }
    let mut objects: Vec<Object> = HEADER_INTS
        .iter()
        .map(|&ty| Object::data(ty, vec![0; 4]))
        .collect();
    objects.push(Object::list(
        SECTION_FILES,
        vec![vec![
            Object::data(FILE_NAME, name64_payload(MAIN_FILE)),
            Object::data(FILE_INT, vec![0; 4]),
            Object::data(FILE_SOURCE, source_payload(source)),
        ]],
    ));
    let slot = || {
        vec![
            Object::data(TABLE_NAME, vec![0; NAME64_BYTES]),
            Object::data(TABLE_VALUE, vec![0xFF; 8]),
        ]
    };
    objects.push(Object::list(
        SECTION_TABLE,
        (0..TABLE_SLOTS).map(|_| slot()).collect(),
    ));
    Ok(Container { objects })
}

/// `bytes` with the not-significant bytes of every `name64` (after its
/// NUL) zeroed: compares an emulated save, whose padding is uninitialised
/// memory, with a device-style file.
pub fn canonical_padding(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let c = parse(bytes)?;
    let mut out = bytes.to_vec();
    fn walk(objs: &[Object], out: &mut [u8]) {
        for o in objs {
            match &o.body {
                Body::Data(p) if kind(o.ty) == 0x0B && p.len() == NAME64_BYTES => {
                    let start = o.offset + 8;
                    if let Some(nul) = p.chunks_exact(2).position(|c| c == [0, 0]) {
                        out[start + nul * 2..start + NAME64_BYTES].fill(0);
                    }
                }
                Body::Data(_) => {}
                Body::List(groups) => groups.iter().for_each(|g| walk(g, out)),
            }
        }
    }
    walk(&c.objects, &mut out);
    Ok(out)
}
