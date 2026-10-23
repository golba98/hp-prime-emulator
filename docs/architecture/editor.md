# IDE Code Editor — Architecture

The PPL editor in the native IDE (`ppl gui`) is built on eframe/egui 0.32.
egui's `TextEdit` provides text storage, selection, clipboard and IME; the
IDE wraps it with everything a code editor needs and keeps all PPL language
knowledge in a toolkit-independent layer.

## Layers

```
ppl-syntax   lexer (real tokens, opt-in comment tokens), diagnostics model
     │
ppl-compiler highlight.rs   one-pass token classification, bracket pairs,
     │                      document symbols (functions, params, locals)
     │       catalog.rs     documented HP Prime commands not emulated
     │       compile.rs     semantic analysis → classified diagnostics
     │
ppl-gui      ide/syntax.rs        token class → egui TextFormat (palette)
             ide/diagnostics.rs   DiagnosticItem, background checker
             ide/editor/
               tab.rs       buffer, TabId, file format, save
               document.rs  per-revision cache: analysis, LineIndex,
                            LayoutJob, Galley
               text_ops.rs  pure edits: indent, newline, pairs, comments
               input.rs     key interception → text_ops, undo recording
               view.rs      the widget: bands, text, squiggles, popup
               gutter.rs    line numbers and markers (visible rows only)
               find.rs / find_bar.rs, completion.rs
               widget_tests.rs   real-frame tests
```

There is one tokenizer. Highlighting, bracket matching, auto-indent, the
outline and completion all use `ppl_syntax::lexer`, either directly or
through `ppl_compiler::highlight::analyze`. Nothing re-tokenizes PPL with
regular expressions.

## Data flow per frame

1. **Input interception** (`input.rs`). This runs before `TextEdit`. If the
   frame contains any key the editor handles (Tab, Shift+Tab, Enter,
   Backspace, `( [ { ) ] } "`, Ctrl+/, Ctrl+Space, or completion
   navigation), the editor applies that frame's events itself, in order,
   and removes them from egui's input:
   - Plain insertions and deletions are applied as text changes.
   - Smart keys go through `text_ops`. Each smart key is recorded on the
     `TextEdit` undo stack as exactly one step (`Undoer::add_undo` before
     and after the edit).
   - The first event it does not handle (arrows, IME, …) stops the run.
     That event and all later ones go to `TextEdit`, so ordering is always
     preserved.

   Without this, `TextEdit` handles Tab by replacing the selection with
   `\t`, and Enter inserts a bare newline.
2. **Sync** (`EditorTab::sync`). The buffer is compared with the last
   analyzed snapshot. Only when it differs does the revision advance, the
   analysis and line index get recomputed, and the `modified` flag get
   re-derived from a hash of the saved content. Undoing back to the saved
   text clears the flag.
3. **Layout** (`DocumentCache::galley`). The layouter returns a cached
   `Arc<Galley>` keyed by revision, `CodeStyle` and pixels-per-point.
   - Code is laid out without wrapping, so a window resize never
     invalidates it.
   - A scale-factor change re-lays out the text from the cached
     `LayoutJob`, without re-analyzing.
4. **Paint** (`view.rs`).
   - A `Shape::Noop` placeholder reserved before the text is filled with
     the background bands: editor fill, current line, debugger line, find
     matches and bracket pair. The bands are always behind the text.
   - Diagnostics squiggles, hover tooltips and the gutter are painted on
     top. Only rows intersecting the clip rect are visited (binary search
     on galley rows); row index equals line index.
5. **Diagnostics.**
   - `app.rs` sends changed buffers to `DiagnosticsWorker`, a thread that
     compiles them and coalesces requests per tab, keeping only the newest
     revision.
   - The tab being edited is checked after 250 ms of quiet; other open
     tabs are checked immediately.
   - Results for superseded revisions are discarded. Check Syntax (F7) and
     Run compile synchronously.

## State and identity

- `TabId` is a stable `u64` per tab. The `TextEdit` id and the scroll-area
  id are salted with it, so cursor, undo history and scroll position belong
  to a buffer, not to a tab index. Closing or reordering tabs never moves
  history between files.
- `NavigateToLocation { tab: Option<TabId>, line, col }` resolves the tab
  at the moment it is handled. Problems entries carry their `TabId`.

## Files and encodings

`EditorTab::open` records a `FileFormat` (encoding, UTF-8 BOM, CRLF), and
`save` writes the file back in that format. The write is atomic: a
temporary file in the same directory, then a rename.

- **Binary `.hpprgm` container** (a Connectivity Kit export): the text
  alone cannot rebuild it, so saving in place is refused with an
  explanation and Save As is offered. Save As writes UTF-16LE (with BOM)
  for `.hpprgm`, otherwise UTF-8.
- **Plain UTF-16 files** keep their final newline. The loader previously
  trimmed it.

## Editing features

