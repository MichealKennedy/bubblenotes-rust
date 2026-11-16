//! The menu a right click on a note opens: a native menu, at the pointer,
//! and the two small windows it opens (a note's Markdown, a new category's
//! name).
//!
//! The menu is described as plain data first (`entries`: labels, check
//! marks, what each does), then turned into native items, so what it shows
//! for a note can be tested without a window.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use nsh_base::rich_editor::model::Node;
use nsh_base::{button, theme};
use nsh_core::desktop::WindowPosition;
use nsh_core::element::desktop::{MenuItem, displays, popup_menu, window};
use nsh_core::element::input;
use nsh_core::file_dialog::{FileFilter, SaveDialog};
use nsh_core::prelude::*;

use crate::alarms;
use crate::app::{App, Note, NoteCommand};
use crate::formats::Format;
use crate::model::{Alarm, Edge, Look, NoteMeta, Rect, Theme, builtin_themes};
use crate::store;

const MINUTE: f64 = 60_000.0;
const HOUR: f64 = 60.0 * MINUTE;

thread_local! {
    /// The note "Copy Note" took (what's saved about it, and its text), for
    /// "Paste Note".
    static COPIED: RefCell<Option<(NoteMeta, Node)>> = const { RefCell::new(None) };
}

/// The note whose Markdown the Markdown window shows.
#[derive(Clone, Default)]
struct MarkdownFor(Option<Rc<str>>);

/// The note the "New category" prompt is for.
#[derive(Clone, Default)]
struct NewCategoryFor(Option<Rc<str>>);

// ----- The menu as data ----------------------------------------------------------

/// How "Arrange" places notes.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Arrange {
    Cascade,
    Tile,
    Stack,
    Snap,
}

/// What a menu item does.
#[derive(Clone, Debug, PartialEq)]
enum Action {
    NewNote,
    /// A new note from the template with this id.
    FromTemplate(String),
    SaveTemplate,
    /// Show the note only with this program (`None`: always).
    Pin(Option<String>),
    EditConfiguration,
    ViewMarkdown,
    SendToBack,
    RollUp,
    AutoSize,
    AlwaysOnTop,
    ShowTitle,
    Lock,
    Ghost,
    FadeIdle,
    /// Dock to this edge of the screen (`None`: undock).
    Dock(Option<Edge>),
    Print,
    SavePdf,
    Bubble,
    Duplicate,
    CopyNote,
    PasteNote,
    CopyText,
    CopyMarkdown,
    Export,
    /// Sleep for this many milliseconds.
    SleepFor(f64),
    SleepUntilTomorrow,
    SleepUntilMonday,
    Wake,
    /// An alarm in this many milliseconds.
    AlarmIn(f64),
    AlarmTomorrow,
    AlarmCustom,
    AlarmClear,
    MoveTo(String),
    NewCategory,
    Theme(Look),
    /// Put back the text as it was then.
    RestoreVersion(f64),
    Delete,
    Hide,
    Arrange(Arrange),
}

#[derive(Clone, Debug, PartialEq)]
enum Entry {
    Item { label: String, shortcut: Option<&'static str>, checked: Option<bool>, disabled: bool, action: Option<Action> },
    Separator,
    Submenu { label: String, entries: Vec<Entry> },
}

impl Entry {
    fn checked(mut self, on: bool) -> Self {
        if let Entry::Item { checked, .. } = &mut self {
            *checked = Some(on);
        }
        self
    }

    fn disabled(mut self, off: bool) -> Self {
        if let Entry::Item { disabled, .. } = &mut self {
            *disabled = off;
        }
        self
    }

    fn shortcut(mut self, keystroke: &'static str) -> Self {
        if let Entry::Item { shortcut, .. } = &mut self {
            *shortcut = Some(keystroke);
        }
        self
    }
}

fn item(label: impl Into<String>, action: Action) -> Entry {
    Entry::Item { label: label.into(), shortcut: None, checked: None, disabled: false, action: Some(action) }
}

/// A line that only says something.
fn note_line(label: impl Into<String>) -> Entry {
    Entry::Item { label: label.into(), shortcut: None, checked: None, disabled: true, action: None }
}

fn submenu(label: &str, entries: Vec<Entry>) -> Entry {
    Entry::Submenu { label: label.into(), entries }
}

/// Everything the menu shows depends on.
struct MenuState<'a> {
    meta: &'a NoteMeta,
    now: f64,
    can_paste: bool,
    markdown_shown: bool,
    categories: &'a [String],
    user_themes: &'a [Theme],
    /// The templates: their ids and titles.
    templates: &'a [(String, String)],
    /// The program in front before this note was clicked ("" when unknown).
    front_app: &'a str,
    /// When the note's earlier versions are from, newest first.
    versions: &'a [f64],
}

