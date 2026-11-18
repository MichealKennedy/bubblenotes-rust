//! The window's pieces: the toolbar, the slash menu, the selection menu
//! (with its link editor and "…" menu), buttons, and how the document looks.
//! All the menus are drawn from the one action catalog (`actions`).

use std::collections::BTreeMap;
use std::rc::Rc;

use nsh_base::rich_editor::view::{RichEditorClasses, RichEditorSelection};
use nsh_base::theme;
use nsh_core::element::desktop::{MenuItem, popup_menu};
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
    /// The note can't be edited here (locked, or in the trash): no
    /// formatting menus.
    pub read_only: Signal<bool>,
    /// Bumped on every change of the editor's state: what active states,
    /// menus and the status bar follow.
    pub revision: Signal<u64>,
    pub selection: Signal<Option<RichEditorSelection>>,
    pub status: Signal<String>,
    /// The selection menu shows its link editor.
    pub editing_link: Signal<bool>,
    /// The selection menu's text colors panel.
    pub color_panel: Signal<ColorPanel>,
    /// What an action that needs the app's UI does.
    pub app: Stored<Rc<dyn Fn(AppAction)>>,
}

/// The selection menu's text colors: closed, the named colors, or those and
/// the picker for any color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColorPanel {
    Closed,
    Named,
    Custom,
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

/// A compact button: tinted while `on`, dimmed while not `enabled`.
pub fn button(
    label: &'static str,
    description: &'static str,
    on: impl Fn() -> bool + 'static,
    enabled: impl Fn() -> bool + 'static,
    press: impl Fn() + 'static,
) -> impl IntoElement {
    let on = Rc::new(on);
    let enabled = Rc::new(enabled);
    let (enabled_style, enabled_press) = (enabled.clone(), enabled.clone());
    view()
        .min_w(px(28.0))
        .h(px(28.0))
        .px_2()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .bg(move || if on() { tint(0.12) } else { tint(0.0) })
        .hover(|s| s.bg(tint(0.07)))
        .style_when(move || !enabled_style(), |s| s.opacity(0.35))
        .active(|s| s.opacity(0.6))
        .label(description)
        // Not a stop of the keyboard focus: a press leaves it in the text, so
        // the selection (and the menu over it) stays.
        .focusable(false)
        .cursor(Cursor::Pointer)
        .on_press(move || {
            if enabled_press() {
                press()
            }
        })
        .child(text(label).text_size(px(13.0)).text_color(move || theme().foreground))
}

/// The theme's ink at `alpha`: hover and pressed tints that read on both
/// light and dark surfaces.
pub fn tint(alpha: f32) -> AdaptiveColor {
    theme().foreground.opacity(alpha)
}

/// A button for a catalog action.
fn action_button(ctx: Ctx, id: &'static str) -> impl IntoElement {
    let action = actions::get(id);
    button(action.glyph, action.label, move || ctx.is_active(action), move || ctx.is_enabled(action), move || ctx.perform(action))
}

pub fn separator() -> impl IntoElement {
    view().w(px(1.0)).h(px(18.0)).mx_1().bg(move || theme().border)
}

/// Sets the text color at the selection (`None`: back to the default).
fn set_color(ctx: Ctx, css: Option<&str>) {
    set_style(ctx, "color", css);
}

/// Sets a text style attribute at the selection (`None`: off).
fn set_style(ctx: Ctx, key: &str, value: Option<&str>) {
    ctx.editor.with_untracked(|e| actions::set_style(e, key, value));
    ctx.changed();
}

/// The text color at the selection, as CSS (`None`: the default).
fn current_color(ctx: Ctx) -> Option<String> {
    current_style(ctx, "color")
}

/// A text style attribute at the selection, lowercase (`None`: not set).
fn current_style(ctx: Ctx, key: &str) -> Option<String> {
    ctx.revision.get();
    ctx.editor.with(|e| actions::style_of(e, key).map(|v| v.to_lowercase()))
}

/// The text colors as a native menu: the default, the named colors (the
/// current one checked) and "Custom color…", which opens the selection
/// menu's picker.
fn color_menu_items(ctx: Ctx) -> Vec<MenuItem> {
    let current = untracked(|| current_color(ctx));
    let mut items = vec![MenuItem::new("Default").checked(current.is_none()).on_select(move || set_color(ctx, None)), MenuItem::separator()];
    items.extend(actions::COLORS.iter().map(|(name, css)| {
        MenuItem::new(*name).checked(current.as_deref() == Some(*css)).on_select(move || set_color(ctx, Some(css)))
    }));
    items.push(MenuItem::separator());
    let selected = untracked(|| ctx.editor.with(|e| !e.state().selection().empty()));
    items.push(MenuItem::new("Custom color…").disabled(!selected).on_select(move || ctx.color_panel.set(ColorPanel::Custom)));
    items
}

// ----- Toolbar --------------------------------------------------------------------

/// A toolbar button's width with the gap after it.
const TOOL_WIDTH: f32 = 30.0;
/// A separator's width with its margins.
const SEPARATOR_WIDTH: f32 = 11.0;
/// A color dot's width with the gap after it.
const SWATCH_WIDTH: f32 = 22.0;

