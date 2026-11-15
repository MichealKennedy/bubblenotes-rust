//! The window's pieces: the toolbar, the slash menu, the selection menu
//! (with its link editor and "…" menu), buttons, and how the document looks.
//! All the menus are drawn from the one action catalog (`actions`).

use std::collections::BTreeMap;
use std::rc::Rc;

use nsh_base::rich_editor::view::{RichEditorClasses, RichEditorSelection};
use nsh_base::theme;
use nsh_core::prelude::*;
use serde_json::json;

use crate::actions::{self, Action, AppAction};
use crate::kit::Editor;
use crate::{slash, toggle};

/// The window's shared state the pieces read and change (all `Copy`
/// handles).
#[derive(Clone, Copy)]
pub struct Ctx {
    pub editor: Signal<Editor>,
    /// Bumped on every change of the editor's state: what active states,
    /// menus and the status bar follow.
    pub revision: Signal<u64>,
    pub selection: Signal<Option<RichEditorSelection>>,
    pub status: Signal<String>,
    /// The selection menu shows its link editor.
    pub editing_link: Signal<bool>,
    /// What an action that needs the app's UI does.
    pub app: Stored<Rc<dyn Fn(AppAction)>>,
}

impl Ctx {
    pub fn changed(self) {
        self.revision.update(|r| *r += 1);
    }

    pub fn perform(self, action: &'static Action) {
        let app = self.app.get();
        let editor = self.editor.get_untracked();
        action.perform(&editor, &*app);
        self.changed();
    }

    fn is_active(self, action: &'static Action) -> bool {
        self.revision.get();
        self.editor.with(|e| action.is_active(e))
    }

    fn is_enabled(self, action: &'static Action) -> bool {
        self.revision.get();
        self.editor.with(|e| action.enabled(e))
    }

    /// The slash menu's state: where its `/` is and the text after it.
    pub fn slash_open(self) -> Option<(usize, String)> {
        self.revision.get();
        self.editor.with(|e| slash::state(e).and_then(|s| slash::open(&s)))
    }
}

// ----- Buttons ------------------------------------------------------------------

/// A compact button: raised while `on`, dimmed while not `enabled`.
pub fn button(
    label: &'static str,
    description: &'static str,
    on: impl Fn() -> bool + 'static,
    enabled: impl Fn() -> bool + 'static,
    press: impl Fn() + 'static,
) -> impl IntoElement {
    let on = Rc::new(on);
    let enabled = Rc::new(enabled);
    let (on_bg, on_text, enabled_style, enabled_press) = (on.clone(), on.clone(), enabled.clone(), enabled.clone());
    view()
        .min_w(px(28.0))
        .h(px(28.0))
        .px_2()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .bg(move || if on_bg() { theme().primary } else { theme().secondary })
        .style_when(move || !enabled_style(), |s| s.opacity(0.35))
        .active(|s| s.opacity(0.6))
        .label(description)
        .on_press(move || {
            if enabled_press() {
                press()
            }
        })
        .child(text(label).text_size(px(13.0)).text_color(move || {
            if on_text() { theme().primary_foreground } else { theme().secondary_foreground }
        }))
}

/// A button for a catalog action.
fn action_button(ctx: Ctx, id: &'static str) -> impl IntoElement {
    let action = actions::get(id);
    button(action.glyph, action.label, move || ctx.is_active(action), move || ctx.is_enabled(action), move || ctx.perform(action))
}

pub fn separator() -> impl IntoElement {
    view().w(px(1.0)).h(px(18.0)).mx_1().bg(move || theme().border)
}

/// A text-color button: a dot of `rgb`, or the default color when `None`.
fn swatch(ctx: Ctx, color: Option<(&'static str, u32)>) -> impl IntoElement {
    view()
        .w(px(20.0))
        .h(px(20.0))
        .items_center()
        .justify_center()
        .rounded(px(10.0))
        .border_1()
        .border_color(move || theme().border)
        .bg(move || -> AdaptiveColor {
            match color {
                Some((_, rgb)) => nsh_core::color::rgb(rgb).into(),
                None => theme().background,
            }
        })
        .active(|s| s.opacity(0.6))
        .label(if color.is_some() { "Text color" } else { "Default text color" })
        .on_press(move || {
            ctx.editor.with_untracked(|e| match color {
                Some((css, _)) => e.run("setTextStyle", json!({ "color": css })),
                None => e.run("unsetTextStyle", json!({})),
            });
            ctx.changed();
        })
        .child(text(if color.is_none() { "A" } else { "" }).text_size(px(11.0)))
}

// ----- Toolbar --------------------------------------------------------------------

/// The toolbar: the file buttons, then the catalog's groups and the colors.
pub fn toolbar(ctx: Ctx, file_buttons: Vec<AnyElement>) -> impl IntoElement {
    let mut bar = view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(4.0))
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(move || theme().border)
        .children(file_buttons);
    for (i, group) in actions::TOOLBAR.iter().enumerate() {
        bar = bar.child(separator()).children(group.iter().map(|id| action_button(ctx, id).into_element()));
        // The colors go after the marks.
        if i == 2 {
            bar = bar
                .child(separator())
                .child(swatch(ctx, None))
                .children(actions::COLORS.iter().map(|(css, _, rgb)| swatch(ctx, Some((css, *rgb))).into_element()));
        }
    }
    bar
}

