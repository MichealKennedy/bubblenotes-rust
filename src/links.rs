//! Links between notes. Typing `[[` opens a menu of the notes; picking one
//! (or typing its name and `]]`) puts a link to it in the text, which shows
//! the note's title as it is now and opens the note on a click.
//!
//! A link is an inline `noteLink` node holding the note's id (which never
//! changes) and its title when the link was made (for when the note is
//! gone). In a note's file it's `<span data-note-link="ID">Title</span>`;
//! in Markdown and plain text, `[[Title]]`.

use std::rc::Rc;
use std::sync::Arc;

use nsh_base::rich_editor::{
    extensions::{
        Extension,
        suggestion::{SuggestionOptions, SuggestionState, SuggestionTrigger, replace_suggestion, suggestion, suggestion_state},
    },
    model::{AttrParse, AttrRender, AttrSpec, DomOutputSpec, Fragment, Node, NodeSpec, ParseRule, Schema, TextRender},
    state::{EditorState, Plugin},
    view::{NodeView, NodeViewCx, NodeViews},
};
use nsh_base::theme;
use nsh_core::prelude::*;
use serde_json::{Value, json};

use crate::app::{App, NoteCommand};
use crate::kit::Editor;
use crate::ui::Ctx;

/// The node type, and the plugin's key.
pub const NODE: &str = "noteLink";
const KEY: &str = "noteLink";
/// The most notes the menu lists.
const MOST: usize = 7;

fn text_attr() -> AttrSpec {
    let mut attr = AttrSpec::default();
    attr.default = Some(Value::String(String::new()));
    attr.parse_html = Some(AttrParse::Skip);
    attr.render_html = Some(AttrRender::Skip);
    attr
}

/// The `noteLink` node type.
pub fn extension() -> Extension {
    Extension::node(NODE, |_| {
        let mut spec = NodeSpec::default();
        spec.group = Some("inline".into());
        spec.inline = true;
        spec.atom = true;
        spec.attrs = vec![("noteId".into(), text_attr()), ("title".into(), text_attr())];
        spec.parse_dom = vec![ParseRule::tag("span[data-note-link]").with_get_attrs(|input| {
            let span = input.element()?;
            let attrs = [("noteId", span.attr("data-note-link").unwrap_or_default().to_string()), ("title", span.text_content().trim().to_string())];
            Some(attrs.into_iter().map(|(k, v)| (k.to_string(), Value::String(v))).collect())
        })];
        spec.to_dom = Some(DomOutputSpec::render(|node| {
            let get = |name: &str| node.attr(name).and_then(Value::as_str).unwrap_or_default().to_string();
            DomOutputSpec::tag("span").attr("data-note-link", get("noteId")).child(DomOutputSpec::Text(get("title")))
        }));
        spec.render_text = Some(TextRender::Template("[[{title}]]".into()));
        spec
    })
}

/// The plugin following a typed `[[`: its menu's state, and the keys the
/// menu takes while open.
pub fn plugin() -> Plugin {
    let mut trigger = SuggestionTrigger::default();
    trigger.char = "[[".into();
    trigger.allow_spaces = true;
    trigger.allowed_prefixes = None;
    let mut options = SuggestionOptions::default();
    options.key = KEY.into();
    options.trigger = trigger;
    options.capture_keys = ["ArrowUp", "ArrowDown", "Enter", "Tab"].map(String::from).to_vec();
    options.allow = Some(Arc::new(|state: &EditorState, range, _| {
        state.doc().resolve(range.from).is_ok_and(|at| at.parent().ty().name() != "codeBlock")
    }));
    suggestion(options)
}

pub fn with_node_views(views: NodeViews) -> NodeViews {
    views.with(NODE, Arc::new(LinkView))
}

fn state(editor: &Editor) -> Option<Arc<SuggestionState>> {
    suggestion_state(&editor.state(), KEY)
}