/// What a toolbar slot holds: an action, or the text colors.
#[derive(Clone, Copy)]
enum Slot {
    Action(&'static Action),
    Colors,
}

impl Slot {
    fn width(self) -> f32 {
        match self {
            Slot::Action(_) => TOOL_WIDTH,
            Slot::Colors => SWATCH_WIDTH * 2.0,
        }
    }
}

/// The catalog's toolbar groups in order, the colors after the marks.
fn toolbar_groups() -> Vec<Vec<Slot>> {
    let mut groups: Vec<Vec<Slot>> = Vec::new();
    for (i, group) in actions::TOOLBAR.iter().enumerate() {
        groups.push(group.iter().map(|id| Slot::Action(actions::get(id))).collect());
        if i == 2 {
            groups.push(vec![Slot::Colors]);
        }
    }
    groups
}

/// How many slots, in order, fit in `width`, keeping room for "More" when
/// some don't.
fn slots_that_fit(groups: &[Vec<Slot>], width: f32) -> usize {
    let total: usize = groups.iter().map(Vec::len).sum();
    let mut used = 0.0;
    let mut count = 0;
    let mut fits = 0;
    for (g, group) in groups.iter().enumerate() {
        for (i, slot) in group.iter().enumerate() {
            used += slot.width() + if g > 0 && i == 0 { SEPARATOR_WIDTH } else { 0.0 };
            count += 1;
            let more = if count < total { SEPARATOR_WIDTH + TOOL_WIDTH } else { 0.0 };
            if used + more > width {
                return fits;
            }
            fits = count;
        }
    }
    fits
}

/// The formatting toolbar: the catalog's groups on one line, what doesn't
/// fit the width behind "More" (a native menu).
pub fn toolbar(ctx: Ctx) -> impl IntoElement {
    let width = signal(0.0_f32);
    let fits = signal(usize::MAX);
    let groups = Rc::new(toolbar_groups());
    let measure = groups.clone();
    effect(move || {
        let now = slots_that_fit(&measure, width.get());
        if now != fits.get_untracked() {
            fits.set(now);
        }
    });
    view()
        .flex_1()
        .min_w(px(0.0))
        .h(px(40.0))
        .justify_center()
        .overflow_hidden()
        .on_layout(move |b: &Bounds<Points>| {
            if (b.size.width.0 - width.get_untracked()).abs() > 0.5 {
                width.set(b.size.width.0);
            }
        })
        .child(dynamic(move || {
            let fits = fits.get();
            let mut bar = nsh_base::toolbar().accessibility_label("Formatting").items_center();
            let mut hidden: Vec<Vec<Slot>> = Vec::new();
            let mut index = 0;
            for (g, group) in groups.iter().enumerate() {
                let shown: Vec<Slot> = group.iter().copied().filter(|_| {
                    index += 1;
                    index <= fits
                }).collect();
                if shown.len() < group.len() {
                    hidden.push(group[shown.len()..].to_vec());
                }
                if shown.is_empty() {
                    continue;
                }
                if g > 0 {
                    bar = bar.child(nsh_base::toolbar_separator().w(px(1.0)).h(px(18.0)).mx(px(5.0)).bg(move || theme().border));
                }
                let mut run = nsh_base::toolbar_group().items_center().gap(px(2.0));
                for slot in shown {
                    run = run.child(match slot {
                        Slot::Action(action) => tool(ctx, action),
                        Slot::Colors => swatches(ctx),
                    });
                }
                bar = bar.child(run);
            }
            if !hidden.is_empty() {
                bar = bar
                    .child(nsh_base::toolbar_separator().w(px(1.0)).h(px(18.0)).mx(px(5.0)).bg(move || theme().border))
                    .child(more_button(ctx, hidden));
            }
            bar
        }))
}

/// A hint bubble for a toolbar item: its name, and its shortcut muted.
pub(crate) fn hint(label: impl Into<String>, shortcut: &'static str) -> impl IntoTooltip {
    let bubble = nsh_base::tooltip()
        .flex_row()
        .gap(px(6.0))
        .px_2()
        .py_1()
        .rounded(px(6.0))
        .bg(move || theme().foreground)
        .shadow_md()
        .child(text(label.into()).text_size(px(12.0)).text_color(move || theme().background));
    if shortcut.is_empty() {
        bubble
    } else {
        bubble.child(text(shortcut).text_size(px(12.0)).text_color(move || theme().background.opacity(0.65)))
    }
}

/// `content` filling its button, with `tip` (components take no tooltip of
/// their own).
pub fn hinted(tip: impl IntoTooltip, content: impl IntoElement) -> View {
    view().size_full().items_center().justify_center().tooltip(tip).child(content)
}

/// A toolbar item's look: a 28-point square, tinted on hover and while on.
macro_rules! tool_look {
    ($el:expr, $on:expr, $enabled:expr) => {{
        let (on, enabled) = ($on, $enabled);
        $el.w(px(28.0))
            .h(px(28.0))
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .bg(move || if on() { tint(0.12) } else { tint(0.0) })
            .hover(|s| s.bg(tint(0.07)))
            .active(|s| s.opacity(0.6))
            .style_when(move || !enabled(), |s| s.opacity(0.35))
    }};
}

/// A toolbar item for a catalog action: a toggle when it has an on state,
/// else a button.
fn tool(ctx: Ctx, action: &'static Action) -> AnyElement {
    let glyph = text(action.glyph).text_size(px(13.0)).text_color(move || theme().foreground);
    let enabled = move || ctx.is_enabled(action);
    if action.active.is_some() {
        let toggle = nsh_base::toggle()
            .cursor(Cursor::Pointer)
            .pressed(move || ctx.is_active(action))
            .accessibility_label(action.label)
            .on_change(move |_: &bool| ctx.perform(action));
        tool_look!(toggle, move || ctx.is_active(action), enabled).child(hinted(hint(action.label, action.hint), glyph)).into_element()
    } else {
        let button = nsh_base::button().accessibility_label(action.label).cursor(Cursor::Pointer).on_press(move || {
            if ctx.is_enabled(action) {
                ctx.perform(action);
            }
        });
        tool_look!(button, || false, enabled).child(hinted(hint(action.label, action.hint), glyph)).into_element()
    }
}

/// The text colors: the default, and a dot of the current color that
/// opens the menu of all of them.
fn swatches(ctx: Ctx) -> AnyElement {
    let dot = |label: &'static str, fill: Box<dyn Fn() -> AdaptiveColor>, glyph: &'static str, press: Box<dyn Fn()>| {
        nsh_base::button()
            .accessibility_label(label)
            .w(px(18.0))
            .h(px(18.0))
            .items_center()
            .justify_center()
            .rounded_full()
            .border_1()
            .border_color(move || tint(0.18))
            .bg(fill)
            .hover(|s| s.scale(1.12))
            .active(|s| s.opacity(0.6))
            .cursor(Cursor::Pointer)
            .on_press(press)
            .child(hinted(hint(label, ""), text(glyph).text_size(px(10.0)).text_color(move || theme().foreground)))
            .into_element()
    };
    let current = move || current_color(ctx).and_then(|c| actions::color_rgb(&c));
    nsh_base::toolbar_group()
        .accessibility_label("Text color")
        .items_center()
        .gap(px(4.0))
        .px(px(2.0))
        .child(dot("Default text color", Box::new(move || theme().background), "A", Box::new(move || set_color(ctx, None))))
        .child(dot(
            "Text color",
            Box::new(move || match current() {
                Some(rgb) => nsh_core::color::rgb(rgb).into(),
                None => theme().background,
            }),
            "▾",
            Box::new(move || {
                let _ = popup_menu(color_menu_items(ctx));
            }),
        ))
        .into_element()
}

/// "More": a native menu with the slots that didn't fit, by group.
fn more_button(ctx: Ctx, hidden: Vec<Vec<Slot>>) -> AnyElement {
    let hidden = Rc::new(hidden);
    let button = nsh_base::button().accessibility_label("More formatting").cursor(Cursor::Pointer).on_press(move || {
        let mut items = Vec::new();
        for (i, group) in hidden.iter().enumerate() {
            if i > 0 {
                items.push(MenuItem::separator());
            }
            for slot in group {
                items.push(match *slot {
                    Slot::Action(action) => MenuItem::new(action.label)
                        .checked(untracked(|| ctx.is_active(action)))
                        .disabled(!untracked(|| ctx.is_enabled(action)))
                        .on_select(move || ctx.perform(action)),
                    Slot::Colors => MenuItem::submenu("Text color", color_menu_items(ctx)),
                });
            }
        }
        let _ = popup_menu(items);
    });
    tool_look!(button, || false, || true)
        .child(hinted(hint("More", ""), text("⋯").text_size(px(15.0)).text_color(move || theme().foreground)))
        .into_element()
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
    let window = nsh_core::element::desktop::current_window();
    let index = signal(0_usize);
    let keys_seen = signal(0_u64);
    // Set only when the `/` or its query changes, so the menu isn't built
    // again (and its scroll reset) on every change of the editor's state.
    let open = signal(None::<(usize, String)>);
    let items = move || open.get().map(|(_, q)| actions::slash_matches(&q)).unwrap_or_default();
    // The delayed step below runs under this owner: started from a press
    // handler alone it would be dropped before it ran.
    let owner = Owner::current();
    let pick = move |action: &'static Action| {
        ctx.editor.with_untracked(slash::remove_query);
        ctx.perform(action);
        // A click on the menu takes the focus from the text as it ends:
        // give it back once it has. Not when the action opened the link
        // editor: its field has the focus.
        if ctx.editing_link.get_untracked() {
            return;
        }
        owner.run(|| {
            let _ = set_timeout(std::time::Duration::from_millis(60), move || {
                ctx.editor.with_untracked(|e| e.session.focus());
            });
        });
    };
    // Where the pointer last was over the menu: rows follow it only when it
    // really moves, so a row sliding under a resting pointer (the menu
    // opening there, the arrows scrolling it) doesn't steal the highlight.
    let pointer = stored(None::<(f32, f32)>);
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
    // Right after the `/` is typed its line isn't laid out yet, so its place
    // isn't known: ask again a frame later.
    let retry = signal(0_u32);
    dynamic(move || {
        retry.get();
        let (from, _) = open.get()?;
        let Some(caret) = ctx.editor.with_untracked(|e| e.session.page_coords_at_pos(from)) else {
            let _ = set_timeout(std::time::Duration::from_millis(16), move || retry.update(|n| *n += 1));
            return None;
        };
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
        // In a small note the menu moves left and narrows to stay inside the
        // window (it can't be drawn outside it).
        let window_width = window.state().map(|s| s.bounds.size.width.0).unwrap_or(f32::MAX);
        let menu_width = 260.0_f32.min(window_width - 8.0);
        let left = caret.origin.x.0.min(window_width - menu_width - 4.0).max(4.0);
        let room = match edge {
            Edge::Top(top) => height - top - 8.0,
            Edge::Bottom(bottom) => height - bottom - 8.0,
        };
        let max_height = (SLASH_MENU_MAX_HEIGHT - 10.0).min(room.max(72.0));
        let menu = popup().left(px(left)).w(px(menu_width));
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
                .max_h(px(max_height))
                .children(found.into_iter().enumerate().map(|(i, action)| {
                    menu_row(action, move || index.get() == i, move |at| {
                        if pointer.get().is_some_and(|last| last != at) {
                            index.set(i);
                        }
                        pointer.replace(Some(at));
                    }, move || pick(action))
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
        // Its own text colour: inside a note it would take the note's.
        .text_color(move || theme().popover_foreground)
        .shadow_lg()
}

/// A row of a menu: the action's glyph in a tile and its label.
fn menu_row(
    action: &'static Action,
    highlighted: impl Fn() -> bool + 'static,
    hover: impl Fn((f32, f32)) + 'static,
    press: impl Fn() + 'static,
) -> View {
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .px_2()
        .py(px(4.0))
        .rounded(px(6.0))
        .bg(move || if highlighted() { theme().accent } else { theme().popover })
        .on_mouse_move(move |e: &MouseEvent| hover((e.page_position.x.0, e.page_position.y.0)))
        // Not a stop of the keyboard focus: a press leaves the focus in the
        // text, so typing goes on after picking (a row that took it would
        // take it away with it when the menu closes).
        .focusable(false)
        .cursor(Cursor::Pointer).on_press(press)
        .child(
            view()
                .w(px(20.0))
                .h(px(20.0))
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .border_1()
                .border_color(move || theme().border)
                .bg(move || theme().background)
                .child(text(action.glyph).text_size(px(10.5)).text_color(move || theme().popover_foreground)),
        )
        // Its own colour: text here doesn't take the popup's.
        .child(text(action.label).text_size(px(13.0)).text_color(move || theme().popover_foreground))
}

// ----- The selection menu -------------------------------------------------------

/// The menu over a text selection (the editor's bubble menu): lists,
/// bold/italic/underline/strike, indent, link, and "…" for the rest. It
/// turns into a link editor for "Link".
pub fn selection_menu(ctx: Ctx) -> impl IntoElement {
    // The menu can't be drawn outside its window: in a small note it wraps.
    let window = nsh_core::element::desktop::current_window();
    let sized = window.clone();
    let fit = move || sized.state().map(|s| s.bounds.size.width.0 - 12.0).unwrap_or(400.0);
    let more = signal(false);
    let link = signal(String::new());
    // The link's text, as shown in the link editor.
    let link_text = signal(String::new());
    // What the link editor edits: the range it was opened on (the whole
    // link when it was opened in one; empty for new linked text), its text
    // then, and whether it was a link.
    let link_range = stored(None::<(usize, usize)>);
    let link_was = stored(String::new());
    let had_link = signal(false);
    // A new selection closes "…" and the colors (not a change of the text
    // at the same place: a color being picked); outside the link being
    // edited, it closes the link editor too (it would edit the wrong text).
    let place = stored(None::<(u32, u32)>);
    effect(move || {
        let now = ctx.selection.with(|s| s.as_ref().map(|s| (s.anchor, s.head)));
        if place.get() != now {
            place.replace(now);
            more.set(false);
            ctx.color_panel.set(ColorPanel::Closed);
            if let (Some((anchor, head)), Some((from, to))) = (now, link_range.get())
                && untracked(|| ctx.editing_link.get())
            {
                let inside = |p: u32| (from as u32..=to as u32).contains(&p);
                if !inside(anchor) || !inside(head) {
                    ctx.editing_link.set(false);
                }
            }
        }
    });
    let visible = move || {
        let selected = ctx.selection.with(|s| s.as_ref().is_some_and(|s| !s.empty && (s.kind == "text" || s.kind == "cell")));
        (selected || ctx.editing_link.get()) && ctx.slash_open().is_none() && !ctx.read_only.get()
    };
    // Opening the link editor takes the range, its text and its address.
    effect(move || {
        if ctx.editing_link.get() {
            untracked(|| {
                let (range, text, href, linked) = ctx.editor.with(|e| {
                    let state = e.state();
                    let (from, to) = (state.selection().from(), state.selection().to());
                    let href = e.attributes("link").get("href").and_then(|h| h.as_str().map(String::from));
                    (Some((from, to)), state.doc().text_between(from, to, Some(" "), None), href, e.is_active("link", None))
                });
                link_range.replace(range);
                link_was.replace(text.clone());
                link_text.set(text);
                link.set(href.unwrap_or_default());
                had_link.set(linked);
            });
        } else {
            link_range.replace(None);
        }
    });
    // Applies the editor's text and address to its range (no address: the
    // link is removed).
    let apply_link = move || {
        let Some((from, to)) = link_range.get() else { return };
        let href = normalize_href(&link.get_untracked());
        let typed = link_text.get_untracked();
        let text = if typed.trim().is_empty() {
            if from == to { link.get_untracked().trim().to_string() } else { link_was.get() }
        } else {
            typed
        };
        let text = (text != link_was.get() || from == to).then_some(text);
        ctx.editor.with_untracked(|e| e.run_command(&crate::blocks::set_link(from, to, text, href)));
        ctx.editing_link.set(false);
        ctx.changed();
        ctx.editor.with_untracked(|e| e.session.focus());
    };
    dynamic(move || {
        if !visible() {
            return None;
        }
        let fit = fit();
        let surface = view()
            .max_w(px(fit))
            .p(px(4.0))
            .gap(px(4.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(move || theme().border)
            .bg(move || theme().popover)
            .shadow_lg();
        if ctx.editing_link.get() {
            let field = |placeholder: &'static str, value: Signal<String>, focus: bool| {
                nsh_core::element::input()
                    .h(px(28.0))
                    .px_2()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(move || theme().border)
                    .text_size(px(13.0))
                    // Its own colour: inside a note it would take the note's.
                    .text_color(move || theme().popover_foreground)
                    .placeholder(placeholder)
                    .auto_focus(focus)
                    .value(move || value.get())
                    .on_change(move |t: &str| value.set(t.to_string()))
                    .on_submit(move |_: &str| apply_link())
            };
            let new = untracked(|| !had_link.get());
            let mut buttons = view().flex_row().justify_end().gap(px(2.0));
            if !new {
                buttons = buttons.child(button("Remove link", "Remove link", || false, || true, move || {
                    link.set(String::new());
                    link_text.set(String::new());
                    apply_link();
                }));
            }
            buttons = buttons
                .child(button(if new { "Insert" } else { "Apply" }, if new { "Insert link" } else { "Apply link" }, || false, || true, apply_link))
                .child(button("✕", "Cancel", || false, || true, move || ctx.editing_link.set(false)));
            // A new link's address first: its text is optional.
            let text_field = field(if new { "Text to show (optional)" } else { "Text" }, link_text, new);
            let address = field("Paste or type a link…", link, !new);
            return Some(surface.w(px(320.0_f32.min(fit))).child(text_field).child(address).child(buttons).into_element());
        }
        // Table cells selected: what goes on cells.
        if ctx.selection.with(|s| s.as_ref().is_some_and(|s| s.kind == "cell")) {
            let run = move |name: &'static str| {
                move || {
                    ctx.editor.with_untracked(|e| e.run(name, json!({})));
                    ctx.changed();
                }
            };
            let can = move |name: &'static str| {
                move || {
                    ctx.revision.get();
                    ctx.editor.with(|e| e.can(name, &json!({})))
                }
            };
            let cell_color = move || {
                let mut items = vec![MenuItem::new("None").on_select(move || set_cell_color(ctx, None)), MenuItem::separator()];
                items.extend(actions::COLORS.iter().map(|(name, css)| MenuItem::new(*name).on_select(move || set_cell_color(ctx, Some(css)))));
                let _ = popup_menu(items);
            };
            return Some(
                surface
                    .flex_row()
                    .items_center()
                    .gap(px(2.0))
                    .child(button("Merge cells", "Merge the selected cells", || false, can("mergeCells"), run("mergeCells")))
                    .child(button("Split cell", "Split a merged cell", || false, can("splitCell"), run("splitCell")))
                    .child(separator())
                    .child(button("Cell color ▾", "Color the selected cells", || false, || true, cell_color))
                    .into_element(),
            );
        }
        let mut row = view().flex_row().flex_wrap().items_center().gap(px(2.0));
        for (i, group) in actions::SELECTION.iter().enumerate() {
            if i > 0 {
                row = row.child(separator());
            }
            row = row.children(group.iter().map(|id| action_button(ctx, id).into_element()));
            if i == 0 {
                row = row.child(separator()).child(list_button(ctx));
            }
        }
        row = row
            .child(separator())
            .child(color_button(ctx, move || more.set(false)))
            .child(button("…", "More", move || more.get(), || true, move || {
                ctx.color_panel.set(ColorPanel::Closed);
                more.update(|m| *m = !*m);
            }));
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
                    menu_row(action, move || hovered.get(), move |_| hovered.set(true), move || {
                        more.set(false);
                        ctx.perform(action);
                    })
                    .on_mouse_leave(move |_: &MouseEvent| hovered.set(false))
                    .into_element()
                }))
        };
        // The menu can't be drawn outside its window either: in a short
        // note the panel scrolls.
        let room = window.state().map(|s| s.bounds.size.height.0 - 200.0).unwrap_or(400.0).max(110.0);
        let more_panel = view()
            .style_when(move || !more.get(), |s| s.hidden())
            .border_t_1()
            .border_color(move || theme().border)
            .pt_1()
            .child(
                scroll()
                    .max_h(px(room))
                    .child(section("Insert", actions::SELECTION_MORE_INSERT))
                    .child(section("Format", actions::SELECTION_MORE_FORMAT)),
            );
        Some(surface.child(row).child(more_panel).child(color_panel(ctx, room)).into_element())
    })
}