/// The note's menu, as data.
fn entries(s: &MenuState) -> Vec<Entry> {
    let m = s.meta;
    let mut out = vec![item("New Note", Action::NewNote)];
    if !s.templates.is_empty() {
        out.push(submenu("New from Template", s.templates.iter().map(|(id, title)| item(title.clone(), Action::FromTemplate(id.clone()))).collect()));
    }
    out.extend([
        item("Edit configuration…", Action::EditConfiguration),
        item("View as markdown", Action::ViewMarkdown).checked(s.markdown_shown),
        Entry::Separator,
    ]);
    // Pinned to a program: shown only while it's in front.
    let pin = match &m.pinned_app {
        Some(app) => item(format!("Show only with {app}"), Action::Pin(None)).checked(true),
        None if s.front_app.is_empty() => item("Show only with a program", Action::Pin(None)).checked(false).disabled(true),
        None => item(format!("Show only with {}", s.front_app), Action::Pin(Some(s.front_app.to_string()))).checked(false),
    };
    if m.look.always_on_top {
        out.push(item("Send this note to back", Action::SendToBack).disabled(true));
        out.push(note_line("(but is always on top)"));
    } else {
        out.push(item("Send this note to back", Action::SendToBack));
    }
    out.extend([
        item(if m.rolled_up { "Roll Down" } else { "Roll Up" }, Action::RollUp),
        item("Auto size this note", Action::AutoSize),
        item("Always On Top", Action::AlwaysOnTop).checked(m.look.always_on_top),
        item("Show Title", Action::ShowTitle).checked(m.look.show_title),
        item("Lock Note", Action::Lock).checked(m.locked),
        item("Ghost (clicks pass through)", Action::Ghost).checked(m.ghost),
        item("Fade when idle", Action::FadeIdle).checked(m.fade_idle),
        pin,
        item("Show as Bubble", Action::Bubble).checked(m.bubble.enabled),
        submenu(
            "Dock to screen edge",
            vec![
                item("Left", Action::Dock(Some(Edge::Left))).checked(m.docked == Some(Edge::Left)),
                item("Right", Action::Dock(Some(Edge::Right))).checked(m.docked == Some(Edge::Right)),
                item("Top", Action::Dock(Some(Edge::Top))).checked(m.docked == Some(Edge::Top)),
                Entry::Separator,
                item("Undock", Action::Dock(None)).disabled(m.docked.is_none()),
            ],
        ),
        Entry::Separator,
        item("Duplicate Note", Action::Duplicate).shortcut("mod+d"),
        item("Save as Template", Action::SaveTemplate),
        item("Copy Note", Action::CopyNote),
        item("Paste Note", Action::PasteNote).disabled(!s.can_paste),
        item("Copy text only", Action::CopyText),
        item("Copy as markdown", Action::CopyMarkdown),
        item("Export note to file…", Action::Export),
        item("Print…", Action::Print),
        item("Save as PDF…", Action::SavePdf),
        Entry::Separator,
        submenu("Sleep", sleep_entries(m, s.now)),
        submenu("Alarm", alarm_entries(m, s.now)),
        submenu("Move to", category_entries(m, s.categories)),
        submenu("Theme", theme_entries(m, s.user_themes)),
        submenu("Earlier versions", version_entries(s.versions, s.now)),
        Entry::Separator,
        item("Delete", Action::Delete),
        item("Hide", Action::Hide),
        Entry::Separator,
        submenu(
            "Arrange",
            vec![
                item("Cascade all notes", Action::Arrange(Arrange::Cascade)),
                item("Tile all notes", Action::Arrange(Arrange::Tile)),
                item("Stack here", Action::Arrange(Arrange::Stack)),
                item("Snap to nearest corner", Action::Arrange(Arrange::Snap)),
            ],
        ),
    ]);
    out
}

fn sleep_entries(m: &NoteMeta, now: f64) -> Vec<Entry> {
    let mut out = vec![
        item("15 minutes", Action::SleepFor(15.0 * MINUTE)),
        item("1 hour", Action::SleepFor(HOUR)),
        item("4 hours", Action::SleepFor(4.0 * HOUR)),
        item("Until tomorrow 9:00", Action::SleepUntilTomorrow),
        item("Until Monday 9:00", Action::SleepUntilMonday),
    ];
    if m.asleep(now) {
        out.extend([Entry::Separator, item("Wake up", Action::Wake)]);
    }
    out
}

fn alarm_entries(m: &NoteMeta, now: f64) -> Vec<Entry> {
    let mut out = Vec::new();
    if m.alarm.is_some() {
        let alarm_only = NoteMeta { sleep_until: None, ..m.clone() };
        if let Some(text) = alarms::describe(&alarm_only, now) {
            out.extend([note_line(text), Entry::Separator]);
        }
    }
    out.extend([
        item("In 15 minutes", Action::AlarmIn(15.0 * MINUTE)),
        item("In 1 hour", Action::AlarmIn(HOUR)),
        item("Tomorrow 9:00", Action::AlarmTomorrow),
        Entry::Separator,
        item("Custom…", Action::AlarmCustom),
    ]);
    if m.alarm.is_some() {
        out.push(item("Clear alarm", Action::AlarmClear));
    }
    out
}

/// The versions shown in the menu (the newest ones).
const VERSIONS_SHOWN: usize = 15;