/// A link to the note `id`, titled `title`, followed by a space to type on.
fn link_fragment(schema: &Schema, id: &str, title: &str) -> Option<Fragment> {
    let link = schema.node_from_json(&json!({ "type": NODE, "attrs": { "noteId": id, "title": title } })).ok()?;
    let space = schema.node_from_json(&json!({ "type": "text", "text": " " })).ok()?;
    Some(Fragment::from(vec![link, space]))
}

/// Puts a link to the note in place of the `[[` and what was typed after
/// it. Whether it did.
fn insert_link(editor: &Editor, id: &str, title: &str) -> bool {
    link_fragment(&editor.kit.schema, id, title).is_some_and(|fragment| editor.session.run(&replace_suggestion(KEY, fragment)))
}

// ----- The link in the text -----------------------------------------------------

/// A link: the note's title as it is now, underlined; a press opens the
/// note on the desktop. A link to a note that's gone is struck through.
struct LinkView;

impl NodeView for LinkView {
    fn create(&self, _: &NodeViewCx, node: &Node) -> AnyElement {
        let get = |name: &str| node.attrs().get(name).and_then(Value::as_str).unwrap_or_default().to_string();
        let (id, was_titled) = (get("noteId"), get("title"));
        let app = App::current();
        let find_id = id.clone();
        // The note's title now, and whether the note is still there.
        let target = move || {
            app.and_then(|app| {
                app.notes.with(|notes| {
                    notes.iter().find(|n| *n.id == *find_id).map(|n| n.meta.with(|m| (m.title.clone(), m.trashed.is_none())))
                })
            })
        };
        let (title, alive) = (target.clone(), target);
        // A text of its own (not a box around one), so it sits on the line's
        // baseline like the words beside it.
        text(move || title().map_or_else(|| was_titled.clone(), |(title, _)| title))
            .underline()
            // The editor rests a node view's box on the baseline; its text's
            // own baseline is a descender higher.
            .translate_y(px(4.0))
            .rounded(px(4.0))
            .bg(nsh_core::Color(0x0000001a))
            .hover(|s| s.bg(nsh_core::Color(0x0000002e)))
            .style_when(move || !alive().is_some_and(|(_, alive)| alive), |s| s.line_through().opacity(0.6))
            .cursor(Cursor::Pointer)
            .label("Open the linked note")
            .on_press(move || {
                let Some(app) = app else { return };
                if let Some(note) = app.note(&id).filter(|n| n.meta.with_untracked(|m| m.trashed.is_none())) {
                    app.show_on_desktop(&note);
                    app.command.set(Some((note.id.clone(), NoteCommand::Focus)));
                }
            })
            .into_element()
    }
}

// ----- The menu ------------------------------------------------------------------

/// A row of the menu: a note to link to, or a new note to make.
#[derive(Clone, PartialEq)]
enum Choice {
    Note { id: Rc<str>, title: String, category: String },
    New(String),
}

/// The menu's rows for what's typed after `[[`: the notes whose title has
/// it (those starting with it first), then a new note of that name unless
/// one is called exactly that.
fn choices(notes: &[(Rc<str>, String, String)], query: &str) -> Vec<Choice> {
    let query = query.trim().trim_end_matches(']').trim();
    let wanted = query.to_lowercase();
    let mut found: Vec<&(Rc<str>, String, String)> = notes.iter().filter(|(_, title, _)| title.to_lowercase().contains(&wanted)).collect();
    found.sort_by_key(|(_, title, _)| (!title.to_lowercase().starts_with(&wanted), title.to_lowercase()));
    let exact = found.iter().any(|(_, title, _)| title.to_lowercase() == wanted);
    let mut out: Vec<Choice> =
        found.into_iter().take(MOST).map(|(id, title, category)| Choice::Note { id: id.clone(), title: title.clone(), category: category.clone() }).collect();
    if !query.is_empty() && !exact {
        out.push(Choice::New(query.to_string()));
    }
    out
}

