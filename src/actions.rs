//! Every editing action, once: its name, glyph, hint, search words, what it
//! runs and when it shows as active. The toolbar, the slash menu, the
//! selection menu and its "…" menu are lists of these ids.

use nsh_base::rich_editor::commands::BoxCommand;
use serde_json::{Value, json};

use crate::editor_view::media;
use crate::kit::Editor;
use crate::toggle;

/// What an action does.
pub enum Run {
    /// A named editor command with its JSON arguments.
    Command(&'static str, fn() -> Value),
    /// A command made here.
    Make(fn() -> BoxCommand),
    /// Indents (`true`) or outdents the list item at the selection, of
    /// whichever list it's in.
    ListIndent(bool),
    /// Removes marks, then turns the blocks back into paragraphs.
    ClearFormatting,
    /// Needs the app's UI (a dialog, an input).
    App(AppAction),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AppAction {
    InsertImage,
    EditLink,
}

/// A node or mark name, and the attributes it must have (when they matter).
pub type ActiveWhen = (&'static str, fn() -> Option<Value>);

pub struct Action {
    pub id: &'static str,
    pub label: &'static str,
    /// What a compact button shows.
    pub glyph: &'static str,
    pub hint: &'static str,
    /// Words the slash menu finds it by (besides its label).
    pub keywords: &'static [&'static str],
    pub run: Run,
    /// The node or mark that's active while the selection is in or has it,
    /// with attributes when they matter.
    pub active: Option<ActiveWhen>,
}

const fn none() -> Option<Value> {
    None
}

macro_rules! cmd {
    ($name:literal) => {
        Run::Command($name, || json!({}))
    };
    ($name:literal, $args:expr) => {
        Run::Command($name, || $args)
    };
}

pub const ACTIONS: &[Action] = &[
    Action { id: "undo", label: "Undo", glyph: "↶", hint: "Ctrl/⌘+Z", keywords: &[], run: cmd!("undo"), active: None },
    Action { id: "redo", label: "Redo", glyph: "↷", hint: "Ctrl/⌘+Shift+Z", keywords: &[], run: cmd!("redo"), active: None },
    // Blocks.
    Action { id: "text", label: "Text", glyph: "¶", hint: "Plain paragraph", keywords: &["paragraph", "p", "plain"], run: cmd!("setParagraph"), active: Some(("paragraph", none)) },
    Action { id: "h1", label: "Heading 1", glyph: "H1", hint: "Big section heading", keywords: &["h1", "title"], run: cmd!("toggleHeading", json!({ "level": 1 })), active: Some(("heading", || Some(json!({ "level": 1 })))) },
    Action { id: "h2", label: "Heading 2", glyph: "H2", hint: "Medium section heading", keywords: &["h2", "subtitle"], run: cmd!("toggleHeading", json!({ "level": 2 })), active: Some(("heading", || Some(json!({ "level": 2 })))) },
    Action { id: "h3", label: "Heading 3", glyph: "H3", hint: "Small section heading", keywords: &["h3"], run: cmd!("toggleHeading", json!({ "level": 3 })), active: Some(("heading", || Some(json!({ "level": 3 })))) },
    Action { id: "bullet", label: "Bulleted list", glyph: "•", hint: "A simple list", keywords: &["ul", "bullet", "unordered"], run: cmd!("toggleBulletList"), active: Some(("bulletList", none)) },
    Action { id: "numbered", label: "Numbered list", glyph: "1.", hint: "A list with numbers", keywords: &["ol", "ordered", "number"], run: cmd!("toggleOrderedList"), active: Some(("orderedList", none)) },
    Action { id: "todo", label: "Checklist", glyph: "☑", hint: "Tasks with checkboxes", keywords: &["todo", "task", "checkbox", "check"], run: cmd!("toggleTaskList"), active: Some(("taskList", none)) },
    Action { id: "toggle", label: "Toggle list", glyph: "▸", hint: "A title whose content folds away", keywords: &["toggle", "collapse", "fold", "details"], run: Run::Make(toggle::make_toggle), active: Some(("toggle", none)) },
    Action { id: "quote", label: "Quote", glyph: "❝", hint: "A quotation", keywords: &["blockquote", "citation"], run: cmd!("toggleBlockquote"), active: Some(("blockquote", none)) },
    Action { id: "codeblock", label: "Code block", glyph: "{ }", hint: "Monospaced code", keywords: &["code", "pre", "snippet"], run: cmd!("toggleCodeBlock"), active: Some(("codeBlock", none)) },
    Action { id: "table", label: "Table", glyph: "⊞", hint: "3 × 3 with a header row", keywords: &["grid"], run: cmd!("insertTable", json!({ "rows": 3, "cols": 3, "withHeaderRow": true })), active: None },
    Action { id: "image", label: "Image", glyph: "🖼", hint: "A picture from your computer", keywords: &["picture", "photo", "img"], run: Run::App(AppAction::InsertImage), active: None },
    Action { id: "youtube", label: "YouTube video", glyph: "▶", hint: "A card for a YouTube link", keywords: &["video", "youtube", "embed", "film"], run: Run::Make(media::youtube_command), active: Some(("youtube", none)) },
    Action { id: "divider", label: "Divider", glyph: "—", hint: "A horizontal line", keywords: &["hr", "rule", "separator", "line"], run: cmd!("setHorizontalRule"), active: None },
    // Marks.
    Action { id: "bold", label: "Bold", glyph: "B", hint: "Ctrl/⌘+B", keywords: &[], run: cmd!("toggleBold"), active: Some(("bold", none)) },
    Action { id: "italic", label: "Italic", glyph: "I", hint: "Ctrl/⌘+I", keywords: &[], run: cmd!("toggleItalic"), active: Some(("italic", none)) },
    Action { id: "underline", label: "Underline", glyph: "U", hint: "Ctrl/⌘+U", keywords: &[], run: cmd!("toggleUnderline"), active: Some(("underline", none)) },
    Action { id: "strike", label: "Strikethrough", glyph: "S", hint: "Ctrl/⌘+Shift+S", keywords: &[], run: cmd!("toggleStrike"), active: Some(("strike", none)) },
    Action { id: "code", label: "Inline code", glyph: "</>", hint: "Ctrl/⌘+E", keywords: &[], run: cmd!("toggleCode"), active: Some(("code", none)) },
    Action { id: "highlight", label: "Highlight", glyph: "🖍", hint: "Ctrl/⌘+Shift+H", keywords: &[], run: cmd!("toggleHighlight"), active: Some(("highlight", none)) },
    Action { id: "subscript", label: "Subscript", glyph: "x₂", hint: "", keywords: &[], run: cmd!("toggleSubscript"), active: Some(("subscript", none)) },
    Action { id: "superscript", label: "Superscript", glyph: "x²", hint: "", keywords: &[], run: cmd!("toggleSuperscript"), active: Some(("superscript", none)) },
    Action { id: "link", label: "Link", glyph: "🔗", hint: "Ctrl/⌘+K", keywords: &["url", "href"], run: Run::App(AppAction::EditLink), active: Some(("link", none)) },
    Action { id: "clear", label: "Clear formatting", glyph: "⌫", hint: "Plain text again", keywords: &[], run: Run::ClearFormatting, active: None },
    // Layout.
    Action { id: "outdent", label: "Decrease indent", glyph: "⇤", hint: "Shift+Tab in a list", keywords: &[], run: Run::ListIndent(false), active: None },
    Action { id: "indent", label: "Increase indent", glyph: "⇥", hint: "Tab in a list", keywords: &[], run: Run::ListIndent(true), active: None },
    Action { id: "align-left", label: "Align left", glyph: "↤", hint: "Ctrl/⌘+Shift+L", keywords: &[], run: cmd!("setTextAlign", json!("left")), active: Some(("paragraph", || Some(json!({ "textAlign": "left" })))) },
    Action { id: "align-center", label: "Center", glyph: "↔", hint: "Ctrl/⌘+Shift+E", keywords: &[], run: cmd!("setTextAlign", json!("center")), active: Some(("paragraph", || Some(json!({ "textAlign": "center" })))) },
    Action { id: "align-right", label: "Align right", glyph: "↦", hint: "Ctrl/⌘+Shift+R", keywords: &[], run: cmd!("setTextAlign", json!("right")), active: Some(("paragraph", || Some(json!({ "textAlign": "right" })))) },
];

/// Where each action appears.
pub const TOOLBAR: &[&[&str]] = &[
    &["undo", "redo"],
    &["text", "h1", "h2", "h3"],
    &["bold", "italic", "underline", "strike", "code", "highlight", "subscript", "superscript", "link"],
    &["bullet", "numbered", "todo", "toggle", "outdent", "indent"],
    &["quote", "codeblock", "table", "image", "youtube", "divider"],
    &["align-left", "align-center", "align-right", "clear"],
];
pub const SLASH: &[&str] =
    &["text", "h1", "h2", "h3", "bullet", "numbered", "todo", "toggle", "quote", "codeblock", "table", "image", "youtube", "divider"];
pub const SELECTION: &[&[&str]] =
    &[&["bullet", "numbered", "todo", "toggle"], &["bold", "italic", "underline", "strike"], &["outdent", "indent"], &["link"]];
pub const SELECTION_MORE_INSERT: &[&str] = &["table", "image", "youtube", "divider", "codeblock", "quote"];
pub const SELECTION_MORE_FORMAT: &[&str] = &["code", "highlight", "clear"];

/// The text colors: a CSS color for the document, and the class the editor
/// draws it with.
pub const COLORS: &[(&str, &str, u32)] = &[
    ("#dc2626", "text-red-600", 0xdc2626),
    ("#2563eb", "text-blue-600", 0x2563eb),
    ("#16a34a", "text-green-600", 0x16a34a),
];

pub fn get(id: &str) -> &'static Action {
    ACTIONS.iter().find(|a| a.id == id).unwrap_or_else(|| panic!("no action {id:?}"))
}

impl Action {
    /// Runs it on the editor; an app action goes to `app`.
    pub fn perform(&self, editor: &Editor, app: &dyn Fn(AppAction)) {
        match &self.run {
            Run::Command(name, args) => {
                editor.run(name, args());
            }
            Run::Make(make) => {
                editor.run_command(&make());
            }
            Run::ListIndent(deeper) => {
                editor.run(if *deeper { "sinkListItem" } else { "liftListItem" }, list_item(editor));
            }
            Run::ClearFormatting => {
                editor.run("unsetAllMarks", json!({}));
                editor.run("clearNodes", json!({}));
            }
            Run::App(action) => app(*action),
        }
    }