// ----- The slash menu ---------------------------------------------------------

/// The tallest the slash menu gets before it scrolls.
const SLASH_MENU_MAX_HEIGHT: f32 = 320.0;

/// Which edge of a popup is placed, as a distance from the window's top or
/// bottom.
#[derive(Clone, Copy)]
enum Edge {
    Top(f32),
    Bottom(f32),
}

/// The slash menu: under the `/` (above it near the window's bottom),
/// filtered by what's typed after it; ↑ ↓ move, Enter or Tab picks, a click
/// picks. `window_height` keeps it inside the window.
pub fn slash_menu(ctx: Ctx, window_height: Signal<f32>) -> impl IntoElement {
    let index = signal(0_usize);
    let keys_seen = signal(0_u64);
    // Set only when the `/` or its query changes, so the menu isn't built
    // again (and its scroll reset) on every change of the editor's state.
    let open = signal(None::<(usize, String)>);
    let items = move || open.get().map(|(_, q)| actions::slash_matches(&q)).unwrap_or_default();
    let pick = move |action: &'static Action| {
        ctx.editor.with_untracked(slash::remove_query);
        ctx.perform(action);
    };
    effect(move || {
        let now = ctx.slash_open();
        let before = open.get_untracked();
        if now != before {
            if now.as_ref().map(|(_, q)| q) != before.as_ref().map(|(_, q)| q) {
                index.set(0);
            }
            open.set(now.clone());
        }
        // The keys the plugin captured since we last looked.
        let Some(state) = ctx.editor.with(slash::state) else { return };
        if state.key_count == keys_seen.get_untracked() {
            return;
        }
        keys_seen.set(state.key_count);
        let found = untracked(items);
        if now.is_none() || found.is_empty() {
            return;
        }
        match state.last_key.as_deref() {
            Some("ArrowDown") => index.update(|i| *i = (*i + 1) % found.len()),
            Some("ArrowUp") => index.update(|i| *i = (*i + found.len() - 1) % found.len()),
            Some("Enter" | "Tab") => pick(found[index.get_untracked().min(found.len() - 1)]),
            _ => {}
        }
    });
    dynamic(move || {
        let (from, _) = open.get()?;
        let caret = ctx.editor.with_untracked(|e| e.session.page_coords_at_pos(from))?;
        let below = caret.origin.y.0 + caret.size.height.0 + 4.0;
        let height = window_height.get();
        let edge = if below + SLASH_MENU_MAX_HEIGHT <= height || caret.origin.y.0 < height / 2.0 {
            Edge::Top(below)
        } else {
            Edge::Bottom(height - caret.origin.y.0 + 4.0)
        };
        let found = items();
        // The highlighted row stays in view as the arrows move it.
        let rows: Vec<ElementRef> = found.iter().map(|_| element_ref()).collect();
        let reveal = rows.clone();
        effect(move || {
            if let Some(row) = reveal.get(index.get()) {
                row.reveal(RevealOptions::default().vertical(ScrollAlignment::Nearest).animated(false));
            }
        });
        let menu = popup().left(px(caret.origin.x.0)).w(px(260.0));
        let menu = match edge {
            Edge::Top(top) => menu.top(px(top)),
            Edge::Bottom(bottom) => menu.bottom(px(bottom)),
        };
        Some(menu.child(if found.is_empty() {
            view()
                .px_3()
                .py_2()
                .child(text("No matching blocks").text_size(px(13.0)).text_color(move || theme().muted_foreground))
                .into_element()
        } else {
            // The cap goes on the scroll view itself, so it scrolls rather
            // than growing.
            scroll()
                .max_h(px(SLASH_MENU_MAX_HEIGHT - 10.0))
                .children(found.into_iter().enumerate().map(|(i, action)| {
                    menu_row(action, move || index.get() == i, move || index.set(i), move || pick(action))
                        .element_ref(&rows[i])
                        .into_element()
                }))
                .into_element()
        }))
    })
}

