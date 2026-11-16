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
use crate::model::{Edge, NoteMeta, Rect, Rgb};
use crate::ui::TextLook;
use crate::{bubble, drag, editor_view, layout, note_menu, win};

/// The title bar's height, and a rolled-up note's.
pub const TITLE_HEIGHT: f32 = 30.0;
/// The strip a note without a title is dragged by.
const HANDLE_HEIGHT: f32 = 12.0;
/// How much of a docked note shows at its screen edge while it's tucked
/// away, in points.
const DOCK_STRIP: f32 = 14.0;
/// How long after it stops being used a docked note slides away
/// (milliseconds).
const DOCK_AFTER: f64 = 450.0;
/// How long after it stops being used a note fades (milliseconds).
const FADE_AFTER: f64 = 600.0;
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
    shape_rect_at(meta, bubble_open, 0.0)
}

/// As `shape_rect`, for a docked note slid out by `docked_out` (0: tucked
/// away, 1: all of it).
pub fn shape_rect_at(meta: &NoteMeta, bubble_open: bool, docked_out: f32) -> Rect {
    if meta.bubble.enabled {
        return bubble::window_rect(meta, bubble_open);
    }
    let height = if meta.rolled_up { TITLE_HEIGHT } else { meta.rect.h };
    let rect = Rect { h: height, ..meta.rect };
    match meta.docked {
        Some(edge) => docked_rect(rect, edge, docked_out, &drag::work_areas()),
        None => rect,
    }
}

/// Where the note's window is while the note is in use: a docked note
/// slid out. What a panel beside it stands by.
pub fn rest_rect(meta: &NoteMeta) -> Rect {
    shape_rect_at(meta, false, 1.0)
}

/// The work area a window at `rect` is on: the one holding its middle,
/// else the nearest.
fn area_for(rect: Rect, areas: &[Rect]) -> Option<Rect> {
    let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
    let distance = |a: &Rect| {
        let dx = (a.x - cx).max(cx - (a.x + a.w)).max(0.0);
        let dy = (a.y - cy).max(cy - (a.y + a.h)).max(0.0);
        dx * dx + dy * dy
    };
    areas.iter().min_by(|a, b| distance(a).total_cmp(&distance(b))).copied()
}

/// The window of a note of `rect` docked to `edge` of its screen (one of
/// `areas`, the work areas), slid out by `out`: flush with the edge, as far
/// along it as the note was, and inside the screen. Tucked away, only a
/// strip along the edge shows.
pub fn docked_rect(rect: Rect, edge: Edge, out: f32, areas: &[Rect]) -> Rect {
    let Some(a) = area_for(rect, areas) else { return rect };
    let out = out.clamp(0.0, 1.0);
    let along = |at: f32, length: f32, start: f32, room: f32| at.min(start + room - length).max(start);
    let shown = |length: f32| DOCK_STRIP.min(length) + (length - DOCK_STRIP.min(length)) * out;
    match edge {
        Edge::Left => Rect { x: a.x, y: along(rect.y, rect.h, a.y, a.h), w: shown(rect.w), h: rect.h },
        Edge::Right => Rect { x: a.x + a.w - shown(rect.w), y: along(rect.y, rect.h, a.y, a.h), w: shown(rect.w), h: rect.h },
        Edge::Top => Rect { x: along(rect.x, rect.w, a.x, a.w), y: a.y, w: rect.w, h: shown(rect.h) },
    }
}

