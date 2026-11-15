//! The Notes Manager window: the notes and their categories on the left
//! (a search field, a tree, the trash), the selected note on the right
//! (its title, category, the formatting toolbar, its text, a status line).
//!
//! Categories are folders: renaming one moves its notes (`App::update`),
//! and empty ones are remembered in the settings. Moving a note is its
//! menu's "Move to" (NSH has no drag and drop between views).

use std::rc::Rc;

use chrono::{Local, TimeZone};
use nsh_base::{SelectItem, TreeEntry, TreeItem, h_resizable, resizable_panel, select, theme, tree};
use nsh_core::element::desktop::{MenuItem, Window, popup_menu, window};
use nsh_core::element::{ImageSrc, input};
use nsh_core::prelude::*;

use crate::app::{App, Note};
use crate::model::NoteMeta;
use crate::note_window::color;
use crate::ui::{self, TextLook, tint};
use crate::{editor_view, store};

/// Tree ids: a category's path or a note's id after a prefix, so they
/// never clash.
const CATEGORY: &str = "cat:";
const NOTE: &str = "note:";
/// The category picker's value for "No category".
const NO_CATEGORY: &str = "/";

const ICON_SEARCH: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg>"#;
const ICON_FOLDER: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.7-.9l-.8-1.2A2 2 0 0 0 7.9 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/></svg>"#;
const ICON_CHEVRON: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>"#;
const ICON_CHEVRON_DOWN: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>"#;
const ICON_DESKTOP: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8M12 17v4"/></svg>"#;
const ICON_MOON: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/></svg>"#;
const ICON_BELL: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/></svg>"#;
const ICON_BUBBLE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/></svg>"#;
const ICON_PLUS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.25" stroke-linecap="round" stroke-linejoin="round"><path d="M12 5v14M5 12h14"/></svg>"#;
const ICON_FOLDER_PLUS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 10v6M9 13h6"/><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.7-.9l-.8-1.2A2 2 0 0 0 7.9 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/></svg>"#;
const ICON_TRASH: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>"#;
const ICON_RESTORE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/></svg>"#;
const ICON_X: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 6 6 18M6 6l12 12"/></svg>"#;
const ICON_CHECK: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg>"#;

/// A naming in progress in the sidebar (native menus can't take text).
#[derive(Clone, Debug, PartialEq)]
enum Naming {
    /// A new category under this one ("" for the top).
    NewCategory(String),
    /// This category gets a new name.
    RenameCategory(String),
}

/// The window's own state (all `Copy` handles).
#[derive(Clone, Copy)]
struct Manager {
    app: App,
    query: Signal<String>,
    /// The tree's expanded ids.
    expanded: Signal<Vec<SharedString>>,
    naming: Signal<Option<Naming>>,
    trash_open: Signal<bool>,
    /// Bumped to put the keyboard in the selected note's title.
    rename: Signal<u64>,
}

pub fn manager_window(app: App) -> Window {
    window()
        .id("manager")
        .title("Bubble Notes")
        .default_size(size(px(980.0), px(660.0)))
        .restore_bounds(true)
        .open(move || app.manager_open.get())
        .on_open_change(move |change| app.manager_open.set(change.open))
        .content(move || {
            let m = Manager {
                app,
                query: signal(String::new()),
                expanded: signal(untracked(|| app.categories()).into_iter().map(|c| SharedString::from(format!("{CATEGORY}{c}"))).collect()),
                naming: signal(None),
                trash_open: signal(false),
                rename: signal(0),
            };
            if app.selected.get_untracked().is_none_or(|id| app.note(&id).is_none()) {
                app.selected.set(most_recent(app).map(|n| n.id));
            }
            // A note selected from elsewhere (a note's menu) shows in the tree.
            effect(move || {
                let Some(note) = app.selected.get().and_then(|id| app.note(&id)) else { return };
                let category = note.meta.with_untracked(|meta| meta.category.clone());
                let wanted: Vec<SharedString> = ancestors(&category).map(|c| SharedString::from(format!("{CATEGORY}{c}"))).collect();
                if untracked(|| m.expanded.with(|e| wanted.iter().any(|w| !e.contains(w)))) {
                    m.expanded.update(|e| e.extend(wanted.into_iter().filter(|w| !e.contains(w)).collect::<Vec<_>>()));
                }
            });
            view().flex_1().bg(move || theme().background).child(
                h_resizable()
                    .handle_class("w-px bg-border")
                    .handle_accessibility_label("Resize the sidebar")
                    .child(resizable_panel().size(px(270.0)).size_range(px(210.0)..px(440.0)).child(sidebar(m)))
                    .child(resizable_panel().child(note_pane(m))),
            )
        })
}

/// The live note changed last, if any.
fn most_recent(app: App) -> Option<Note> {
    app.notes.with_untracked(|notes| {
        notes
            .iter()
            .filter(|n| n.meta.with_untracked(|m| m.trashed.is_none()))
            .max_by(|a, b| a.meta.with_untracked(|m| m.modified).total_cmp(&b.meta.with_untracked(|m| m.modified)))
            .cloned()
    })
}

/// The live notes (not in the trash), read reactively.
fn live_notes(app: App) -> Vec<Note> {
    app.notes.with(|notes| notes.iter().filter(|n| n.meta.with(|m| m.trashed.is_none())).cloned().collect())
}

// ----- Small pieces ---------------------------------------------------------------

fn icon(markup: &'static str, size: f32) -> Svg {
    svg(ImageSrc::svg_markup(markup)).w(px(size)).h(px(size)).flex_shrink_0()
}

/// A colour dot: a note's background.
fn dot(rgb: impl Fn() -> u32 + 'static) -> View {
    view()
        .w(px(9.0))
        .h(px(9.0))
        .flex_shrink_0()
        .rounded_full()
        .bg(move || color(rgb(), 1.0))
        .border_1()
        .border_color(move || tint(0.18))
}