fn version_entries(versions: &[f64], now: f64) -> Vec<Entry> {
    if versions.is_empty() {
        return vec![note_line("None yet: they're kept as you edit")];
    }
    let mut out = vec![note_line("Put the text back as it was:"), Entry::Separator];
    out.extend(versions.iter().take(VERSIONS_SHOWN).map(|&at| item(alarms::describe_past(at, now), Action::RestoreVersion(at))));
    out
}

fn category_entries(m: &NoteMeta, categories: &[String]) -> Vec<Entry> {
    let mut out: Vec<Entry> = categories.iter().map(|c| item(c.as_str(), Action::MoveTo(c.clone())).checked(*c == m.category)).collect();
    out.extend([
        item("No category", Action::MoveTo(String::new())).checked(m.category.is_empty()),
        Entry::Separator,
        item("New category…", Action::NewCategory),
    ]);
    out
}

fn theme_entries(m: &NoteMeta, user_themes: &[Theme]) -> Vec<Entry> {
    let entry = |theme: &Theme| item(theme.name.as_str(), Action::Theme(theme.look.clone())).checked(same_colors(&m.look, &theme.look));
    let mut out: Vec<Entry> = builtin_themes().iter().map(entry).collect();
    if !user_themes.is_empty() {
        out.push(Entry::Separator);
        out.extend(user_themes.iter().map(entry));
    }
    out
}

/// The look `theme` gives a note that looks like `look`: its colours and
/// type, keeping how the note behaves.
fn themed(look: &Look, theme: &Look) -> Look {
    Look { always_on_top: look.always_on_top, show_title: look.show_title, ..theme.clone() }
}

/// Whether `look` is what `theme` would make it.
fn same_colors(look: &Look, theme: &Look) -> bool {
    themed(look, theme) == *look
}

/// The native menu for `entries`; choosing an item hands its action to `run`.
fn to_menu(entries: Vec<Entry>, run: &Rc<dyn Fn(Action)>) -> Vec<MenuItem> {
    entries
        .into_iter()
        .map(|entry| match entry {
            Entry::Separator => MenuItem::separator(),
            Entry::Submenu { label, entries } => MenuItem::submenu(label, to_menu(entries, run)),
            Entry::Item { label, shortcut, checked, disabled, action } => {
                let mut native = MenuItem::new(label).disabled(disabled);
                if let Some(keystroke) = shortcut {
                    native = native.shortcut(keystroke);
                }
                if let Some(on) = checked {
                    native = native.checked(on);
                }
                if let Some(action) = action {
                    let run = run.clone();
                    native = native.on_select(move || run(action.clone()));
                }
                native
            }
        })
        .collect()
}

// ----- Showing it and doing what was chosen ----------------------------------------

pub fn show(app: App, note: &Note) {
    let meta = note.meta.get_untracked();
    let categories = untracked(|| app.categories());
    let user_themes = app.settings.with_untracked(|s| s.themes.clone());
    let markdown_shown = global::<MarkdownFor>().with_untracked(|f| f.0.as_deref() == Some(&*note.id));
    let templates: Vec<(String, String)> =
        untracked(|| app.templates()).iter().map(|t| (t.id.to_string(), t.meta.with_untracked(|m| m.title.clone()))).collect();
    let front_app = app.foreground_app.get_untracked();
    let state = MenuState {
        meta: &meta,
        now: store::now_ms(),
        can_paste: COPIED.with(|c| c.borrow().is_some()),
        markdown_shown,
        categories: &categories,
        user_themes: &user_themes,
        templates: &templates,
        front_app: &front_app,
        versions: &app.versions(note),
    };
    let note = note.clone();
    let run: Rc<dyn Fn(Action)> = Rc::new(move |action| perform(app, &note, action));
    let _ = popup_menu(to_menu(entries(&state), &run));
}

