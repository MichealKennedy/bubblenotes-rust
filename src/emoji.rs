//! Emoji by name: typing `:` (at a line's start or after a space) and two
//! letters or more opens a menu of the emoji whose name or keywords match;
//! picking one, or typing the closing `:` (`:smile:`), puts it in place of
//! what was typed. A `:` inside a word or a time (`10:30`) doesn't open it.
//! The names and keywords are NSH's emoji picker's (Unicode CLDR).

use std::sync::{Arc, OnceLock};

use nsh_base::rich_editor::{
    extensions::suggestion::{SuggestionOptions, SuggestionState, SuggestionTrigger, replace_suggestion, suggestion, suggestion_state},
    model::Fragment,
    state::{EditorState, Plugin},
};
use nsh_base::theme;
use nsh_core::prelude::*;

use crate::kit::Editor;
use crate::ui::Ctx;

/// The plugin's key.
const KEY: &str = "emoji";

/// The most rows the menu shows.
const MOST: usize = 8;

/// The least typed after the `:` before the menu opens.
const LEAST: usize = 2;

/// An emoji, its name and its keywords (lowercase).
struct Entry {
    emoji: String,
    name: String,
    keywords: Vec<String>,
}

/// Every emoji, read once from the picker's table (`emoji\tgroup\tversion
/// \tskin tones\tname\tkeywords…`).
fn entries() -> &'static [Entry] {
    static ENTRIES: OnceLock<Vec<Entry>> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        nsh_base::EMOJI_DATA
            .lines()
            .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
            .filter_map(|line| {
                let mut fields = line.split('\t');
                let emoji = fields.next()?.to_string();
                let name = fields.nth(3)?.to_lowercase();
                let keywords = fields.map(str::to_lowercase).collect();
                Some(Entry { emoji, name, keywords })
            })
            .collect()
    })
}

/// The emoji for `query` (lowercase): a name starting with it first, then a
/// name word or keyword starting with it, then any containing it.
fn matches(query: &str) -> Vec<(&'static str, &'static str)> {
    let query = query.trim().to_lowercase();
    if query.chars().count() < LEAST {
        return Vec::new();
    }
    let rank = |e: &Entry| {
        if e.name.starts_with(&query) {
            Some(0)
        } else if e.name.split(' ').any(|w| w.starts_with(&query)) || e.keywords.iter().any(|k| k.starts_with(&query)) {
            Some(1)
        } else if e.name.contains(&query) {
            Some(2)
        } else {
            None
        }
    };
    let mut found: Vec<(u8, &Entry)> = entries().iter().filter_map(|e| rank(e).map(|r| (r, e))).collect();
    // Stable: within a rank, the table's order (the picker's, common first).
    found.sort_by_key(|(r, _)| *r);
    found.into_iter().take(MOST).map(|(_, e)| (e.emoji.as_str(), e.name.as_str())).collect()
}

/// The plugin following a typed `:`.
pub fn plugin() -> Plugin {
    let mut trigger = SuggestionTrigger::default();
    trigger.char = ":".into();
    // `:smile:`: the closing `:` is typed into the query (and picks).
    trigger.allow_to_include_char = true;
    let mut options = SuggestionOptions::default();
    options.key = KEY.into();
    options.trigger = trigger;
    options.capture_keys = ["ArrowUp", "ArrowDown", "Enter", "Tab"].map(String::from).to_vec();
    // Active (taking Enter and the arrows) only while what's typed after the
    // `:` names an emoji: `:)` or a lone `:` and Enter stay the text's.
    // Not in code.
    options.allow = Some(Arc::new(|state: &EditorState, range, _| {
        let doc = state.doc();
        let in_code = doc.resolve(range.from).is_ok_and(|at| at.parent().ty().name() == "codeBlock");
        let query = doc.text_between((range.from + 1).min(range.to), range.to, None, None);
        !in_code && !matches(query.trim_end_matches(':')).is_empty()
    }));
    suggestion(options)
}

fn state(editor: &Editor) -> Option<Arc<SuggestionState>> {
    suggestion_state(&editor.state(), KEY)
}

