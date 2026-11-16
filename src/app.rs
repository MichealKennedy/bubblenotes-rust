//! The app's state, shared by every window: the notes (each with its live
//! editor), the settings, and what changes them. `App` is a `Copy` handle;
//! windows keep it and call its methods.
//!
//! Saving is automatic: a note's text is written a moment after it stops
//! changing, and so is the index (looks, positions, settings) after
//! `App::update` or `App::update_settings`.
//!
//! Other modules add what they need to `App` in `impl App` blocks of their
//! own (menus, alarms, the clipboard...), so this file stays the core.

use std::rc::Rc;
use std::time::Duration;

use nsh_base::rich_editor::model::Node;
use nsh_core::prelude::*;
use serde_json::Value;

use crate::formats::{self, Format};
use crate::kit::{AppEvent, Editor, Kit, Relay};
use crate::model::{Index, NoteMeta, Rect, Settings};
use crate::store::{self, Vault};

/// How long after the last change a note is written.
const SAVE_AFTER: f64 = 600.0;

/// One note: what's saved about it, and its live editor (the same session
/// in every window that shows it, so they all show the same text).
#[derive(Clone)]
pub struct Note {
    pub id: Rc<str>,
    pub meta: Signal<NoteMeta>,
    pub editor: Editor,
    /// The document as last written to its file.
    saved: Signal<Node>,
}

impl PartialEq for Note {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Note {
    /// The note's text without formatting.
    pub fn plain_text(&self) -> String {
        Format::Text.serialize(&self.editor.doc()).unwrap_or_default()
    }

    pub fn markdown(&self) -> String {
        Format::Markdown.serialize(&self.editor.doc()).unwrap_or_default()
    }
}

/// What can be asked of a note's desktop window (`App::command`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NoteCommand {
    /// Behind every other window.
    SendToBack,
    /// In front, with the keyboard focus.
    Focus,
    /// As tall as its text needs.
    AutoSize,
}

thread_local! {
    /// The running app, for code the framework calls without it (an
    /// editor's node views).
    static CURRENT: std::cell::Cell<Option<App>> = const { std::cell::Cell::new(None) };
}

#[derive(Clone, Copy)]
pub struct App {
    /// Every note, deleted ones (in the trash) included.
    pub notes: Signal<Vec<Note>>,
    pub settings: Signal<Settings>,
    pub kit: Stored<Rc<Kit>>,
    pub vault: Stored<Rc<Vault>>,
    /// Bumped on every change of any editor's state: what toolbars' active
    /// states and word counts follow.
    pub revision: Signal<u64>,
    /// Now, in milliseconds since the Unix epoch, ticking about once a
    /// second: reading it keeps sleeping notes and countdowns up to date.
    pub now: Signal<f64>,
    pub manager_open: Signal<bool>,
    /// The note the Notes Manager shows.
    pub selected: Signal<Option<Rc<str>>>,
    /// The note whose configuration panel is open.
    pub config_for: Signal<Option<Rc<str>>>,
    pub options_open: Signal<bool>,
    /// A note window being dragged: its note and its top left corner now,
    /// in points (`None` when none is). A panel beside it follows this.
    pub dragging: Signal<Option<(Rc<str>, f32, f32)>>,
    /// Something asked of a note's desktop window (its note's id, what).
    /// The window takes it and sets this back to `None`.
    pub command: Signal<Option<(Rc<str>, NoteCommand)>>,
    /// The note whose right-click menu is showing.
    pub menu_for: Signal<Option<Rc<str>>>,
    /// The quick search box is open.
    pub search_open: Signal<bool>,
    /// The program in front, or the last one that was before this app came
    /// in front (its file name in lower case; empty until one is known):
    /// what notes pinned to a program follow.
    pub foreground_app: Signal<String>,
    /// Ctrl+Alt is held: ghost notes take clicks for the while, so they
    /// can be moved, edited or made solid again.
    pub ghosts_solid: Signal<bool>,
    /// The last thing that went wrong, for whoever shows it.
    pub problem: Signal<Option<String>>,
    /// The owner the app's signals and tasks live under.
    pub owner: Owner,
    index_dirty: Signal<bool>,
    /// When an editor last changed (0 once everything is written).
    text_changed: Signal<f64>,
}

