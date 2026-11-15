# Bubble Notes

Floating notes for the Windows desktop, built with [NSH](https://nsh.dev), in Rust. The app lives in the tray; each note is a small frameless window of its own, and the Notes Manager lists every note by category.

## Features

- **Floating notes:** rounded, frameless, any colour or gradient, see-through backgrounds. Drag the title to move (snaps to screen edges), the corner grip to resize, double-click the title to roll a note up
- **Right-click menu** on every note: always on top, lock, show title, sleep, alarm, move to a category, theme, duplicate, copy and paste, copy as Markdown, export, arrange
- **Edit configuration:** a panel beside the note with its colours, text and title styling, transparency, sleep, alarm and bubble settings. Changes apply at once
- **Bubbles:** a note can show as a small bubble (glossy sphere, circle, square, triangle or an emoji) that opens into the note on hover or click, to any side
- **Sleep and alarms:** a sleeping note hides until its time; an alarm shows the note with a notification and a sound, and can repeat
- **Notes Manager:** category tree, full-text search, trash, the full formatting toolbar
- **Tray icon:** click to show or hide all notes; right-click for New Note, Show All, Hide All, Notes Manager, Options, Exit
- **Rich text:** type `/` for headings, checklists, toggles, tables and more; select text to format it; Markdown shortcuts as you type
- **Images and YouTube:** paste or drop images into a note; a pasted YouTube link becomes a card that opens the video in your browser
- **Links between notes:** type `[[` to pick a note (or `[[Name]]` in full); the link shows the note's current title and opens it on a click
- **Bubble badges:** a bubble shows the time to its note's alarm, or how many checklist items are done
- **Snapping and rescue:** dragged notes snap to screen edges and to each other; notes left on an unplugged monitor come back
- **Ghost notes:** a ghost note lets clicks through to what's under it; hold Ctrl+Alt to reach it again
- **Quick search:** Ctrl+Alt+F opens a search box from anywhere; type, pick with the arrows, Enter brings the note up
- **Templates:** "Save as Template" keeps a copy of a note in the Templates category; "New from Template" (tray and note menus) starts a note from one
- **Pin to a program:** "Show only with …" in a note's menu shows the note only while that program is in front
- **Colour by category:** right-click a category in the Notes Manager to choose the colour its new notes get (subcategories follow)
- **Global shortcuts:** new note (Ctrl+Alt+N), new note from the clipboard (Ctrl+Alt+V), show or hide all notes (Ctrl+Alt+H), search (Ctrl+Alt+F); change them in Options

## Where notes are kept

`Documents/Bubble Notes`: one HTML file per note, a folder per category, images in `attachments/`. Notes save as you type. HTML files you add to the folder become notes; `.bubblenotes/index.json` holds looks, positions, alarms and settings, and `.bubblenotes/trash/` holds deleted notes for 30 days.

## Run and test

```sh
cargo run
cargo test
```

Rust 1.97+ (`rustup update stable`). Windows: Visual Studio Build Tools with the C++ workload. The app is built for Windows; the Linux and macOS backends are declared but untested.

For development, `BUBBLENOTES_DIR=<folder>` uses another notes folder (and lets the app run beside the installed one, without global shortcuts or the start-with-Windows entry), `BUBBLENOTES_OPEN=manager,options,config,search` opens those windows at start, and `BUBBLENOTES_DEV=select|slash|back|autosize|config` does that to a note two seconds after start (for checking without the mouse).

## Code map

| Module | What |
|---|---|
| `model.rs` | What's saved about a note (look, bubble, alarm, position) and the settings: plain data |
| `store.rs` | The notes folder on disk: files, categories, index, trash |
| `app.rs` | The shared state every window uses (`App`, `Note`) and what changes it; autosave |
| `note_window.rs`, `bubble.rs` | A note floating on the desktop; a note shown as a bubble |
| `note_menu.rs` | A note's right-click menu and what its items do |
| `config_panel.rs` | "Edit configuration", beside a note |
| `manager.rs` | The Notes Manager window |
| `tray.rs`, `options.rs`, `quick_search.rs` | The tray icon and its menu; the Options window; the quick search box |
| `alarms.rs`, `system.rs` | Sleep and alarms; global shortcuts and starting with Windows |
| `editor_view.rs`, `media.rs` | A note's text as an editor element; pasted images and YouTube cards |
| `kit.rs`, `actions.rs`, `toggle.rs`, `slash.rs`, `formats.rs`, `ui.rs` | The editor: schema and plugins, the action catalog, toggle blocks, the slash menu, Markdown/HTML, toolbar and menus |
| `links.rs` | Links between notes: the `[[` menu and the link in the text |
| `drag.rs`, `layout.rs`, `win.rs` | Moving windows by hand; snapping and off-screen rescue; the few things asked of Windows directly |

## Things worth knowing

- **The main window never shows.** NSH ends the app when its main window closes, so it stays hidden and hosts the tray, the menus and every other window
- **Note windows are see-through**, which rules out a system shadow and click-through: a window's bounds are always exactly what's visible
- **Notes are dragged by the app, not the system** (`drag.rs`): a system drag region swallows clicks, and this way double-click rolls a note up and the configuration panel follows along
- **One editor session per note**, shown by both the floating note and the manager, so they always agree
- **No video inside notes:** NSH 0.5.1's web view draws only in the main window, so YouTube links open in the browser
- **Ghost notes go around NSH:** it refuses click-through windows to Rust apps, so `win.rs` sets the window style itself and sets it again after NSH resets it
- **Right-click menus are native**: a drawn menu would be clipped by a small note's window

## Patched framework

`vendor/nsh-base` and `vendor/nsh-platform-windows` are NSH 0.5.1 with two
fixes of ours, used through `[patch.crates-io]` in `Cargo.toml`. Search them
for `BUBBLENOTES PATCH`:

- `vendor/nsh-platform-windows/src/text_input.rs`: a text field whose text
  is fully transparent draws nothing. Without it the rich editor's hidden
  input drew a black speck at the caret.
- `vendor/nsh-base/src/rich_editor/view/geometry.rs`: on Windows the caret
  counts the spaces at a line's end. Without it the caret stood still after
  a typed space.

When NSH is upgraded: copy the new crates over these and apply the same
changes again, or drop the copies and the `[patch.crates-io]` section if NSH
has fixed both.
