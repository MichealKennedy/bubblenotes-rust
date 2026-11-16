//! A note floating on the desktop: a frameless, see-through window whose
//! only visible part is the note's rounded card (title, text, resize grip).
//!
//! The window's bounds are always exactly what shows (`shape_rect`): the
//! whole note, only its title when rolled up, or its bubble. A see-through
//! window gets no click-through, so anything bigger would swallow clicks.

use nsh_core::Color;
use nsh_core::desktop::{ResizeEdge, WindowAttributes, WindowFrame, WindowFrameStyle, WindowLevel, WindowPosition};
use nsh_core::element::desktop::{Window, current_window, window};
use nsh_core::prelude::*;

use crate::app::{App, Note, NoteCommand};
use crate::model::{NoteMeta, Rect, Rgb};
use crate::ui::TextLook;
use crate::{bubble, drag, editor_view, layout, note_menu, win};

/// The title bar's height, and a rolled-up note's.
pub const TITLE_HEIGHT: f32 = 30.0;
/// The strip a note without a title is dragged by.
const HANDLE_HEIGHT: f32 = 12.0;
const MIN_WIDTH: f32 = 140.0;
const MIN_HEIGHT: f32 = 90.0;

/// `rgb` with `alpha` (0 to 1).
pub fn color(rgb: Rgb, alpha: f32) -> Color {
    Color((rgb << 8) | (alpha.clamp(0.0, 1.0) * 255.0).round() as u32)
}

/// The window's title: the note's, then (unseen) its id, so the window can
/// be told from another note's of the same name (`win::set_click_through`
/// finds it by its title).
fn window_title(meta: &NoteMeta) -> String {
    format!("{}\u{2063}{}", meta.title, meta.id)
}

/// The window's id for a note.
pub fn window_id(note_id: &str) -> String {
    format!("note:{note_id}")
}

/// Where the note's window is and how big, for how it shows now.
/// `bubble_open`: the note is a bubble and it's open.
pub fn shape_rect(meta: &NoteMeta, bubble_open: bool) -> Rect {
    if meta.bubble.enabled {
        return bubble::window_rect(meta, bubble_open);
    }
    let height = if meta.rolled_up { TITLE_HEIGHT } else { meta.rect.h };
    Rect { h: height, ..meta.rect }
}

fn frame(rect: Rect) -> WindowFrame {
    WindowFrame { x: Some(px(rect.x)), y: Some(px(rect.y)), width: Some(px(rect.w)), height: Some(px(rect.h)), animated: None }
}

fn attributes(always_on_top: bool, resizable: bool) -> WindowAttributes {
    let mut a = WindowAttributes::default();
    a.frame = Some(WindowFrameStyle::None);
    a.transparent = Some(true);
    a.taskbar = Some(false);
    a.shadow = Some(false);
    a.resizable = Some(resizable);
    a.level = Some(if always_on_top { WindowLevel::Floating } else { WindowLevel::Normal });
    a
}

impl App {
    /// Docks the note to `edge` of the screen it's on, or undocks it.
    pub fn dock(self, note: &Note, edge: Option<crate::model::Edge>) {
        self.update(note, |m| m.docked = edge);
    }

    /// Where the other notes on the desktop are (their windows as they
    /// show when not opened from a bubble): what a dragged note snaps to.
    pub fn other_note_windows(self, id: &str) -> Vec<Rect> {
        untracked(|| self.desktop_notes()).iter().filter(|n| &*n.id != id).map(|n| n.meta.with_untracked(|m| shape_rect(m, false))).collect()
    }

    /// Brings back the notes that are off every screen (a monitor was
    /// unplugged, the resolution changed). Reads the displays reactively:
    /// call it in an effect, and it runs again when they change.
    pub fn rescue_notes(self) {
        let screens: Vec<Rect> = nsh_core::element::desktop::displays()
            .iter()
            .map(|d| Rect { x: d.visible_bounds.origin.x.0, y: d.visible_bounds.origin.y.0, w: d.visible_bounds.size.width.0, h: d.visible_bounds.size.height.0 })
            .collect();
        untracked(|| {
            for note in self.notes.get_untracked() {
                let meta = note.meta.get_untracked();
                if meta.trashed.is_some() || !meta.on_desktop {
                    continue;
                }
                let Some(to) = layout::rescued(shape_rect(&meta, false), &screens) else { continue };
                self.update(&note, |m| {
                    if m.bubble.enabled {
                        m.bubble.x = to.x;
                        m.bubble.y = to.y;
                    } else {
                        m.rect.x = to.x;
                        m.rect.y = to.y;
                    }
                });
            }
        });
    }
}