impl App {
    /// Opens the notes folder and loads every note. Call once, from the
    /// root component.
    pub fn start() -> Self {
        let relay = Relay::default();
        let kit = Kit::new(relay.clone());
        let root = Vault::chosen_root();
        let vault = Rc::new(Vault::open(&root).unwrap_or_else(|e| panic!("can't open the notes folder {}: {e}", root.display())));
        let Index { settings, notes: metas } = vault.load();
        let first_run = metas.is_empty() && !vault.root().join(".bubblenotes").join("index.json").exists();

        let app = Self {
            notes: signal(Vec::new()),
            settings: signal(settings),
            kit: stored(kit),
            vault: stored(vault),
            revision: signal(0),
            now: signal(store::now_ms()),
            manager_open: signal(false),
            selected: signal(None),
            config_for: signal(None),
            options_open: signal(false),
            dragging: signal(None),
            command: signal(None),
            menu_for: signal(None),
            search_open: signal(false),
            foreground_app: signal(String::new()),
            ghosts_solid: signal(false),
            problem: signal(None),
            owner: Owner::current(),
            index_dirty: signal(false),
            text_changed: signal(0.0),
        };
        CURRENT.with(|current| current.set(Some(app)));
        let notes: Vec<Note> = metas.into_iter().map(|meta| app.open_note(meta)).collect();
        app.notes.set(notes);
        if first_run {
            let welcome = app.new_note("");
            let doc = Format::Markdown.parse(&app.kit.get().schema, include_str!("welcome.md")).expect("the welcome note is Markdown");
            app.set_doc(&welcome, doc);
            app.update(&welcome, |m| m.title = "Welcome".into());
        }

        // Editors report changes through the relay (they may run off the
        // app's thread).
        let _ = spawn(async move {
            loop {
                for event in relay.next().await {
                    match event {
                        AppEvent::Changed => {
                            app.revision.update(|r| *r += 1);
                            app.text_changed.set(store::now_ms());
                        }
                        AppEvent::NewNote => {
                            app.new_note("");
                        }
                        AppEvent::Link => {}
                    }
                }
            }
        });
        // The clock, and saving what changed once it settles.
        let _ = set_interval(Duration::from_millis(300), move || {
            let now = store::now_ms();
            if (now / 1000.0).floor() != (app.now.get_untracked() / 1000.0).floor() {
                app.now.set(now);
            }
            let changed = app.text_changed.get_untracked();
            if changed > 0.0 && now - changed >= SAVE_AFTER {
                app.save_texts();
            }
            if app.index_dirty.get_untracked() {
                app.save_index();
            }
        });
        app
    }

    /// A note for `meta`, its text read from its file.
    fn open_note(self, meta: NoteMeta) -> Note {
        let kit = self.kit.get();
        let vault = self.vault.get();
        let file = vault.path_of(&meta);
        let doc = vault
            .read_body(&meta)
            .ok()
            .and_then(|html| Format::Html.parse(&kit.schema, &html).ok())
            .map(|doc| formats::images_to_absolute(&kit.schema, &doc, &file));
        let editor = kit.editor(doc);
        self.owner.run(|| Note { id: Rc::from(meta.id.as_str()), saved: signal(editor.doc()), meta: signal(meta), editor })
    }

    /// The running app, once it has started (`None` in tests).
    pub fn current() -> Option<Self> {
        CURRENT.with(|current| current.get())
    }

    // ----- Finding notes -----------------------------------------------------------

    pub fn note(self, id: &str) -> Option<Note> {
        self.notes.with_untracked(|notes| notes.iter().find(|n| &*n.id == id).cloned())
    }

    /// The notes floating on the desktop now (read reactively): not
    /// deleted, not asleep, and not hidden by "Hide All".
    pub fn desktop_notes(self) -> Vec<Note> {
        if !self.settings.with(|s| s.notes_shown) {
            return Vec::new();
        }
        let now = self.now.get();
        self.notes.with(|notes| {
            notes
                .iter()
                .filter(|n| {
                    n.meta.with(|m| {
                        m.on_desktop
                            && m.trashed.is_none()
                            && !m.asleep(now)
                            // A note pinned to a program shows with it.
                            && m.pinned_app.as_ref().is_none_or(|app| self.foreground_app.with(|front| front == app))
                    })
                })
                .cloned()
                .collect()
        })
    }

