//! Commands on the block at the caret, and on the text there: move it up or
//! down, duplicate it, delete it; put today's date or the time in.
//!
//! "The block" is what a reader sees as one: the paragraph or heading at the
//! caret, a list item whose first line it is (with what's nested in it), or
//! a toggle whose title it is (with its body).

use std::sync::Arc;

use nsh_base::rich_editor::commands::BoxCommand;
use nsh_base::rich_editor::model::{Fragment, ResolvedPos, Slice};
use nsh_base::rich_editor::state::{EditorState, Selection, TextSelection};

/// The depth of the block at the caret (at least 1), by the rule above.
fn block_depth(from: &ResolvedPos) -> Option<usize> {
    let mut depth = from.depth;
    if depth == 0 {
        return None;
    }
    if from.node(depth).ty().name() == "toggleTitle" && depth >= 2 {
        depth -= 1;
    } else if depth >= 2
        && matches!(from.node(depth - 1).ty().name(), "listItem" | "taskItem")
        && from.index(depth - 1) == 0
    {
        depth -= 1;
    }
    Some(depth)
}

/// The selection moved by `delta`, as it was (a caret or a range).
fn shifted(state: &EditorState, doc: &nsh_base::rich_editor::model::Node, delta: isize) -> Option<Selection> {
    let selection = state.selection();
    let at = |pos: usize| doc.resolve(pos.checked_add_signed(delta)?).ok();
    Some(TextSelection::new(at(selection.anchor())?, at(selection.head())?).into())
}

/// Swaps the block at the caret with the one before it (`up`) or after it.
fn move_block(up: bool) -> BoxCommand {
    Arc::new(move |state, dispatch| {
        let from = state.selection().resolved_from().clone();
        let Some(depth) = block_depth(&from) else { return false };
        let parent = from.node(depth - 1);
        let index = from.index(depth - 1);
        let block = parent.child(index).clone();
        let (start, end) = (from.before(depth), from.after(depth));
        let other_index = if up { index.checked_sub(1) } else { Some(index + 1) };
        let Some(other) = other_index.and_then(|i| parent.maybe_child(i)).cloned() else { return false };
        let Some(dispatch) = dispatch else { return true };
        let size = other.node_size();
        let mut tr = state.tr();
        let (range, content, delta) = if up {
            ((start - size, end), vec![block, other], -(size as isize))
        } else {
            ((start, end + size), vec![other, block], size as isize)
        };
        if tr.replace_with(range.0, range.1, Fragment::from(content)).is_err() {
            return false;
        }
        if let Some(selection) = shifted(state, tr.doc(), delta) {
            tr.set_selection(selection);
        }
        tr.scroll_into_view();
        dispatch(tr);
        true
    })
}

pub fn move_up() -> BoxCommand {
    move_block(true)
}

pub fn move_down() -> BoxCommand {
    move_block(false)
}

/// Puts a copy of the block at the caret after it; the caret goes into the
/// copy, where it was in the block.
pub fn duplicate() -> BoxCommand {
    Arc::new(|state, dispatch| {
        let from = state.selection().resolved_from().clone();
        let Some(depth) = block_depth(&from) else { return false };
        let block = from.node(depth).clone();
        let Some(dispatch) = dispatch else { return true };
        let size = block.node_size();
        let mut tr = state.tr();
        if tr.insert_node(from.after(depth), block).is_err() {
            return false;
        }
        if let Some(selection) = shifted(state, tr.doc(), size as isize) {
            tr.set_selection(selection);
        }
        tr.scroll_into_view();
        dispatch(tr);
        true
    })
}

