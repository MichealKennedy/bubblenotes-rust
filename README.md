# Rich Notes

A WYSIWYG rich-text note editor built with [NSH](https://nsh.dev), in Rust. You edit formatted text directly; it's built on NSH's `RichEditor`, a Rust port of ProseMirror with Tiptap's extensions.

## Features

- **Selection menu** (select text): bulleted, numbered, check and toggle lists; bold, italic, underline, strikethrough; decrease/increase indent; link (an inline URL box; Ctrl/⌘+K too); **…** for Insert (table, image, divider, code block, quote) and Format (inline code, highlight, clear formatting)
- **Slash menu:** type `/` at a line's start or after a space; filter as you type (`/h2`, `/todo`, `/tog`); ↑ ↓, Enter or Tab, Esc, or click
- **Toggles:** collapsible blocks with a title and body; the arrow folds them. Saved in HTML as `<details>`
- **Images:** inserted from anywhere on your computer; on save they're copied into `attachments/` next to the note and linked by relative path (works in browsers, Obsidian, GitHub)
- **Typing shortcuts:** `## `, `- `, `1. `, `[ ] `, `> `, ```` ``` ````, `**bold**`, `*italic*`, `~~strike~~`, `` `code` ``
- **Toolbar** with everything above plus headings, highlight, sub/superscript, text color, alignment, undo/redo; buttons show what's active and dim when they can't apply
- **Files:** Markdown (`.md`) or HTML (`.html`) by extension; `.txt` opens as text. Saving as Markdown checks whether Markdown can hold the note and, if not, says what it would drop (toggles, underline, highlight, color, alignment…) and offers HTML

Shortcuts: Ctrl/⌘+N, O, S; Ctrl/⌘+K link; the editor's own (B, I, U, E code, Shift+S strike, Shift+H highlight, Shift+L/E/R align, Z/Shift+Z undo/redo; Tab/Shift+Tab indent in lists).

## Run and test

```sh
cargo run
cargo test       # formats, losses, toggles through HTML, image copying, the action catalog
```

Rust 1.97+ (`rustup update stable`). Windows: Visual Studio Build Tools with the C++ workload (for Rust's linker; LLVM is no longer needed). Linux: GTK 4.14+ and GStreamer development packages. macOS 14+ with Xcode command line tools.

## Code map

| Module | What |
|---|---|
| `kit.rs` | The editor setup, **built once**: schema, plugins, named commands; each open note is an `Editor` (its own session over the shared kit). `Relay` carries what the editor reports from other threads (file shortcuts, state changes) to the app |
| `actions.rs` | **Every editing action, once** (name, glyph, hint, search words, what it runs, when it's active or enabled). The toolbar, slash menu, selection menu and its "…" menu are lists of ids |
| `toggle.rs` | The toggle block: three node types, their HTML, the arrow (a node view), and "turn this block into a toggle" |
| `slash.rs` | The slash menu's plugin (NSH's port of Tiptap's `Suggestion`) |
| `formats.rs` | Markdown/HTML/text, the Markdown loss check, image paths (absolute while open, relative on disk); unit tests |
| `files.rs` | New, open, save (with the Markdown check), insert image, copying images into `attachments/` |
| `ui.rs` | Toolbar, slash menu, selection menu (link editor, "…"), buttons, and the document's look (NSH's editor draws nothing itself) |
| `lib.rs` | The window: wires the pieces together |

Adding an action: one line in `ACTIONS`, then its id in whichever menus should show it.

## Things worth knowing

- **One schema for every note.** A document's node types belong to the schema instance that made them, and an editor can't edit another schema's document (edits are silently refused). `Kit` is built once for that reason
- **Plugin order matters:** the slash plugin goes first, so its Enter comes before the keymap's
- **No Save As shortcut:** Ctrl/⌘+Shift+S is strikethrough, as in Notion and Tiptap. Save As is a toolbar button
- **Indent** applies to list items (Markdown and the editor's model have no indented paragraphs)
- **Markdown normalizes a little** on save: table separators are re-spaced; GitHub callouts (`> [!NOTE]`) are escaped
- **Sub/superscript** show as smaller text (NSH's layout doesn't shift the baseline); code blocks aren't syntax-highlighted

## Is it 100% Rust?

The app and everything compiled into it are Rust: no JavaScript engine, no web view, and no C compiled from source. What it does use is the operating system's own UI libraries, as any native app must: GTK 4 and GStreamer on Linux (C libraries, through Rust bindings), AppKit on macOS, Win32 on Windows.