/// The selection menu's list button: the list the selection is in (a
/// bullet when none) and a ▾; it opens a menu of the lists, the current one
/// checked.
fn list_button(ctx: Ctx) -> impl IntoElement {
    let current = move || actions::LISTS.iter().map(|id| actions::get(id)).find(|a| ctx.is_active(a));
    view()
        .flex_row()
        .h(px(28.0))
        .px(px(6.0))
        .gap(px(3.0))
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .bg(move || if current().is_some() { tint(0.12) } else { tint(0.0) })
        .hover(|s| s.bg(tint(0.07)))
        .active(|s| s.opacity(0.6))
        .label("Lists")
        .focusable(false)
        .cursor(Cursor::Pointer)
        .on_press(move || {
            let items = actions::LISTS
                .iter()
                .map(|id| {
                    let action = actions::get(id);
                    MenuItem::new(action.label).checked(untracked(|| ctx.is_active(action))).on_select(move || ctx.perform(action))
                })
                .collect();
            let _ = popup_menu(items);
        })
        .child(hinted(
            hint("Lists", ""),
            view()
                .flex_row()
                .items_center()
                .gap(px(3.0))
                .child(text(move || current().map_or("•", |a| a.glyph)).text_size(px(13.0)).text_color(move || theme().popover_foreground))
                .child(text("▾").text_size(px(9.0)).text_color(move || theme().muted_foreground)),
        ))
}