| Feature | Where |
|---|---|
| Tab / Shift+Tab on a selection | `text_ops::indent` / `dedent` |
| Enter: keep indentation, indent after a block opener, re-align a line starting with `END`/`ELSE`/`UNTIL`/`DEFAULT` | `text_ops::newline`, `block_effect` (lexer tokens, so `BEGINNER` or `"THEN"` never count) |
| Bracket and quote pairing, wrap selection, step over a matched closer, delete an empty pair | `text_ops::type_char`, `backspace_pair` |
| Toggle line comment (Ctrl+/) | `text_ops::toggle_line_comment` |
| Bracket matching | `SourceAnalysis::bracket_partner` (strings and comments excluded) |
| Find / replace (match case, whole word, wraparound, replace all as one undo step) | `find.rs`, `find_bar.rs` |
| Completion (Ctrl+Space, or after two identifier characters) | `completion.rs`: keywords, builtin table, documented catalog (flagged *not emulated*), file symbols. Nothing invented. |
| Go to line (`line` or `line:col`, validated) | `dialogs.rs` |
| Edit menu (Undo, Redo, Cut, Copy, Paste, Select All, …) | replays the same key events into the editor (`IdeAction::Edit`) |
| Breakpoints | gutter click → `IdeAction::ToggleBreakpoint` |

**Code folding is not supported.** `TextEdit` cannot hide text ranges, and
folding would need a custom text widget (see Limitations).

## Visual design

- **Code font:** egui's bundled Hack. DejaVu Sans (already bundled in
  `ppl-graphics`) is installed as a fallback for every family, so ✓ ▲ ▼ ☐
  and `ⅈ ℯ ᴇ` render instead of showing as empty boxes.
- **Size and spacing:** the default size is 14 pt, adjustable with
  Ctrl+= / Ctrl+- / Ctrl+0 or Settings, range 9–28, and persisted. Row
  height is 1.45 × the size, rounded to whole points. No synthetic italics.
- **Palette** (`theme::syntax`): one hue per token class. A unit test
  enforces WCAG contrast against the editor background and the
  current-line band: at least 4.5:1 for code and 3:1 for comments.
- **Layout:**
  - The editor fills the central panel with no frame margin.
  - The explorer is capped at 22% of the window width; the calculator
    panel at 25–32% (minimum 200 px); the dock at 30–45% of the height.
  - Dragged panel sizes are written back and persisted.
  - The calculator screen scales down instead of clipping.
  - The toolbar switches to icon labels below 1100 px.
  - The tab strip scrolls and reveals the active tab.

## Performance

Criterion benchmark `cargo bench -p ppl-bench --bench editor`, release
build, one 1920×1080 headless egui frame including tessellation:

| Measure | 1.2k lines | 6.2k lines | 24.8k lines |
|---|---|---|---|
| Idle frame, before → after | 0.60 → 0.03 ms | 3.6 → 0.08 ms | 17.3 → 0.63 ms |
| Highlight, before → after | 0.50 → 0.42 ms | 3.5 → 2.3 ms | 31.8 → 11.7 ms |
| Diagnostics compile, before → after | 0.65 → 0.61 ms | 4.0 → 3.1 ms | 26.3 → 16.6 ms (now off the UI thread) |
| Frame after a one-character edit, before → after | 2.4 → 2.0 ms | 14.5 → 12.5 ms | 73 → 47 ms |

What changed:

- The idle-frame cost came from cloning the whole buffer and the whole
  `LayoutJob` every frame, plus painting every gutter row.
- The superlinear highlight cost came from the lexer lower-casing the rest
  of the file at every `#` token.
- The remaining per-keystroke cost on very large files is egui
  re-splitting and re-hashing the job into per-line paragraphs; egui
  already caches each paragraph's layout.

## Testing

| Layer | Tests |
|---|---|
| Lexer, highlighter | `ppl-syntax` unit tests; `ppl-compiler/src/highlight.rs` (every token class, scoping, unclosed and multi-line strings, escapes, brackets, full byte coverage of every fixture and every truncated prefix of them) |
| Diagnostics | `ppl-compiler/tests/diagnostics.rs` (class, severity, exact line/column); `ppl-vm/tests/unsupported.rs` (runtime behavior of unsupported calls) |
| Editing operations | `ppl-gui/src/ide/editor/text_ops.rs` (pure functions) |
| Widget | `ppl-gui/src/ide/editor/widget_tests.rs`: real `egui::Context` frames with synthetic keys and pointer, window sizes 800×500 to 3840×2160, scale 1–2. Covers multi-line Tab, auto-indent, pairing, burst input, undo/redo, per-tab state, closing tabs, gutter clicks, paint order of the current-line band, find/replace, completion, debugger-line scrolling, and that large buffers do no idle work |
| App integration | `ppl-gui/src/app.rs` tests: compatibility-only programs, runtime errors in Problems, navigation by `TabId`, background diagnostics, inactive tabs |
| Real window (Xvfb) | `tests/ide_xvfb_smoke/script_editor.json` and `script_editor_hidpi.json`, checked by `assert_editor.py`. Window screenshots at 800×500 … 3840×2160 and at scale 2; cursor and layout cache must survive every resize |

## Limitations

- **No code folding.** It needs a custom text widget in place of
  `TextEdit`. That was deliberately not done, to keep egui's IME,
  selection and clipboard behavior.
- **Fallback glyphs:** `ⅈ ℯ ᴇ` come from DejaVu Sans, which is
  proportional, so they are not exactly one monospace cell wide.
- **Very large files:** above roughly 20k lines a keystroke costs about
  45 ms (release build), dominated by egui's paragraph splitting.
- **Diagnostics timing:** while typing, diagnostics refer to the text as of
  the last check (up to 250 ms old). Squiggles may sit slightly off until
  the next result arrives.