/// Puts `emoji` in place of the `:` and what was typed after it.
fn insert(editor: &Editor, emoji: &str) -> bool {
    let text = editor.kit.schema.text(emoji, Vec::new());
    editor.session.run(&replace_suggestion(KEY, Fragment::from(vec![text])))
}

/// The menu under a typed `:name`: ↑ ↓ move, Enter or Tab picks, a click
/// picks, a closing `:` picks the first. `window_height` keeps it inside
/// the window.
pub fn menu(ctx: Ctx, window_height: Signal<f32>) -> impl IntoElement {
    let window = nsh_core::element::desktop::current_window();
    let owner = Owner::current();
    let index = signal(0_usize);
    let keys_seen = signal(0_u64);
    let open = signal(None::<(usize, String)>);
    let rows = move || open.get().map(|(_, q)| matches(q.trim_end_matches(':'))).unwrap_or_default();
    let pick = move |emoji: &'static str| {
        ctx.editor.with_untracked(|e| insert(e, emoji));
        ctx.changed();
        // A click on the menu takes the focus from the text: give it back.
        owner.run(|| {
            let _ = set_timeout(std::time::Duration::from_millis(60), move || ctx.editor.with_untracked(|e| e.session.focus()));
        });
    };
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
        // `:name:` typed out picks the best match.
        if let Some((_, query)) = &now
            && let Some(name) = query.strip_suffix(':')
            && let Some((emoji, _)) = matches(name).first()
        {
            return pick(emoji);
        }
        let found = untracked(rows);
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
            Some("Enter" | "Tab") => pick(found[index.get_untracked().min(found.len() - 1)].0),
            _ => {}
        }
    });
    // Rows follow the pointer only when it really moves (see the slash menu).
    let pointer = stored(None::<(f32, f32)>);
    let retry = signal(0_u32);
    dynamic(move || {
        retry.get();
        let (from, _) = open.get()?;
        let found = rows();
        if found.is_empty() {
            return None;
        }
        let Some(caret) = ctx.editor.with_untracked(|e| e.session.page_coords_at_pos(from)) else {
            let _ = set_timeout(std::time::Duration::from_millis(16), move || retry.update(|n| *n += 1));
            return None;
        };
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
        // Under the `:`, or over it when there's more room there.
        let menu = if below + wanted <= height || caret.origin.y.0 < height / 2.0 {
            menu.top(px(below))
        } else {
            menu.bottom(px(height - caret.origin.y.0 + 4.0))
        };
        Some(menu.children(found.into_iter().enumerate().map(move |(i, (emoji, name))| {
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
                .cursor(Cursor::Pointer)
                .on_press(move || pick(emoji))
                .child(text(emoji).text_size(px(18.0)))
                .child(text(name).flex_1().truncate().text_size(px(13.0)))
                .into_element()
        })))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_keywords_find_emoji() {
        assert!(matches("s").is_empty(), "one letter isn't enough");
        let smile: Vec<_> = matches("smil").into_iter().map(|(e, _)| e).collect();
        assert_eq!(smile.first(), Some(&"😊"), "names starting with it come first: {smile:?}");
        assert_eq!(matches("thumbs").first().map(|(e, _)| *e), Some("👍"));
        assert!(matches("zzzzqq").is_empty());
    }

    /// Typed text with the caret after it: whether the menu is active.
    fn active_after(typed: &str) -> bool {
        let kit = crate::kit::Kit::new(crate::kit::Relay::default());
        let editor = kit.editor(None);
        for ch in typed.chars() {
            let state = editor.state();
            let mut tr = state.tr();
            tr.insert_text(&ch.to_string(), None, None);
            editor.session.dispatch(tr).unwrap();
        }
        state(&editor).is_some_and(|s| s.active)
    }

    #[test]
    fn only_a_name_opens_the_menu() {
        assert!(active_after("hi :smil"));
        assert!(!active_after("hi :)"), "an emoticon leaves Enter to the text");
        assert!(!active_after("at 10:30"));
        assert!(!active_after("Agenda:"));
    }
}
