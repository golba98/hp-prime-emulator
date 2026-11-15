//! Literal desktop text through the installed G1's own Paste/text-event ABI.
//! The main UI owner calls the same methods as Paste selection. Established
//! Unicode math aliases consult genuine editor/caret queries; literal editors
//! and strings retain their scalars. No widget-memory edit or host evaluation.
use crate::{
    clipboard,
    guest_call::{Arg, Call, Returned, Step},
    memory::GuestMemory,
    sha256,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
const APP: u32 = 0x30d6_a908;
const MANAGER: u32 = 0x30d6_a910;
const SET_TEXT: u32 = 0x30af_d6e4;
const FIND_VIEW: u32 = 0x306a_59c0;
const INIT_EVENT: u32 = 0x3061_7220;
const DROP_EVENT: u32 = 0x3068_5b14;
const EVENT_BYTES: usize = 28;
const GET_TEXT: u32 = 0x3060_eb30;
const GET_CURSOR: u32 = 0x306a_e460;
const HOME_TEXT: u32 = 0x3099_32e0;
const EDITOR_TEXT: u32 = 0x309c_2028;
const MAIN_FRAME: u32 = 0x30cd_bf88;
const RANGES: [(u32, u32); 13] = [
    (SET_TEXT, 0x30af_d788),
    (FIND_VIEW, 0x306a_5a0c),
    (INIT_EVENT, 0x3061_724c),
    (DROP_EVENT, 0x3068_5b28),
    (0x3061_24c8, 0x3061_2514),
    (0x30b1_c328, 0x30b1_c32c),
    (0x3060_c458, 0x3060_c610),
    (GET_TEXT, 0x3060_ebfc),
    (GET_CURSOR, 0x306a_e478),
    (HOME_TEXT, 0x3099_3320),
    (0x306c_264c, 0x306c_268c),
    (0x3060_c13c, 0x3060_c380),
    (MAIN_FRAME, 0x30cd_bfe8),
];
const CODE: &str = "5f34fe415f903d68268ea043342cdd4a1865f3b9e5aa2a742af35957d9a9dceb";
const MAX_UNITS: usize = ppl_engine::session::MAX_TEXT_UTF16_UNITS;
type Result<T> = std::result::Result<T, String>;
pub(crate) fn supported(mem: &GuestMemory) -> bool {
    let bytes = RANGES
        .iter()
        .try_fold(Vec::new(), |mut bytes, (start, end)| {
            bytes.extend(mem.read_bytes_alloc(*start, (end - start) as usize).ok()?);
            Some(bytes)
        });
    bytes.is_some_and(|bytes| sha256::hex(&bytes) == CODE)
}
pub(crate) fn validate_text(text: &str) -> Result<()> {
    ppl_engine::session::validate_desktop_text(text).map_err(str::to_owned)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Binding {
    pub token: u64,
    pub sha256: String,
    pub units: usize,
    pub paste: bool,
}
impl Binding {
    pub(crate) fn matches(&self, text: &str, paste: bool) -> bool {
        self.paste == paste
            && self.units == text.encode_utf16().count()
            && self.sha256 == sha256::hex(text.as_bytes())
    }
    fn valid(&self) -> bool {
        self.token > 0
            && self.units <= MAX_UNITS
            && self.sha256.len() == 64
            && self.sha256.bytes().all(|b| b.is_ascii_hexdigit())
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    next: u64,
    slot: Option<Slot>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Slot {
    binding: Binding,
    cancelled: bool,
    outcome: Option<Result<()>>,
}
#[derive(Default)]
pub(crate) struct Mailbox {
    state: Mutex<State>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MailboxImage {
    version: u32,
    state: State,
}
impl Default for MailboxImage {
    fn default() -> Self {
        Self {
            version: 1,
            state: State::default(),
        }
    }
}
impl MailboxImage {
    pub(crate) fn restore(&self) -> Result<Mailbox> {
        if self.version != 1
            || self.state.slot.as_ref().is_some_and(|s| {
                !s.binding.valid()
                    || s.binding.token > self.state.next
                    || s.outcome
                        .as_ref()
                        .is_some_and(|o| o.as_ref().err().is_some_and(|e| e.len() > 4096))
            })
        {
            return Err("Invalid desktop-text completion image".into());
        }
        Ok(Mailbox {
            state: Mutex::new(self.state.clone()),
        })
    }
}
impl Mailbox {
    pub(crate) fn image(&self) -> MailboxImage {
        MailboxImage {
            version: 1,
            state: self.state.lock().unwrap().clone(),
        }
    }
    fn reserve(&self, text: &str, paste: bool) -> Result<Binding> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Text completion owner is poisoned")?;
        if state
            .slot
            .as_ref()
            .is_some_and(|s| s.cancelled && s.outcome.is_some())
        {
            state.slot = None;
        }
        if state.slot.is_some() {
            return Err("A firmware text operation is already pending".into());
        }
        state.next = state
            .next
            .checked_add(1)
            .ok_or("Text operation identifiers exhausted")?;
        let binding = Binding {
            token: state.next,
            sha256: sha256::hex(text.as_bytes()),
            units: text.encode_utf16().count(),
            paste,
        };
        state.slot = Some(Slot {
            binding: binding.clone(),
            cancelled: false,
            outcome: None,
        });
        Ok(binding)
    }
    pub(crate) fn validate_binding(&self, binding: &Binding, pending: bool) -> Result<()> {
        let state = self
            .state
            .lock()
            .map_err(|_| "Text completion owner is poisoned")?;
        if !state
            .slot
            .as_ref()
            .is_some_and(|s| &s.binding == binding && (!pending || s.outcome.is_none()))
        {
            return Err("Desktop text plan/completion binding mismatch".into());
        }
        Ok(())
    }
    pub(crate) fn cancelled(&self, binding: &Binding) -> bool {
        self.state
            .lock()
            .unwrap()
            .slot
            .as_ref()
            .is_none_or(|s| s.binding != *binding || s.cancelled)
    }
    pub(crate) fn cancel(&self, binding: &Binding) {
        let mut state = self.state.lock().unwrap();
        if let Some(slot) = state.slot.as_mut().filter(|s| s.binding == *binding) {
            slot.cancelled = true;
        }
    }
    fn finish(&self, binding: &Binding, outcome: Result<()>) {
        if let Some(slot) = self
            .state
            .lock()
            .unwrap()
            .slot
            .as_mut()
            .filter(|s| s.binding == *binding && s.outcome.is_none())
        {
            slot.outcome = Some(outcome);
        }
    }
    pub(crate) fn take(&self, binding: &Binding) -> Result<Option<Result<()>>> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Text completion owner is poisoned")?;
        let slot = state
            .slot
            .as_ref()
            .filter(|s| s.binding == *binding)
            .ok_or("Desktop text completion was lost")?;
        if slot.outcome.is_none() {
            return Ok(None);
        }
        Ok(state.slot.take().unwrap().outcome)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Stage {
    SetClipboard,
    FindView,
    OfferRecord,
    QueryText,
    QueryCursor,
    InitEvent,
    FindViewAgain,
    DispatchText,
    DestroyEvent,
}
/// Lexical state at the actual firmware caret. Only the established Home/CAS
/// math editor enables these desktop Unicode aliases; literal editors keep the
/// supplied scalars. The clipboard itself always keeps the original source.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Lexical {
    quoted: bool,
    escaped: bool,
}
impl Lexical {
    fn accept(&mut self, unit: u16) {
        if self.quoted && self.escaped {
            self.escaped = false;
        } else if self.quoted && unit == u16::from(b'\\') {
            self.escaped = true;
        } else if unit == u16::from(b'"') {
            self.quoted = !self.quoted;
        }
    }
}
fn aliases(text: &str, mut state: Lexical) -> String {
    text.chars()
        .map(|ch| {
            let result = if state.quoted {
                ch
            } else {
                match ch {
                    'ⅈ' => '\u{e003}',
                    '×' | '·' => '*',
                    '÷' => '/',
                    '−' => '-',
                    _ => ch,
                }
            };
            if ch.is_ascii() {
                state.accept(ch as u16);
            } else {
                state.escaped = false;
            }
            result
        })
        .collect()
}
fn has_alias(text: &str) -> bool {
    text.contains(['ⅈ', '×', '·', '÷', '−'])
}
fn legacy_flags() -> u32 {
    0x100
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Image {
    version: u32,
    pub binding: Binding,
    stage: Stage,
    pub started: bool,
    app: u32,
    source: u32,
    view: u32,
    event: u32,
    #[serde(default = "legacy_flags")]
    flags: u32,
    #[serde(default)]
    editor: u32,
    #[serde(default)]
    context: u32,
    #[serde(default)]
    argument: u32,
    #[serde(default)]
    lexical: Option<Lexical>,
}
pub(crate) struct Plan {
    image: Image,
    memory: Arc<GuestMemory>,
    mailbox: Arc<Mailbox>,
    finished: bool,
}
fn args(values: [u32; 4]) -> [Arg; 4] {
    values.map(Arg::Value)
}
fn bound_text(mem: &GuestMemory, address: u32, binding: &Binding) -> Result<String> {
    let bytes = mem
        .read_bytes_alloc(address, (binding.units + 1) * 2)
        .map_err(|e| e.to_string())?;
    let mut units = bytes
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect::<Vec<_>>();
    if units.pop() != Some(0) || units.contains(&0) {
        return Err("Desktop text source terminator mismatch".into());
    }
    let text = String::from_utf16(&units).map_err(|e| e.to_string())?;
    if sha256::hex(text.as_bytes()) != binding.sha256 {
        return Err("Desktop text differs from its bound source".into());
    }
    Ok(text)
}
fn text_at(mem: &GuestMemory, address: u32, binding: &Binding) -> Result<()> {
    bound_text(mem, address, binding).map(|_| ())
}
fn callback(mem: &GuestMemory, view: u32, offset: u32) -> Result<u32> {
    mem.validate_range(view, 4).map_err(|e| e.to_string())?;
    let table = mem.read_u32(view).map_err(|e| e.to_string())?;
    let target = mem
        .read_u32(
            table
                .checked_add(offset)
                .ok_or("View method address overflow")?,
        )
        .map_err(|e| e.to_string())?;
    if target == 0 || target % 4 != 0 {
        return Err("Active firmware view has no admitted ARM text method".into());
    }
    mem.validate_range(target, 4).map_err(|e| e.to_string())?;
    Ok(target)
}
impl Image {
    pub(crate) fn validate(&self, mem: &GuestMemory) -> Result<()> {
        if self.version != 1
            || !self.binding.valid()
            || !supported(mem)
            || self.app == 0
            || !matches!(self.flags, 0 | 0x100)
            || mem.read_u32(APP).ok() != Some(self.app)
        {
            return Err("Firmware desktop-text ABI/source identity mismatch".into());
        }
        mem.validate_range(self.app, 0x10a8)
            .map_err(|e| e.to_string())?;
        if self.source != 0 {
            if mem
                .heap_size_of(self.source)
                .is_none_or(|n| n < (self.binding.units + 1) * 2)
            {
                return Err("Desktop source allocation is absent".into());
            }
            text_at(mem, self.source, &self.binding)?;
        } else if !self.binding.paste || self.stage == Stage::SetClipboard {
            return Err("Desktop source ownership/phase mismatch".into());
        }
        if self.binding.paste && self.stage != Stage::SetClipboard {
            let record = mem.read_u32(self.app + 0x10a4).map_err(|e| e.to_string())?;
            let text = clipboard::text_record(mem, record)?;
            if !self.binding.matches(&text, true) {
                return Err("Firmware clipboard changed during its bound paste".into());
            }
        }
        if self.argument != 0 {
            if self.argument == self.source
                || self.argument == self.event
                || mem
                    .heap_size_of(self.argument)
                    .is_none_or(|n| n < (self.binding.units + 1) * 2)
            {
                return Err("Desktop math argument allocation is absent or aliased".into());
            }
            let original = self.original(mem)?;
            let lexical = self
                .lexical
                .ok_or("Desktop math argument lacks lexical provenance")?;
            let expected = aliases(&original, lexical);
            let actual = mem
                .read_wstr_bounded(self.argument, MAX_UNITS + 1)
                .map_err(|e| e.to_string())?
                .ok_or("Missing desktop math argument")?;
            if expected == original || actual != expected {
                return Err(
                    "Desktop math argument differs from its original source/context".into(),
                );
            }
        } else if self.lexical.is_some() {
            return Err("Desktop math argument provenance lacks its allocation".into());
        }
        if matches!(self.stage, Stage::QueryText | Stage::QueryCursor) {
            if self.editor
                != self
                    .app
                    .checked_add(0x110)
                    .ok_or("Editor address overflow")?
            {
                return Err("Desktop math query owner differs".into());
            }
            if self.stage == Stage::QueryCursor {
                if self.context == 0 || mem.read_u32(self.editor + 0xd0).ok() != Some(self.context)
                {
                    return Err("Desktop math query lost its borrowed firmware source".into());
                }
                mem.read_wstr_bounded(self.context, MAX_UNITS + 1)
                    .map_err(|e| e.to_string())?;
            }
        }
        if self.event != 0 {
            if mem.heap_size_of(self.event).is_none_or(|n| n < EVENT_BYTES) {
                return Err("Desktop event allocation is absent".into());
            }
            if matches!(
                self.stage,
                Stage::FindViewAgain | Stage::DispatchText | Stage::DestroyEvent
            ) && (mem.read_u16(self.event).ok().map(|x| x & 255) != Some(10)
                || mem.read_u32(self.event + 8).ok() != Some(self.flags)
                || mem.read_bytes_alloc(self.event + 18, 1).ok().as_deref() != Some(&[0]))
            {
                return Err("Desktop text event layout/ownership mismatch".into());
            }
        } else if matches!(
            self.stage,
            Stage::InitEvent | Stage::FindViewAgain | Stage::DispatchText | Stage::DestroyEvent
        ) {
            return Err("Desktop text event phase lacks its allocation".into());
        }
        if matches!(self.stage, Stage::OfferRecord | Stage::DispatchText) {
            callback(
                mem,
                self.view,
                if self.stage == Stage::OfferRecord {
                    0x58
                } else {
                    0x54
                },
            )?;
        }
        Ok(())
    }
    fn original(&self, mem: &GuestMemory) -> Result<String> {
        if self.source != 0 {
            return bound_text(mem, self.source, &self.binding);
        }
        let record = mem.read_u32(self.app + 0x10a4).map_err(|e| e.to_string())?;
        let text = clipboard::text_record(mem, record)?;
        if !self.binding.matches(&text, true) {
            return Err("Desktop original clipboard source differs".into());
        }
        Ok(text)
    }
    pub(crate) fn restore(&self, mem: Arc<GuestMemory>, mailbox: Arc<Mailbox>) -> Result<Plan> {
        self.validate(&mem)?;
        mailbox.validate_binding(&self.binding, true)?;
        Ok(Plan {
            image: self.clone(),
            memory: mem,
            mailbox,
            finished: false,
        })
    }
}
impl Plan {
    pub(crate) fn new(
        memory: Arc<GuestMemory>,
        mailbox: Arc<Mailbox>,
        text: &str,
        paste: bool,
    ) -> Result<Self> {
        validate_text(text)?;
        if !supported(&memory) {
            return Err("No verified firmware text-entry ABI for this build".into());
        }
        let app = memory.read_u32(APP).map_err(|e| e.to_string())?;
        memory
            .validate_range(app, 0x10a8)
            .map_err(|e| e.to_string())?;
        let bytes = text
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        let source = memory.heap_alloc(bytes.len()).map_err(|e| e.to_string())?;
        if let Err(e) = memory.write_bytes(source, &bytes) {
            memory.heap_free(source);
            return Err(e.to_string());
        }
        let binding = match mailbox.reserve(text, paste) {
            Ok(binding) => binding,
            Err(e) => {
                memory.heap_free(source);
                return Err(e);
            }
        };
        Ok(Self {
            image: Image {
                version: 1,
                binding,
                stage: if paste {
                    Stage::SetClipboard
                } else {
                    Stage::FindView
                },
                started: false,
                app,
                source,
                view: 0,
                event: 0,
                flags: if paste { 0x100 } else { 0 },
                editor: 0,
                context: 0,
                argument: 0,
                lexical: None,
            },
            memory,
            mailbox,
            finished: false,
        })
    }
    pub(crate) fn binding(&self) -> &Binding {
        &self.image.binding
    }
    pub(crate) fn image(&self) -> Result<Image> {
        self.image.validate(&self.memory)?;
        self.mailbox.validate_binding(self.binding(), true)?;
        Ok(self.image.clone())
    }
    fn complete(&mut self, result: Result<()>) -> Step {
        self.mailbox.finish(self.binding(), result);
        self.finished = true;
        Step::Done
    }
    fn prepare_event(&mut self) -> Result<()> {
        self.image.event = self
            .memory
            .heap_alloc(EVENT_BYTES)
            .map_err(|e| e.to_string())?;
        self.memory
            .write_bytes(self.image.event, &[0; EVENT_BYTES])
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    fn begin_delivery(&mut self) -> Result<()> {
        if self.binding().paste {
            self.image.stage = Stage::OfferRecord;
        } else {
            self.prepare_event()?;
            self.image.stage = Stage::InitEvent;
        }
        Ok(())
    }
    fn math_editor(&self) -> Result<Option<u32>> {
        let manager = self.memory.read_u32(MANAGER).map_err(|e| e.to_string())?;
        // Base-window constructor/attach code establishes parent+0x1c as the
        // last child and child+0x18 as its parent. A modal dialog replaces this
        // main frame. Do not turn note/program names into math expressions.
        let root = self
            .memory
            .read_u32(manager + 0x1c)
            .map_err(|e| e.to_string())?;
        if root == 0
            || self.memory.read_u32(root).ok() != Some(MAIN_FRAME)
            || self.memory.read_u32(root + 0x18).ok() != Some(manager)
        {
            return Ok(None);
        }
        let editor = self
            .image
            .app
            .checked_add(0x110)
            .ok_or("Editor address overflow")?;
        let node = self
            .memory
            .read_u32(editor + 0xd4)
            .map_err(|e| e.to_string())?;
        // Selection replacement has a distinct insertion boundary. Keep its
        // literal Paste semantics rather than infer a quote state from its end.
        if node != 0
            && self
                .memory
                .read_u32(node + 0x24)
                .map_err(|e| e.to_string())?
                != 0
        {
            return Ok(None);
        }
        let method = callback(&self.memory, self.image.view, 0x54)?;
        if method == HOME_TEXT || (self.image.view == editor && method == EDITOR_TEXT) {
            Ok(Some(editor))
        } else {
            Ok(None)
        }
    }
    fn prepare_aliases(&mut self, cursor: u32) -> Result<()> {
        let context = self
            .memory
            .read_wstr_bounded(self.image.context, MAX_UNITS + 1)
            .map_err(|e| e.to_string())?
            .ok_or("Firmware math editor has no source")?;
        let prefix = context.encode_utf16().collect::<Vec<_>>();
        if cursor as usize > prefix.len() {
            return Err("Firmware math caret lies outside its serialized source".into());
        }
        let mut lexical = Lexical::default();
        for unit in &prefix[..cursor as usize] {
            lexical.accept(*unit);
        }
        let original = self.image.original(&self.memory)?;
        let converted = aliases(&original, lexical);
        if converted != original {
            let bytes = converted
                .encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>();
            let argument = self
                .memory
                .heap_alloc(bytes.len())
                .map_err(|e| e.to_string())?;
            if let Err(e) = self.memory.write_bytes(argument, &bytes) {
                self.memory.heap_free(argument);
                return Err(e.to_string());
            }
            self.image.argument = argument;
            self.image.lexical = Some(lexical);
        }
        self.image.context = 0;
        self.begin_delivery()
    }
    fn step(&mut self, last: Option<Returned>) -> Result<Option<Call>> {
        if self.image.started {
            let returned = last.ok_or("Desktop text call lacks its actual firmware return")?;
            match self.image.stage {
                Stage::SetClipboard => {
                    let record = self
                        .memory
                        .read_u32(self.image.app + 0x10a4)
                        .map_err(|e| e.to_string())?;
                    let text = clipboard::text_record(&self.memory, record)?;
                    if !self.binding().matches(&text, true) {
                        return Err("Firmware did not commit the requested text clipboard".into());
                    }
                    self.memory.heap_free(self.image.source);
                    self.image.source = 0;
                    self.image.stage = Stage::FindView;
                }
                Stage::FindView => {
                    if returned.r0 == 0 {
                        return Err("Firmware has no active view for desktop text".into());
                    }
                    self.image.view = returned.r0;
                    let original = self.image.original(&self.memory)?;
                    if has_alias(&original) {
                        if let Some(editor) = self.math_editor()? {
                            self.image.editor = editor;
                            self.image.stage = Stage::QueryText;
                        } else {
                            self.begin_delivery()?;
                        }
                    } else {
                        self.begin_delivery()?;
                    }
                }
                Stage::QueryText => {
                    if returned.r0 == 0 {
                        return Err("Firmware math source query returned no buffer".into());
                    }
                    self.image.context = returned.r0;
                    self.image.stage = Stage::QueryCursor;
                }
                Stage::QueryCursor => self.prepare_aliases(returned.r0)?,
                Stage::OfferRecord => {
                    if returned.r0 != 0 {
                        return Ok(None);
                    }
                    self.prepare_event()?;
                    self.image.stage = Stage::InitEvent;
                }
                Stage::InitEvent => {
                    self.memory
                        .write_u32(self.image.event + 8, self.image.flags)
                        .map_err(|e| e.to_string())?;
                    self.image.stage = Stage::FindViewAgain;
                }
                Stage::FindViewAgain => {
                    if returned.r0 == 0 {
                        return Err("Firmware text focus closed before dispatch".into());
                    }
                    self.image.view = returned.r0;
                    self.image.stage = Stage::DispatchText;
                }
                Stage::DispatchText => {
                    if returned.r0 == 0 {
                        return Err("Active firmware view did not accept desktop text".into());
                    }
                    self.image.stage = Stage::DestroyEvent;
                }
                Stage::DestroyEvent => return Ok(None),
            }
        } else if last.is_some() {
            return Err("Unstarted desktop text operation has a return".into());
        }
        let call = match self.image.stage {
            Stage::SetClipboard => {
                Call::new(SET_TEXT, args([self.image.app, self.image.source, 0, 1]))
            }
            Stage::FindView | Stage::FindViewAgain => Call::new(
                FIND_VIEW,
                args([
                    self.memory.read_u32(MANAGER).map_err(|e| e.to_string())?,
                    0,
                    0,
                    0,
                ]),
            ),
            Stage::QueryText => Call::new(GET_TEXT, args([self.image.editor, 0, 0, 0])),
            Stage::QueryCursor => Call::new(GET_CURSOR, args([self.image.editor, 0, 0, 0])),
            Stage::OfferRecord => Call::new(
                callback(&self.memory, self.image.view, 0x58)?,
                args([
                    self.image.view,
                    self.memory
                        .read_u32(self.image.app + 0x10a4)
                        .map_err(|e| e.to_string())?,
                    0,
                    0,
                ]),
            ),
            Stage::InitEvent => {
                let text = if self.image.argument != 0 {
                    self.image.argument
                } else if self.binding().paste {
                    self.memory
                        .read_u32(self.image.app + 0x10a4)
                        .map_err(|e| e.to_string())?
                        .checked_add(8)
                        .ok_or("Clipboard text address overflow")?
                } else {
                    self.image.source
                };
                if self.image.argument == 0 {
                    text_at(&self.memory, text, self.binding())?;
                } else {
                    self.image.validate(&self.memory)?;
                }
                Call::new(INIT_EVENT, args([self.image.event, text, 0, 0]))
            }
            Stage::DispatchText => Call::new(
                callback(&self.memory, self.image.view, 0x54)?,
                args([self.image.view, self.image.event, 0, 0]),
            ),
            Stage::DestroyEvent => Call::new(DROP_EVENT, args([self.image.event, 0, 0, 0])),
        };
        self.image.started = true;
        Ok(Some(call))
    }
    pub(crate) fn next(&mut self, last: Option<Returned>) -> Step {
        if self.mailbox.cancelled(self.binding()) {
            return self.complete(Err("Desktop text cancelled".into()));
        }
        match self.step(last) {
            Ok(Some(call)) => Step::Call(call),
            Ok(None) => self.complete(Ok(())),
            Err(e) => self.complete(Err(e)),
        }
    }
}
impl Drop for Plan {
    fn drop(&mut self) {
        let _gate = self.memory.snapshot_gate.enter();
        if !self.finished {
            self.mailbox.finish(
                self.binding(),
                Err("Desktop text call was cancelled before completion".into()),
            );
        }
        for address in [self.image.source, self.image.event, self.image.argument] {
            if address != 0 {
                self.memory.heap_free(address);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn math_aliases_preserve_literal_quoted_and_escaped_scalars() {
        assert_eq!(aliases("π×2−ⅈ÷3", Lexical::default()), "π*2-\u{e003}/3");
        assert_eq!(aliases("\"π×ⅈ\"÷2", Lexical::default()), "\"π×ⅈ\"/2");
        let mut lexical = Lexical::default();
        for unit in "\"a\\".encode_utf16() {
            lexical.accept(unit);
        }
        assert_eq!(aliases("\"ⅈ×\"÷2", lexical), "\"ⅈ×\"/2");
        assert_eq!(
            aliases(
                "ⅈ÷×−",
                Lexical {
                    quoted: true,
                    escaped: false
                }
            ),
            "ⅈ÷×−"
        );
    }
    #[test]
    fn completion_round_trip_binds_source_flags_and_consumes_once() {
        let mailbox = Mailbox::default();
        let text = "α😀\nEND;)";
        validate_text(text).unwrap();
        let binding = mailbox.reserve(text, true).unwrap();
        assert!(mailbox.reserve("other", false).is_err());
        assert!(binding.matches(text, true));
        assert!(!binding.matches(text, false));
        assert!(!binding.matches("α😀\nEND;)", false));
        let bytes = serde_json::to_vec(&mailbox.image()).unwrap();
        let restored = serde_json::from_slice::<MailboxImage>(&bytes)
            .unwrap()
            .restore()
            .unwrap();
        restored.validate_binding(&binding, true).unwrap();
        assert!(restored.take(&binding).unwrap().is_none());
        restored.finish(&binding, Ok(()));
        assert!(restored.validate_binding(&binding, true).is_err());
        assert_eq!(restored.take(&binding).unwrap(), Some(Ok(())));
        assert!(restored.take(&binding).is_err());
    }
    #[test]
    fn cancellation_keeps_ownership_until_completion_and_reserves_fresh_nonce() {
        let mailbox = Mailbox::default();
        let first = mailbox.reserve("first", false).unwrap();
        mailbox.cancel(&first);
        assert!(mailbox.cancelled(&first));
        assert!(mailbox.reserve("second", false).is_err());
        mailbox.finish(&first, Err("cancelled".into()));
        let second = mailbox.reserve("second", true).unwrap();
        assert!(second.token > first.token);
        mailbox.finish(&first, Ok(()));
        assert!(mailbox.take(&second).unwrap().is_none());
        assert!(!mailbox.cancelled(&second));
        let mut image = mailbox.image();
        image.state.slot.as_mut().unwrap().binding.token = image.state.next + 1;
        assert!(image.restore().is_err());
        image.version += 1;
        assert!(image.restore().is_err());
    }
    #[test]
    fn text_budget_counts_utf16_units_and_rejects_embedded_terminators() {
        assert!(validate_text("unmatched )\n\"λ😀").is_ok());
        assert!(validate_text("before\0after").is_err());
        assert!(validate_text(&"😀".repeat(MAX_UNITS / 2)).is_ok());
        assert!(validate_text(&"😀".repeat(MAX_UNITS / 2 + 1)).is_err());
    }
    #[test]
    fn installed_code_plan_round_trips_every_controlled_call_phase_with_fresh_ram() {
        // This is a component/ownership test with controlled callee effects.
        // Real firmware execution is exercised by verify_firmware_literal_text.py.
        let Some(path) = std::env::var_os("PRIME_TEST_FIRMWARE") else {
            return;
        };
        let elf = crate::firmware::ElfImage::load(std::path::Path::new(&path)).unwrap();
        for (source, paste, context, expected, calls) in [
            ("literal λ😀\n);", true, None, "literal λ😀\n);", 7),
            ("π×2−ⅈ÷3", true, Some(""), "π*2-\u{e003}/3", 9),
            ("ⅈ×\"÷2", false, Some("\"prefix"), "ⅈ×\"/2", 7),
        ] {
            round_trip_text_plan(&elf, source, paste, context, expected, calls);
        }
    }
    fn round_trip_text_plan(
        elf: &crate::firmware::ElfImage,
        source: &str,
        paste: bool,
        context: Option<&str>,
        expected: &str,
        calls: usize,
    ) {
        let mut memory = GuestMemory::new();
        memory.map("heap", 0x20000000, 0x20000).unwrap();
        memory.set_heap(0x20000000, 0x20000);
        for segment in &elf.segments {
            let base = segment.vaddr & !4095;
            memory
                .map(
                    "installed-code",
                    base,
                    crate::memory::page_round(segment.mem_size + (segment.vaddr - base) as usize),
                )
                .unwrap();
            memory
                .write_bytes(segment.vaddr, elf.segment_bytes(segment))
                .unwrap();
        }
        assert!(supported(&memory));
        let app = memory.heap_alloc(0x10a8).unwrap();
        memory.write_u32(APP, app).unwrap();
        let view = memory.heap_alloc(4).unwrap();
        let table = memory.heap_alloc(0x5c).unwrap();
        memory.write_u32(view, table).unwrap();
        for offset in [0x54, 0x58] {
            memory.write_u32(table + offset, DROP_EVENT).unwrap();
        }
        let context_ptr = if let Some(context) = context {
            let manager = memory.heap_alloc(0x20).unwrap();
            let root = memory.heap_alloc(0x20).unwrap();
            memory.write_u32(MANAGER, manager).unwrap();
            memory.write_u32(manager + 0x1c, root).unwrap();
            memory.write_u32(root, MAIN_FRAME).unwrap();
            memory.write_u32(root + 0x18, manager).unwrap();
            memory.write_u32(app + 0x110 + 0xd4, 0).unwrap();
            memory.write_u32(table + 0x54, HOME_TEXT).unwrap();
            let bytes = context
                .encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>();
            let ptr = memory.heap_alloc(bytes.len()).unwrap();
            memory.write_bytes(ptr, &bytes).unwrap();
            ptr
        } else {
            0
        };
        let source_bytes = source
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        let record = memory.heap_alloc(8 + source_bytes.len()).unwrap();
        memory.write_bytes(record, &[0; 8]).unwrap();
        memory
            .write_u32(record + 4, (8 + source_bytes.len()) as u32)
            .unwrap();
        memory.write_bytes(record + 8, &source_bytes).unwrap();
        let mut memory = Arc::new(memory);
        let mut mailbox = Arc::new(Mailbox::default());
        let mut plan = Plan::new(memory.clone(), mailbox.clone(), source, paste).unwrap();
        let binding = plan.binding().clone();
        let mut last = None;
        let mut count = 0;
        while let Step::Call(_) = plan.next(last.take()) {
            count += 1;
            let image = plan.image().unwrap();
            let completion = mailbox.image();
            let ram =
                crate::memory::image::MemoryImage::capture(&memory, "literal-component").unwrap();
            let mut bad = image.clone();
            bad.source = 1;
            assert!(bad.restore(memory.clone(), mailbox.clone()).is_err());
            if image.argument != 0 {
                let mut bad = image.clone();
                bad.lexical = None;
                assert!(bad.restore(memory.clone(), mailbox.clone()).is_err());
                let mut bad = image.clone();
                bad.argument = bad.event;
                assert!(bad.restore(memory.clone(), mailbox.clone()).is_err());
            }
            drop(plan);
            memory = Arc::new(ram.restore("literal-component").unwrap());
            mailbox = Arc::new(completion.restore().unwrap());
            plan = image.restore(memory.clone(), mailbox.clone()).unwrap();
            let r0 = match image.stage {
                Stage::SetClipboard => {
                    memory.write_u32(app + 0x10a4, record).unwrap();
                    0
                }
                Stage::FindView | Stage::FindViewAgain => view,
                Stage::QueryText => {
                    memory.write_u32(app + 0x110 + 0xd0, context_ptr).unwrap();
                    context_ptr
                }
                Stage::QueryCursor => context.unwrap().encode_utf16().count() as u32,
                Stage::InitEvent => {
                    let argument = if image.argument != 0 {
                        image.argument
                    } else if paste {
                        record + 8
                    } else {
                        image.source
                    };
                    assert_eq!(
                        memory
                            .read_wstr_bounded(argument, MAX_UNITS + 1)
                            .unwrap()
                            .as_deref(),
                        Some(expected)
                    );
                    memory.write_u16(image.event, 10).unwrap();
                    memory.write_u32(image.event + 4, argument).unwrap();
                    0
                }
                Stage::DispatchText => 1,
                _ => 0,
            };
            last = Some(Returned {
                r0,
                frame: vec![],
                frame_va: 0,
                elapsed_ms: 0.,
            });
        }
        assert_eq!(mailbox.take(&binding).unwrap(), Some(Ok(())));
        assert_eq!(count, calls);
        let event = plan.image.event;
        let argument = plan.image.argument;
        drop(plan);
        assert!(memory.heap_size_of(event).is_none());
        if argument != 0 {
            assert!(memory.heap_size_of(argument).is_none());
        }
        assert!(
            memory.heap_size_of(record).is_some(),
            "borrowed guest clipboard must remain owned by firmware"
        );
        if context_ptr != 0 {
            assert!(
                memory.heap_size_of(context_ptr).is_some(),
                "query text remains guest-owned"
            );
        }
    }
}