/// The edge of its screen the pointer is at (`at`, in points), when it's
/// at one a note can dock to.
fn edge_at(at: (f32, f32), areas: &[Rect]) -> Option<Edge> {
    let a = area_for(Rect { x: at.0, y: at.1, w: 0.0, h: 0.0 }, areas)?;
    if at.0 <= a.x + 1.5 {
        Some(Edge::Left)
    } else if at.0 >= a.x + a.w - 2.5 {
        Some(Edge::Right)
    } else if at.1 <= a.y + 1.5 {
        Some(Edge::Top)
    } else {
        None
    }
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
    /// Docks the note to `edge` of the screen it's on, or undocks it (it
    /// then stands a little off the edge it was at).
    pub fn dock(self, note: &Note, edge: Option<Edge>) {
        self.update(note, |m| match edge {
            Some(_) => {
                m.docked = edge;
                m.rolled_up = false;
                m.bubble.enabled = false;
            }
            None => {
                if let Some(was) = m.docked {
                    let out = rest_rect(m);
                    (m.rect.x, m.rect.y) = match was {
                        Edge::Left => (out.x + 24.0, out.y),
                        Edge::Right => (out.x - 24.0, out.y),
                        Edge::Top => (out.x, out.y + 24.0),
                    };
                }
                m.docked = None;
            }
        });
    }

    /// The note's window was dropped at `at` (its top left corner, in
    /// points) with the pointer at `pointer`: dropped with the pointer at a
    /// screen edge, the note docks there; a docked note dragged away from
    /// its edge undocks, and one dragged along it stays.
    pub fn dropped(self, note: &Note, at: (f32, f32), pointer: Option<(f32, f32)>) {
        let meta = note.meta.get_untracked();
        if meta.bubble.enabled {
            return;
        }
        let areas = drag::work_areas();
        let edge = pointer.and_then(|p| edge_at(p, &areas));
        let here = Rect { x: at.0, y: at.1, ..meta.rect };
        let stays = meta.docked.filter(|&edge| {
            let flush = docked_rect(here, edge, 1.0, &areas);
            (flush.x - here.x).abs() < 40.0 && (flush.y - here.y).abs() < 40.0
        });
        match (edge.or(stays), meta.docked) {
            (Some(edge), _) => self.update(note, |m| {
                // Where it stands slid out, so it's on that screen for sure.
                let flush = docked_rect(here, edge, 1.0, &areas);
                (m.rect.x, m.rect.y) = (flush.x, flush.y);
                m.docked = Some(edge);
                m.rolled_up = false;
            }),
            (None, Some(_)) => self.update(note, |m| {
                (m.rect.x, m.rect.y) = at;
                m.docked = None;
            }),
            (None, None) => {}
        }
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
                // (A docked note is at its screen's edge wherever that is.)
                if meta.trashed.is_some() || !meta.on_desktop || (meta.docked.is_some() && !meta.bubble.enabled) {
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
    // (A docked note is on top: its strip is of no use under a window.)
    let docked = move |m: &NoteMeta| m.docked.is_some() && !m.bubble.enabled;
    let facts = move || meta.with(|m| (window_title(m), m.look.always_on_top || docked(m), !m.locked && !m.rolled_up && (!m.bubble.enabled || bubble_open.get())));
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
            // How far a docked note is slid out: 0 tucked away, 1 all of it.
            let dock_out = signal(0.0_f32);
            // When the app last shaped the window (0: not since it opened).
            let shaped_at = stored(0.0_f64);
            effect(move || {
                let rect = meta.with(|m| shape_rect_at(m, bubble_open.get(), dock_out.get()));
                if last.get() != rect {
                    last.replace(rect);
                    shaped_at.replace(crate::store::now_ms());
                    shaped.set_bounds(frame(rect));
                }
            });
            // A docked note's new size, while the grip is still held.
            let resized = stored(None::<Rect>);
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
                if docked(&m) {
                    // A docked note is where its edge is, and slides: the
                    // reports of that come late, and any could look like a
                    // resize. Only a size given well after the app last
                    // shaped the window, with all of the note out, is the
                    // user's (the grip), and it's taken once the grip is
                    // let go: the note then stands flush again.
                    let full = shape_rect_at(&m, false, 1.0);
                    let settled = crate::store::now_ms() - shaped_at.get() > 600.0;
                    if dock_out.get_untracked() >= 1.0 && settled && !(about(now.w, full.w) && about(now.h, full.h)) {
                        if win::primary_button_down() {
                            resized.replace(Some(now));
                        } else {
                            resized.replace(None);
                            last.replace(now);
                            app.update(&note_moved, |m| {
                                m.rect.w = now.w.max(MIN_WIDTH);
                                m.rect.h = now.h.max(MIN_HEIGHT);
                            });
                        }
                    }
                    return;
                }
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

            // Whether the note is in use: under the pointer, in front (being
            // typed in), dragged, or with its menu or its configuration
            // panel open. A moment after none of those holds, a note that
            // fades when idle fades, and a docked one slides away.
            //
            // `solid` is how solid a fading note is now, from 0 (faded as far
            // as it goes) to 1. Only signals change here, read by styles and
            // by the window's shape: nothing is built again.
            let solid = signal(1.0_f32);
            {
                let (window, id) = (window.clone(), note.id.clone());
                let resized_note = note.clone();
                let idle_since = stored(None::<f64>);
                let _ = set_interval(std::time::Duration::from_millis(40), move || {
                    let (fades, docks) = meta.with_untracked(|m| (m.fade_idle, docked(m)));
                    let (level, out) = (solid.get_untracked(), dock_out.get_untracked());
                    if !fades && !docks {
                        if level != 1.0 {
                            solid.set(1.0);
                        }
                        if out != 0.0 {
                            dock_out.set(0.0);
                        }
                        return;
                    }
                    let is_this = |of: Option<std::rc::Rc<str>>| of.is_some_and(|of| of == id);
                    // Under the pointer: by where the app put the window
                    // (NSH's own word on that, and on the focus, is late).
                    let scale = untracked(|| window.state()).map_or(1.0, |s| s.scale_factor.max(0.1));
                    let over = win::cursor().is_some_and(|(x, y)| {
                        let (x, y, r) = (x / scale, y / scale, last.get());
                        x >= r.x - 0.5 && x < r.x + r.w + 0.5 && y >= r.y - 0.5 && y < r.y + r.h + 0.5
                    });
                    let active = over
                        || win::front_window_title().is_some_and(|front| meta.with_untracked(|m| front == window_title(m)))
                        || is_this(app.dragging.with_untracked(|d| d.as_ref().map(|d| d.0.clone())))
                        || is_this(app.config_for.get_untracked())
                        || is_this(app.menu_for.get_untracked());
                    let now = crate::store::now_ms();
                    let idle = if active {
                        idle_since.replace(None);
                        0.0
                    } else {
                        let since = idle_since.get().unwrap_or(now);
                        idle_since.replace(Some(since));
                        now - since
                    };
                    // Fading: in at once (about 100 ms), out gently (about 300).
                    let target = if !fades || idle < FADE_AFTER { 1.0 } else { 0.0 };
                    if level != target {
                        let step = if target > level { 0.4 } else { 0.13 };
                        solid.set(if (target - level).abs() <= step { target } else { level + step * (target - level).signum() });
                    }
                    // Sliding: out when used, away a moment after.
                    let target = if docks && (active || (out > 0.0 && idle < DOCK_AFTER)) { 1.0 } else { 0.0 };
                    if out != target {
                        let step = 0.25;
                        dock_out.set(if (target - out).abs() <= step { target } else { out + step * (target - out).signum() });
                    }
                    // A docked note's new size, once its grip is let go.
                    if let Some(size) = resized.get()
                        && !win::primary_button_down()
                    {
                        resized.replace(None);
                        if docks && out >= 1.0 {
                            last.replace(size);
                            app.update(&resized_note, |m| {
                                m.rect.w = size.w.max(MIN_WIDTH);
                                m.rect.h = size.h.max(MIN_HEIGHT);
                            });
                        }
                    }
                });
            }

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
            let content = dynamic(move || {
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
            });
            // A docked note sliding keeps its size and is uncovered from its
            // edge: it's laid out whole, held at the side that moves.
            let sliding = move || meta.with(docked) && dock_out.get() < 1.0;
            let held = view().flex_1().style_when(sliding, move |s| {
                let (edge, w, h) = meta.with(|m| (m.docked.unwrap_or(Edge::Left), m.rect.w, if m.rolled_up { TITLE_HEIGHT } else { m.rect.h }));
                let s = s.absolute().w(px(w)).h(px(h));
                match edge {
                    Edge::Left => s.right(px(0.0)).top(px(0.0)),
                    Edge::Right => s.left(px(0.0)).top(px(0.0)),
                    Edge::Top => s.left(px(0.0)).bottom(px(0.0)),
                }
            });
            // Tucked away, the strip that shows is a plain tab in the
            // note's colour, round on its inner side.
            let tab = view()
                .absolute()
                .inset_0()
                .style_when(move || !(meta.with(docked) && dock_out.get() <= 0.0), |s| s.hidden())
                .bg(move || color(meta.with(|m| m.look.background_to.unwrap_or(m.look.background)), 1.0))
                .style_when(move || meta.with(|m| m.docked == Some(Edge::Left)), |s| s.rounded_r(px(6.0)))
                .style_when(move || meta.with(|m| m.docked == Some(Edge::Right)), |s| s.rounded_l(px(6.0)))
                .style_when(move || meta.with(|m| m.docked == Some(Edge::Top)), |s| s.rounded_b(px(6.0)))
                .items_center()
                .justify_center()
                .child(
                    view()
                        .rounded(px(2.0))
                        .bg(move || color(meta.with(|m| m.look.body_color), 0.45))
                        .w(move || px(if meta.with(|m| m.docked == Some(Edge::Top)) { 36.0 } else { 3.0 }))
                        .h(move || px(if meta.with(|m| m.docked == Some(Edge::Top)) { 3.0 } else { 36.0 })),
                );
            view()
                .flex_1()
                .overflow_hidden()
                .opacity(move || 1.0 - (1.0 - solid.get()) * (1.0 - meta.with(|m| m.idle_opacity.clamp(0.05, 1.0))))
                .child(held.child(content))
                .child(tab)
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
    let (press_note, drop_note) = (note.clone(), note.clone());
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
            let (drop_note, drop_window) = (drop_note.clone(), window.clone());
            drag::set_neighbours(app.other_note_windows(&id));
            drag::drag_window(
                app.owner,
                window.clone(),
                move |x, y| app.dragging.set(Some((id.clone(), x, y))),
                move |moved| {
                    dragged.set(moved);
                    let at = app.dragging.get_untracked().map(|(_, x, y)| (x, y));
                    app.dragging.set(None);
                    if let (true, Some(at)) = (moved, at) {
                        let scale = untracked(|| drop_window.state()).map_or(1.0, |s| s.scale_factor.max(0.1));
                        app.dropped(&drop_note, at, win::cursor().map(|(x, y)| (x / scale, y / scale)));
                    }
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

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect { x: 0.0, y: 0.0, w: 1000.0, h: 800.0 };
    const SECOND: Rect = Rect { x: 1000.0, y: 0.0, w: 800.0, h: 600.0 };

    #[test]
    fn a_docked_note_is_flush_with_its_edge_and_tucks_to_a_strip() {
        let note = Rect { x: 300.0, y: 200.0, w: 340.0, h: 420.0 };
        let at = |edge, out| docked_rect(note, edge, out, &[SCREEN, SECOND]);
        assert_eq!(at(Edge::Left, 1.0), Rect { x: 0.0, y: 200.0, w: 340.0, h: 420.0 });
        assert_eq!(at(Edge::Left, 0.0), Rect { x: 0.0, y: 200.0, w: DOCK_STRIP, h: 420.0 });
        assert_eq!(at(Edge::Right, 1.0), Rect { x: 660.0, y: 200.0, w: 340.0, h: 420.0 });
        assert_eq!(at(Edge::Right, 0.0), Rect { x: 1000.0 - DOCK_STRIP, y: 200.0, w: DOCK_STRIP, h: 420.0 });
        assert_eq!(at(Edge::Top, 1.0), Rect { x: 300.0, y: 0.0, w: 340.0, h: 420.0 });
        assert_eq!(at(Edge::Top, 0.0), Rect { x: 300.0, y: 0.0, w: 340.0, h: DOCK_STRIP });
        // Halfway out.
        assert_eq!(at(Edge::Left, 0.5).w, DOCK_STRIP + (340.0 - DOCK_STRIP) / 2.0);
        // It stays on its screen along the edge, and on the screen it's on.
        let low = Rect { y: 700.0, ..note };
        assert_eq!(docked_rect(low, Edge::Left, 1.0, &[SCREEN]).y, 380.0);
        let other = Rect { x: 1200.0, ..note };
        assert_eq!(docked_rect(other, Edge::Left, 1.0, &[SCREEN, SECOND]).x, 1000.0);
        assert_eq!(docked_rect(other, Edge::Right, 0.0, &[SCREEN, SECOND]).x, 1800.0 - DOCK_STRIP);
        // With no screen known, it's left where it is.
        assert_eq!(docked_rect(note, Edge::Top, 0.0, &[]), note);
    }

    #[test]
    fn the_pointer_at_a_screen_edge_docks_there() {
        let areas = [SCREEN, SECOND];
        assert_eq!(edge_at((0.0, 300.0), &areas), Some(Edge::Left));
        assert_eq!(edge_at((999.0, 300.0), &areas), Some(Edge::Right));
        assert_eq!(edge_at((500.0, 0.0), &areas), Some(Edge::Top));
        assert_eq!(edge_at((500.0, 300.0), &areas), None);
        assert_eq!(edge_at((500.0, 799.0), &areas), None);
        // The second screen's own edges.
        assert_eq!(edge_at((1799.0, 300.0), &areas), Some(Edge::Right));
        assert_eq!(edge_at((1400.0, 0.0), &areas), Some(Edge::Top));
    }
}