    /// Whether it could do something at the selection now (dimmed if not).
    pub fn enabled(&self, editor: &Editor) -> bool {
        match &self.run {
            Run::ListIndent(deeper) => {
                editor.can(if *deeper { "sinkListItem" } else { "liftListItem" }, &list_item(editor))
            }
            _ => true,
        }
    }

    pub fn is_active(&self, editor: &Editor) -> bool {
        self.active.is_some_and(|(name, attrs)| editor.is_active(name, attrs()))
    }

    /// Whether the slash menu finds it for `query`, and how well (a word
    /// starting with it is better than one containing it).
    pub fn matches(&self, query: &str) -> Option<bool> {
        let query = query.trim().to_lowercase();
        let words = std::iter::once(self.label.to_lowercase()).chain(self.keywords.iter().map(|k| k.to_string()));
        let words: Vec<String> = words.collect();
        if words.iter().any(|w| w.starts_with(&query)) {
            Some(true)
        } else if words.iter().any(|w| w.contains(&query)) {
            Some(false)
        } else {
            None
        }
    }
}

/// The list item type at the selection: a checklist's or another list's.
fn list_item(editor: &Editor) -> Value {
    json!({ "name": if editor.is_active("taskItem", None) { "taskItem" } else { "listItem" } })
}

/// The slash menu's actions for `query`, those with a word starting with it
/// first.
pub fn slash_matches(query: &str) -> Vec<&'static Action> {
    let mut found: Vec<(bool, &'static Action)> =
        SLASH.iter().map(|id| get(id)).filter_map(|a| a.matches(query).map(|starts| (starts, a))).collect();
    found.sort_by_key(|(starts, _)| !*starts);
    found.into_iter().map(|(_, a)| a).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_id_exists() {
        let lists: Vec<&[&str]> = TOOLBAR
            .iter()
            .copied()
            .chain(SELECTION.iter().copied())
            .chain([SLASH, SELECTION_MORE_INSERT, SELECTION_MORE_FORMAT])
            .collect();
        for list in lists {
            for id in list {
                get(id);
            }
        }
    }

    #[test]
    fn slash_search_prefers_word_starts() {
        let names: Vec<_> = slash_matches("he").iter().map(|a| a.id).collect();
        assert_eq!(&names[..3], &["h1", "h2", "h3"]);
        assert_eq!(slash_matches("todo")[0].id, "todo");
        assert_eq!(slash_matches("toggle")[0].id, "toggle");
        assert_eq!(slash_matches("you")[0].id, "youtube");
        assert_eq!(slash_matches("video")[0].id, "youtube");
        assert!(slash_matches("zzz").is_empty());
    }
}
