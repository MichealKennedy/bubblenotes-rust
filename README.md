# Bubble Notes

Floating notes for the Windows desktop, built with [NSH](https://nsh.dev), in Rust. The app lives in the tray; each note is a small frameless window of its own, and the Notes Manager lists every note by category.

## Features

- **Floating notes:** rounded, frameless, any colour or gradient, see-through backgrounds. Drag the title to move (snaps to screen edges), the corner grip to resize, double-click the title to roll a note up
- **Right-click menu** on every note: always on top, lock, show title, sleep, alarm, move to a category, theme, earlier versions, duplicate, copy and paste, copy as Markdown, export, print, save as PDF, arrange
- **Edit configuration:** a panel beside the note with its colours, text and title styling, transparency, sleep, alarm and bubble settings. Changes apply at once
- **Bubbles:** a note can show as a small bubble (glossy sphere, circle, square, triangle or an emoji) that opens into the note on hover or click, to any side
- **Sleep and alarms:** a sleeping note hides until its time; an alarm shows the note with a notification and a sound, and can repeat
- **Notes Manager:** category tree, full-text search, trash, the full formatting toolbar
- **Tray icon:** click to show or hide all notes; right-click for New Note, Show All, Hide All, Notes Manager, Options, Exit
- **Rich text:** type `/` for headings, checklists, toggles, tables and more; select text to format it; Markdown shortcuts as you type
- **Images and YouTube:** paste or drop images into a note; a pasted YouTube link becomes a card that plays the video in the note (↗ opens it in your browser)
- **Links between notes:** type `[[` to pick a note (or `[[Name]]` in full); the link shows the note's current title and opens it on a click
- **Bubble badges:** a bubble shows the time to its note's alarm, or how many checklist items are done
- **Snapping and rescue:** dragged notes snap to screen edges and to each other; notes left on an unplugged monitor come back
- **Ghost notes:** a ghost note lets clicks through to what's under it; hold Ctrl+Alt to reach it again
- **Quick search:** Ctrl+Alt+F opens a search box from anywhere; type, pick with the arrows, Enter brings the note up
- **Templates:** "Save as Template" keeps a copy of a note in the Templates category; "New from Template" (tray and note menus) starts a note from one
- **Pin to a program:** "Show only with …" in a note's menu shows the note only while that program is in front
- **Colour by category:** right-click a category in the Notes Manager to choose the colour its new notes get (subcategories follow)
- **Earlier versions:** a note's text is kept as it was before each spell of editing (at most one version every 10 minutes, the newest 40); "Earlier versions" in a note's menu puts one back, keeping the current text as a version too
- **Dates and reminders:** type `@friday`, `@tomorrow 3pm`, `@20 nov` or `@in 2h` and it becomes a date; when it comes, the app reminds of that line, unless it's a ticked checklist item
- **Fade when idle:** a note goes see-through until the pointer is over it or it's being typed in
- **Dock to a screen edge:** drag a note until the pointer touches the left, right or top edge of the screen (or use its menu): it tucks away as a strip there and slides out under the pointer. Drag it away to undock
- **Print and PDF:** "Print…" opens the note in your browser with its print dialog; "Save as PDF…" writes a PDF (using Edge or Chrome, without showing them)
- **Global shortcuts:** new note (Ctrl+Alt+N), new note from the clipboard (Ctrl+Alt+V), show or hide all notes (Ctrl+Alt+H), search (Ctrl+Alt+F); change them in Options

## Where notes are kept

`Documents/Bubble Notes`: one HTML file per note, a folder per category, images in `attachments/`. Notes save as you type. HTML files you add to the folder become notes; `.bubblenotes/index.json` holds looks, positions, alarms and settings, `.bubblenotes/trash/` holds deleted notes for 30 days, and `.bubblenotes/history/` holds each note's earlier versions.

## Portable use, and the crash log

- **Portable:** put the program's file in a folder with a folder named
  `Notes` beside it. The notes, the settings and the crash log are then kept
  in that `Notes` folder, not in Documents, and a copy run this way leaves
  another copy's start-with-Windows entry alone.
- **Crash log:** when the app fails, what went wrong (with the calls that
  led there) is added to `.bubblenotes/crash.log` in the notes folder, a
  message says so, and the app closes. See `src/crash.rs`.

## Updates

The app updates itself from the releases of its GitHub repository
(`MichealKennedy/bubblenotes-rust`), by the rules of `nsh-updater`:

- **Looking:** 20 seconds after it starts and then once a day, the app gets
  the latest release's `latest.json` and its signature. Options has a switch
  for this ("Check for updates") and a "Check now" button.
- **Trust:** the manifest must be signed with the release key (its public
  half is `PUBLIC_KEY` in `src/update.rs`), be for this app and name a newer
  version; the program file downloaded must have the size and SHA-256 the
  manifest says. Anything else is refused and nothing is changed.
- **Installing:** with "Install updates automatically" (on by default) a
  newer version is put in place of the program's file by itself; else the
  tray menu and Options offer it. Either way it only starts at the app's
  next start (Options and the tray menu offer a restart): the app never
  restarts by itself. Only the program's file is replaced.
- **Telling:** a notification says when a version was installed by itself
  and waits for a restart, or (automatic installing off) that one is
  available, once for each version.
- **A bad release:** the replaced program file is kept beside the new one
  for the new version's first minute of running. If the new version fails
  in that minute (anything the crash log catches), the old file is put
  back and started, and that version is no longer installed by itself
  (Options still offers it). A version that hangs, or can't start at all,
  isn't caught.
- **Releasing:** set the version in `Cargo.toml` and `nsh.json`, then run
  `.\release.ps1 "What changed."` and publish the three files it writes to
  `dist\` as a GitHub release (it prints the command). The release key is
  in `~\.bubblenotes\release.secret`: keep a backup of it, and never commit
  it. Without it no release can be made that the apps out there accept.
- A development run (`BUBBLENOTES_DIR`) never asks GitHub. With
  `BUBBLENOTES_UPDATE=<url of a latest.json>` it asks there instead (a
  server on this machine, for testing); `BUBBLENOTES_DEV=crashat:<version>`
  makes that version fail three seconds after its start.

## Run and test

```sh
cargo run
cargo test
```

Rust 1.97+ (`rustup update stable`). Windows: Visual Studio Build Tools with the C++ workload. The app is built for Windows; the Linux and macOS backends are declared but untested.

For development, `BUBBLENOTES_DIR=<folder>` uses another notes folder (and lets the app run beside the installed one, without global shortcuts or the start-with-Windows entry), `BUBBLENOTES_OPEN=manager,options,config,search` opens those windows at start, and `BUBBLENOTES_DEV=select|slash|back|autosize|config|configs|version|print|pdf:<file>|dock:left|dock:right|dock:top|dock:none|managerlater|crash|fault|fetch:<url>` does that to a note two seconds after start (for checking without the mouse). `BUBBLENOTES_MUTE=1` plays videos without sound.

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
| `update.rs`, `crash.rs` | Updates from GitHub releases; the crash log |
| `alarms.rs`, `due.rs`, `system.rs` | Sleep, alarms and reminders; dates typed in the text (`@friday`); global shortcuts and starting with Windows |
| `print.rs` | Printing a note and saving it as a PDF, through a browser |
| `editor_view.rs`, `media.rs` | A note's text as an editor element; pasted images and YouTube cards |
| `kit.rs`, `actions.rs`, `toggle.rs`, `slash.rs`, `formats.rs`, `ui.rs` | The editor: schema and plugins, the action catalog, toggle blocks, the slash menu, Markdown/HTML, toolbar and menus |
| `links.rs` | Links between notes: the `[[` menu and the link in the text |
| `drag.rs`, `layout.rs`, `win.rs` | Moving windows by hand; snapping and off-screen rescue; the few things asked of Windows directly |

## Things worth knowing

- **The main window never shows.** NSH ends the app when its main window closes, so it stays hidden and hosts the tray, the menus and every other window
- **Note windows are see-through**, which rules out a system shadow and click-through: a window's bounds are always exactly what's visible
- **Notes are dragged by the app, not the system** (`drag.rs`): a system drag region swallows clicks, and this way double-click rolls a note up and the configuration panel follows along
- **One editor session per note**, shown by both the floating note and the manager, so they always agree
- **Video inside notes** is YouTube's player in a web view (WebView2), made when a card's picture is pressed. Its page is given a web address of its own: the player refuses to play in a page without one
- **NSH's word on a window's focus and bounds comes late.** What must know now (fading, docking, a bubble's place) asks Windows (`win.rs`) or goes by where the app itself put the window
- **A list item may start with a toggle**: the starter kit's items must start with a paragraph, so `kit.rs` replaces the two item types with ones that needn't
- **Ghost notes go around NSH:** it refuses click-through windows to Rust apps, so `win.rs` sets the window style itself and sets it again after NSH resets it
- **Right-click menus are native**: a drawn menu would be clipped by a small note's window

## Patched framework

`vendor/nsh-base` and `vendor/nsh-platform-windows` are NSH 0.5.1 with a few
fixes of ours, used through `[patch.crates-io]` in `Cargo.toml`. Search them
for `BUBBLENOTES PATCH`:

- `vendor/nsh-platform-windows/src/text_input.rs`: a text field whose text
  is fully transparent draws nothing. Without it the rich editor's hidden
  input drew a black speck at the caret.
- `vendor/nsh-base/src/rich_editor/view/geometry.rs`: on Windows the caret
  counts the spaces at a line's end, also on a line that holds only a link
  (or another inline view). Without it the caret stood still after a typed
  space.
- `vendor/nsh-platform-windows/src/app_window.rs` and `surface.rs`: a
  window of the app's own that is resized waits for the layout at its new
  size before it draws (`Surface::await_turn`), and one that is made smaller
  shows its smaller content first (`shrink_first`); resized by its left
  or top edge, it lays out first and moves in step with the screen's
  frames. Without them a note resized by an edge showed its far corners
  square, or jumping, for a frame.

When NSH is upgraded: copy the new crates over these and apply the same
changes again, or drop the copies and the `[patch.crates-io]` section if NSH
has fixed them.