/// Deletes the block at the caret. A block alone in its list or quote takes
/// that with it; the note's only block becomes an empty paragraph.
pub fn delete() -> BoxCommand {
    Arc::new(|state, dispatch| {
        let from = state.selection().resolved_from().clone();
        let Some(mut depth) = block_depth(&from) else { return false };
        while depth > 1 && from.node(depth - 1).child_count() == 1 {
            depth -= 1;
        }
        let Some(dispatch) = dispatch else { return true };
        let (start, end) = (from.before(depth), from.after(depth));
        let mut tr = state.tr();
        let done = if depth == 1 && from.node(0).child_count() == 1 {
            let Some(paragraph) = state.schema().node_type("paragraph").and_then(|ty| ty.create(None, Fragment::empty(), Vec::new()).ok())
            else {
                return false;
            };
            tr.replace_with_node(start, end, paragraph).is_ok()
        } else {
            tr.delete(start, end).is_ok()
        };
        if !done {
            return false;
        }
        let at = start.min(tr.doc().content().size());
        if let Ok(pos) = tr.doc().resolve(at) {
            tr.set_selection(Selection::near(&pos, 1));
        }
        tr.scroll_into_view();
        dispatch(tr);
        true
    })
}

/// Puts `text` in place of the selection.
fn insert(text: String) -> BoxCommand {
    Arc::new(move |state, dispatch| {
        if !state.selection().resolved_from().parent().is_textblock() {
            return false;
        }
        let Some(dispatch) = dispatch else { return true };
        let mut tr = state.tr();
        tr.insert_text(&text, None, None);
        tr.scroll_into_view();
        dispatch(tr);
        true
    })
}

/// Puts plain `text` in place of the selection, each line its own
/// paragraph: the first line joins the text before the selection, the last
/// the text after it, as pasting does.
pub fn paste_text(text: &str) -> BoxCommand {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    Arc::new(move |state, dispatch| {
        if !state.selection().resolved_from().parent().is_textblock() {
            return false;
        }
        let Some(dispatch) = dispatch else { return true };
        let mut tr = state.tr();
        if text.contains('\n') {
            let Some(paragraph) = state.schema().node_type("paragraph") else { return false };
            let lines: Option<Vec<_>> = text
                .split('\n')
                .map(|line| {
                    let content = if line.is_empty() { Fragment::empty() } else { Fragment::from(vec![state.schema().text(line, vec![])]) };
                    paragraph.create(None, content, Vec::new()).ok()
                })
                .collect();
            let Some(lines) = lines else { return false };
            tr.replace_selection(Slice::new(Fragment::from(lines), 1, 1));
        } else {
            tr.insert_text(&text, None, None);
        }
        tr.scroll_into_view();
        dispatch(tr);
        true
    })
}

/// Edits the link over `from..to`: `text` (when given) replaces what's
/// there, keeping its other marks; `href` is the address (empty: the link
/// is removed). The caret goes after it.
pub fn set_link(from: usize, to: usize, text: Option<String>, href: String) -> BoxCommand {
    Arc::new(move |state, dispatch| {
        let doc = state.doc();
        if to > doc.content().size() || from > to {
            return false;
        }
        let Some(link_type) = state.schema().mark_type("link") else { return false };
        let link = if href.is_empty() {
            None
        } else {
            let attrs = [("href".to_string(), serde_json::Value::String(href.clone()))].into_iter().collect();
            match state.schema().mark("link", Some(attrs)) {
                Ok(mark) => Some(mark),
                Err(_) => return false,
            }
        };
        if from == to && (link.is_none() || text.as_deref().is_none_or(str::is_empty)) {
            return false;
        }
        let Some(dispatch) = dispatch else { return true };
        let mut tr = state.tr();
        let mut end = to;
        if let Some(text) = text.as_deref().filter(|t| !t.is_empty()) {
            // The marks the text had (at its start), the link's replaced.
            let at = doc.resolve(if from < to { from + 1 } else { from }).map(|p| p.marks()).unwrap_or_default();
            let mut marks: Vec<_> = at.into_iter().filter(|m| m.ty().name() != "link").collect();
            if let Some(link) = &link {
                marks = link.add_to_set(&marks);
            }
            let node = state.schema().text(text, marks);
            if tr.replace_with(from, to, Fragment::from(vec![node])).is_err() {
                return false;
            }
            end = from + text.chars().map(char::len_utf16).sum::<usize>();
        } else {
            let removed = tr.remove_mark(from, to, nsh_base::rich_editor::transform::MarkMatch::Type(link_type)).is_ok();
            if !removed || link.as_ref().is_some_and(|link| tr.add_mark(from, to, link.clone()).is_err()) {
                return false;
            }
        }
        if let Ok(at) = tr.doc().resolve(end) {
            tr.set_selection(TextSelection::new(at.clone(), at).into());
        }
        tr.set_stored_marks(Some(Vec::new()));
        tr.scroll_into_view();
        dispatch(tr);
        true
    })
}