/// A small square icon button with a hint.
fn icon_button(markup: &'static str, label: &'static str, press: impl Fn() + 'static) -> impl IntoElement {
    nsh_base::button()
        .accessibility_label(label)
        .w(px(24.0))
        .h(px(24.0))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .text_color(move || theme().muted_foreground)
        .hover(|s| s.bg(tint(0.08)))
        .active(|s| s.opacity(0.6))
        .cursor(Cursor::Pointer).on_press(press)
        .child(ui::hinted(tip(label), icon(markup, 14.0)))
}

fn tip(label: impl Into<String>) -> impl IntoTooltip {
    nsh_base::tooltip()
        .px_2()
        .py_1()
        .rounded(px(6.0))
        .bg(move || theme().foreground)
        .shadow_md()
        .child(text(label.into()).text_size(px(12.0)).text_color(move || theme().background))
}

/// A calm hint centred in its space: a title and a line under it.
fn empty_state(title: &'static str, line: impl Into<String>) -> View {
    view()
        .flex_1()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .p_6()
        .child(text(title).text_size(px(14.0)).font_weight(FontWeight::MEDIUM).text_color(move || theme().foreground.opacity(0.8)))
        .child(text(line.into()).text_size(px(13.0)).text_center().text_color(move || theme().muted_foreground))
}

/// A sidebar section's label ("NOTES").
fn section_label(label: &'static str) -> impl IntoElement {
    text(label)
        .text_size(px(11.0))
        .font_weight(FontWeight::SEMIBOLD)
        .letter_spacing(px(0.6))
        .text_color(move || theme().muted_foreground)
}

// ----- The sidebar ----------------------------------------------------------------

fn sidebar(m: Manager) -> impl IntoElement {
    let app = m.app;
    view()
        .flex_1()
        .min_w(px(0.0))
        .bg(move || theme().sidebar)
        .child(
            view()
                .px_3()
                .pt_3()
                .pb_2()
                .gap_2()
                .child(search_field(m))
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .child(sidebar_button(ICON_PLUS, "New note", true, move || new_note(m, None)))
                        .child(sidebar_button(ICON_FOLDER_PLUS, "New category", false, move || {
                            m.naming.set(Some(Naming::NewCategory(String::new())))
                        })),
                )
                .child(dynamic(move || m.naming.get().map(|naming| naming_card(m, naming)))),
        )
        .child(dynamic(move || {
            if !m.query.with(|q| q.trim().is_empty()) {
                return search_results(m).into_element();
            }
            if live_notes(app).is_empty() && app.categories().is_empty() {
                return empty_state("No notes yet", "Make your first one with “New note”.").into_element();
            }
            view()
                .flex_1()
                .min_h(px(0.0))
                .child(view().px(px(20.0)).pt_2().pb_1().child(section_label("NOTES")))
                .child(note_tree(m))
                .into_element()
        }))
        .child(trash_section(m))
}

fn search_field(m: Manager) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .h(px(32.0))
        .px_2()
        .rounded(px(8.0))
        .bg(move || tint(0.05))
        .border_1()
        .border_color(move || tint(0.06))
        .child(icon(ICON_SEARCH, 14.0).text_color(move || theme().muted_foreground))
        .child(
            input()
                .flex_1()
                .min_w(px(0.0))
                .text_size(px(13.0))
                .text_color(move || theme().foreground)
                .placeholder("Search notes")
                .value(move || m.query.get())
                .on_change(move |t: &str| m.query.set(t.to_string())),
        )
        .child(dynamic(move || {
            (!m.query.with(String::is_empty)).then(|| icon_button(ICON_X, "Clear search", move || m.query.set(String::new())))
        }))
}

/// "New note" (filled) and "New category" (quiet).
fn sidebar_button(markup: &'static str, label: &'static str, primary: bool, press: impl Fn() + 'static) -> impl IntoElement {
    let ink = move || if primary { theme().primary_foreground } else { theme().foreground };
    nsh_base::button()
        .accessibility_label(label)
        .flex_1()
        .flex_row()
        .h(px(30.0))
        .gap(px(6.0))
        .items_center()
        .justify_center()
        .rounded(px(8.0))
        .bg(move || if primary { theme().primary } else { tint(0.06) })
        .hover(move |s| if primary { s.opacity(0.88) } else { s.bg(tint(0.1)) })
        .active(|s| s.opacity(0.7))
        .cursor(Cursor::Pointer).on_press(press)
        .child(icon(markup, 14.0).text_color(ink))
        .child(text(label).text_size(px(13.0)).font_weight(FontWeight::MEDIUM).text_color(ink))
}

/// The card where a category is named: what's being named, the name, and
/// confirm / cancel.
fn naming_card(m: Manager, naming: Naming) -> impl IntoElement {
    let (caption, start) = match &naming {
        Naming::NewCategory(parent) if parent.is_empty() => ("New category".to_string(), String::new()),
        Naming::NewCategory(parent) => (format!("New category in {parent}"), String::new()),
        Naming::RenameCategory(old) => (format!("Rename “{}”", last_part(old)), last_part(old).to_string()),
    };
    let name = signal(start);
    let confirm = Rc::new(move || {
        let typed = name.get_untracked();
        let done = match &naming {
            Naming::NewCategory(parent) => new_category(m, parent, &typed),
            Naming::RenameCategory(old) => rename_category(m, old, &typed),
        };
        if done {
            m.naming.set(None);
        }
    });
    let (submit, press) = (confirm.clone(), confirm);
    view()
        .gap(px(6.0))
        .p_2()
        .rounded(px(8.0))
        .bg(move || theme().background)
        .border_1()
        .border_color(move || theme().border)
        .shadow_sm()
        .child(text(caption).text_size(px(11.0)).text_color(move || theme().muted_foreground))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(
                    input()
                        .flex_1()
                        .min_w(px(0.0))
                        .h(px(28.0))
                        .px_2()
                        .rounded(px(6.0))
                        .bg(move || tint(0.05))
                        .text_size(px(13.0))
                        .text_color(move || theme().foreground)
                        .placeholder("Name")
                        .auto_focus(true)
                        .value(move || name.get())
                        .on_change(move |t: &str| name.set(t.to_string()))
                        .on_submit(move |_: &str| submit()),
                )
                .child(icon_button(ICON_CHECK, "Save", move || press()))
                .child(icon_button(ICON_X, "Cancel", move || m.naming.set(None))),
        )
}