    /// Every category (read reactively).
    pub fn categories(self) -> Vec<String> {
        let index = self.index();
        self.vault.get().categories(&index)
    }

    /// Everything the index file holds, as it is now (read reactively).
    fn index(self) -> Index {
        Index { settings: self.settings.get(), notes: self.notes.with(|notes| notes.iter().map(|n| n.meta.get()).collect()) }
    }

    // ----- Changing notes ----------------------------------------------------------

    /// Changes what's saved about a note. Its file follows a new title or
    /// category.
    pub fn update(self, note: &Note, change: impl FnOnce(&mut NoteMeta)) {
        let before = note.meta.get_untracked();
        let mut meta = before.clone();
        change(&mut meta);
        if meta == before {
            return;
        }
        if (meta.title != before.title || meta.category != before.category) && meta.trashed.is_none() {
            // Its images are next to its file: write it first, so they're
            // found from the old place.
            self.save_text(note);
            let old_category = before.category.clone();
            if let Err(e) = self.vault.get().place(&mut meta) {
                self.problem.set(Some(format!("Couldn't move “{}”: {e}", meta.title)));
                meta.title = before.title.clone();
                meta.category = before.category.clone();
                meta.file = before.file.clone();
            }
            note.meta.set(meta);
            // The title is in the file, and the images go along.
            self.write_text(note);
            if old_category != note.meta.with_untracked(|m| m.category.clone()) {
                self.notes.update(|_| {});
            }
        } else {
            note.meta.set(meta);
        }
        self.index_dirty.set(true);
    }

    pub fn update_settings(self, change: impl FnOnce(&mut Settings)) {
        self.settings.update(change);
        self.index_dirty.set(true);
    }

    /// A new, empty note on the desktop, with the default look, placed so
    /// it doesn't cover the last one.
    pub fn new_note(self, category: &str) -> Note {
        let now = store::now_ms();
        let count = self.notes.with_untracked(|n| n.iter().filter(|n| n.meta.with_untracked(|m| m.trashed.is_none())).count());
        let step = (count % 8) as f32 * 28.0;
        let mut meta = NoteMeta {
            id: store::new_id(),
            title: "Untitled".into(),
            category: category.into(),
            created: now,
            modified: now,
            look: self.settings.with_untracked(|s| s.look_for(category)),
            rect: Rect { x: 160.0 + step, y: 140.0 + step, ..Rect::default() },
            ..NoteMeta::default()
        };
        meta.bubble.x = meta.rect.x;
        meta.bubble.y = meta.rect.y;
        self.add(meta, None)
    }

    /// A copy of `note` beside it.
    pub fn duplicate(self, note: &Note) -> Note {
        let now = store::now_ms();
        let mut meta = note.meta.get_untracked();
        meta.id = store::new_id();
        meta.file = String::new();
        meta.title = format!("{} copy", meta.title);
        meta.created = now;
        meta.modified = now;
        meta.rect.x += 28.0;
        meta.rect.y += 28.0;
        meta.bubble.x += 28.0;
        meta.bubble.y += 28.0;
        meta.alarm = None;
        meta.sleep_until = None;
        self.add(meta, Some(note.editor.doc()))
    }

    /// The templates: the notes in the "Templates" category (read
    /// reactively), by title.
    pub fn templates(self) -> Vec<Note> {
        let mut found: Vec<Note> = self.notes.with(|notes| {
            notes.iter().filter(|n| n.meta.with(|m| m.trashed.is_none() && m.category == crate::model::TEMPLATES)).cloned().collect()
        });
        found.sort_by_key(|n| n.meta.with(|m| m.title.to_lowercase()));
        found
    }