fn perform(app: App, note: &Note, action: Action) {
    let id = note.id.clone();
    let now = store::now_ms();
    match action {
        Action::NewNote => {
            let category = note.meta.with_untracked(|m| m.category.clone());
            app.new_note(&category);
        }
        Action::FromTemplate(template) => {
            if let Some(template) = app.note(&template) {
                let fresh = app.new_from_template(&template);
                app.command.set(Some((fresh.id.clone(), NoteCommand::Focus)));
            }
        }
        Action::SaveTemplate => {
            app.save_as_template(note);
        }
        Action::Pin(program) => app.update(note, |m| m.pinned_app = program),
        Action::EditConfiguration => app.config_for.set(Some(id)),
        Action::ViewMarkdown => global::<MarkdownFor>().update(|f| f.0 = if f.0.as_deref() == Some(&*id) { None } else { Some(id) }),
        Action::SendToBack => app.command.set(Some((id, NoteCommand::SendToBack))),
        Action::RollUp => app.update(note, |m| m.rolled_up = !m.rolled_up),
        Action::AutoSize => app.command.set(Some((id, NoteCommand::AutoSize))),
        Action::AlwaysOnTop => app.update(note, |m| m.look.always_on_top = !m.look.always_on_top),
        Action::ShowTitle => app.update(note, |m| m.look.show_title = !m.look.show_title),
        Action::Lock => app.update(note, |m| m.locked = !m.locked),
        Action::Ghost => app.update(note, |m| m.ghost = !m.ghost),
        Action::FadeIdle => app.update(note, |m| m.fade_idle = !m.fade_idle),
        Action::Dock(edge) => app.dock(note, edge),
        Action::Print => crate::print::print(app, note),
        Action::SavePdf => crate::print::save_pdf(app, note.clone()),
        Action::Bubble => app.update(note, |m| {
            m.bubble.enabled = !m.bubble.enabled;
            // The bubble appears where the note is.
            if m.bubble.enabled {
                m.bubble.x = m.rect.x;
                m.bubble.y = m.rect.y;
            }
        }),
        Action::Duplicate => {
            app.duplicate(note);
        }
        Action::CopyNote => {
            let copy = (note.meta.get_untracked(), note.editor.doc());
            COPIED.with(|c| *c.borrow_mut() = Some(copy));
            set_clipboard(note.plain_text());
        }
        Action::PasteNote => paste(app, note, now),
        Action::CopyText => set_clipboard(note.plain_text()),
        Action::CopyMarkdown => set_clipboard(note.markdown()),
        Action::Export => export(app, note.clone()),
        Action::SleepFor(ms) => app.sleep_for(note, ms),
        Action::SleepUntilTomorrow => app.sleep_for(note, alarms::tomorrow_morning(now) - now),
        Action::SleepUntilMonday => app.sleep_for(note, alarms::monday_morning(now) - now),
        Action::Wake => app.wake(note),
        Action::AlarmIn(ms) => app.set_alarm(note, Some(Alarm { at: now + ms, repeat: Default::default() })),
        Action::AlarmTomorrow => app.set_alarm(note, Some(Alarm { at: alarms::tomorrow_morning(now), repeat: Default::default() })),
        Action::AlarmCustom => app.config_for.set(Some(id)),
        Action::AlarmClear => app.set_alarm(note, None),
        Action::MoveTo(category) => app.update(note, |m| m.category = category),
        Action::NewCategory => global::<NewCategoryFor>().set(NewCategoryFor(Some(id))),
        Action::Theme(look) => app.update(note, |m| m.look = themed(&m.look, &look)),
        Action::RestoreVersion(at) => app.restore_version(note, at),
        Action::Delete => app.trash(note),
        Action::Hide => app.update(note, |m| m.on_desktop = false),
        Action::Arrange(how) => arrange(app, note, how),
    }
}

/// A full copy of the copied note, beside `here`.
fn paste(app: App, here: &Note, now: f64) {
    let Some((mut meta, doc)) = COPIED.with(|c| c.borrow().clone()) else { return };
    let (x, y) = here.meta.with_untracked(|m| if m.bubble.enabled { (m.bubble.x, m.bubble.y) } else { (m.rect.x, m.rect.y) });
    meta.id = store::new_id();
    meta.file = String::new();
    meta.created = now;
    meta.modified = now;
    meta.on_desktop = true;
    meta.trashed = None;
    meta.alarm = None;
    meta.sleep_until = None;
    meta.rect.x = x + 28.0;
    meta.rect.y = y + 28.0;
    meta.bubble.x = x + 28.0;
    meta.bubble.y = y + 28.0;
    let note = app.add(meta, Some(doc));
    app.show_on_desktop(&note);
}

/// Saves the note as Markdown, HTML or plain text, by the extension the
/// user picks.
fn export(app: App, note: Note) {
    let stem = store::file_stem_for(&note.meta.with_untracked(|m| m.title.clone()));
    let dialog = SaveDialog::new()
        .title("Export note")
        .file_name(format!("{stem}.md"))
        .filter(FileFilter::new("Markdown", ["md"]))
        .filter(FileFilter::new("HTML", ["html", "htm"]))
        .filter(FileFilter::new("Plain text", ["txt"]));
    // Under the app's owner: the note's window may close meanwhile.
    app.owner.run(|| {
        let _ = spawn(async move {
            let path = match pick_save_path(dialog).await {
                Ok(Some(path)) => path,
                Ok(None) => return,
                Err(e) => return app.problem.set(Some(format!("Couldn't show the save dialog: {e}"))),
            };
            let content = match export_kind(&path) {
                ExportKind::Html => Format::Html.serialize(&note.editor.doc()),
                ExportKind::Text => Ok(note.plain_text()),
                ExportKind::Markdown => Ok(note.markdown()),
            };
            if let Err(e) = content.and_then(|text| std::fs::write(&path, text).map_err(|e| e.to_string())) {
                app.problem.set(Some(format!("Couldn't export to {}: {e}", path.display())));
            }
        });
    });
}

#[derive(Debug, PartialEq)]
enum ExportKind {
    Markdown,
    Html,
    Text,
}

fn export_kind(path: &Path) -> ExportKind {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("html" | "htm") => ExportKind::Html,
        Some("txt") => ExportKind::Text,
        _ => ExportKind::Markdown,
    }
}

// ----- Arrange -------------------------------------------------------------------