/// The menu under a typed `[[`: the notes to link to, filtered by what's
/// typed after it; ↑ ↓ move, Enter or Tab picks, a click picks, and typing
/// `]]` picks the note of that name (or makes it). `window_height` keeps
/// it inside the window.
pub fn menu(ctx: Ctx, app: App, window_height: Signal<f32>) -> impl IntoElement {
    let window = nsh_core::element::desktop::current_window();
    let owner = Owner::current();
    let index = signal(0_usize);
    let keys_seen = signal(0_u64);
    let open = signal(None::<(usize, String)>);
    let rows = move || {
        let Some((_, query)) = open.get() else { return Vec::new() };
        let notes: Vec<(Rc<str>, String, String)> = app.notes.with(|notes| {
            notes
                .iter()
                .filter_map(|n| n.meta.with(|m| m.trashed.is_none().then(|| (n.id.clone(), m.title.clone(), m.category.clone()))))
                .collect()
        });
        choices(&notes, &query)
    };
    let pick = move |choice: Choice| {
        let (id, title) = match choice {
            Choice::Note { id, title, .. } => (id, title),
            Choice::New(title) => {
                let note = app.new_note("");
                app.update(&note, |m| m.title = title.clone());
                (note.id.clone(), title)
            }
        };
        ctx.editor.with_untracked(|e| insert_link(e, &id, &title));
        ctx.changed();
        // A click on the menu takes the focus from the text as it ends:
        // give it back once it has.
        owner.run(|| {
            let _ = set_timeout(std::time::Duration::from_millis(60), move || ctx.editor.with_untracked(|e| e.session.focus()));
        });
    };
    let pick_keys = pick.clone();
    effect(move || {
        ctx.revision.get();
        if ctx.read_only.get() {
            return;
        }
        let Some(state) = ctx.editor.with(state) else { return };
        let now = state.active.then(|| (state.range.from, state.query.clone().unwrap_or_default()));
        let before = open.get_untracked();
        if now != before {
            if now.as_ref().map(|(_, q)| q) != before.as_ref().map(|(_, q)| q) {
                index.set(0);
            }
            open.set(now.clone());
        }
        let found = untracked(rows);
        // `[[Name]]` typed out in full links to that note.
        if let Some((_, query)) = &now
            && query.trim_end().ends_with("]]")
            && let Some(first) = found.first()
        {
            let name = query.trim().trim_end_matches(']').trim().to_lowercase();
            let exact = found.iter().find(|c| matches!(c, Choice::Note { title, .. } if title.to_lowercase() == name));
            return pick_keys(exact.or(found.last()).unwrap_or(first).clone());
        }
        if state.key_count == keys_seen.get_untracked() {
            return;
        }
        keys_seen.set(state.key_count);
        if now.is_none() || found.is_empty() {
            return;
        }
        match state.last_key.as_deref() {
            Some("ArrowDown") => index.update(|i| *i = (*i + 1) % found.len()),
            Some("ArrowUp") => index.update(|i| *i = (*i + found.len() - 1) % found.len()),
            Some("Enter" | "Tab") => pick_keys(found[index.get_untracked().min(found.len() - 1)].clone()),
            _ => {}
        }
    });
    // Rows follow the pointer only when it really moves (see the slash menu).
    let pointer = stored(None::<(f32, f32)>);
    let retry = signal(0_u32);
    dynamic(move || {
        retry.get();
        let (from, _) = open.get()?;
        let Some(caret) = ctx.editor.with_untracked(|e| e.session.page_coords_at_pos(from)) else {
            let _ = set_timeout(std::time::Duration::from_millis(16), move || retry.update(|n| *n += 1));
            return None;
        };
        let found = rows();
        if found.is_empty() {
            return None;
        }
        let height = window_height.get();
        let window_width = window.state().map(|s| s.bounds.size.width.0).unwrap_or(f32::MAX);
        let menu_width = 240.0_f32.min(window_width - 8.0);
        let left = caret.origin.x.0.min(window_width - menu_width - 4.0).max(4.0);
        let below = caret.origin.y.0 + caret.size.height.0 + 4.0;
        let wanted = found.len() as f32 * 34.0 + 10.0;
        let menu = view()
            .absolute()
            .left(px(left))
            .w(px(menu_width))
            .p(px(4.0))
            .rounded(px(10.0))
            .overflow_hidden()
            .border_1()
            .border_color(move || theme().border)
            .bg(move || theme().popover)
            .text_color(move || theme().popover_foreground)
            .shadow_lg();
        // Under the `[[`, or over it when there's more room there.
        let menu = if below + wanted <= height || caret.origin.y.0 < height / 2.0 {
            menu.top(px(below))
        } else {
            menu.bottom(px(height - caret.origin.y.0 + 4.0))
        };
        let pick = pick.clone();
        Some(menu.children(found.into_iter().enumerate().map(move |(i, choice)| {
            let pick = pick.clone();
            let picked = choice.clone();
            let (label, hint) = match &choice {
                Choice::Note { title, category, .. } => (title.clone(), category.clone()),
                Choice::New(title) => (format!("New note “{title}”"), String::new()),
            };
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .h(px(32.0))
                .px_2()
                .rounded(px(6.0))
                .bg(move || if index.get() == i { theme().accent } else { theme().popover })
                .focusable(false)
                .on_mouse_move(move |e: &MouseEvent| {
                    let at = (e.page_position.x.0, e.page_position.y.0);
                    if pointer.get().is_some_and(|last| last != at) {
                        index.set(i);
                    }
                    pointer.replace(Some(at));
                })
                .cursor(Cursor::Pointer).on_press(move || pick(picked.clone()))
                .child(text(label).flex_1().truncate().text_size(px(13.0)))
                .child(text(hint).truncate().text_size(px(11.0)).text_color(move || theme().muted_foreground))
                .into_element()
        })))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::{Format, html_fragment};
    use crate::kit::{Kit, Relay};

    fn notes() -> Vec<(Rc<str>, String, String)> {
        [("a", "Shopping", ""), ("b", "Shop hours", "Work"), ("c", "Workshop", ""), ("d", "Ideas", "")]
            .into_iter()
            .map(|(id, title, category)| (Rc::from(id), title.to_string(), category.to_string()))
            .collect()
    }

    fn titles(choices: &[Choice]) -> Vec<String> {
        choices
            .iter()
            .map(|c| match c {
                Choice::Note { title, .. } => title.clone(),
                Choice::New(title) => format!("+{title}"),
            })
            .collect()
    }

    #[test]
    fn the_menu_lists_matching_notes_and_offers_a_new_one() {
        assert_eq!(titles(&choices(&notes(), "")), ["Ideas", "Shop hours", "Shopping", "Workshop"]);
        assert_eq!(titles(&choices(&notes(), "shop")), ["Shop hours", "Shopping", "Workshop", "+shop"]);
        // A note of exactly that name: no new one offered. `]]` isn't part of the name.
        assert_eq!(titles(&choices(&notes(), "ideas]]")), ["Ideas"]);
        assert_eq!(titles(&choices(&notes(), "zzz")), ["+zzz"]);
    }

    #[test]
    fn a_link_round_trips_through_the_note_file_and_reads_as_brackets() {
        let kit = Kit::new(Relay::default());
        let html = r#"<p>See <span data-note-link="a1">Shopping</span> first</p>"#;
        let doc = Format::Html.parse(&kit.schema, html).unwrap();
        let mut found = None;
        doc.descendants(&mut |node, _, _, _| {
            if node.ty().name() == NODE {
                found = Some((node.attrs().get("noteId").cloned(), node.attrs().get("title").cloned()));
            }
            true
        });
        assert_eq!(found, Some((Some(json!("a1")), Some(json!("Shopping")))));
        assert_eq!(html_fragment(&doc).unwrap(), html);
        assert_eq!(Format::Text.serialize(&doc).unwrap().trim(), "See [[Shopping]] first");
        assert_eq!(Format::Markdown.serialize(&doc).unwrap().trim(), "See [[Shopping]] first");
    }

    #[test]
    fn a_link_fragment_is_the_link_and_a_space() {
        let kit = Kit::new(Relay::default());
        let fragment = link_fragment(&kit.schema, "a1", "Shopping").unwrap();
        assert_eq!(fragment.child_count(), 2);
        assert_eq!(fragment.child(0).ty().name(), NODE);
    }
}
