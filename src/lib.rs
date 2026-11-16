//! Bubble Notes: floating notes for the desktop, built with NSH (nsh.dev),
//! all Rust.
//!
//! The app lives in the tray. Each note on the desktop is a small frameless
//! window of its own; the Notes Manager lists every note by category. Notes
//! are HTML files in a folder (`Documents/Bubble Notes`), saved as you type.
//!
//! The app's main window never shows: it only hosts the tray icon, the
//! menus and the other windows (closing it would end the app).
//!
//! Modules:
//! - `model`: what's saved about a note and the settings (plain data)
//! - `store`: the notes folder on disk
//! - `app`: the shared state and what changes it (`App`, `Note`)
//! - `note_window`: a note floating on the desktop; `bubble`: a note as a bubble
//! - `note_menu`: a note's right-click menu
//! - `config_panel`: "Edit configuration", beside a note
//! - `manager`: the Notes Manager window
//! - `tray`: the tray icon and its menu; `options`: the Options window;
//!   `quick_search`: the search box a global shortcut opens
//! - `alarms`: sleep and alarms; `system`: global shortcuts, start with Windows
//! - `editor_view`: a note's text as an editor element; `media`: pasted images, YouTube cards
//! - `kit`, `actions`, `toggle`, `slash`, `formats`, `ui`: the editor itself
//!   (schema, actions, toggles, slash menu, file formats, menus' look)
//! - `links`: links between notes
//! - `drag`, `layout`, `win`: moving windows by hand; snapping and rescue;
//!   what's asked of Windows directly

// NSH's `spawn` hands back a handle that's also a future; dropping it
// doesn't cancel the task (it runs under its owner until done), which is
// how this app uses it.
#![allow(clippy::let_underscore_future)]

mod actions;
mod alarms;
mod app;
mod bubble;
mod config_panel;
mod drag;
mod due;
mod editor_view;
mod formats;
mod kit;
mod layout;
mod links;
mod manager;
mod media;
mod model;
mod note_menu;
mod note_window;
mod options;
mod print;
mod quick_search;
mod slash;
mod store;
mod system;
mod toggle;
mod tray;
mod ui;
mod win;

use nsh_core::desktop::WindowAttributes;
use nsh_core::element::desktop::{main_window, on_app_quit};
use nsh_core::prelude::*;

/// The app's UI set: what a TSX entry could import from `bubblenotes/native`.
#[native_module("bubblenotes/native")]
fn native(registry: &mut Registry) {
    registry.register::<BubbleNotes>();
}

/// The root: the hidden main window's content, which is every other window.
#[component]
pub fn BubbleNotes() -> impl IntoElement {
    let app = app::App::start();
    app.empty_old_trash();
    tray::install(app);
    alarms::install(app);
    system::install(app);
    effect(move || drag::set_snap(app.settings.with(|s| s.snap_to_edges)));
    effect(move || app.rescue_notes());
    // The program in front, for the notes pinned to one.
    let _ = set_interval(std::time::Duration::from_millis(400), move || {
        if let Some(front) = win::foreground_app()
            && app.foreground_app.with_untracked(|was| *was != front)
        {
            app.foreground_app.set(front);
        }
    });
    // Holding Ctrl+Alt makes ghost notes solid for the while.
    let _ = set_interval(std::time::Duration::from_millis(120), move || {
        let any = app.notes.with_untracked(|notes| notes.iter().any(|n| n.meta.with_untracked(|m| m.ghost)));
        let held = any && win::ctrl_alt_down();
        if app.ghosts_solid.get_untracked() != held {
            app.ghosts_solid.set(held);
        }
    });
    // For development: BUBBLENOTES_OPEN=manager,options,config,search opens those
    // windows at start.
    if let Ok(open) = std::env::var("BUBBLENOTES_OPEN") {
        app.manager_open.set(open.contains("manager"));
        app.options_open.set(open.contains("options"));
        app.search_open.set(open.contains("search"));
        if open.contains("config") {
            app.config_for.set(app.notes.with_untracked(|n| n.first().map(|n| n.id.clone())));
        }
    }
    // For development: BUBBLENOTES_DEV=select|slash|back|autosize|unghost does that
    // to the first note on the desktop two seconds after start.
    if let Ok(dev) = std::env::var("BUBBLENOTES_DEV") {
        let _ = set_timeout(std::time::Duration::from_secs(2), move || {
            let notes = untracked(|| app.desktop_notes());
            // "back" sends the note in front; the others use the first.
            let Some(note) = (if dev == "back" { notes.last() } else { notes.first() }).cloned() else { return };
            match dev.as_str() {
                "select" => {
                    note.editor.run("selectAll", serde_json::json!({}));
                }
                "slash" => {
                    note.editor.run("insertContent", serde_json::json!({ "content": "/" }));
                }
                "back" => app.command.set(Some((note.id.clone(), app::NoteCommand::SendToBack))),
                // "config": the note's configuration panel, as its menu opens it.
                "config" => app.config_for.set(Some(note.id.clone())),
                // "configs": the panel opened, shut, opened again, then
                // moved to the next note, two seconds apart.
                "configs" => {
                    let ids = [Some(note.id.clone()), None, Some(note.id.clone()), notes.get(1).map(|n| n.id.clone())];
                    for (i, id) in ids.into_iter().enumerate() {
                        let _ = set_timeout(std::time::Duration::from_secs(2 * i as u64), move || app.config_for.set(id.clone()));
                    }
                }
                "autosize" => app.command.set(Some((note.id.clone(), app::NoteCommand::AutoSize))),
                // "newin:Work/Ideas": a new note in that category.
                dev if dev.starts_with("newin:") => {
                    app.new_note(&dev["newin:".len()..]);
                }
                // "template": a template from the first note, then a note from it.
                "template" => {
                    let template = app.save_as_template(&note);
                    app.new_from_template(&template);
                }
                // "version": the text as it was at the newest earlier version.
                "version" => {
                    if let Some(at) = app.versions(&note).first() {
                        app.restore_version(&note, *at);
                    }
                }
                "print" => print::print(app, &note),
                // "pdf:C:\out.pdf": the note saved there as a PDF.
                dev if dev.starts_with("pdf:") => print::save_pdf_to(app, note.clone(), dev["pdf:".len()..].into()),
                "unghost" => {
                    for note in app.notes.get_untracked() {
                        app.update(&note, |m| m.ghost = false);
                    }
                }
                _ => {}
            }
        });
    }

    // The main window stays hidden: closing it would end the app.
    let main = main_window();
    let mut attributes = WindowAttributes::default();
    attributes.close_on_press = Some(false);
    main.set(attributes);
    let hide = main.clone();
    main.on_close_press(move |_| hide.hide());
    // Starting the app again brings this (empty) window forward: show the
    // Notes Manager instead.
    let again = main.clone();
    let _ = spawn(async move {
        loop {
            next_second_instance().await;
            again.hide();
            app.manager_open.set(true);
        }
    });
    on_app_quit(move || {
        app.save_now();
        async {}
    });

    view()
        .child(each(move || app.desktop_notes(), |note| note.id.clone(), move |note, _| note_window::note_window(app, note.get_untracked())))
        .child(manager::manager_window(app))
        .child(config_panel::config_window(app))
        .child(options::options_window(app))
        .child(quick_search::search_window(app))
        .child(note_menu::windows(app))
}