/// The categories and their notes. A press selects a note (and opens or
/// closes a category), a double press shows the note on the desktop, a
/// right press opens its menu.
fn note_tree(m: Manager) -> impl IntoElement {
    let app = m.app;
    tree()
        .items(move || {
            let notes: Vec<(String, String, String)> = live_notes(app)
                .iter()
                .map(|n| n.meta.with(|meta| (meta.id.clone(), meta.title.clone(), meta.category.clone())))
                .collect();
            tree_items(&app.categories(), &notes)
        })
        .expanded(move || Some(m.expanded.get()))
        .selected(move || app.selected.get().map(|id| SharedString::from(format!("{NOTE}{id}"))))
        .accessibility_label("Notes")
        .flex_1()
        .min_h(px(0.0))
        .list_class("px-2 pb-3")
        .item_class("group rounded-md hover:bg-primary/5 selected:bg-primary/10")
        .on_expanded_change(move |ids: &Vec<SharedString>| m.expanded.set(ids.clone()))
        .on_selected_change(move |id: &SharedString| {
            if let Some(id) = id.strip_prefix(NOTE) {
                app.selected.set(Some(Rc::from(id)));
            }
        })
        .on_action(move |id: &SharedString| {
            if let Some(note) = id.strip_prefix(NOTE).and_then(|id| app.note(id)) {
                app.show_on_desktop(&note);
            }
        })
        .render_item(move |entry: &TreeEntry| tree_row(m, entry))
}

fn tree_row(m: Manager, entry: &TreeEntry) -> AnyElement {
    let indent = 6.0 + entry.depth as f32 * 16.0;
    if let Some(path) = entry.id.strip_prefix(CATEGORY) {
        let path: Rc<str> = Rc::from(path);
        let id = entry.id.clone();
        let (count_path, menu_path) = (path.clone(), path.clone());
        let folder = entry.folder;
        return view()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .h(px(30.0))
            .pl(px(indent))
            .pr_2()
            .w_full()
            .on_context_menu(move |_: &ContextMenuEvent| category_menu(m, &menu_path))
            .child(
                view()
                    .w(px(12.0))
                    .items_center()
                    .style_when(move || !folder, |s| s.invisible())
                    .child(
                        icon(ICON_CHEVRON, 11.0)
                            .text_color(move || theme().muted_foreground)
                            .transition_transform()
                            .duration(120.0)
                            .style_when(move || m.expanded.with(|e| e.contains(&id)), |s| s.rotate(deg(90.0))),
                    ),
            )
            .child(icon(ICON_FOLDER, 15.0).text_color(move || theme().muted_foreground))
            .child(text(entry.label.to_string()).truncate().flex_1().text_size(px(13.0)).text_color(move || theme().foreground))
            .child(
                text(move || {
                    let count = notes_in(m.app, &count_path).len();
                    if count == 0 { String::new() } else { count.to_string() }
                })
                .text_size(px(11.0))
                .text_color(move || theme().muted_foreground),
            )
            .into_element();
    }
    let Some(note) = entry.id.strip_prefix(NOTE).and_then(|id| m.app.note(id)) else {
        return view().into_element();
    };
    let meta = note.meta;
    let app = m.app;
    let marker = move |markup: &'static str, shown: fn(&NoteMeta, f64) -> bool| {
        icon(markup, 12.0)
            .text_color(move || theme().muted_foreground.opacity(0.85))
            .style_when(move || !meta.with(|m| shown(m, app.now.get())), |s| s.hidden())
    };
    view()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .h(px(30.0))
        .pl(px(indent + if entry.depth == 0 { 3.0 } else { 0.0 }))
        .pr_2()
        .w_full()
        .on_context_menu(move |_: &ContextMenuEvent| note_menu(m, &note))
        .child(dot(move || meta.with(|m| m.look.background)))
        .child(text(move || meta.with(|m| m.title.clone())).truncate().flex_1().text_size(px(13.0)).text_color(move || theme().foreground))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(5.0))
                .child(marker(ICON_BELL, |m, _| m.alarm.is_some()))
                .child(marker(ICON_MOON, |m, now| m.asleep(now)))
                .child(marker(ICON_BUBBLE, |m, _| m.bubble.enabled))
                .child(marker(ICON_DESKTOP, |m, now| m.on_desktop && !m.asleep(now))),
        )
        .into_element()
}

/// The flat list a search shows: every live note whose title or text holds
/// the query, with a bit of text around the match.
fn search_results(m: Manager) -> impl IntoElement {
    let app = m.app;
    dynamic(move || {
        let query = m.query.get();
        app.revision.get();
        let found: Vec<(Note, String)> = live_notes(app)
            .into_iter()
            .filter_map(|note| {
                let title = note.meta.with(|meta| meta.title.clone());
                let body = note.plain_text();
                let in_body = snippet(&body, query.trim());
                let in_title = find_folded(&title, query.trim()).is_some();
                (in_title || in_body.is_some()).then(|| (note, in_body.unwrap_or_else(|| snippet_start(&body))))
            })
            .collect();
        if found.is_empty() {
            return empty_state("No matches", format!("No note holds “{}”.", query.trim())).into_element();
        }
        view()
            .flex_1()
            .min_h(px(0.0))
            .child(view().px(px(20.0)).pt_2().pb_1().child(section_label("RESULTS")))
            .child(
                scroll().flex_1().child(
                    view()
                        .px_2()
                        .pb_3()
                        .gap(px(2.0))
                        .children(found.into_iter().map(|(note, around)| result_row(m, note, around).into_element())),
                ),
            )
            .into_element()
    })
}

