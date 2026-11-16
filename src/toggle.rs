//! Toggles (collapsible blocks), as Notion's and ClickUp's: a title line and
//! a body that hides. Three node types, after NSH's gallery block editor:
//! `toggle` (`toggleTitle toggleBody`), `toggleTitle` (inline content) and
//! `toggleBody` (blocks). `open` is on the toggle (its arrow) and on the
//! body (a class hides a closed one); a press on the arrow sets both in one
//! undoable step.
//!
//! In HTML a toggle is `<details>` with a `<summary>`, which browsers fold.
//! Markdown has no toggles: saving one as Markdown asks first.

use std::sync::Arc;

use nsh_base::rich_editor::{
    commands::BoxCommand,
    extensions::Extension,
    model::{AttrParse, AttrRender, AttrSpec, DomOutputSpec, Fragment, Node, NodeSpec, ParseRule},
    view::{NodeView, NodeViewCx, NodeViews},
};
use nsh_base::theme;
use nsh_core::prelude::*;
use serde_json::{Value, json};

/// The three node types, as one extension.
pub fn extension() -> Extension {
    // Before the editor's own Enter, which can't split a title and so does
    // nothing there.
    Extension::new("toggleKit").with_priority(1000).add_keyboard_shortcuts(|_| vec![("Enter".into(), enter_title())]).add_extensions(|_| {
        vec![
            Extension::node("toggle", |_| {
                let mut spec = NodeSpec::default();
                spec.content = Some("toggleTitle toggleBody".into());
                spec.group = Some("block".into());
                spec.defining = true;
                spec.attrs = vec![("open".into(), open_attr())];
                spec.parse_dom = vec![ParseRule::tag("details").with_get_attrs(|input| {
                    let open = input.element()?.has_attr("open");
                    Some([("open".to_string(), Value::Bool(open))].into_iter().collect())
                })];
                spec.to_dom = Some(DomOutputSpec::render(|node| {
                    let open = node.attr("open").and_then(Value::as_bool).unwrap_or(true);
                    let details = DomOutputSpec::tag("details").attr("data-toggle", "");
                    if open { details.attr("open", "") } else { details }.hole()
                }));
                spec
            }),
            Extension::node("toggleTitle", |_| {
                let mut spec = NodeSpec::default();
                spec.content = Some("inline*".into());
                spec.defining = true;
                spec.parse_dom = vec![ParseRule::tag("summary")];
                spec.to_dom = Some(DomOutputSpec::tag("summary").hole());
                spec
            }),
            Extension::node("toggleBody", |_| {
                let mut spec = NodeSpec::default();
                spec.content = Some("block+".into());
                spec.defining = true;
                spec.attrs = vec![("open".into(), open_attr())];
                spec.parse_dom = vec![ParseRule::tag("div[data-toggle-body]").with_get_attrs(|input| {
                    let open = input.element()?.attr("data-open") != Some("false");
                    Some([("open".to_string(), Value::Bool(open))].into_iter().collect())
                })];
                spec.to_dom = Some(DomOutputSpec::render(|node| {
                    let open = node.attr("open").and_then(Value::as_bool).unwrap_or(true);
                    DomOutputSpec::tag("div").attr("data-toggle-body", "").attr("data-open", open).hole()
                }));
                spec
            }),
        ]
    })
}

/// `open`, read and written by the types' own rules (`<details open>`,
/// `data-open`), not as an HTML attribute of its name: browsers take any
/// `open="…"` as open.
fn open_attr() -> AttrSpec {
    let mut attr = AttrSpec::default();
    attr.default = Some(Value::Bool(true));
    attr.parse_html = Some(AttrParse::Skip);
    attr.render_html = Some(AttrRender::Skip);
    attr
}

/// The toggle's arrow, drawn by a node view around the toggle's content.
pub fn node_views() -> NodeViews {
    NodeViews::new().with("toggle", Arc::new(ToggleView))
}

struct ToggleView;

impl NodeView for ToggleView {
    fn create(&self, cx: &NodeViewCx, _: &Node) -> AnyElement {
        let node = cx.node();
        let get_pos = cx.get_pos();
        let session = cx.session.clone();
        let is_open = move || node.with(|n| n.attrs().get("open").and_then(Value::as_bool).unwrap_or(true));
        view()
            .flex_row()
            .items_start()
            .child(
                view()
                    .w(px(22.0))
                    .h(px(22.0))
                    .mt(px(2.0))
                    .mr(px(4.0))
                    .items_center()
                    .justify_center()
                    .rounded(px(5.0))
                    .active(|s| s.opacity(0.5))
                    .label(move || if is_open() { "Collapse".to_string() } else { "Expand".to_string() })
                    .cursor(Cursor::Pointer).on_press(move || {
                        let Some(pos) = get_pos() else { return };
                        let state = session.state();
                        let Some(command) = set_open(state.doc(), pos, !is_open()) else { return };
                        session.run(&command);
                    })
                    .child(
                        text(move || if is_open() { "▾" } else { "▸" })
                            .text_size(px(14.0))
                            .text_color(move || theme().muted_foreground),
                    ),
            )
            .child(view().flex_1().children(cx.take_content()))
            .into_element()
    }