/// From the work area's edges.
const MARGIN: f32 = 24.0;
const CASCADE_STEP: f32 = 28.0;
const GAP: f32 = 12.0;
const STACK_STEP: f32 = 6.0;
const TILE_MAX: (f32, f32) = (320.0, 300.0);
const TILE_MIN: (f32, f32) = (140.0, 90.0);

/// Where a note shows: its bubble when it's one.
fn placed(m: &NoteMeta) -> Rect {
    if m.bubble.enabled { Rect { x: m.bubble.x, y: m.bubble.y, w: m.bubble.size, h: m.bubble.size } } else { m.rect }
}

/// Puts a note at `rect`: a bubble only moves.
fn place(m: &mut NoteMeta, rect: Rect) {
    if m.bubble.enabled {
        m.bubble.x = rect.x;
        m.bubble.y = rect.y;
    } else {
        m.rect = rect;
    }
}

fn arrange(app: App, note: &Note, how: Arrange) {
    let here = note.meta.with_untracked(placed);
    let work = work_area(here);
    let notes: Vec<Note> = untracked(|| app.desktop_notes());
    let rects: Vec<Rect> = notes.iter().map(|n| n.meta.with_untracked(placed)).collect();
    let this = notes.iter().position(|n| n.id == note.id);
    let moved = match (how, this) {
        (Arrange::Snap, _) => {
            app.update(note, |m| place(m, snap_corner(work, here)));
            return;
        }
        (Arrange::Cascade, _) => cascade(work, &rects),
        (Arrange::Tile, _) => tile(work, &rects),
        (Arrange::Stack, Some(this)) => stack(here, &rects, this),
        (Arrange::Stack, None) => return,
    };
    for (other, rect) in notes.iter().zip(moved) {
        app.update(other, |m| place(m, rect));
    }
    if how == Arrange::Stack {
        app.command.set(Some((note.id.clone(), NoteCommand::Focus)));
    }
}

/// The work area (without the taskbar) of the display the note is on.
fn work_area(note: Rect) -> Rect {
    let (cx, cy) = (note.x + note.w / 2.0, note.y + note.h / 2.0);
    let all = displays();
    let contains = |b: &Bounds<Points>| cx >= b.origin.x.0 && cx < b.origin.x.0 + b.size.width.0 && cy >= b.origin.y.0 && cy < b.origin.y.0 + b.size.height.0;
    all.iter()
        .find(|d| contains(&d.bounds))
        .or_else(|| all.iter().find(|d| d.primary))
        .or_else(|| all.first())
        .map(|d| {
            let b = d.visible_bounds;
            Rect { x: b.origin.x.0, y: b.origin.y.0, w: b.size.width.0, h: b.size.height.0 }
        })
        .unwrap_or(Rect { x: 0.0, y: 0.0, w: 1280.0, h: 720.0 })
}

/// Each note 28pt down and right from the one before, from the work area's
/// top left; back to the start when one would leave the work area.
fn cascade(work: Rect, rects: &[Rect]) -> Vec<Rect> {
    let mut step = 0.0;
    rects
        .iter()
        .map(|r| {
            let mut offset = MARGIN + step * CASCADE_STEP;
            if step > 0.0 && (offset + r.w > work.w || offset + r.h > work.h) {
                step = 0.0;
                offset = MARGIN;
            }
            step += 1.0;
            Rect { x: work.x + offset, y: work.y + offset, ..*r }
        })
        .collect()
}

/// A grid filling the work area, 12pt apart; each note takes its cell's
/// size, at most 320 x 300.
fn tile(work: Rect, rects: &[Rect]) -> Vec<Rect> {
    let n = rects.len();
    if n == 0 {
        return Vec::new();
    }
    // Columns for cells about as wide as the screen is.
    let cols = ((n as f32 * work.w / work.h.max(1.0)).sqrt().ceil() as usize).clamp(1, n);
    let rows = n.div_ceil(cols);
    let cell_w = (work.w - GAP * (cols as f32 + 1.0)) / cols as f32;
    let cell_h = (work.h - GAP * (rows as f32 + 1.0)) / rows as f32;
    let (w, h) = (cell_w.min(TILE_MAX.0).max(TILE_MIN.0), cell_h.min(TILE_MAX.1).max(TILE_MIN.1));
    (0..n)
        .map(|i| {
            let (col, row) = ((i % cols) as f32, (i / cols) as f32);
            Rect { x: work.x + GAP + col * (cell_w + GAP), y: work.y + GAP + row * (cell_h + GAP), w, h }
        })
        .collect()
}

/// Every note at `at`'s place, each 6pt further down and right, the note
/// at `this` last (on top).
fn stack(at: Rect, rects: &[Rect], this: usize) -> Vec<Rect> {
    let last = rects.len().saturating_sub(1);
    rects
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let order = match i.cmp(&this) {
                std::cmp::Ordering::Less => i,
                std::cmp::Ordering::Equal => last,
                std::cmp::Ordering::Greater => i - 1,
            };
            let offset = order as f32 * STACK_STEP;
            Rect { x: at.x + offset, y: at.y + offset, ..*r }
        })
        .collect()
}