fn result_row(m: Manager, note: Note, around: String) -> impl IntoElement {
    let app = m.app;
    let meta = note.meta;
    let id = note.id.clone();
    let selected = move || app.selected.with(|s| s.as_deref() == Some(&*id));
    let (press_note, menu_note) = (note.clone(), note.clone());
    view()
        .px_3()
        .py(px(7.0))
        .gap(px(3.0))
        .rounded(px(6.0))
        .bg(move || if selected() { tint(0.1) } else { tint(0.0) })
        .hover(|s| s.bg(tint(0.05)))
        .cursor(Cursor::Pointer).on_press(move |e: &PressEvent| {
            app.selected.set(Some(press_note.id.clone()));
            if e.clicks == 2 {
                app.show_on_desktop(&press_note);
            }
        })
        .on_context_menu(move |_: &ContextMenuEvent| note_menu(m, &menu_note))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(dot(move || meta.with(|m| m.look.background)))
                .child(
                    text(move || meta.with(|m| m.title.clone()))
                        .truncate()
                        .flex_1()
                        .text_size(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().foreground),
                )
                .child(text(move || meta.with(|m| m.category.clone())).truncate().text_size(px(11.0)).text_color(move || theme().muted_foreground)),
        )
        .child(
            text(around)
                .number_of_lines(2)
                .text_size(px(12.0))
                .line_height(px(17.0))
                .pl(px(17.0))
                .text_color(move || theme().muted_foreground),
        )
}

/// The trash at the bottom of the sidebar: folded until opened; each note
/// can be restored or deleted for good.
fn trash_section(m: Manager) -> impl IntoElement {
    let app = m.app;
    let trashed = move || -> Vec<Note> {
        app.notes.with(|notes| notes.iter().filter(|n| n.meta.with(|m| m.trashed.is_some())).cloned().collect())
    };
    view()
        .border_t_1()
        .border_color(move || theme().border)
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .h(px(38.0))
                .px(px(18.0))
                .hover(|s| s.bg(tint(0.04)))
                .cursor(Cursor::Pointer).on_press(move || m.trash_open.update(|o| *o = !*o))
                .child(
                    icon(ICON_CHEVRON, 11.0)
                        .text_color(move || theme().muted_foreground)
                        .transition_transform()
                        .duration(120.0)
                        .style_when(move || m.trash_open.get(), |s| s.rotate(deg(90.0))),
                )
                .child(icon(ICON_TRASH, 14.0).text_color(move || theme().muted_foreground))
                .child(text("Trash").flex_1().text_size(px(13.0)).text_color(move || theme().foreground))
                .child(text(move || {
                    let count = trashed().len();
                    if count == 0 { String::new() } else { count.to_string() }
                })
                .text_size(px(11.0))
                .text_color(move || theme().muted_foreground)),
        )
        .child(dynamic(move || {
            if !m.trash_open.get() {
                return None;
            }
            let notes = trashed();
            let days = app.settings.with(|s| s.trash_days);
            Some(if notes.is_empty() {
                view()
                    .px(px(20.0))
                    .pb_3()
                    .child(text("The trash is empty.").text_size(px(12.0)).text_color(move || theme().muted_foreground))
                    .into_element()
            } else {
                view()
                    .child(
                        scroll()
                            .max_h(px(190.0))
                            .child(view().px_2().children(notes.into_iter().map(|note| trash_row(m, note).into_element()))),
                    )
                    .child(
                        text(format!("Deleted notes go after {days} days."))
                            .text_size(px(11.0))
                            .px(px(20.0))
                            .pt_1()
                            .pb_2()
                            .text_color(move || theme().muted_foreground),
                    )
                    .into_element()
            })
        }))
}

fn trash_row(m: Manager, note: Note) -> impl IntoElement {
    let app = m.app;
    let meta = note.meta;
    let id = note.id.clone();
    let selected = move || app.selected.with(|s| s.as_deref() == Some(&*id));
    let (press, restore, delete, menu) = (note.clone(), note.clone(), note.clone(), note);
    view()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .h(px(30.0))
        .pl(px(10.0))
        .pr_1()
        .rounded(px(6.0))
        .bg(move || if selected() { tint(0.1) } else { tint(0.0) })
        .hover(|s| s.bg(tint(0.05)))
        .cursor(Cursor::Pointer).on_press(move || app.selected.set(Some(press.id.clone())))
        .on_context_menu(move |_: &ContextMenuEvent| trash_menu(m, &menu))
        .child(dot(move || meta.with(|m| m.look.background)).opacity(0.6))
        .child(text(move || meta.with(|m| m.title.clone())).truncate().flex_1().text_size(px(13.0)).text_color(move || theme().muted_foreground))
        .child(icon_button(ICON_RESTORE, "Restore", move || app.restore(&restore)))
        .child(icon_button(ICON_X, "Delete forever", move || confirm_delete_forever(app, &delete)))
}

// ----- The selected note --------------------------------------------------------------

/// The right side: the selected note, or a hint. Made again only when the
/// selection changes, never as the note's text or details do.
fn note_pane(m: Manager) -> impl IntoElement {
    let app = m.app;
    view().flex_1().min_w(px(0.0)).child(dynamic(move || {
        let selected = app.selected.get();
        match selected.and_then(|id| untracked(|| app.note(&id))) {
            // Built untracked: what building it reads of the note must not
            // make it again (the text would lose the keyboard focus).
            Some(note) => untracked(|| note_view(m, note).into_element()),
            None => {
                if untracked(|| live_notes(app).is_empty()) {
                    empty_state("Your notes will show here", "Make one with “New note” on the left.").into_element()
                } else {
                    empty_state("No note selected", "Pick a note on the left to read and edit it.").into_element()
                }
            }
        }
    }))
}