/// The selection menu's text color button: an "A" over a bar of the
/// current color; it opens and closes the colors panel.
fn color_button(ctx: Ctx, opened: impl Fn() + 'static) -> impl IntoElement {
    let bar = move || -> AdaptiveColor {
        match current_color(ctx).and_then(|c| actions::color_rgb(&c)) {
            Some(rgb) => nsh_core::color::rgb(rgb).into(),
            None => theme().popover_foreground,
        }
    };
    view()
        .w(px(28.0))
        .h(px(28.0))
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .bg(move || if ctx.color_panel.get() != ColorPanel::Closed { tint(0.12) } else { tint(0.0) })
        .hover(|s| s.bg(tint(0.07)))
        .active(|s| s.opacity(0.6))
        .label("Text color")
        .focusable(false)
        .cursor(Cursor::Pointer)
        .on_press(move || {
            opened();
            ctx.color_panel.update(|p| *p = if *p == ColorPanel::Closed { ColorPanel::Named } else { ColorPanel::Closed });
        })
        .child(hinted(
            hint("Text color", ""),
            view()
                .items_center()
                .child(text("A").text_size(px(13.0)).text_color(move || theme().popover_foreground))
                .child(view().w(px(14.0)).h(px(3.0)).rounded(px(1.5)).bg(bar)),
        ))
}