/// `rect` moved into the work area's nearest corner, 12pt from its edges.
fn snap_corner(work: Rect, rect: Rect) -> Rect {
    let left = rect.x + rect.w / 2.0 < work.x + work.w / 2.0;
    let top = rect.y + rect.h / 2.0 < work.y + work.h / 2.0;
    Rect {
        x: if left { work.x + GAP } else { work.x + work.w - GAP - rect.w },
        y: if top { work.y + GAP } else { work.y + work.h - GAP - rect.h },
        ..rect
    }
}

// ----- Windows -------------------------------------------------------------------

/// Windows this module owns (a note's Markdown view, the new category
/// prompt), mounted once in the app's root.
pub fn windows(app: App) -> impl IntoElement {
    let shown = global::<MarkdownFor>();
    // A note that went to the trash (or for good) takes its view along.
    effect(move || {
        let Some(id) = shown.with(|f| f.0.clone()) else { return };
        let gone = app.notes.with(|notes| !notes.iter().any(|n| n.id == id && n.meta.with(|m| m.trashed.is_none())));
        if gone {
            shown.set(MarkdownFor(None));
        }
    });
    view().child(markdown_window(app)).child(category_window(app))
}

fn markdown_window(app: App) -> impl IntoElement {
    let shown = global::<MarkdownFor>();
    let note = move || shown.with(|f| f.0.clone()).and_then(|id| app.note(&id));
    window()
        .id("note-markdown")
        .title(move || format!("{} — Markdown", note().map(|n| n.meta.with(|m| m.title.clone())).unwrap_or_default()))
        .default_size(size(px(560.0), px(520.0)))
        .open(move || shown.with(|f| f.0.is_some()))
        .on_open_change(move |change| {
            if !change.open {
                shown.set(MarkdownFor(None));
            }
        })
        .content(move || {
            // It follows the note's text as it's typed.
            let markdown = move || {
                app.revision.get();
                note().map(|n| n.markdown()).unwrap_or_default()
            };
            view()
                .flex_1()
                .gap_3()
                .p_4()
                .bg(move || theme().background)
                .child(
                    input()
                        .multiline(true)
                        .readonly(true)
                        .scrollbar(true)
                        .value(markdown)
                        .flex_1()
                        .p_3()
                        .rounded(px(10.0))
                        .border_1()
                        .border_color(move || theme().border)
                        .bg(move || theme().card)
                        .text_color(move || theme().card_foreground)
                        .font_family("Cascadia Mono, Consolas, monospace")
                        .text_size(px(13.0)),
                )
                .child(
                    view().flex_row().justify_end().child(dialog_button("Copy", true, move || {
                        if let Some(n) = note() {
                            set_clipboard(n.markdown());
                        }
                    })),
                )
        })
}

fn category_window(app: App) -> impl IntoElement {
    let asked = global::<NewCategoryFor>();
    let close = move || asked.set(NewCategoryFor(None));
    window()
        .id("new-category")
        .title("New category")
        .default_size(size(px(360.0), px(168.0)))
        .default_position(WindowPosition::Center)
        .open(move || asked.with(|a| a.0.is_some()))
        .on_open_change(move |change| {
            if !change.open {
                close();
            }
        })
        .content(move || {
            let name = signal(String::new());
            let ok = move || {
                let category = store::clean_category(&name.get_untracked());
                let note = asked.with_untracked(|a| a.0.clone()).and_then(|id| app.note(&id));
                if let (false, Some(note)) = (category.is_empty(), note) {
                    app.update(&note, |m| m.category = category);
                    close();
                }
            };
            view()
                .flex_1()
                .gap_3()
                .p_4()
                .bg(move || theme().background)
                .child(text("Move the note to a new category:").text_size(px(13.0)).text_color(move || theme().foreground))
                .child(
                    input()
                        .auto_focus(true)
                        .placeholder("Category name (Work/Ideas for a sub-category)")
                        .on_change(move |v: &str| name.set(v.to_string()))
                        .on_submit(move |_: &str| ok())
                        .h(px(32.0))
                        .px_3()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(move || theme().input)
                        .bg(move || theme().card)
                        .text_color(move || theme().card_foreground)
                        .text_size(px(13.0)),
                )
                .child(
                    view()
                        .flex_row()
                        .justify_end()
                        .gap_2()
                        .child(dialog_button("Cancel", false, close))
                        .child(dialog_button("OK", true, ok)),
                )
        })
}