fn note_view(m: Manager, note: Note) -> impl IntoElement {
    let app = m.app;
    let meta = note.meta;
    let trashed = move || meta.with(|m| m.trashed.is_some());
    let ctx = editor_view::ctx_for(app, &note);
    let restore = note.clone();
    view()
        .flex_1()
        .min_h(px(0.0))
        .child(dynamic(move || {
            trashed().then(|| {
                let restore = restore.clone();
                view()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .px_6()
                    .py_2()
                    .bg(move || theme().destructive.opacity(0.1))
                    .child(icon(ICON_TRASH, 14.0).text_color(move || theme().destructive))
                    .child(text("This note is in the trash.").flex_1().text_size(px(13.0)).text_color(move || theme().foreground))
                    .child(ui::button("Restore", "Restore this note", || false, || true, move || app.restore(&restore)))
            })
        }))
        .child(
            view()
                .px(px(28.0))
                .pt_5()
                .pb_2()
                .gap_2()
                .child(title_field(m, note.clone()))
                .child(view().flex_row().items_center().gap_2().child(category_picker(app, note.clone()))),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .px(px(22.0))
                .border_b_1()
                .border_color(move || theme().border)
                .style_when(trashed, |s| s.hidden())
                .child(ui::toolbar(ctx)),
        )
        .child(
            view().flex_1().min_h(px(0.0)).px(px(22.0)).child(editor_view::note_editor(ctx, move || !trashed(), "Start writing…", move || TextLook {
                color: None,
                selection: app.settings.with(|s| s.selection_strength),
                size: 15.0,
                font: String::new(),
                compact: false,
            })),
        )
        .child(status_line(app, ctx, note))
}

