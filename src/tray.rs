//! The tray icon: a click shows or hides the notes, a right click opens
//! the app's menu.

use nsh_core::element::ImageSrc;
use nsh_core::element::desktop::{MenuItem, quit, set_tray, tray};
use nsh_core::prelude::*;

use crate::app::App;

/// The app's icon: a glossy pink bubble with a speech mark.
pub const ICON_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">
<defs><radialGradient id="g" cx="0.35" cy="0.3" r="0.8"><stop offset="0" stop-color="#ff9dbd"/><stop offset="0.6" stop-color="#e0457b"/><stop offset="1" stop-color="#a81f52"/></radialGradient></defs>
<circle cx="32" cy="32" r="29" fill="url(#g)"/>
<path d="M32 18c-9.4 0-17 5.8-17 13 0 4 2.3 7.500 6 9.900-.3 2.400-1.300 4.600-3 6.100 3.700-.2 6.900-1.500 9.300-3.500 1.500.3 3.100.5 4.700.5 9.400 0 17-5.800 17-13s-7.600-13-17-13z" fill="none" stroke="#fff" stroke-width="3.500" stroke-linejoin="round"/>
</svg>"##;

/// Shows the tray icon and keeps its menu's check marks up to date.
pub fn install(app: App) {
    // Whether any note is a ghost, set only when that changes (the menu
    // isn't made again for every other change of a note).
    let any_ghost = signal(false);
    effect(move || {
        let now = app.notes.with(|notes| notes.iter().any(|n| n.meta.with(|m| m.ghost)));
        if any_ghost.get_untracked() != now {
            any_ghost.set(now);
        }
    });
    // The templates (ids and titles), likewise set only when they change.
    let templates = signal(Vec::<(String, String)>::new());
    effect(move || {
        let now: Vec<(String, String)> = app.templates().iter().map(|t| (t.id.to_string(), t.meta.with(|m| m.title.clone()))).collect();
        if templates.with_untracked(|was| *was != now) {
            templates.set(now);
        }
    });
    effect(move || {
        let shown = app.settings.with(|s| s.notes_shown);
        let ghosts = any_ghost.get();
        let from_templates: Vec<MenuItem> = templates
            .get()
            .into_iter()
            .map(|(id, title)| {
                MenuItem::new(title).on_select(move || {
                    if let Some(template) = app.note(&id) {
                        let fresh = app.new_from_template(&template);
                        app.command.set(Some((fresh.id.clone(), crate::app::NoteCommand::Focus)));
                    }
                })
            })
            .collect();
        let no_templates = from_templates.is_empty();
        set_tray(
            tray()
                .icon(ImageSrc::svg_markup(ICON_SVG))
                .tooltip("Bubble Notes")
                .on_click(move |_| app.update_settings(|s| s.notes_shown = !s.notes_shown))
                .menu([
                    MenuItem::new("New Note").on_select(move || {
                        let note = app.new_note("");
                        app.show_on_desktop(&note);
                    }),
                    MenuItem::submenu("New from Template", from_templates).disabled(no_templates),
                    MenuItem::new("Search Notes…").on_select(move || app.search_open.set(true)),
                    MenuItem::separator(),
                    MenuItem::new("Show All").checked(shown).on_select(move || app.show_all()),
                    MenuItem::new("Hide All").on_select(move || app.update_settings(|s| s.notes_shown = false)),
                    MenuItem::new("Make Ghost Notes Solid").disabled(!ghosts).on_select(move || {
                        for note in app.notes.get_untracked() {
                            app.update(&note, |m| m.ghost = false);
                        }
                    }),
                    MenuItem::separator(),
                    MenuItem::new("Notes Manager").on_select(move || app.manager_open.set(true)),
                    MenuItem::new("Options…").on_select(move || app.options_open.set(true)),
                    MenuItem::separator(),
                    MenuItem::new("Exit").on_select(move || {
                        app.save_now();
                        quit();
                    }),
                ]),
        );
    });
}

impl App {
    /// Shows every note on the desktop, the hidden and sleeping ones too.
    pub fn show_all(self) {
        for note in self.notes.get_untracked() {
            if note.meta.with_untracked(|m| m.trashed.is_none() && (!m.on_desktop || m.sleep_until.is_some())) {
                self.update(&note, |m| {
                    m.on_desktop = true;
                    m.sleep_until = None;
                });
            }
        }
        self.update_settings(|s| s.notes_shown = true);
    }
}