/// A popup's box: absolutely placed, on the popover surface.
fn popup() -> View {
    view()
        .absolute()
        .p(px(4.0))
        .rounded(px(10.0))
        .overflow_hidden()
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().popover)
        .shadow_lg()
}

/// A row of a menu: the action's glyph in a tile, its label and hint.
fn menu_row(
    action: &'static Action,
    highlighted: impl Fn() -> bool + 'static,
    hover: impl Fn() + 'static,
    press: impl Fn() + 'static,
) -> View {
    view()
        .flex_row()
        .items_center()
        .gap_3()
        .px_2()
        .py(px(6.0))
        .rounded(px(6.0))
        .bg(move || if highlighted() { theme().accent } else { theme().popover })
        .on_mouse_enter(move |_: &MouseEvent| hover())
        .on_press(press)
        .child(
            view()
                .w(px(30.0))
                .h(px(30.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .border_1()
                .border_color(move || theme().border)
                .bg(move || theme().background)
                .child(text(action.glyph).text_size(px(13.0))),
        )
        .child(
            view()
                .child(text(action.label).text_size(px(14.0)))
                .child(text(action.hint).text_size(px(12.0)).text_color(move || theme().muted_foreground)),
        )
}

// ----- The selection menu -------------------------------------------------------

/// The menu over a text selection (the editor's bubble menu): lists,
/// bold/italic/underline/strike, indent, link, and "…" for the rest. It
/// turns into a link editor for "Link".
pub fn selection_menu(ctx: Ctx) -> impl IntoElement {
    let more = signal(false);
    let link = signal(String::new());
    // A new selection closes "…" and the link editor.
    effect(move || {
        ctx.selection.with(|_| ());
        more.set(false);
    });
    let visible = move || {
        let selected = ctx.selection.with(|s| s.as_ref().is_some_and(|s| !s.empty && s.kind == "text"));
        (selected || ctx.editing_link.get()) && ctx.slash_open().is_none()
    };
    // Opening the link editor starts from the link's address.
    effect(move || {
        if ctx.editing_link.get() {
            let href = ctx.editor.with_untracked(|e| e.attributes("link").get("href").and_then(|h| h.as_str().map(String::from)));
            link.set(href.unwrap_or_default());
        }
    });
    let apply_link = move || {
        let href = normalize_href(&link.get_untracked());
        ctx.editor.with_untracked(|e| {
            if href.is_empty() {
                e.run("unsetLink", json!({}));
            } else {
                e.run("setLink", json!({ "href": href }));
            }
        });
        ctx.editing_link.set(false);
        ctx.changed();
    };
    dynamic(move || {
        if !visible() {
            return None;
        }
        let surface = view()
            .p(px(4.0))
            .gap(px(4.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(move || theme().border)
            .bg(move || theme().popover)
            .shadow_lg();
        if ctx.editing_link.get() {
            return Some(
                surface
                    .flex_row()
                    .items_center()
                    .w(px(380.0))
                    .child(
                        nsh_core::element::input()
                            .flex_1()
                            .h(px(28.0))
                            .px_2()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(move || theme().border)
                            .text_size(px(13.0))
                            .placeholder("Paste or type a link…")
                            .auto_focus(true)
                            .value(move || link.get())
                            .on_change(move |t: &str| link.set(t.to_string()))
                            .on_submit(move |_: &str| apply_link()),
                    )
                    .child(button("Apply", "Apply link", || false, || true, apply_link))
                    .child(button("Remove", "Remove link", || false, move || ctx.is_active(actions::get("link")), move || {
                        link.set(String::new());
                        apply_link();
                    }))
                    .child(button("✕", "Cancel", || false, || true, move || ctx.editing_link.set(false)))
                    .into_element(),
            );
        }
        let mut row = view().flex_row().items_center().gap(px(2.0));
        for (i, group) in actions::SELECTION.iter().enumerate() {
            if i > 0 {
                row = row.child(separator());
            }
            row = row.children(group.iter().map(|id| action_button(ctx, id).into_element()));
        }
        row = row.child(separator()).child(button("…", "More", move || more.get(), || true, move || more.update(|m| *m = !*m)));
        let section = |title: &'static str, ids: &'static [&'static str]| {
            view()
                .child(
                    text(title)
                        .text_size(px(11.0))
                        .px_2()
                        .pt_1()
                        .text_color(move || theme().muted_foreground),
                )
                .children(ids.iter().map(move |id| {
                    let action = actions::get(id);
                    let hovered = signal(false);
                    menu_row(action, move || hovered.get(), move || hovered.set(true), move || {
                        more.set(false);
                        ctx.perform(action);
                    })
                    .on_mouse_leave(move |_: &MouseEvent| hovered.set(false))
                    .into_element()
                }))
        };
        let more_panel = view()
            .style_when(move || !more.get(), |s| s.hidden())
            .border_t_1()
            .border_color(move || theme().border)
            .pt_1()
            .child(section("Insert", actions::SELECTION_MORE_INSERT))
            .child(section("Format", actions::SELECTION_MORE_FORMAT));
        Some(surface.child(row).child(more_panel).into_element())
    })
}

/// A link as typed: `example.com` gets `https://`; a scheme or an anchor
/// stays as it is.
fn normalize_href(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() || text.contains("://") || text.starts_with('#') || text.starts_with("mailto:") || text.starts_with('/') {
        text.to_string()
    } else {
        format!("https://{text}")
    }
}

// ----- How the document looks ------------------------------------------------------

/// The editor's classes: NSH's editor draws nothing of its own, so every
/// node and mark gets its look here (after NSH's gallery look).
pub fn editor_classes() -> RichEditorClasses {
    let map = |pairs: &mut dyn Iterator<Item = (&str, &str)>| -> BTreeMap<String, String> {
        pairs.map(|(k, v)| (k.to_string(), v.to_string())).collect()
    };
    let mut classes = RichEditorClasses::default();
    classes.content = Some("gap-3 px-2 py-5".into());
    classes.placeholder = Some("text-muted-foreground".into());
    classes.list_marker = Some("text-muted-foreground".into());
    classes.task_checkbox =
        Some("size-4 mr-2 mt-1 rounded border border-border checked:bg-primary checked:border-primary".into());
    let nodes: &[(&str, &str)] = &[
        ("paragraph", "leading-relaxed"),
        ("heading[level=1]", "text-3xl font-bold tracking-tight"),
        ("heading[level=2]", "text-2xl font-semibold tracking-tight"),
        ("heading[level=3]", "text-xl font-semibold"),
        ("blockquote", "border-l-4 border-border pl-4 text-muted-foreground italic"),
        ("codeBlock", "font-mono text-sm bg-muted rounded-lg px-4 py-3"),
        ("bulletList", "gap-1.5 pl-6"),
        ("orderedList", "gap-1.5 pl-6"),
        ("taskList", "gap-1.5"),
        ("horizontalRule", "h-px bg-border my-3"),
        ("image", "rounded-lg max-w-full"),
        ("table", "border border-border rounded"),
        ("tableHeader", "font-semibold bg-muted px-3 py-1.5 border border-border"),
        ("tableCell", "px-3 py-1.5 border border-border"),
        // Alignment is an attribute of paragraphs and headings.
        ("paragraph[textAlign=center]", "text-center"),
        ("paragraph[textAlign=right]", "text-right"),
        ("heading[textAlign=center]", "text-center"),
        ("heading[textAlign=right]", "text-right"),
    ];
    classes.nodes = Some(map(&mut nodes.iter().chain(toggle::CLASSES).copied()));
    let marks: &[(&str, &str)] = &[
        ("bold", "font-bold"),
        ("italic", "italic"),
        ("underline", "underline"),
        ("strike", "line-through"),
        ("code", "font-mono text-sm bg-muted rounded px-1"),
        ("link", "text-primary underline"),
        ("highlight", "bg-yellow-200 text-black rounded-sm"),
        // NSH's text layout doesn't shift the baseline: smaller text.
        ("subscript", "text-xs"),
        ("superscript", "text-xs"),
    ];
    // Text color is `textStyle`'s `color`: a class per catalog color.
    let colors: Vec<(String, &str)> =
        actions::COLORS.iter().map(|(css, class, _)| (format!("textStyle[color={css}]"), *class)).collect();
    let mut all = marks.iter().copied().map(|(k, v)| (k.to_string(), v)).chain(colors);
    classes.marks = Some(std::iter::from_fn(|| all.next()).map(|(k, v)| (k, v.to_string())).collect());
    classes
}