/// The selection menu's colors: the default, the named colors and a "+"
/// for any color, which shows the picker under them. A picked color goes
/// on the selection at once.
fn color_panel(ctx: Ctx, room: f32) -> impl IntoElement {
    // What the colors go on: the text (`color`) or behind it
    // (`backgroundColor`).
    let target = signal("color");
    let dot = move |label: &'static str, css: Option<&'static str>| {
        let rgb = css.and_then(actions::color_rgb);
        let chosen = move || current_style(ctx, target.get()).as_deref() == css;
        view()
            .w(px(22.0))
            .h(px(22.0))
            .rounded_full()
            .items_center()
            .justify_center()
            .border_2()
            .border_color(move || if chosen() { theme().popover_foreground } else { tint(0.18) })
            .bg(move || -> AdaptiveColor {
                match rgb {
                    Some(rgb) => nsh_core::color::rgb(rgb).into(),
                    None => theme().background,
                }
            })
            .hover(|s| s.scale(1.12))
            .label(label)
            .focusable(false)
            .cursor(Cursor::Pointer)
            .on_press(move || set_style(ctx, target.get_untracked(), css))
            .child(hinted(hint(label, ""), text(if rgb.is_none() { "A" } else { "" }).text_size(px(10.0)).text_color(move || theme().foreground)))
            .into_element()
    };
    let custom = view()
        .w(px(22.0))
        .h(px(22.0))
        .rounded_full()
        .items_center()
        .justify_center()
        .border_2()
        .border_color(move || if ctx.color_panel.get() == ColorPanel::Custom { theme().popover_foreground } else { tint(0.18) })
        .hover(|s| s.scale(1.12))
        .label("Custom color")
        .focusable(false)
        .cursor(Cursor::Pointer)
        .on_press(move || ctx.color_panel.update(|p| *p = if *p == ColorPanel::Custom { ColorPanel::Named } else { ColorPanel::Custom }))
        .child(hinted(hint("Custom color", ""), text("+").text_size(px(14.0)).text_color(move || theme().popover_foreground)));
    let picked = move || current_style(ctx, target.get()).and_then(|c| actions::color_rgb(&c)).unwrap_or(0);
    let picker = move || {
        nsh_base::color_picker()
            .value(move || Some(Hsla::from(nsh_core::color::rgb(picked()))))
            .on_change(move |e: &nsh_base::ColorPickerEvent| {
                if let nsh_base::ColorPickerEvent::Change(c, _) = e {
                    let rgb = nsh_core::Color::from(*c).0 >> 8;
                    set_style(ctx, target.get_untracked(), Some(&format!("#{rgb:06x}")));
                }
            })
            .gap(px(8.0))
            .pt(px(6.0))
            .child(
                nsh_base::color_area()
                    .accessibility_label("Saturation and brightness")
                    .w(px(228.0))
                    .h(px(100.0))
                    .rounded(px(8.0))
                    .thumb_class("size-3.5 rounded-full border-2 border-white shadow-md"),
            )
            .child(
                nsh_base::color_slider()
                    .color_space(nsh_base::ColorSpace::Hsb)
                    .channel(nsh_base::ColorChannel::Hue)
                    .accessibility_label("Hue")
                    .w(px(228.0))
                    .h(px(16.0))
                    .justify_center()
                    .track_class("h-3 rounded-full")
                    .thumb_class("size-3.5 rounded-full border-2 border-white shadow-md"),
            )
            .child(
                nsh_base::color_field()
                    .accessibility_label("Hex color")
                    .w(px(228.0))
                    .h(px(28.0))
                    .px_2()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(move || theme().border)
                    .text_size(px(12.0))
                    .text_color(move || theme().popover_foreground),
            )
    };
    view()
        .style_when(move || ctx.color_panel.get() == ColorPanel::Closed, |s| s.hidden())
        .border_t_1()
        .border_color(move || theme().border)
        .pt(px(6.0))
        // The menu can't be drawn outside its window: in a short note the
        // panel scrolls.
        .child(
            scroll().max_h(px(room)).child(
                view()
                    .px(px(6.0))
                    .pb(px(6.0))
                    .gap(px(6.0))
                    .child(
                        view()
                            .flex_row()
                            .gap(px(2.0))
                            .child(button("Text", "Color the text", move || target.get() == "color", || true, move || target.set("color")))
                            .child(button("Highlight", "Color behind the text", move || target.get() == "backgroundColor", || true, move || {
                                target.set("backgroundColor")
                            })),
                    )
                    .child(
                        view()
                            .flex_row()
                            .flex_wrap()
                            .gap(px(6.0))
                            .max_w(px(232.0))
                            .child(dot("Default", None))
                            .children(actions::COLORS.iter().map(move |(name, css)| dot(name, Some(css))))
                            .child(custom),
                    )
                    .child(dynamic(move || (ctx.color_panel.get() == ColorPanel::Custom).then(picker))),
            ),
        )
}