/// The title, edited in place. It's saved when the field is left or Enter
/// is pressed, not per key: each change renames the note's file.
fn title_field(m: Manager, note: Note) -> impl IntoElement {
    let app = m.app;
    let meta = note.meta;
    let draft = signal(meta.with_untracked(|m| m.title.clone()));
    // A rename from elsewhere shows here.
    effect(move || {
        let title = meta.with(|m| m.title.clone());
        draft.set(title);
    });
    let field = element_ref();
    let focus = field.clone();
    let asked = m.rename.get_untracked();
    effect(move || {
        if m.rename.get() != asked {
            focus.focus();
        }
    });
    let commit = Rc::new(move || {
        let typed = draft.get_untracked().trim().to_string();
        let current = meta.with_untracked(|m| m.title.clone());
        if typed.is_empty() {
            draft.set(current);
        } else if typed != current {
            app.update(&note, |m| m.title = typed);
        }
    });
    let (on_blur, on_submit) = (commit.clone(), commit);
    input()
        .element_ref(&field)
        .w_full()
        .h(px(40.0))
        .px_2()
        .ml(px(-8.0))
        .rounded(px(8.0))
        .hover(|s| s.bg(tint(0.04)))
        .text_size(px(24.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || theme().foreground)
        .placeholder("Untitled")
        .readonly(move || meta.with(|m| m.trashed.is_some()))
        .value(move || draft.get())
        .on_change(move |t: &str| draft.set(t.to_string()))
        .on_blur(move || on_blur())
        .on_submit(move |_: &str| on_submit())
}

/// The note's category: a searchable list of every category.
fn category_picker(app: App, note: Note) -> impl IntoElement {
    let meta = note.meta;
    let label = move || meta.with(|m| if m.category.is_empty() { "No category".to_string() } else { m.category.replace('/', " / ") });
    select()
        .cursor(Cursor::Pointer)
        .items(move || {
            std::iter::once(SelectItem::new(NO_CATEGORY, "No category"))
                .chain(app.categories().into_iter().map(|c| SelectItem::new(c.clone(), c.replace('/', " / "))))
                .collect::<Vec<_>>()
        })
        .value(move || Some(SharedString::from(meta.with(|m| if m.category.is_empty() { NO_CATEGORY.to_string() } else { m.category.clone() }))))
        .searchable(true)
        .search_placeholder("Find a category")
        .accessibility_label("Category")
        .disabled(move || meta.with(|m| m.trashed.is_some()))
        .popup_class("min-w-[240px] max-h-[320px] p-1 rounded-lg border border-border bg-popover shadow-lg")
        .list_class("gap-px")
        .search_class("h-8 px-2 mb-1 rounded-md bg-muted text-[13px] text-foreground")
        .item_class("px-2 py-1.5 rounded-md text-[13px] text-popover-foreground focus:bg-accent selected:font-semibold")
        .empty(text("No such category").text_size(px(12.0)).px_2().py_2().text_color(move || theme().muted_foreground))
        .on_change(move |value: &Option<SharedString>| {
            let category = value.as_deref().filter(|v| *v != NO_CATEGORY).unwrap_or_default().to_string();
            app.update(&note, |m| m.category = category);
        })
        .trigger(
            view()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .h(px(26.0))
                .px_2()
                .rounded(px(6.0))
                .bg(move || tint(0.05))
                .hover(|s| s.bg(tint(0.09)))
                .child(icon(ICON_FOLDER, 13.0).text_color(move || theme().muted_foreground))
                .child(text(label).text_size(px(12.0)).text_color(move || theme().foreground.opacity(0.8)))
                .child(icon(ICON_CHEVRON_DOWN, 12.0).text_color(move || theme().muted_foreground)),
        )
}

/// Words, the editor's last message, when it was last changed, and the
/// desktop toggle.
fn status_line(app: App, ctx: ui::Ctx, note: Note) -> impl IntoElement {
    let meta = note.meta;
    let words_note = note.clone();
    let on_desktop = move || meta.with(|m| m.on_desktop && !m.asleep(app.now.get()));
    view()
        .flex_row()
        .items_center()
        .gap_3()
        .h(px(32.0))
        .px_4()
        .border_t_1()
        .border_color(move || theme().border)
        .child(
            text(move || {
                app.revision.get();
                let words = words_note.plain_text().split_whitespace().count();
                if words == 1 { "1 word".to_string() } else { format!("{words} words") }
            })
            .text_size(px(12.0))
            .text_color(move || theme().muted_foreground),
        )
        .child(
            text(move || ctx.status.get())
                .truncate()
                .flex_1()
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
        )
        .child(
            text(move || format!("Edited {}", when(meta.with(|m| m.modified), app.now.get())))
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
        )
        .child(
            nsh_base::button()
                .accessibility_label("On desktop")
                .selected(on_desktop)
                .disabled(move || meta.with(|m| m.trashed.is_some()))
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .h(px(24.0))
                .px_2()
                .rounded(px(6.0))
                .bg(move || if on_desktop() { tint(0.1) } else { tint(0.0) })
                .hover(|s| s.bg(tint(0.07)))
                .active(|s| s.opacity(0.6))
                .cursor(Cursor::Pointer).on_press(move || {
                    if on_desktop() {
                        app.update(&note, |m| m.on_desktop = false);
                    } else {
                        app.show_on_desktop(&note);
                    }
                })
                .child(icon(ICON_DESKTOP, 13.0).text_color(move || if on_desktop() { theme().foreground } else { theme().muted_foreground }))
                .child(
                    text(move || if on_desktop() { "On desktop" } else { "Not on desktop" })
                        .text_size(px(12.0))
                        .text_color(move || if on_desktop() { theme().foreground } else { theme().muted_foreground }),
                ),
        )
}

// ----- Menus and what they do ---------------------------------------------------------

fn note_menu(m: Manager, note: &Note) {
    let app = m.app;
    let meta = note.meta.get_untracked();
    let shown = meta.on_desktop && !meta.asleep(app.now.get_untracked());
    let with = |run: fn(Manager, &Note)| {
        let note = note.clone();
        move || run(m, &note)
    };
    let mut moves = vec![
        {
            let note = note.clone();
            MenuItem::new("No category").checked(meta.category.is_empty()).on_select(move || app.update(&note, |m| m.category = String::new()))
        },
        MenuItem::separator(),
    ];
    for category in untracked(|| app.categories()) {
        let note = note.clone();
        let checked = category == meta.category;
        let label = category.replace('/', " / ");
        moves.push(MenuItem::new(label).checked(checked).on_select(move || {
            let category = category.clone();
            app.update(&note, |m| m.category = category);
        }));
    }
    let _ = popup_menu(vec![
        if shown {
            MenuItem::new("Hide from desktop").on_select(with(|m, n| m.app.update(n, |meta| meta.on_desktop = false)))
        } else {
            MenuItem::new("Show on desktop").on_select(with(|m, n| m.app.show_on_desktop(n)))
        },
        MenuItem::new("Rename").on_select(with(|m, n| {
            m.app.selected.set(Some(n.id.clone()));
            m.rename.update(|r| *r += 1);
        })),
        MenuItem::new("Duplicate").on_select(with(|m, n| {
            let copy = m.app.duplicate(n);
            m.app.selected.set(Some(copy.id));
        })),
        MenuItem::submenu("Move to", moves),
        if meta.category == crate::model::TEMPLATES {
            MenuItem::new("New note from this template").on_select(with(|m, n| {
                m.app.new_from_template(n);
            }))
        } else {
            MenuItem::new("Save as template").on_select(with(|m, n| {
                m.app.save_as_template(n);
            }))
        },
        MenuItem::separator(),
        MenuItem::new("Delete").on_select(with(|m, n| {
            m.app.trash(n);
            if m.app.selected.get_untracked().as_deref() == Some(&*n.id) {
                m.app.selected.set(most_recent(m.app).map(|n| n.id));
            }
        })),
    ]);
}

fn trash_menu(m: Manager, note: &Note) {
    let app = m.app;
    let (restore, delete) = (note.clone(), note.clone());
    let _ = popup_menu(vec![
        MenuItem::new("Restore").on_select(move || app.restore(&restore)),
        MenuItem::separator(),
        MenuItem::new("Delete forever").on_select(move || app.delete_forever(&delete)),
    ]);
}

/// Deleting for good can't be undone: a menu asks first.
fn confirm_delete_forever(app: App, note: &Note) {
    let note = note.clone();
    let title = note.meta.with_untracked(|m| m.title.clone());
    let _ = popup_menu(vec![
        MenuItem::new(format!("Delete “{title}” forever")).on_select(move || app.delete_forever(&note)),
        MenuItem::new("Cancel"),
    ]);
}

fn category_menu(m: Manager, path: &str) {
    let empty = untracked(|| notes_in(m.app, path).is_empty());
    let (here, sub, rename, delete) = (path.to_string(), path.to_string(), path.to_string(), path.to_string());
    // The look new notes get here: a theme, or the default (its parent's).
    let app = m.app;
    let own = app.settings.with_untracked(|s| s.category_looks.get(path).cloned());
    let mut looks = vec![
        {
            let path = path.to_string();
            MenuItem::new("Default").checked(own.is_none()).on_select(move || {
                app.update_settings(|s| {
                    s.category_looks.remove(&path);
                })
            })
        },
        MenuItem::separator(),
    ];
    let themes = crate::model::builtin_themes().into_iter().chain(app.settings.with_untracked(|s| s.themes.clone()));
    for theme in themes {
        let path = path.to_string();
        let checked = own.as_ref() == Some(&theme.look);
        looks.push(MenuItem::new(theme.name.clone()).checked(checked).on_select(move || {
            let look = theme.look.clone();
            app.update_settings(|s| {
                s.category_looks.insert(path.clone(), look);
            })
        }));
    }
    let _ = popup_menu(vec![
        MenuItem::new("New note here").on_select(move || new_note(m, Some(here.clone()))),
        MenuItem::submenu("Colour of new notes", looks),
        MenuItem::new("New category inside…").on_select(move || m.naming.set(Some(Naming::NewCategory(sub.clone())))),
        MenuItem::new("Rename category…").on_select(move || m.naming.set(Some(Naming::RenameCategory(rename.clone())))),
        MenuItem::separator(),
        MenuItem::new(if empty { "Delete category" } else { "Delete category (move its notes out first)" })
            .disabled(!empty)
            .on_select(move || delete_category(m, &delete)),
    ]);
}

/// A new note, in `category` (else the selected note's), kept off the
/// desktop and selected, its title ready to type.
fn new_note(m: Manager, category: Option<String>) {
    let app = m.app;
    let category = category.unwrap_or_else(|| {
        app.selected
            .get_untracked()
            .and_then(|id| app.note(&id))
            .filter(|n| n.meta.with_untracked(|m| m.trashed.is_none()))
            .map(|n| n.meta.with_untracked(|m| m.category.clone()))
            .unwrap_or_default()
    });
    m.query.set(String::new());
    let note = app.new_note(&category);
    app.update(&note, |meta| meta.on_desktop = false);
    app.selected.set(Some(note.id.clone()));
    m.rename.update(|r| *r += 1);
}

/// The live notes in `category` or under it.
fn notes_in(app: App, category: &str) -> Vec<Note> {
    live_notes(app).into_iter().filter(|n| n.meta.with(|m| within(&m.category, category))).collect()
}

/// Makes the category `name` under `parent`; false when the name is empty.
fn new_category(m: Manager, parent: &str, name: &str) -> bool {
    let name = store::clean_category(name);
    if name.is_empty() {
        return false;
    }
    let path = if parent.is_empty() { name } else { format!("{parent}/{name}") };
    if !untracked(|| m.app.categories()).contains(&path) {
        m.app.update_settings(|s| s.categories.push(path.clone()));
    }
    m.expanded.update(|e| {
        for c in ancestors(&path) {
            let id = SharedString::from(format!("{CATEGORY}{c}"));
            if !e.contains(&id) {
                e.push(id);
            }
        }
    });
    true
}

/// Gives the category `old` (and what's under it) the last part `name`,
/// moving its notes; false when the name is empty.
fn rename_category(m: Manager, old: &str, name: &str) -> bool {
    let app = m.app;
    let name = store::clean_category(&name.replace('/', "-"));
    if name.is_empty() {
        return false;
    }
    let new = match old.rsplit_once('/') {
        Some((parent, _)) => format!("{parent}/{name}"),
        None => name,
    };
    if new == old {
        return true;
    }
    let before = untracked(|| app.categories());
    for note in untracked(|| notes_in(app, old)) {
        let moved = note.meta.with_untracked(|meta| renamed(&meta.category, old, &new));
        app.update(&note, |meta| meta.category = moved);
    }
    // Deleted notes go back to the new place when restored.
    for note in app.notes.get_untracked() {
        if note.meta.with_untracked(|meta| meta.trashed.is_some() && within(&meta.category, old)) {
            let moved = note.meta.with_untracked(|meta| renamed(&meta.category, old, &new));
            app.update(&note, |meta| meta.category = moved);
        }
    }
    // The empty categories under it, remembered under the new name.
    let empty: Vec<String> = before.iter().filter(|c| within(c, old)).map(|c| renamed(c, old, &new)).collect();
    app.update_settings(|s| {
        s.categories.retain(|c| !within(c, old));
        for c in empty {
            if !s.categories.contains(&c) {
                s.categories.push(c);
            }
        }
    });
    remove_folders(app, before.iter().filter(|c| within(c, old)));
    m.expanded.update(|e| {
        for id in e.iter_mut() {
            if let Some(path) = id.strip_prefix(CATEGORY).filter(|p| within(p, old)) {
                *id = SharedString::from(format!("{CATEGORY}{}", renamed(path, old, &new)));
            }
        }
    });
    true
}

/// Deletes an empty category (and the empty ones under it). One that still
/// holds notes stays: its menu item is disabled.
fn delete_category(m: Manager, path: &str) {
    let app = m.app;
    if !untracked(|| notes_in(app, path)).is_empty() {
        return;
    }
    let before = untracked(|| app.categories());
    app.update_settings(|s| s.categories.retain(|c| !within(c, path)));
    remove_folders(app, before.iter().filter(|c| within(c, path)));
}

/// Removes the folders of `categories` that are empty, deepest first.
fn remove_folders<'a>(app: App, categories: impl Iterator<Item = &'a String>) {
    let mut folders: Vec<&String> = categories.collect();
    folders.sort_by_key(|c| std::cmp::Reverse(c.matches('/').count()));
    let vault = app.vault.get();
    for folder in folders {
        vault.remove_empty_category(folder);
    }
    // The categories are read from the disk too: read them again.
    app.notes.update(|_| {});
}