/// The desktop window of `note`. Put it in the tree while the note is on
/// the desktop.
pub fn note_window(app: App, note: Note) -> Window {
    let meta = note.meta;
    // A bubble's note is shut until the pointer or a click opens it.
    let bubble_open = signal(false);
    let start = shape_rect(&meta.get_untracked(), false);
    // What the window's title and attributes come from, set only when it
    // really changes: NSH applies a window's attributes again each time
    // they're given, which takes the keyboard focus from the text, and the
    // note changes on every save.
    let facts = move || meta.with(|m| (window_title(m), m.look.always_on_top, !m.locked && !m.rolled_up && (!m.bubble.enabled || bubble_open.get())));
    let shown = signal(untracked(facts));
    effect(move || {
        let now = facts();
        if shown.with_untracked(|s| *s != now) {
            shown.set(now);
        }
    });
    window()
        .id(window_id(&note.id))
        .title(move || shown.with(|s| s.0.clone()))
        .default_size(size(px(start.w), px(start.h)))
        .default_position(WindowPosition::At(point(px(start.x), px(start.y))))
        .attributes(move || shown.with(|s| attributes(s.1, s.2)))
        .content(move || {
            let window = current_window();

            // The window takes the shape the note has now.
            let shaped = window.clone();
            let last = stored(start);
            effect(move || {
                let rect = meta.with(|m| shape_rect(m, bubble_open.get()));
                if last.get() != rect {
                    last.replace(rect);
                    shaped.set_bounds(frame(rect));
                }
            });
            // And the note remembers where the user put its window.
            let note_moved = note.clone();
            window.on_bounds_change(move |b| {
                let now = Rect { x: b.origin.x.0, y: b.origin.y.0, w: b.size.width.0, h: b.size.height.0 };
                // The report comes a moment after the change, and rounded to
                // whole pixels. So a window the app itself shaped (in any of
                // the note's shapes: by now it may have another) is told by
                // being about where the app put it, and is not the user's
                // doing: taking it for a move would drag the note a little
                // each time, or far when the shape has changed since.
                let about = |a: f32, b: f32| (a - b).abs() <= 1.0;
                let near = |r: Rect| about(now.x, r.x) && about(now.y, r.y) && about(now.w, r.w) && about(now.h, r.h);
                let m = meta.get_untracked();
                if m.bubble.enabled {
                    let (shut, open) = (bubble::window_rect(&m, false), bubble::window_rect(&m, true));
                    if near(shut) || near(open) {
                        return;
                    }
                    // Which of the two the user moved shows in its size (a
                    // resized one is the open one).
                    let is_open = !(about(now.w, shut.w) && about(now.h, shut.h));
                    last.replace(now);
                    app.update(&note_moved, |m| bubble::window_moved(m, is_open, now));
                } else {
                    if near(m.rect) || near(Rect { h: TITLE_HEIGHT, ..m.rect }) {
                        return;
                    }
                    let rolled = about(now.h, TITLE_HEIGHT);
                    last.replace(now);
                    app.update(&note_moved, |m| {
                        m.rect.x = now.x;
                        m.rect.y = now.y;
                        m.rect.w = now.w.max(MIN_WIDTH);
                        if !rolled {
                            m.rect.h = now.h.max(MIN_HEIGHT);
                        }
                    });
                }
            });

            // A ghost note lets clicks through (unless Ctrl+Alt is held).
            // NSH sets the window's styles again whenever its attributes
            // change, so this runs after every change of the note, a
            // moment later (when NSH is done).
            let ghostly = stored(false);
            effect(move || {
                let ghost = meta.with(|m| m.ghost) && !app.ghosts_solid.get();
                if !ghost && !ghostly.get() {
                    return;
                }
                app.owner.run(|| {
                    let _ = set_timeout(std::time::Duration::from_millis(200), move || {
                        let title = meta.with_untracked(window_title);
                        let ghost = meta.with_untracked(|m| m.ghost) && !app.ghosts_solid.get_untracked();
                        if win::set_click_through(&title, ghost) {
                            ghostly.replace(ghost);
                        }
                    });
                });
            });

            // What menus ask of the window.
            let (commanded, command_id) = (window.clone(), note.id.clone());
            let sized_note = note.clone();
            effect(move || {
                let Some((id, command)) = app.command.get() else { return };
                if id != command_id {
                    return;
                }
                app.command.set(None);
                match command {
                    NoteCommand::Focus => commanded.focus(),
                    NoteCommand::SendToBack => {
                        // A one-time push behind every other window.
                        let mut back = WindowAttributes::default();
                        back.level = Some(WindowLevel::DesktopIcon);
                        commanded.set(back);
                    }
                    NoteCommand::AutoSize => {
                        // As tall as the text: the end of the document's place
                        // in the window, plus the room under it.
                        let editor = &sized_note.editor;
                        let end = editor.doc().content().size();
                        // The very end isn't a place in the text: the last
                        // one that is has coordinates.
                        let last = (0..4).find_map(|back| editor.session.page_coords_at_pos(end.saturating_sub(back)));
                        let Some(last) = last else { return };
                        let wanted = (last.origin.y.0 + last.size.height.0 + 18.0).clamp(MIN_HEIGHT, 900.0);
                        app.update(&sized_note, |m| {
                            if !m.bubble.enabled && !m.rolled_up {
                                m.rect.h = wanted;
                            }
                        });
                    }
                }
            });

            let (card_note, bubble_note) = (note.clone(), note.clone());
            // Only switching between note and bubble makes the content
            // again: any other change (a move, a save) must leave the editor
            // as it is, with its focus and scroll position.
            let is_bubble = signal(meta.with_untracked(|m| m.bubble.enabled));
            effect(move || {
                let now = meta.with(|m| m.bubble.enabled);
                if is_bubble.get_untracked() != now {
                    is_bubble.set(now);
                }
            });
            dynamic(move || {
                let is_bubble = is_bubble.get();
                // Built untracked: whatever building it reads (the note's
                // settings among it) must not make it again.
                untracked(|| {
                    if is_bubble {
                        let inner = card_note.clone();
                        bubble::bubble_view(app, bubble_note.clone(), bubble_open, move || card(app, inner.clone()).into_element()).into_element()
                    } else {
                        card(app, card_note.clone()).into_element()
                    }
                })
            })
        })
}