    /// The same element follows the toggle as it changes.
    fn update(&self, _: &Node, _: &[nsh_base::rich_editor::state::Decoration]) -> bool {
        true
    }

    fn editable_content_slot(&self) -> bool {
        true
    }
}

/// A command opening or closing the toggle at `pos`: its arrow and its body.
fn set_open(doc: &Node, pos: usize, open: bool) -> Option<BoxCommand> {
    let toggle = doc.node_at(pos)?;
    let body = pos + 1 + toggle.child(0).node_size();
    Some(Arc::new(move |state, dispatch| {
        let Some(dispatch) = dispatch else { return true };
        let mut tr = state.tr();
        if tr.set_node_attribute(pos, "open", Value::Bool(open)).is_err()
            || tr.set_node_attribute(body, "open", Value::Bool(open)).is_err()
        {
            return false;
        }
        dispatch(tr);
        true
    }))
}

/// Enter in a toggle's title: the caret goes into the body (opened), on a
/// new first line that takes what was after the caret in the title. An
/// empty first line already there is used as it is.
fn enter_title() -> BoxCommand {
    Arc::new(|state, dispatch| {
        let from = state.selection().resolved_from().clone();
        if from.parent().ty().name() != "toggleTitle" || !state.selection().empty() {
            return false;
        }
        let Some(dispatch) = dispatch else { return true };
        let (pos, end) = (from.pos, from.end(from.depth));
        let toggle = from.node(from.depth - 1);
        let toggle_pos = from.before(from.depth - 1);
        let tail = from.parent().content().cut(pos - from.start(from.depth), end - from.start(from.depth));
        let first_is_empty = toggle.child(1).first_child().is_some_and(|n| n.ty().name() == "paragraph" && n.content().size() == 0);
        let mut tr = state.tr();
        if tail.size() > 0 && tr.delete(pos, end).is_err() {
            return false;
        }
        // The title now ends at `pos`: the body starts after it, its first
        // block one further in.
        let body = pos + 1;
        if tail.size() > 0 || !first_is_empty {
            let Some(paragraph) = state.schema().node_type("paragraph").and_then(|ty| ty.create(None, tail, Vec::new()).ok()) else { return false };
            if tr.insert_node(body + 1, paragraph).is_err() {
                return false;
            }
        }
        let _ = tr.set_node_attribute(toggle_pos, "open", Value::Bool(true));
        let _ = tr.set_node_attribute(body, "open", Value::Bool(true));
        if let Ok(at) = tr.doc().resolve(body + 2) {
            tr.set_selection(nsh_base::rich_editor::state::TextSelection::new(at.clone(), at).into());
        }
        tr.scroll_into_view();
        dispatch(tr);
        true
    })
}

/// A command turning the text block at the selection into a toggle whose
/// title is that block's text, with an empty body. The caret goes into the
/// body, or into the title when the block was empty (a new toggle from the
/// slash menu starts with its title).
pub fn make_toggle() -> BoxCommand {
    Arc::new(|state, dispatch| {
        let from = state.selection().resolved_from().clone();
        let block = from.parent();
        if !block.is_textblock() || from.depth == 0 {
            return false;
        }
        let Some(dispatch) = dispatch else { return true };
        let (start, end) = (from.before(from.depth), from.after(from.depth));
        let title = block.to_json().get("content").cloned().unwrap_or(json!([]));
        let toggle = json!({
            "type": "toggle",
            "attrs": { "open": true },
            "content": [
                { "type": "toggleTitle", "content": title },
                { "type": "toggleBody", "attrs": { "open": true }, "content": [{ "type": "paragraph" }] },
            ],
        });
        let Ok(node) = state.schema().node_from_json(&toggle) else { return false };
        let title_size = node.child(0).node_size();
        let mut tr = state.tr();
        if tr.replace_with(start, end, Fragment::from(node)).is_err() {
            return false;
        }
        // Into the title (past the toggle's start and the title's), or the
        // body's paragraph (past the title, the body's start and the
        // paragraph's start).
        let caret = if block.child_count() == 0 { start + 2 } else { start + 1 + title_size + 2 };
        if let Ok(at) = tr.doc().resolve(caret) {
            tr.set_selection(nsh_base::rich_editor::state::TextSelection::new(at.clone(), at).into());
        }
        dispatch(tr);
        true
    })
}

/// The editor's classes for toggles: room for the arrow comes from the node
/// view; a closed body is hidden.
pub const CLASSES: &[(&str, &str)] = &[
    ("toggle", "gap-1"),
    ("toggleTitle", "leading-relaxed font-medium"),
    ("toggleBody", "gap-2"),
    ("toggleBody[open=false]", "hidden"),
];