    /// Keeps a copy of `note` as a template (off the desktop).
    pub fn save_as_template(self, note: &Note) -> Note {
        let now = store::now_ms();
        let mut meta = note.meta.get_untracked();
        meta.id = store::new_id();
        meta.file = String::new();
        meta.category = crate::model::TEMPLATES.into();
        meta.created = now;
        meta.modified = now;
        meta.on_desktop = false;
        meta.alarm = None;
        meta.sleep_until = None;
        meta.pinned_app = None;
        self.add(meta, Some(note.editor.doc()))
    }

    /// A new note on the desktop that starts as a copy of `template`: its
    /// text, look and size.
    pub fn new_from_template(self, template: &Note) -> Note {
        let fresh = self.new_note("");
        let from = template.meta.get_untracked();
        self.set_doc(&fresh, template.editor.doc());
        self.update(&fresh, |m| {
            m.title = from.title.clone();
            m.look = from.look.clone();
            m.rect.w = from.rect.w;
            m.rect.h = from.rect.h;
            m.bubble = crate::model::Bubble { x: m.bubble.x, y: m.bubble.y, ..from.bubble.clone() };
        });
        fresh
    }

    /// Adds a note for `meta` (which needs an id but no file yet), with
    /// `doc` as its text.
    pub fn add(self, mut meta: NoteMeta, doc: Option<Node>) -> Note {
        if let Err(e) = self.vault.get().place(&mut meta) {
            self.problem.set(Some(format!("Couldn't make the note's file: {e}")));
        }
        let editor = self.kit.get().editor(doc);
        let note = self.owner.run(|| Note { id: Rc::from(meta.id.as_str()), saved: signal(editor.doc()), meta: signal(meta), editor });
        self.write_text(&note);
        self.notes.update(|notes| notes.push(note.clone()));
        self.index_dirty.set(true);
        note
    }

    /// Replaces a note's text.
    pub fn set_doc(self, note: &Note, doc: Node) {
        let state = note.editor.state();
        let mut tr = state.tr();
        let size = state.doc().content().size();
        if tr.replace_with(0, size, doc.content().clone()).is_ok() {
            let _ = note.editor.session.dispatch(tr);
        }
    }

    /// When the note's earlier versions are from, newest first.
    pub fn versions(self, note: &Note) -> Vec<f64> {
        self.vault.get().versions(&note.id)
    }

    /// Puts back the note's text as it was at `at` (one of `versions`).
    /// The text it has now is kept as a version first.
    pub fn restore_version(self, note: &Note, at: f64) {
        self.save_text(note);
        let (vault, kit) = (self.vault.get(), self.kit.get());
        let meta = note.meta.get_untracked();
        let restored = vault.read_version(&note.id, at).map_err(|e| e.to_string()).and_then(|html| Format::Html.parse(&kit.schema, &html));
        match restored {
            Ok(doc) => {
                if let Ok(now) = vault.read_body(&meta) {
                    let _ = vault.keep_version(&note.id, meta.modified, &now, 0.0);
                }
                // Its images are where the note's file has them.
                self.set_doc(note, formats::images_to_absolute(&kit.schema, &doc, &vault.path_of(&meta)));
            }
            Err(e) => self.problem.set(Some(format!("Couldn't read the earlier version of “{}”: {e}", meta.title))),
        }
    }

    /// Moves a note to the trash (it can be restored for a while).
    pub fn trash(self, note: &Note) {
        self.save_text(note);
        let mut meta = note.meta.get_untracked();
        if let Err(e) = self.vault.get().trash(&mut meta) {
            return self.problem.set(Some(format!("Couldn't delete “{}”: {e}", meta.title)));
        }
        meta.alarm = None;
        meta.sleep_until = None;
        note.meta.set(meta);
        if self.config_for.get_untracked().as_deref() == Some(&*note.id) {
            self.config_for.set(None);
        }
        self.notes.update(|_| {});
        self.index_dirty.set(true);
    }

    pub fn restore(self, note: &Note) {
        let mut meta = note.meta.get_untracked();
        if let Err(e) = self.vault.get().restore(&mut meta) {
            return self.problem.set(Some(format!("Couldn't restore “{}”: {e}", meta.title)));
        }
        note.meta.set(meta);
        self.notes.update(|_| {});
        self.index_dirty.set(true);
    }