/// The note's card: what a note looks like, filling its parent.
pub fn card(app: App, note: Note) -> impl IntoElement {
    // Built untracked: what building it reads of the note (its look, its
    // title) must not make whatever holds the card build it again, which
    // would take the keyboard focus from the text on every save.
    untracked(move || card_built(app, note))
}

fn card_built(app: App, note: Note) -> impl IntoElement {
    let meta = note.meta;
    let window = current_window();
    let look = move || meta.with(|m| m.look.clone());
    let menu_note = note.clone();
    let ctx = editor_view::ctx_for(app, &note);

    let body = view()
        .flex_1()
        .px(px(6.0))
        .style_when(move || meta.with(|m| m.rolled_up), |s| s.hidden())
        .child(editor_view::note_editor(ctx, move || !meta.with(|m| m.locked), "Type here…", move || {
            let look = look();
            TextLook { color: Some(look.body_color), selection: app.settings.with(|s| s.selection_strength), size: look.body_size, font: look.body_font, compact: true }
        }));

    view()
        .flex_1()
        .rounded(move || px(look().corner_radius))
        .overflow_hidden()
        .bg(move || color(look().background, look().opacity))
        .style_when(
            move || look().background_to.is_some(),
            move |s| {
                let look = look();
                s.bg_linear_to_b()
                    .gradient_from(color(look.background, look.opacity))
                    .gradient_to(color(look.background_to.unwrap_or(look.background), look.opacity))
            },
        )
        .on_context_menu(move |_: &ContextMenuEvent| note_menu::show(app, &menu_note))
        .child(title_bar(app, note.clone(), window))
        .child(body)
        .child(grip(note))
}

/// The title: drags the note, a double click rolls it up or down. A note
/// with its title hidden keeps a thin strip to drag it by.
fn title_bar(app: App, note: Note, window: nsh_core::element::desktop::WindowRef) -> impl IntoElement {
    let meta = note.meta;
    let shown = move || meta.with(|m| m.look.show_title || m.rolled_up);
    let dragged = signal(false);
    let press_note = note.clone();
    let drag_id = note.id.clone();
    view()
        .h(move || px(if shown() { TITLE_HEIGHT } else { HANDLE_HEIGHT }))
        .px(px(12.0))
        .justify_center()
        .cursor(Cursor::Default)
        .on_press_down(move || {
            if meta.with_untracked(|m| m.locked) {
                return;
            }
            let id = drag_id.clone();
            drag::set_neighbours(app.other_note_windows(&id));
            drag::drag_window(
                app.owner,
                window.clone(),
                move |x, y| app.dragging.set(Some((id.clone(), x, y))),
                move |moved| {
                    dragged.set(moved);
                    app.dragging.set(None);
                },
            );
        })
        .on_press(move |e: &PressEvent| {
            if e.clicks == 2 && !dragged.get_untracked() {
                app.update(&press_note, |m| m.rolled_up = !m.rolled_up);
            }
        })
        .child(
            text(move || if shown() { meta.with(|m| m.title.clone()) } else { String::new() })
                .truncate()
                .text_size(move || px(meta.with(|m| m.look.title_size)))
                .style_when(move || meta.with(|m| m.look.title_bold), |s| s.font_weight(FontWeight::SEMIBOLD))
                .text_color(move || color(meta.with(|m| m.look.title_color), 1.0)),
        )
}

/// The resize grip in the bottom right corner: three diagonal lines.
fn grip(note: Note) -> impl IntoElement {
    let meta = note.meta;
    view()
        .absolute()
        .right(px(0.0))
        .bottom(px(0.0))
        .w(px(18.0))
        .h(px(18.0))
        .cursor(Cursor::NwseResize)
        .style_when(move || meta.with(|m| m.rolled_up || m.locked), |s| s.hidden())
        .window_resize(ResizeEdge::BottomRight)
        .child(canvas(move || {
            let ink = color(meta.with(|m| m.look.body_color), 0.45);
            let mut painter = Painter::new().stroke_color(ink).line_width(1.2);
            for inset in [5.0_f32, 9.0, 13.0] {
                painter = painter.begin_path().move_to(px(18.0 - 4.0), px(18.0 - inset)).line_to(px(18.0 - inset), px(18.0 - 4.0)).stroke();
            }
            painter
        }).w(px(18.0)).h(px(18.0)))
}