// ----- Pure helpers ---------------------------------------------------------------------

/// `category` is `parent` or under it.
fn within(category: &str, parent: &str) -> bool {
    category == parent || category.strip_prefix(parent).is_some_and(|rest| rest.starts_with('/'))
}

/// `category` (within `old`) moved to `new`.
fn renamed(category: &str, old: &str, new: &str) -> String {
    format!("{new}{}", &category[old.len().min(category.len())..])
}

/// A category path and its parents, the top first ("Work", "Work/Ideas").
fn ancestors(category: &str) -> impl Iterator<Item = String> + '_ {
    category.match_indices('/').map(|(i, _)| category[..i].to_string()).chain((!category.is_empty()).then(|| category.to_string()))
}

fn last_part(category: &str) -> &str {
    category.rsplit('/').next().unwrap_or(category)
}

/// The tree: each category with its subcategories then its notes (by
/// title), then the notes with no category. `notes` are (id, title,
/// category).
fn tree_items(categories: &[String], notes: &[(String, String, String)]) -> Vec<TreeItem> {
    fn notes_of(notes: &[(String, String, String)], category: &str) -> Vec<TreeItem> {
        let mut found: Vec<&(String, String, String)> = notes.iter().filter(|n| n.2 == category).collect();
        found.sort_by_key(|n| n.1.to_lowercase());
        found.into_iter().map(|(id, title, _)| TreeItem::new(format!("{NOTE}{id}"), title.clone())).collect()
    }
    fn under(categories: &[String], notes: &[(String, String, String)], parent: &str) -> Vec<TreeItem> {
        categories
            .iter()
            .filter(|c| match c.rsplit_once('/') {
                Some((p, _)) => p == parent,
                None => parent.is_empty(),
            })
            .map(|c| {
                TreeItem::new(format!("{CATEGORY}{c}"), last_part(c).to_string())
                    .children(under(categories, notes, c))
                    .children(notes_of(notes, c))
            })
            .collect()
    }
    let mut items = under(categories, notes, "");
    items.extend(notes_of(notes, ""));
    items
}