    /// Deletes a note for good.
    pub fn delete_forever(self, note: &Note) {
        let meta = note.meta.get_untracked();
        if let Err(e) = self.vault.get().delete_forever(&meta) {
            return self.problem.set(Some(format!("Couldn't delete “{}”: {e}", meta.title)));
        }
        self.notes.update(|notes| notes.retain(|n| n.id != note.id));
        if self.selected.get_untracked().as_deref() == Some(&*note.id) {
            self.selected.set(None);
        }
        self.index_dirty.set(true);
    }

    /// Removes notes that have been in the trash longer than the settings
    /// allow.
    pub fn empty_old_trash(self) {
        let limit = self.settings.with_untracked(|s| s.trash_days) as f64 * 86_400_000.0;
        let now = store::now_ms();
        let old: Vec<Note> = self.notes.with_untracked(|notes| {
            notes.iter().filter(|n| n.meta.with_untracked(|m| m.trashed.is_some_and(|at| now - at > limit))).cloned().collect()
        });
        for note in old {
            self.delete_forever(&note);
        }
    }

    /// Shows the note on the desktop (waking it) and brings it forward.
    pub fn show_on_desktop(self, note: &Note) {
        self.update(note, |m| {
            m.on_desktop = true;
            m.sleep_until = None;
        });
        if !self.settings.with_untracked(|s| s.notes_shown) {
            self.update_settings(|s| s.notes_shown = true);
        }
    }

    // ----- Saving ------------------------------------------------------------------

    /// Writes every note whose text changed.
    fn save_texts(self) {
        self.text_changed.set(0.0);
        for note in self.notes.get_untracked() {
            self.save_text(&note);
        }
    }

    /// Writes the note when its text differs from its file.
    pub(crate) fn save_text(self, note: &Note) {
        let changed = !note.saved.with_untracked(|saved| formats::same_content(&note.editor.doc(), saved));
        if changed && note.meta.with_untracked(|m| m.trashed.is_none()) {
            self.write_text(note);
            note.meta.update(|m| m.modified = store::now_ms());
            self.index_dirty.set(true);
        }
    }

    /// Writes the note's file, its images copied next to it.
    fn write_text(self, note: &Note) {
        let meta = note.meta.get_untracked();
        let vault = self.vault.get();
        let file = vault.path_of(&meta);
        let editor = &note.editor;
        let result = formats::images_for_saving(&editor.kit.schema, &editor.doc(), &file).and_then(|(doc, moved)| {
            let html = formats::html_fragment(&doc)?;
            vault.write_body(&meta, &html).map_err(|e| e.to_string())?;
            Ok(moved)
        });
        match result {
            Ok(moved) => {
                // The open note follows the images that moved, so the next
                // save doesn't copy them again.
                follow_moved_images(editor, &moved);
                note.saved.set(editor.doc());
            }
            Err(e) => self.problem.set(Some(format!("Couldn't save “{}”: {e}", meta.title))),
        }
    }

    fn save_index(self) {
        self.index_dirty.set(false);
        let index = untracked(|| self.index());
        if let Err(e) = self.vault.get().save_index(&index) {
            self.problem.set(Some(format!("Couldn't save the notes' settings: {e}")));
        }
    }

    /// Writes everything that's waiting, now (before quitting).
    pub fn save_now(self) {
        self.save_texts();
        self.save_index();
    }
}

/// Points the open document's images at their copies (one step, outside
/// the undo history's way).
fn follow_moved_images(editor: &Editor, moved: &[(String, String)]) {
    if moved.is_empty() {
        return;
    }
    let state = editor.state();
    let mut tr = state.tr();
    state.doc().descendants(&mut |node, pos, _, _| {
        if node.ty().name() == "image" {
            let src = node.attrs().get("src").and_then(Value::as_str).unwrap_or_default();
            if let Some((_, new)) = moved.iter().find(|(old, _)| old == src) {
                let _ = tr.set_node_attribute(pos, "src", Value::String(new.clone()));
            }
        }
        true
    });
    let _ = editor.session.dispatch(tr);
}