/// A dialog's button: `primary` is the filled one.
fn dialog_button(label: &'static str, primary: bool, press: impl Fn() + 'static) -> impl IntoElement {
    button()
        .cursor(Cursor::Pointer).on_press(press)
        .h(px(30.0))
        .px_4()
        .items_center()
        .justify_center()
        .rounded(px(8.0))
        .border_1()
        .border_color(move || if primary { theme().primary } else { theme().border })
        .bg(move || if primary { theme().primary } else { theme().secondary })
        .active(|s| s.opacity(0.7))
        .child(text(label).text_size(px(13.0)).text_color(move || if primary { theme().primary_foreground } else { theme().secondary_foreground }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state<'a>(meta: &'a NoteMeta, categories: &'a [String]) -> MenuState<'a> {
        MenuState { meta, now: 1_000_000.0, can_paste: false, markdown_shown: false, categories, user_themes: &[], templates: &[], front_app: "", versions: &[] }
    }

    /// The labels at the top level ("---" for separators, "label >" for
    /// submenus, "[x]" or "[ ]" for check marks, "(off)" for disabled).
    fn labels(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|e| match e {
                Entry::Separator => "---".to_string(),
                Entry::Submenu { label, .. } => format!("{label} >"),
                Entry::Item { label, checked, disabled, .. } => {
                    let mark = match checked {
                        Some(true) => "[x] ",
                        Some(false) => "[ ] ",
                        None => "",
                    };
                    format!("{mark}{label}{}", if *disabled { " (off)" } else { "" })
                }
            })
            .collect()
    }

    fn sub<'a>(entries: &'a [Entry], name: &str) -> &'a [Entry] {
        entries
            .iter()
            .find_map(|e| match e {
                Entry::Submenu { label, entries } if label == name => Some(entries.as_slice()),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn the_menu_of_a_plain_note() {
        let meta = NoteMeta::default();
        let menu = entries(&state(&meta, &[]));
        assert_eq!(
            labels(&menu),
            [
                "New Note",
                "Edit configuration…",
                "[ ] View as markdown",
                "---",
                "Send this note to back",
                "Roll Up",
                "Auto size this note",
                "[ ] Always On Top",
                "[x] Show Title",
                "[ ] Lock Note",
                "[ ] Ghost (clicks pass through)",
                "[ ] Fade when idle",
                "[ ] Show only with a program (off)",
                "[ ] Show as Bubble",
                "Dock to screen edge >",
                "---",
                "Duplicate Note",
                "Save as Template",
                "Copy Note",
                "Paste Note (off)",
                "Copy text only",
                "Copy as markdown",
                "Export note to file…",
                "Print…",
                "Save as PDF…",
                "---",
                "Sleep >",
                "Alarm >",
                "Move to >",
                "Theme >",
                "Earlier versions >",
                "---",
                "Delete",
                "Hide",
                "---",
                "Arrange >",
            ]
        );
        assert_eq!(labels(sub(&menu, "Sleep")), ["15 minutes", "1 hour", "4 hours", "Until tomorrow 9:00", "Until Monday 9:00"]);
        assert_eq!(labels(sub(&menu, "Alarm")), ["In 15 minutes", "In 1 hour", "Tomorrow 9:00", "---", "Custom…"]);
        assert_eq!(labels(sub(&menu, "Move to")), ["[x] No category", "---", "New category…"]);
        let themes = labels(sub(&menu, "Theme"));
        assert_eq!(themes[0], "[x] Butter");
        assert!(themes[1..].iter().all(|t| t.starts_with("[ ]")));
        assert_eq!(labels(sub(&menu, "Arrange")), ["Cascade all notes", "Tile all notes", "Stack here", "Snap to nearest corner"]);
        // The shortcut is valid: `MenuItem::shortcut` would panic otherwise.
        let run: Rc<dyn Fn(Action)> = Rc::new(|_| {});
        assert_eq!(labels(sub(&menu, "Earlier versions")), ["None yet: they're kept as you edit (off)"]);
        assert_eq!(labels(sub(&menu, "Dock to screen edge")), ["[ ] Left", "[ ] Right", "[ ] Top", "---", "Undock (off)"]);
        assert_eq!(to_menu(menu, &run).len(), 36);
    }

    #[test]
    fn the_menu_follows_the_note() {
        let mut meta = NoteMeta { rolled_up: true, locked: true, category: "Work".into(), sleep_until: Some(2_000_000.0), ..NoteMeta::default() };
        meta.look.always_on_top = true;
        meta.look.show_title = false;
        meta.bubble.enabled = true;
        meta.alarm = Some(Alarm { at: 5_000_000.0, repeat: crate::model::Repeat::Daily });
        let categories = vec!["Home".to_string(), "Work".to_string()];
        let mut s = state(&meta, &categories);
        s.can_paste = true;
        s.markdown_shown = true;
        let menu = entries(&s);
        let top = labels(&menu);
        assert_eq!(top[2], "[x] View as markdown");
        assert_eq!(top[4..7], ["Send this note to back (off)", "(but is always on top) (off)", "Roll Down"]);
        assert!(top.contains(&"[x] Always On Top".to_string()));
        assert!(top.contains(&"[ ] Show Title".to_string()));
        assert!(top.contains(&"[x] Lock Note".to_string()));
        assert!(top.contains(&"[x] Show as Bubble".to_string()));
        assert!(top.contains(&"Paste Note".to_string()));
        assert_eq!(labels(sub(&menu, "Sleep"))[5..], ["---", "Wake up"]);
        let alarm = labels(sub(&menu, "Alarm"));
        assert!(alarm[0].starts_with("Alarm ") && alarm[0].ends_with(", repeats daily (off)"), "{}", alarm[0]);
        assert_eq!(alarm[1], "---");
        assert_eq!(alarm.last().map(String::as_str), Some("Clear alarm"));
        assert_eq!(labels(sub(&menu, "Move to")), ["[ ] Home", "[x] Work", "[ ] No category", "---", "New category…"]);
    }

    #[test]
    fn a_theme_keeps_behaviour_and_is_checked_on_a_pinned_note() {
        let sky = builtin_themes().into_iter().find(|t| t.name == "Sky").unwrap();
        let mut look = Look { always_on_top: true, show_title: false, ..Look::default() };
        look = themed(&look, &sky.look);
        assert!(look.always_on_top && !look.show_title);
        assert_eq!(look.background, sky.look.background);
        assert!(same_colors(&look, &sky.look));
        let mine = vec![Theme { name: "Mine".into(), look: Look { background: 0x123456, ..Look::default() } }];
        let meta = NoteMeta { look, ..NoteMeta::default() };
        let mut s = state(&meta, &[]);
        s.user_themes = &mine;
        let themes = labels(sub(&entries(&s), "Theme"));
        assert!(themes.contains(&"[x] Sky".to_string()));
        assert_eq!(themes[themes.len() - 2..], ["---", "[ ] Mine"]);
    }

    #[test]
    fn exports_by_extension() {
        assert_eq!(export_kind(Path::new("a.HTML")), ExportKind::Html);
        assert_eq!(export_kind(Path::new("a.htm")), ExportKind::Html);
        assert_eq!(export_kind(Path::new("a.txt")), ExportKind::Text);
        assert_eq!(export_kind(Path::new("a.md")), ExportKind::Markdown);
        assert_eq!(export_kind(Path::new("a")), ExportKind::Markdown);
    }

    const WORK: Rect = Rect { x: 0.0, y: 0.0, w: 1920.0, h: 1040.0 };

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn cascading() {
        let notes = [r(500.0, 500.0, 280.0, 260.0), r(0.0, 0.0, 200.0, 200.0), r(9.0, 9.0, 280.0, 260.0)];
        let out = cascade(r(100.0, 50.0, 1000.0, 800.0), &notes);
        assert_eq!(out[0], r(124.0, 74.0, 280.0, 260.0));
        assert_eq!(out[1], r(152.0, 102.0, 200.0, 200.0));
        assert_eq!(out[2], r(180.0, 130.0, 280.0, 260.0));
        // On a small work area it starts over instead of leaving it.
        let out = cascade(r(0.0, 0.0, 400.0, 330.0), &[r(0.0, 0.0, 280.0, 260.0); 4]);
        assert_eq!(out.iter().map(|r| r.x).collect::<Vec<_>>(), [24.0, 52.0, 24.0, 52.0]);
    }

    #[test]
    fn tiling() {
        assert!(tile(WORK, &[]).is_empty());
        let out = tile(WORK, &[r(0.0, 0.0, 100.0, 100.0); 4]);
        // Four notes on a wide screen: three columns, two rows; capped size.
        assert_eq!(out[0], r(12.0, 12.0, 320.0, 300.0));
        assert_eq!(out[1].y, 12.0);
        assert!(out[1].x > out[0].x + 320.0);
        assert_eq!(out[3].x, 12.0);
        assert!(out[3].y > 300.0);
        // Many notes: small cells, all inside the work area, none overlapping.
        let out = tile(WORK, &[r(0.0, 0.0, 280.0, 260.0); 30]);
        for (i, a) in out.iter().enumerate() {
            assert!(a.x >= 0.0 && a.y >= 0.0 && a.x + a.w <= WORK.w && a.y + a.h <= WORK.h, "{a:?}");
            assert!(a.w < 320.0 && a.h < 300.0);
            for b in &out[i + 1..] {
                assert!(a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y);
            }
        }
    }

    #[test]
    fn stacking_puts_this_one_last() {
        let notes = [r(1.0, 1.0, 100.0, 100.0), r(300.0, 300.0, 280.0, 260.0), r(2.0, 2.0, 50.0, 50.0)];
        let out = stack(r(300.0, 300.0, 280.0, 260.0), &notes, 1);
        assert_eq!(out[0], r(300.0, 300.0, 100.0, 100.0));
        assert_eq!(out[2], r(306.0, 306.0, 50.0, 50.0));
        assert_eq!(out[1], r(312.0, 312.0, 280.0, 260.0));
    }

    #[test]
    fn snapping_to_the_nearest_corner() {
        let work = r(0.0, 0.0, 1000.0, 800.0);
        assert_eq!(snap_corner(work, r(100.0, 90.0, 200.0, 100.0)), r(12.0, 12.0, 200.0, 100.0));
        assert_eq!(snap_corner(work, r(700.0, 90.0, 200.0, 100.0)), r(788.0, 12.0, 200.0, 100.0));
        assert_eq!(snap_corner(work, r(100.0, 600.0, 200.0, 100.0)), r(12.0, 688.0, 200.0, 100.0));
        assert_eq!(snap_corner(r(1920.0, 0.0, 1000.0, 800.0), r(2800.0, 700.0, 100.0, 100.0)), r(2808.0, 688.0, 100.0, 100.0));
    }
}
