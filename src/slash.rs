//! The slash menu's plugin: NSH's port of Tiptap's `Suggestion`, following
//! a `/` typed at a line's start or after a space (not inside a code
//! block). While its menu is open it takes ↑, ↓, Enter and Tab (recorded in
//! its state for the app to read); Escape closes it.

use std::sync::Arc;

use nsh_base::rich_editor::{
    extensions::suggestion::{
        SuggestionOptions, SuggestionState, SuggestionTrigger, replace_suggestion, suggestion, suggestion_state,
    },
    model::Fragment,
    state::{EditorState, Plugin},
};

use crate::kit::Editor;

/// The plugin's key.
const KEY: &str = "slash";

pub fn plugin() -> Plugin {
    let mut trigger = SuggestionTrigger::default();
    trigger.char = "/".into();
    let mut options = SuggestionOptions::default();
    options.key = KEY.into();
    options.trigger = trigger;
    options.capture_keys = ["ArrowUp", "ArrowDown", "Enter", "Tab"].map(String::from).to_vec();
    options.allow = Some(Arc::new(|state: &EditorState, range, _| {
        state.doc().resolve(range.from).is_ok_and(|at| at.parent().ty().name() != "codeBlock")
    }));
    suggestion(options)
}

/// The plugin's state in `editor`.
pub fn state(editor: &Editor) -> Option<Arc<SuggestionState>> {
    suggestion_state(&editor.state(), KEY)
}

/// The open menu: where its `/` is, and the text typed after it.
pub fn open(state: &SuggestionState) -> Option<(usize, String)> {
    state.active.then(|| (state.range.from, state.query.clone().unwrap_or_default()))
}

/// Removes the `/` and what was typed after it.
pub fn remove_query(editor: &Editor) {
    editor.session.run(&replace_suggestion(KEY, Fragment::empty()));
}
