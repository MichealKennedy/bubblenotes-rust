//! Rich Notes: a WYSIWYG rich-text note editor built with NSH (nsh.dev), all
//! Rust.
//!
//! You edit formatted text directly. `/` opens a menu of blocks; selecting
//! text opens a menu of formats beside it; Markdown shortcuts format as you
//! type (`## `, `**bold**`, `- `, `[ ] `). Notes are Markdown (`.md`) or
//! HTML (`.html`), by extension; saving as Markdown asks first when the note
//! has formatting Markdown can't hold. Images are copied into `attachments/`
//! next to the note.
//!
//! Modules:
//! - `kit`: the editor setup, built once, and one open document (`Editor`)
//! - `actions`: every editing action, once; the menus list them by id
//! - `toggle`: collapsible toggle blocks
//! - `slash`: the slash menu's plugin
//! - `formats`: Markdown, HTML, text; what Markdown loses; image paths
//! - `files`: new, open, save, insert image
//! - `ui`: toolbar, slash menu, selection menu, buttons, the document's look

// NSH's `spawn` hands back a handle that's also a future; dropping it
// doesn't cancel the task (it runs under its owner until done), which is
// how this app uses it.
#![allow(clippy::let_underscore_future)]

mod actions;
mod files;
mod formats;
mod kit;
mod slash;
mod toggle;
mod ui;

use std::rc::Rc;

use nsh_base::rich_editor::view::{RichEditorChange, RichEditorSelection, rich_editor};
use nsh_base::theme;
use nsh_core::desktop::WindowAttributes;
use nsh_core::element::desktop::current_window;
use nsh_core::prelude::*;

use crate::actions::AppAction;
use crate::files::Files;
use crate::kit::{AppEvent, Kit, Relay};
use crate::ui::Ctx;

/// The app's UI set: what a TSX entry could import from `richnotes/native`.
#[native_module("richnotes/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}

/// The first note: a short tour, in Markdown.
const WELCOME: &str = include_str!("welcome.md");

#[component]
pub fn App() -> impl IntoElement {
    let relay = Relay::default();
    let kit = Kit::new(relay.clone());
    let first = formats::Format::Markdown.parse(&kit.schema, WELCOME).expect("the welcome note is Markdown");
    let editor = signal(kit.editor(Some(first)));
    let kit = stored(kit);

    let revision = signal(0_u64);
    let selection = signal(None::<RichEditorSelection>);
    let status = signal(String::from("Type / for blocks, select text for formats"));
    let editing_link = signal(false);
    // Set below, once `files` exists: what actions needing the app's UI do.
    let app: Stored<Rc<dyn Fn(AppAction)>> = stored(Rc::new(|_| {}) as Rc<dyn Fn(AppAction)>);
    let ctx = Ctx { editor, revision, selection, status, editing_link, app };

    let files = Files {
        ctx,
        kit,
        saved: signal(editor.with_untracked(|e| e.doc())),
        path: signal(None),
        owner: Owner::current(),
    };
    let edit_link = move || {
        // A caret in a link edits the whole link; with nothing selected
        // there's nothing to link.
        editor.with_untracked(|e| {
            if e.state().selection().empty() && e.is_active("link", None) {
                e.run("extendMarkRange", serde_json::json!({ "typeOrName": "link" }));
            }
        });
        if editor.with_untracked(|e| e.state().selection().empty()) {
            status.set("Select the text to link first".into());
        } else {
            editing_link.set(true);
        }
    };
    app.replace(Rc::new(move |action| match action {
        AppAction::InsertImage => files.insert_image(),
        AppAction::EditLink => edit_link(),
    }));

    // The window's title follows the note's name and unsaved changes.
    let window = current_window();
    effect(move || {
        let name = files.path.get().as_deref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
        let mark = if files.dirty() { "• " } else { "" };
        let mut attributes = WindowAttributes::default();
        attributes.title = Some(format!("{mark}{} — Rich Notes", name.as_deref().unwrap_or("Untitled")));
        window.set(attributes);
    });

    // File shortcuts: as window shortcuts while the editor doesn't have the
    // focus, and through the editor's keymap (the relay) while it does.
    let always = || KeyOptions { when: nsh_core::keymap::When::Always, ..Default::default() };
    on_key_with("mod+n", always(), move |_| files.new_file());
    on_key_with("mod+o", always(), move |_| files.open());
    on_key_with("mod+s", always(), move |_| files.save());
    let _ = spawn(async move {
        loop {
            for event in relay.next().await {
                match event {
                    AppEvent::New => files.new_file(),
                    AppEvent::Open => files.open(),
                    AppEvent::Save => files.save(),
                    AppEvent::Link => edit_link(),
                    AppEvent::Changed => ctx.changed(),
                }
            }
        }
    });

    let window_height = signal(f32::MAX);
    let file_button = |label: &'static str, press: Rc<dyn Fn()>| {
        ui::button(label, label, || false, || true, move || press()).into_element()
    };

    view()
        .flex_1()
        .on_layout(move |bounds: &Bounds<Points>| window_height.set(bounds.size.height.0))
        .bg(move || theme().background)
        .text_color(move || theme().foreground)
        .child(ui::toolbar(
            ctx,
            vec![
                file_button("New", Rc::new(move || files.new_file())),
                file_button("Open…", Rc::new(move || files.open())),
                file_button("Save", Rc::new(move || files.save())),
                file_button("Save As…", Rc::new(move || files.save_as("md"))),
            ],
        ))
        // The page. A new note gets a new view (`dynamic` makes it again
        // when `editor` changes).
        .child(
            view().flex_1().items_center().child(view().flex_1().w_full().max_w(px(820.0)).px_6().py_4().child(dynamic(
                move || {
                    rich_editor()
                        .editor(editor.get().session)
                        .classes(ui::editor_classes())
                        .placeholder("Type / for blocks…")
                        .auto_focus(true)
                        .bubble_menu(ui::selection_menu(ctx))
                        .on_change(move |_: &RichEditorChange| ctx.changed())
                        .on_selection_change(move |s: &RichEditorSelection| {
                            selection.set(Some(s.clone()));
                            ctx.changed();
                        })
                        .flex_1()
                },
            ))),
        )
        .child(ui::slash_menu(ctx, window_height))
        // Status bar.
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .px_3()
                .py_1()
                .border_t_1()
                .border_color(move || theme().border)
                .bg(move || theme().muted)
                .child(text(move || status.get()).text_size(px(12.0)).text_color(move || theme().muted_foreground))
                .child(
                    text(move || {
                        revision.get();
                        let (words, chars) = editor.with(|e| {
                            let text = e.doc().text_content();
                            (text.split_whitespace().count(), text.chars().count())
                        });
                        let format = files.path.get().as_deref().map(formats::Format::of).unwrap_or(formats::Format::Markdown);
                        let mark = if files.dirty() { "  ·  Unsaved" } else { "" };
                        format!("{words} words  ·  {chars} chars  ·  {}{mark}", format.name())
                    })
                    .text_size(px(12.0))
                    .text_color(move || theme().muted_foreground),
                ),
        )
}