/// `text`'s characters folded to lower case one for one, so positions in
/// it are positions in `text`.
fn folded(text: &str) -> Vec<char> {
    text.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect()
}

/// Where `query` first is in `text`, ignoring case, as a character index.
pub(crate) fn find_folded(text: &str, query: &str) -> Option<usize> {
    let (text, query) = (folded(text), folded(query));
    if query.is_empty() || query.len() > text.len() {
        return None;
    }
    (0..=text.len() - query.len()).find(|&i| text[i..i + query.len()] == query[..])
}

/// A short bit of `text` around the first match of `query`, on one line,
/// with "…" where it was cut.
pub(crate) fn snippet(text: &str, query: &str) -> Option<String> {
    let at = find_folded(text, query)?;
    let chars: Vec<char> = text.chars().collect();
    let start = at.saturating_sub(32);
    let end = (at + query.chars().count() + 80).min(chars.len());
    let middle: String = chars[start..end].iter().collect();
    let mut words: Vec<&str> = middle.split_whitespace().collect();
    // A cut start begins at the next whole word (unless that's the match).
    if start > 0 && words.len() > 1 && !middle.starts_with(char::is_whitespace) && start + words[0].chars().count() <= at {
        words.remove(0);
    }
    let middle = words.join(" ");
    Some(format!("{}{middle}{}", if start > 0 { "…" } else { "" }, if end < chars.len() { "…" } else { "" }))
}

/// The start of a note's text, for a result found by its title.
pub(crate) fn snippet_start(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut short: String = line.chars().take(110).collect();
    if short.len() < line.len() {
        short.push('…');
    }
    short
}

/// When `ms` was, in words, seen from `now`.
fn when(ms: f64, now: f64) -> String {
    let ago = ((now - ms) / 1000.0).max(0.0);
    if ago < 60.0 {
        return "just now".into();
    }
    if ago < 3600.0 {
        return format!("{} min ago", (ago / 60.0) as u64);
    }
    let (Some(then), Some(today)) = (Local.timestamp_millis_opt(ms as i64).single(), Local.timestamp_millis_opt(now as i64).single()) else {
        return String::new();
    };
    if then.date_naive() == today.date_naive() {
        then.format("today at %H:%M").to_string()
    } else if today.date_naive().pred_opt() == Some(then.date_naive()) {
        then.format("yesterday at %H:%M").to_string()
    } else if then.format("%Y").to_string() == today.format("%Y").to_string() {
        then.format("%-d %b at %H:%M").to_string()
    } else {
        then.format("%-d %b %Y").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_nest_and_notes_follow() {
        let categories = ["Home".to_string(), "Work".into(), "Work/Ideas".into()];
        let notes = [
            ("a".to_string(), "Zebra".to_string(), "Work".to_string()),
            ("b".into(), "apple".into(), "Work".into()),
            ("c".into(), "Launch".into(), "Work/Ideas".into()),
            ("d".into(), "Loose".into(), String::new()),
        ];
        let items = tree_items(&categories, &notes);
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_ref()).collect();
        assert_eq!(ids, ["cat:Home", "cat:Work", "note:d"]);
        let work: Vec<&str> = items[1].children.iter().map(|i| i.id.as_ref()).collect();
        assert_eq!(work, ["cat:Work/Ideas", "note:b", "note:a"]);
        assert_eq!(items[1].children[0].label.as_ref(), "Ideas");
        assert_eq!(items[1].children[0].children[0].id.as_ref(), "note:c");
    }

    #[test]
    fn category_paths() {
        assert!(within("Work/Ideas", "Work") && within("Work", "Work"));
        assert!(!within("Workshop", "Work"));
        assert_eq!(renamed("Work/Ideas", "Work", "Jobs"), "Jobs/Ideas");
        assert_eq!(ancestors("a/b/c").collect::<Vec<_>>(), ["a", "a/b", "a/b/c"]);
        assert_eq!(ancestors("").count(), 0);
        assert_eq!(last_part("a/b"), "b");
    }

    #[test]
    fn search_ignores_case_and_shows_the_match() {
        assert_eq!(find_folded("Hello World", "WORLD"), Some(6));
        assert_eq!(find_folded("Ünïcode Straße", "straße"), Some(8));
        assert_eq!(find_folded("short", "much longer"), None);
        let text = format!("{} needle {}", "word ".repeat(20), "tail ".repeat(30));
        let found = snippet(&text, "NEEDLE").unwrap();
        assert!(found.starts_with('…') && found.ends_with('…') && found.contains("needle"));
        assert_eq!(snippet("a needle\nhere", "needle").as_deref(), Some("a needle here"));
        assert!(snippet("nothing", "needle").is_none());
    }

    #[test]
    fn times_read_naturally() {
        let now = 1_700_000_000_000.0;
        assert_eq!(when(now - 5_000.0, now), "just now");
        assert_eq!(when(now - 600_000.0, now), "10 min ago");
        assert!(!when(now - 90_000_000.0, now).is_empty());
    }
}