/// Text typed at the caret next has no marks (not those around it).
pub fn no_stored_marks() -> BoxCommand {
    Arc::new(|state, dispatch| {
        if !state.selection().empty() {
            return false;
        }
        let Some(dispatch) = dispatch else { return true };
        let mut tr = state.tr();
        tr.set_stored_marks(Some(Vec::new()));
        dispatch(tr);
        true
    })
}

/// Today's date, as "November 16, 2026".
pub fn insert_date() -> BoxCommand {
    insert(chrono::Local::now().format("%B %-d, %Y").to_string())
}

/// The time now, as "3:42 PM".
pub fn insert_time() -> BoxCommand {
    insert(chrono::Local::now().format("%-I:%M %p").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::{Kit, Relay};
    use serde_json::json;

    fn doc_text(editor: &crate::kit::Editor) -> Vec<String> {
        let doc = editor.doc();
        (0..doc.child_count()).map(|i| doc.child(i).text_content()).collect()
    }

    fn editor_with(lines: &[&str]) -> crate::kit::Editor {
        let kit = Kit::new(Relay::default());
        let content: Vec<_> = lines.iter().map(|l| json!({ "type": "paragraph", "content": [{ "type": "text", "text": l }] })).collect();
        let doc = kit.schema.node_from_json(&json!({ "type": "doc", "content": content })).unwrap();
        kit.editor(Some(doc))
    }

    /// Puts the caret in the `index`th top-level block, one character in.
    fn caret_in(editor: &crate::kit::Editor, index: usize) {
        let doc = editor.doc();
        let pos: usize = (0..index).map(|i| doc.child(i).node_size()).sum::<usize>() + 2;
        let state = editor.state();
        let mut tr = state.tr();
        tr.set_selection(TextSelection::create(state.doc(), pos, None));
        editor.session.dispatch(tr).unwrap();
    }

    #[test]
    fn moves_duplicates_and_deletes_blocks() {
        let editor = editor_with(&["a", "b", "c"]);
        caret_in(&editor, 1);
        assert!(editor.run_command(&move_up()));
        assert_eq!(doc_text(&editor), ["b", "a", "c"]);
        assert!(!editor.run_command(&move_up()));
        assert!(editor.run_command(&move_down()));
        assert!(editor.run_command(&move_down()));
        assert_eq!(doc_text(&editor), ["a", "c", "b"]);
        assert!(editor.run_command(&duplicate()));
        assert_eq!(doc_text(&editor), ["a", "c", "b", "b"]);
        assert!(editor.run_command(&delete()));
        assert!(editor.run_command(&delete()));
        assert!(editor.run_command(&delete()));
        assert!(editor.run_command(&delete()));
        assert_eq!(doc_text(&editor), [""]);
    }

    /// What "Paste as plain text" runs: each line of the text a paragraph.
    #[test]
    fn plain_text_paste_makes_paragraphs() {
        let editor = editor_with(&["ab"]);
        caret_in(&editor, 0);
        assert!(editor.run_command(&paste_text("one\r\ntwo")));
        assert_eq!(doc_text(&editor), ["aone", "twob"]);
        assert!(editor.run_command(&paste_text("!")));
        assert_eq!(doc_text(&editor), ["aone", "two!b"]);
    }

    /// Clear formatting at a caret inside highlighted text: what's typed
    /// next is plain.
    #[test]
    fn clearing_at_a_caret_types_plain() {
        let editor = editor_with(&["ab"]);
        caret_in(&editor, 0);
        assert!(editor.run("toggleHighlight", json!({})));
        assert!(editor.run_command(&insert("x".into())));
        crate::actions::get("clear").perform(&editor, &|_| {});
        assert!(editor.run_command(&insert("y".into())));
        let paragraph = editor.doc().child(0).clone();
        let marked: Vec<String> = (0..paragraph.child_count())
            .map(|i| paragraph.child(i))
            .filter(|n| !n.marks().is_empty())
            .map(|n| n.text_content())
            .collect();
        assert_eq!(marked, ["x"]);
    }

    /// The texts with a link mark, and their addresses.
    fn links(editor: &crate::kit::Editor) -> Vec<(String, String)> {
        let paragraph = editor.doc().child(0).clone();
        (0..paragraph.child_count())
            .map(|i| paragraph.child(i).clone())
            .filter_map(|n| {
                let link = n.marks().iter().find(|m| m.ty().name() == "link")?.clone();
                Some((n.text_content(), link.attrs()["href"].as_str()?.to_string()))
            })
            .collect()
    }

    /// "/link": linked text at the caret; what's typed after isn't linked.
    #[test]
    fn inserted_links_end_at_the_link() {
        let editor = editor_with(&["ab"]);
        caret_in(&editor, 0);
        assert!(editor.run_command(&set_link(2, 2, Some("site".into()), "https://example.com".into())));
        assert!(editor.run_command(&insert("!".into())));
        assert_eq!(editor.doc().child(0).text_content(), "asite!b");
        assert_eq!(links(&editor), [("site".to_string(), "https://example.com".to_string())]);
    }

    /// Editing a link: its address alone, its text and address, removing it.
    #[test]
    fn links_are_edited_in_place() {
        let editor = editor_with(&["ab"]);
        caret_in(&editor, 0);
        assert!(editor.run_command(&set_link(2, 2, Some("site".into()), "https://a.com".into())));
        // "site" is at 2..6.
        assert!(editor.run_command(&set_link(2, 6, None, "https://b.com".into())));
        assert_eq!(links(&editor), [("site".to_string(), "https://b.com".to_string())]);
        assert!(editor.run_command(&set_link(2, 6, Some("page".into()), "https://c.com".into())));
        assert_eq!(editor.doc().child(0).text_content(), "apageb");
        assert_eq!(links(&editor), [("page".to_string(), "https://c.com".to_string())]);
        assert!(editor.run_command(&set_link(2, 6, None, String::new())));
        assert!(links(&editor).is_empty());
        assert_eq!(editor.doc().child(0).text_content(), "apageb");
    }

    /// What the Link button runs first in a link: part of it selected
    /// grows to the whole link.
    #[test]
    fn a_link_is_edited_whole() {
        let editor = editor_with(&["ab"]);
        caret_in(&editor, 0);
        assert!(editor.run_command(&set_link(2, 2, Some("site".into()), "https://a.com".into())));
        let state = editor.state();
        let mut tr = state.tr();
        tr.set_selection(TextSelection::create(state.doc(), 3, Some(5)));
        editor.session.dispatch(tr).unwrap();
        assert!(editor.run("extendMarkRange", json!({ "name": "link" })));
        let after = editor.state();
        assert_eq!((after.selection().from(), after.selection().to()), (2, 6));
    }

    /// Text styles: one attribute set or taken off keeps the others; with
    /// none left the style goes.
    #[test]
    fn text_style_attributes_are_kept_apart() {
        use crate::actions::{set_style, style_of};
        let editor = editor_with(&["abcd"]);
        let state = editor.state();
        let mut tr = state.tr();
        tr.set_selection(TextSelection::create(state.doc(), 2, Some(4)));
        editor.session.dispatch(tr).unwrap();
        set_style(&editor, "color", Some("#dc2626"));
        set_style(&editor, "fontSize", Some("20px"));
        assert_eq!(style_of(&editor, "color").as_deref(), Some("#dc2626"));
        set_style(&editor, "color", None);
        assert_eq!(style_of(&editor, "color"), None);
        assert_eq!(style_of(&editor, "fontSize").as_deref(), Some("20px"));
        set_style(&editor, "fontSize", None);
        let styled = (0..editor.doc().child(0).child_count())
            .any(|i| editor.doc().child(0).child(i).marks().iter().any(|m| m.ty().name() == "textStyle"));
        assert!(!styled, "{}", editor.doc().to_json());
    }
}
