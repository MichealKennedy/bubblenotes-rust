//! A note's text as an editor element: the rich editor with the app's
//! look, the selection menu and the slash menu. The floating note and the
//! Notes Manager both show a note through this, over the note's one
//! session, so they always show the same text.
//!
//! Each element also takes images pasted or dropped on it (into the note's
//! `attachments/` folder), and the shortcuts only the focused element can
//! answer (Ctrl/⌘+K edits the link, Ctrl/⌘+D duplicates the note).

pub(crate) use crate::media;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use nsh_base::rich_editor::view::{RichEditorChange, RichEditorFile, RichEditorPastedFiles, RichEditorSelection, rich_editor};
use nsh_core::file_dialog::{FileFilter, OpenDialog};
use nsh_core::prelude::*;
use serde_json::json;

use crate::actions::AppAction;
use crate::app::{App, Note};
use crate::kit::Request;
use crate::ui::{self, Ctx, TextLook};

/// What an editor element needs besides its `Ctx`: the app and the note.
/// Found from the ctx's own `selection` signal, which no other ctx shares.
#[derive(Clone)]
struct Owned {
    selection: Signal<Option<RichEditorSelection>>,
    app: App,
    note: Note,
}

thread_local! {
    /// The live ctxs' notes.
    static OWNED: RefCell<Vec<Owned>> = const { RefCell::new(Vec::new()) };
}

fn owned(ctx: Ctx) -> Option<Owned> {
    OWNED.with(|o| o.borrow().iter().find(|o| o.selection == ctx.selection).cloned())
}

/// The editing state of one editor element (one window's view of a note).
pub fn ctx_for(app: App, note: &Note) -> Ctx {
    let editor = signal(note.editor.clone());
    let selection = signal(None::<RichEditorSelection>);
    let status = signal(String::new());
    let editing_link = signal(false);
    OWNED.with(|o| o.borrow_mut().push(Owned { selection, app, note: note.clone() }));
    cleanup(move || OWNED.with(|o| o.borrow_mut().retain(|o| o.selection != selection)));
    let edit_link = move || {
        // In a link (a caret or part of it) the whole link is edited; with
        // nothing selected, new linked text is put in.
        editor.with_untracked(|e| {
            if e.is_active("link", None) {
                e.run("extendMarkRange", json!({ "name": "link" }));
            }
        });
        editing_link.set(true);
    };
    let owner = app.owner;
    let insert_image = move || {
        owner.run(|| {
            let picked = pick_paths(
                OpenDialog::files().title("Insert image").filter(FileFilter::new("Images", ["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp"])),
            );
            let _ = spawn(async move {
                let Ok(Some(paths)) = picked.await else { return };
                let Some(image) = paths.into_iter().next() else { return };
                editor.with_untracked(|e| media::insert_images(e, &[image]));
            });
        });
    };
    let revision = app.revision;
    let paste_plain = move || {
        owner.run(|| {
            let _ = spawn(async move {
                let text = clipboard_string().await;
                if text.is_empty() {
                    return;
                }
                let pasted = editor.with_untracked(|e| e.run_command(&crate::blocks::paste_text(&text)));
                if pasted {
                    revision.update(|r| *r += 1);
                }
            });
        });
    };
    let actions: Rc<dyn Fn(AppAction)> = Rc::new(move |action| match action {
        AppAction::InsertImage => insert_image(),
        AppAction::EditLink => edit_link(),
        AppAction::PastePlain => paste_plain(),
        // With a caret, the link editor also asks for the text.
        AppAction::InsertLink => edit_link(),
        AppAction::TextSize => style_menu(editor, revision, "fontSize", crate::actions::SIZES),
        AppAction::Font => style_menu(editor, revision, "fontFamily", crate::actions::FONTS),
    });
    let color_panel = signal(ui::ColorPanel::Closed);
    Ctx { editor, read_only: signal(false), revision: app.revision, selection, status, editing_link, color_panel, app: stored(actions) }
}

/// A native menu of `choices` for the text style's `key` at the selection,
/// the current one checked.
fn style_menu(editor: Signal<crate::kit::Editor>, revision: Signal<u64>, key: &'static str, choices: &'static [(&'static str, Option<&'static str>)]) {
    let current = editor.with_untracked(|e| crate::actions::style_of(e, key));
    let items = choices
        .iter()
        .map(|(name, value)| {
            nsh_core::element::desktop::MenuItem::new(*name).checked(current.as_deref() == *value).on_select(move || {
                editor.with_untracked(|e| crate::actions::set_style(e, key, *value));
                revision.update(|r| *r += 1);
            })
        })
        .collect();
    let _ = nsh_core::element::desktop::popup_menu(items);
}

