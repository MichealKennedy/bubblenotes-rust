//! The quick search box: a small window in the middle of the screen, opened
//! by a global shortcut, to find a note by its title or text and bring it
//! up. Type, ↑ ↓ to choose, Enter to open; Escape or a click elsewhere
//! closes it.

use nsh_base::theme;
use nsh_core::desktop::{WindowAttributes, WindowKind, WindowLevel, WindowPosition};
use nsh_core::element::desktop::{Window, window};
use nsh_core::prelude::*;

use crate::app::{App, Note, NoteCommand};
use crate::manager::{find_folded, snippet, snippet_start};

/// The most notes the box lists.
const MOST: usize = 8;

/// A found note: its title, where it is, and a bit of its text.
#[derive(Clone)]
struct Found {
    note: Note,
    title: String,
    category: String,
    around: String,
}

/// The notes for `query`, best first: those whose title holds it, then
/// those whose text does. With no query, the notes changed last.
fn find(app: App, query: &str) -> Vec<Found> {
    let query = query.trim();
    let mut found: Vec<(u8, f64, Found)> = app.notes.with(|notes| {
        notes
            .iter()
            .filter_map(|note| {
                let (title, category, modified, trashed) = note.meta.with(|m| (m.title.clone(), m.category.clone(), m.modified, m.trashed.is_some()));
                if trashed {
                    return None;
                }
                let body = note.plain_text();
                let rank = if query.is_empty() {
                    2
                } else if find_folded(&title, query).is_some() {
                    0
                } else if find_folded(&body, query).is_some() {
                    1
                } else {
                    return None;
                };
                let around = snippet(&body, query).unwrap_or_else(|| snippet_start(&body));
                Some((rank, modified, Found { note: note.clone(), title, category, around }))
            })
            .collect()
    });
    found.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.total_cmp(&a.1)));
    found.into_iter().take(MOST).map(|(_, _, f)| f).collect()
}

/// The search window, open while `App::search_open`.
pub fn search_window(app: App) -> Window {
    let mut attributes = WindowAttributes::default();
    attributes.transparent = Some(true);
    attributes.level = Some(WindowLevel::Floating);
    attributes.taskbar = Some(false);
    window()
        .id("search")
        .kind(WindowKind::Popup)
        .title("Search notes")
        .default_size(size(px(560.0), px(420.0)))
        .default_position(WindowPosition::Center)
        .attributes(attributes)
        .open(move || app.search_open.get())
        .on_open_change(move |change| app.search_open.set(change.open))
        .content(move || search_box(app))
}

fn search_box(app: App) -> impl IntoElement {
    let query = signal(String::new());
    let index = signal(0_usize);
    let results = move || {
        app.revision.get();
        find(app, &query.get())
    };
    let open = move |found: &Found| {
        app.search_open.set(false);
        app.show_on_desktop(&found.note);
        app.command.set(Some((found.note.id.clone(), NoteCommand::Focus)));
    };
    let open_chosen = move || {
        let found = untracked(results);
        if let Some(chosen) = found.get(index.get_untracked().min(found.len().saturating_sub(1))) {
            open(chosen);
        }
    };
    // ↑ and ↓ move through the results while the field has the focus.
    let always = || KeyOptions { when: nsh_core::keymap::When::Always, repeat: true, ..Default::default() };
    let step = move |by: isize| {
        let count = untracked(results).len() as isize;
        if count > 0 {
            index.update(|i| *i = (*i as isize + by).rem_euclid(count) as usize);
        }
    };
    on_key_with("down", always(), move |_| step(1));
    on_key_with("up", always(), move |_| step(-1));
    on_key_with("escape", always(), move |_| app.search_open.set(false));

    view()
        .flex_1()
        .rounded(px(14.0))
        .overflow_hidden()
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().popover)
        .text_color(move || theme().popover_foreground)
        .child(
            view().flex_row().items_center().gap_3().px_4().h(px(54.0)).border_b_1().border_color(move || theme().border).child(text("⌕").text_size(px(20.0)).text_color(move || theme().muted_foreground)).child(
                nsh_core::element::input()
                    .flex_1()
                    .h(px(36.0))
                    .text_size(px(17.0))
                    .placeholder("Search notes…")
                    .auto_focus(true)
                    .value(move || query.get())
                    .on_change(move |t: &str| {
                        query.set(t.to_string());
                        index.set(0);
                    })
                    .on_submit(move |_: &str| open_chosen()),
            ),
        )
        .child(scroll().flex_1().child(dynamic(move || {
            let found = results();
            if found.is_empty() {
                return view()
                    .py(px(28.0))
                    .items_center()
                    .child(text("No note matches").text_size(px(13.0)).text_color(move || theme().muted_foreground))
                    .into_element();
            }
            view()
                .p(px(6.0))
                .gap(px(2.0))
                .children(found.into_iter().enumerate().map(move |(i, found)| {
                    let pressed = found.clone();
                    let dot = found.note.meta;
                    view()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .px_3()
                        .py(px(7.0))
                        .rounded(px(8.0))
                        .bg(move || if index.get() == i { theme().accent } else { theme().popover })
                        .focusable(false)
                        .on_mouse_move(move |_: &MouseEvent| {
                            if index.get_untracked() != i {
                                index.set(i);
                            }
                        })
                        .on_press(move || open(&pressed))
                        .child(view().w(px(10.0)).h(px(10.0)).rounded_full().bg(move || crate::note_window::color(dot.with(|m| m.look.background), 1.0)))
                        .child(
                            view()
                                .flex_1()
                                .min_w(px(0.0))
                                .child(
                                    view()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .child(text(found.title).truncate().text_size(px(14.0)).font_weight(FontWeight::MEDIUM))
                                        .child(text(found.category.replace('/', " / ")).truncate().text_size(px(11.0)).text_color(move || theme().muted_foreground)),
                                )
                                .child(text(found.around).truncate().text_size(px(12.0)).text_color(move || theme().muted_foreground)),
                        )
                        .into_element()
                }))
                .into_element()
        })))
        .child(
            view().px_4().py(px(7.0)).border_t_1().border_color(move || theme().border).child(
                text("↑ ↓ to choose   ·   Enter to open   ·   Esc to close").text_size(px(11.0)).text_color(move || theme().muted_foreground),
            ),
        )
}