/// Colors the selected cells (`None`: no color).
fn set_cell_color(ctx: Ctx, css: Option<&'static str>) {
    ctx.editor.with_untracked(|e| e.run("setCellAttribute", json!({ "name": "background", "value": css })));
    ctx.changed();
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

/// How a note's text is drawn where it shows: its colour (`None`: the
/// theme's), its size in points, its font family (empty: the app's), and
/// whether it's in a small floating note (tighter margins).
#[derive(Clone, Debug, PartialEq)]
pub struct TextLook {
    pub color: Option<u32>,
    /// How dark selected text's background is, in percent.
    pub selection: u32,
    pub size: f32,
    pub font: String,
    pub compact: bool,
}

/// The colours the document's details are drawn in (borders, code
/// backgrounds, muted text, links), as class fragments after `text-`,
/// `bg-` or `border-`. On a note they're the note's own text colour made
/// translucent, so they read on any background colour; elsewhere (`None`)
/// they're the theme's.
struct Inks {
    /// Muted text: placeholders, list markers, quotes.
    muted: String,
    /// A faint fill: inline code, code blocks, table headers.
    fill: String,
    /// Lines: quote bars, rules, table borders, checkboxes.
    line: String,
    /// A checked checkbox.
    checked: String,
    link: &'static str,
}

impl Inks {
    fn for_color(color: Option<u32>) -> Self {
        match color {
            Some(rgb) => {
                // Light text means a dark note: a lighter blue for links.
                let (r, g, b) = ((rgb >> 16) & 0xff, (rgb >> 8) & 0xff, rgb & 0xff);
                let light_text = 299 * r + 587 * g + 114 * b > 128_000;
                Self {
                    muted: format!("[color:#{rgb:06x}]/65"),
                    fill: format!("[#{rgb:06x}]/10"),
                    line: format!("[#{rgb:06x}]/25"),
                    checked: format!("[#{rgb:06x}]/70"),
                    link: if light_text { "sky-300" } else { "blue-700" },
                }
            }
            None => Self {
                muted: "muted-foreground".into(),
                fill: "muted".into(),
                line: "border".into(),
                checked: "primary".into(),
                link: "primary",
            },
        }
    }
}

/// The editor's classes: NSH's editor draws nothing of its own, so every
/// node and mark gets its look here (after NSH's gallery look).
pub fn editor_classes(look: &TextLook) -> RichEditorClasses {
    let mut classes = RichEditorClasses::default();
    let mut content = String::from(if look.compact { "gap-2 px-1.5 pt-0.5 pb-3" } else { "gap-3 px-2 py-5" });
    content.push_str(&format!(" text-[length:{}px]", look.size));
    if let Some(color) = look.color {
        content.push_str(&format!(" text-[color:#{color:06x}]"));
    }
    if !look.font.is_empty() {
        content.push_str(&format!(" font-[family-name:'{}']", look.font.replace('\'', "").replace(' ', "_")));
    }
    let Inks { muted, fill, line, checked, link } = Inks::for_color(look.color);
    classes.content = Some(content.into());
    // Selected text: the text's own colour, see-through, so it reads on any
    // note colour.
    let strength = look.selection.clamp(5, 90);
    classes.selection = Some(match look.color {
        Some(color) => format!("bg-[#{color:06x}]/{strength}").into(),
        None => format!("bg-primary/{strength}").into(),
    });
    // The caret in the text's colour (the theme's would be white on a pale
    // note in dark mode).
    if let Some(color) = look.color {
        classes.caret = Some(format!("bg-[#{color:06x}]").into());
    }
    classes.placeholder = Some(format!("text-{muted}").into());
    classes.list_marker = Some(format!("text-{muted}").into());
    classes.task_checkbox =
        Some(format!("size-4 mr-2 mt-1 rounded cursor-pointer border border-{line} checked:bg-{checked} checked:border-{checked}").into());
    let nodes: Vec<(&str, String)> = vec![
        ("paragraph", "leading-relaxed".into()),
        ("heading[level=1]", "text-3xl font-bold tracking-tight".into()),
        ("heading[level=2]", "text-2xl font-semibold tracking-tight".into()),
        ("heading[level=3]", "text-xl font-semibold".into()),
        ("blockquote", format!("border-l-4 border-{line} pl-4 text-{muted} italic")),
        ("codeBlock", format!("font-mono text-sm bg-{fill} rounded-lg px-4 py-3")),
        ("bulletList", "gap-1.5 pl-6".into()),
        ("orderedList", "gap-1.5 pl-6".into()),
        ("taskList", "gap-1.5".into()),
        ("horizontalRule", format!("h-px bg-{line} my-3")),
        ("image", "rounded-lg max-w-full".into()),
        ("table", format!("border border-{line} rounded")),
        ("tableHeader", format!("font-semibold bg-{fill} px-3 py-1.5 border border-{line}")),
        ("tableCell", format!("px-3 py-1.5 border border-{line}")),
        // A cell's own color, see-through like a highlight.
        ("tableCell[background=*]", "bg-[{}]/30".into()),
        ("tableHeader[background=*]", "bg-[{}]/30".into()),
        // Alignment is an attribute of paragraphs and headings.
        ("paragraph[textAlign=center]", "text-center".into()),
        ("paragraph[textAlign=right]", "text-right".into()),
        ("heading[textAlign=center]", "text-center".into()),
        ("heading[textAlign=right]", "text-right".into()),
    ];
    let toggles = toggle::CLASSES.iter().map(|(k, v)| (*k, v.to_string()));
    classes.nodes = Some(nodes.into_iter().chain(toggles).map(|(k, v)| (k.to_string(), v)).collect::<BTreeMap<_, _>>());
    let marks: Vec<(String, String)> = vec![
        ("bold".into(), "font-bold".into()),
        ("italic".into(), "italic".into()),
        ("underline".into(), "underline".into()),
        ("strike".into(), "line-through".into()),
        ("code".into(), format!("font-mono text-sm bg-{fill} rounded px-1")),
        ("link".into(), format!("text-{link} underline")),
        // Its own dark text: readable on any note.
        ("highlight".into(), "bg-yellow-200 text-black rounded-sm".into()),
        // NSH's text layout doesn't shift the baseline: smaller text.
        ("subscript".into(), "text-xs".into()),
        ("superscript".into(), "text-xs".into()),
    ];
    // Text color is `textStyle`'s `color`: any color, drawn as it is.
    let colors = [
        ("textStyle[color=*]", "text-[color:{}]"),
        // A highlight color, see-through so the text reads on it.
        ("textStyle[backgroundColor=*]", "bg-[{}]/40 rounded-sm"),
        ("textStyle[fontSize=*]", "text-[length:{}]"),
        ("textStyle[fontFamily=*]", "font-[family-name:'{}']"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()));
    classes.marks = Some(marks.into_iter().chain(colors).collect());
    classes
}