/// Inserts the images among pasted or dropped files at the selection:
/// files with their bytes are saved into the note's `attachments/` folder
/// first; files on disk are inserted by path (saving the note copies them
/// there).
fn insert_pasted(ctx: Ctx, files: &[RichEditorFile], uris: &[String]) {
    let Some(Owned { app, note, .. }) = owned(ctx) else { return };
    let meta = note.meta.get_untracked();
    if meta.trashed.is_some() || meta.locked {
        return;
    }
    // Where the note's file is now (its title or category may have moved it).
    let file = app.vault.get().path_of(&meta);
    let Some(dir) = file.parent() else { return };
    let stamp = media::stamp_now();
    let mut images: Vec<PathBuf> = Vec::new();
    for f in files {
        let Some(ext) = media::image_extension(&f.name, &f.mime) else { continue };
        match media::save_pasted(dir, &stamp, ext, &f.data) {
            Ok(path) => images.push(path),
            Err(e) => app.problem.set(Some(format!("Couldn't paste the image: {e}"))),
        }
    }
    images.extend(
        uris.iter()
            .filter_map(|uri| media::path_from_uri(uri))
            .filter(|path| media::image_extension(&path.to_string_lossy(), "").is_some()),
    );
    if ctx.editor.with_untracked(|e| media::insert_images(e, &images)) {
        ctx.changed();
    }
}

/// The editor element for `ctx`'s note, filling its parent. `editable`
/// is false for a locked note; `look` is how its text is drawn (the
/// element is made again when it changes).
pub fn note_editor(
    ctx: Ctx,
    editable: impl Fn() -> bool + Clone + 'static,
    placeholder: &'static str,
    look: impl Fn() -> TextLook + 'static,
) -> impl IntoElement {
    // A locked note shows no formatting menus.
    let locked = editable.clone();
    effect(move || {
        let read_only = !locked();
        if ctx.read_only.get_untracked() != read_only {
            ctx.read_only.set(read_only);
        }
    });
    // Only a real change of the look makes the element again.
    let current = signal(untracked(&look));
    effect(move || {
        let now = look();
        if current.with_untracked(|c| *c != now) {
            current.set(now);
        }
    });

    // The shortcuts the keymap left for the focused element.
    let focused = signal(false);
    effect(move || {
        ctx.revision.get();
        if !focused.get_untracked() {
            return;
        }
        if Request::Link.take() {
            untracked(|| (ctx.app.get())(AppAction::EditLink));
        }
        if Request::Duplicate.take()
            && let Some(Owned { app, note, .. }) = owned(ctx)
        {
            untracked(|| app.duplicate(&note));
        }
    });

    // The slash menu places itself in window coordinates (the caret's), so
    // it goes in a layer whose origin is the window's: this element's
    // place in the window, taken back.
    let frame = element_ref();
    let origin = signal((0.0_f32, 0.0_f32));
    let height = signal(0.0_f32);
    let window_height = signal(f32::MAX);
    let width = signal(0.0_f32);
    // Images fit inside the text's side margins.
    let text_width = signal(0.0_f32);
    let owner = Owner::current();
    let measure = move |bounds: &Bounds<Points>| {
        height.set(bounds.size.height.0);
        set_changed(width, bounds.size.width.0);
        set_changed(text_width, bounds.size.width.0 - 16.0);
        owner.run(|| {
            let _ = spawn(async move {
                if let Some(m) = frame.measure().await {
                    let (page, bottom) = ((m.page.x.0, m.page.y.0), m.page.y.0 + height.get_untracked());
                    set_changed(origin, page);
                    // This element reaches the window's bottom (the note's
                    // card): the menu stays above it.
                    set_changed(window_height, bottom);
                }
            });
        });
    };

    view()
        .flex_1()
        // Its text scrolls inside it; a long note never grows the window.
        .min_h(px(0.0))
        .element_ref(&frame)
        .on_layout(measure)
        .on_file_drop(move |e: &FileDropEvent| insert_pasted(ctx, &[], &e.uris))
        .child(dynamic(move || {
            let editable = editable.clone();
            provide_context(media::TextWidth(text_width));
            rich_editor()
                .editor(ctx.editor.get_untracked().session)
                .classes(ui::editor_classes(&current.get()))
                .placeholder(placeholder)
                .editable(editable)
                .bubble_menu(ui::selection_menu(ctx))
                .on_change(move |_: &RichEditorChange| ctx.changed())
                .on_selection_change(move |s: &RichEditorSelection| {
                    ctx.selection.set(Some(s.clone()));
                    ctx.changed();
                })
                .on_paste_files(move |p: &RichEditorPastedFiles| insert_pasted(ctx, &p.files, &p.uris))
                .on_focus(move || {
                    // A request left by an editor that's gone is stale.
                    Request::Link.take();
                    Request::Duplicate.take();
                    focused.set(true);
                })
                .on_blur(move || focused.set(false))
                .flex_1()
                .min_h(px(0.0))
        }))
        .child(
            view()
                .absolute()
                .left(move || px(-origin.get().0))
                .top(move || px(-origin.get().1))
                .w(move || px(origin.get().0 + width.get()))
                .h(move || px(if window_height.get() == f32::MAX { 0.0 } else { window_height.get() }))
                .pointer_events(PointerEvents::BoxNone)
                .child(ui::slash_menu(ctx, window_height))
                .child(crate::due::menu(ctx, window_height))
                .children(owned(ctx).map(|o| crate::links::menu(ctx, o.app, window_height).into_element()))
                .child(crate::emoji::menu(ctx, window_height)),
        )
}

/// Sets `signal` only to a new value (what reads it isn't run again for
/// the same one).
fn set_changed<T: PartialEq + 'static>(signal: Signal<T>, value: T) {
    if signal.with_untracked(|v| *v != value) {
        signal.set(value);
    }
}
