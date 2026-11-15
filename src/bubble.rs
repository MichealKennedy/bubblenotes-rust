//! A note shown as a bubble: a small shape on the desktop that opens into
//! the note.
//!
//! The window is exactly the bubble while it's shut, and the bubble with
//! its note beside it (or the note alone, over it) while it's open
//! (`window_rect`). The bubble stays where it is on the screen whichever
//! side the note opens to; `window_moved` is the inverse, for when the user
//! drags the bubble or resizes the open note.
//!
//! Opening rolls the note out of the bubble: the window takes its open
//! size at once (`note_window` applies `window_rect` when `open` changes)
//! and the note's card is revealed inside it over `animate_ms`; shutting
//! rolls it back in first, then shrinks the window.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use nsh_core::element::ImageSrc;
use nsh_core::element::desktop::current_window;
use nsh_core::prelude::*;

use crate::app::{App, Note};
use crate::model::{Bubble, BubbleDirection, BubbleOpenOn, BubbleShape, NoteMeta, Rect, Rgb};
use crate::note_window::{TITLE_HEIGHT, color};
use crate::{drag, note_menu, win};

/// The space between the bubble and its open note.
const GAP: f32 = 8.0;
/// The smallest note a resize leaves (as `note_window`'s own limits).
const MIN_WIDTH: f32 = 140.0;
const MIN_HEIGHT: f32 = 90.0;
/// How long the pointer may be outside before the note shuts: enough to
/// cross the gap between the bubble and its note.
const LEAVE_GRACE: Duration = Duration::from_millis(250);

// ----- Geometry ----------------------------------------------------------------------

/// Where the bubble and the note sit inside the open window, and its size.
#[derive(Debug, PartialEq)]
struct Layout {
    bubble: (f32, f32),
    w: f32,
    h: f32,
}

/// The open window's layout for a bubble of `size` and a note of `w` x `h`.
fn layout(direction: BubbleDirection, size: f32, w: f32, h: f32) -> Layout {
    match direction {
        BubbleDirection::Bottom => Layout { bubble: (0.0, 0.0), w: w.max(size), h: size + GAP + h },
        BubbleDirection::Top => Layout { bubble: (0.0, h + GAP), w: w.max(size), h: size + GAP + h },
        BubbleDirection::Right => Layout { bubble: (0.0, 0.0), w: size + GAP + w, h: h.max(size) },
        BubbleDirection::Left => Layout { bubble: (w + GAP, 0.0), w: size + GAP + w, h: h.max(size) },
        BubbleDirection::OnTop => Layout { bubble: ((w - size) / 2.0, (h - size) / 2.0), w, h },
    }
}

/// The note's size as it shows: only its title when rolled up.
fn note_size(meta: &NoteMeta) -> (f32, f32) {
    (meta.rect.w, if meta.rolled_up { TITLE_HEIGHT } else { meta.rect.h })
}

/// The window of a note shown as a bubble: the bubble alone, or the bubble
/// with its note open beside it.
pub fn window_rect(meta: &NoteMeta, open: bool) -> Rect {
    let b = &meta.bubble;
    if !open {
        return Rect { x: b.x, y: b.y, w: b.size, h: b.size };
    }
    let (w, h) = note_size(meta);
    let l = layout(b.direction, b.size, w, h);
    Rect { x: b.x - l.bubble.0, y: b.y - l.bubble.1, w: l.w, h: l.h }
}

/// The user moved or resized the bubble's window to `now`.
pub fn window_moved(meta: &mut NoteMeta, open: bool, now: Rect) {
    if !open {
        meta.bubble.x = now.x;
        meta.bubble.y = now.y;
        return;
    }
    let expected = window_rect(meta, true);
    let (size, direction) = (meta.bubble.size, meta.bubble.direction);
    // What a resize gave the note: the window less the bubble's part.
    let (w, h) = match direction {
        BubbleDirection::Bottom | BubbleDirection::Top => (now.w, now.h - size - GAP),
        BubbleDirection::Right | BubbleDirection::Left => (now.w - size - GAP, now.h),
        BubbleDirection::OnTop => (now.w, now.h),
    };
    if (now.w - expected.w).abs() > 0.5 {
        meta.rect.w = w.max(MIN_WIDTH);
    }
    if (now.h - expected.h).abs() > 0.5 && !meta.rolled_up {
        meta.rect.h = h.max(MIN_HEIGHT);
    }
    // The window's corner stayed put (a move, or the grip at the bottom
    // right): the bubble is where the layout puts it from there.
    let (w, h) = note_size(meta);
    let l = layout(direction, size, w, h);
    meta.bubble.x = now.x + l.bubble.0;
    meta.bubble.y = now.y + l.bubble.1;
}

// ----- Drawing -----------------------------------------------------------------------

fn hex(rgb: Rgb) -> String {
    format!("#{:06x}", rgb & 0xFF_FFFF)
}

/// `a` moved toward `b` by `t` (0 to 1).
fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let channel = |shift: u32| {
        let (x, y) = (((a >> shift) & 0xFF) as f32, ((b >> shift) & 0xFF) as f32);
        ((x + (y - x) * t).round() as u32) << shift
    };
    channel(16) | channel(8) | channel(0)
}

/// The bubble's shape as an SVG of `size` points, or `None` for the Emoji
/// shape (only its label shows). Drawn in a 100 x 100 box.
fn shape_svg(b: &Bubble) -> Option<String> {
    let size = b.size.max(8.0);
    // The border's width in the 100-unit box.
    let border = b.border_width.max(0.0) * 100.0 / size;
    let stroke = if border > 0.0 {
        format!(r#" stroke="{}" stroke-width="{border:.2}""#, hex(b.border_color))
    } else {
        String::new()
    };
    let flat = |id: &str| match b.color_to {
        Some(to) => format!(
            r#"<linearGradient id="{id}" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{}"/><stop offset="1" stop-color="{}"/></linearGradient>"#,
            hex(b.color),
            hex(to)
        ),
        None => format!(r#"<linearGradient id="{id}"><stop offset="0" stop-color="{0}"/><stop offset="1" stop-color="{0}"/></linearGradient>"#, hex(b.color)),
    };
    let body = match b.shape {
        BubbleShape::Emoji => return None,
        BubbleShape::Sphere => {
            // A glossy ball: lit from the top left, a soft dark rim, light
            // bouncing back at the bottom, a bright shine on top and a
            // faint shadow under it.
            let light = mix(b.color, 0xFFFFFF, 0.45);
            let edge = b.color_to.unwrap_or_else(|| mix(b.color, 0x000000, 0.38));
            format!(
                r##"<defs>
<radialGradient id="base" cx="0.36" cy="0.3" r="0.78"><stop offset="0" stop-color="{light}"/><stop offset="0.5" stop-color="{mid}"/><stop offset="1" stop-color="{edge}"/></radialGradient>
<radialGradient id="rim" cx="0.5" cy="0.5" r="0.5"><stop offset="0.76" stop-color="#000" stop-opacity="0"/><stop offset="1" stop-color="#000" stop-opacity="0.26"/></radialGradient>
<radialGradient id="glow" cx="0.5" cy="0.5" r="0.5"><stop offset="0" stop-color="#fff" stop-opacity="0.38"/><stop offset="1" stop-color="#fff" stop-opacity="0"/></radialGradient>
<radialGradient id="drop" cx="0.5" cy="0.5" r="0.5"><stop offset="0" stop-color="#000" stop-opacity="0.28"/><stop offset="1" stop-color="#000" stop-opacity="0"/></radialGradient>
<linearGradient id="shine" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#fff" stop-opacity="0.92"/><stop offset="1" stop-color="#fff" stop-opacity="0.06"/></linearGradient>
</defs>
<ellipse cx="50" cy="94.5" rx="31" ry="4.5" fill="url(#drop)"/>
<circle cx="50" cy="47" r="45" fill="url(#base)"/>
<circle cx="50" cy="47" r="45" fill="url(#rim)"/>
<ellipse cx="50" cy="73" rx="28" ry="15" fill="url(#glow)"/>
<ellipse cx="45" cy="23" rx="26" ry="14.5" fill="url(#shine)"/>
<circle cx="50" cy="47" r="{r:.2}" fill="none"{stroke}/>"##,
                light = hex(light),
                mid = hex(b.color),
                edge = hex(edge),
                r = 45.0 - border / 2.0,
            )
        }
        BubbleShape::Circle => {
            format!(r#"<defs>{}</defs><circle cx="50" cy="50" r="{:.2}" fill="url(#fill)"{stroke}/>"#, flat("fill"), 50.0 - border / 2.0)
        }
        BubbleShape::Square => {
            let inset = border / 2.0;
            format!(
                r#"<defs>{}</defs><rect x="{inset:.2}" y="{inset:.2}" width="{side:.2}" height="{side:.2}" rx="22" fill="url(#fill)"{stroke}/>"#,
                flat("fill"),
                side = 100.0 - border,
            )
        }
        BubbleShape::Triangle => {
            // Rounded corners: the outline stroked wide in the fill, over a
            // wider stroke in the border's colour.
            let round = 12.0;
            let m = round / 2.0 + border + 1.0;
            let points = format!("M50 {:.2} L{:.2} {:.2} L{m:.2} {:.2} Z", m + 2.0, 100.0 - m, 100.0 - m, 100.0 - m);
            let outline = if border > 0.0 {
                format!(
                    r#"<path d="{points}" fill="{c}" stroke="{c}" stroke-width="{w:.2}" stroke-linejoin="round"/>"#,
                    c = hex(b.border_color),
                    w = round + border * 2.0
                )
            } else {
                String::new()
            };
            format!(
                r#"<defs>{}</defs>{outline}<path d="{points}" fill="url(#fill)" stroke="url(#fill)" stroke-width="{round}" stroke-linejoin="round"/>"#,
                flat("fill")
            )
        }
    };
    Some(format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 100 100">{body}</svg>"#))
}

/// The label's size: an emoji or a letter or two fill much of the bubble,
/// longer text shrinks to fit across it.
fn label_size(b: &Bubble) -> f32 {
    let chars = b.label.chars().count().max(1) as f32;
    match b.shape {
        BubbleShape::Emoji => b.size * 0.78,
        BubbleShape::Triangle => (b.size * 0.34).min(b.size * 0.8 / (chars * 0.62)),
        _ if chars <= 2.0 => b.size * 0.44,
        _ => (b.size * 0.4).min(b.size * 0.84 / (chars * 0.6)),
    }
}

/// The bubble itself: its shape, its label centred on it.
fn bubble_shape(meta: Signal<NoteMeta>, badge: impl Fn() -> Option<String> + Copy + 'static) -> View {
    let bubble = move || meta.with(|m| m.bubble.clone());
    let size = move || px(meta.with(|m| m.bubble.size));
    // The label sits on the shape's visual centre: a sphere's is above its
    // shadow, a triangle's low down.
    let nudge = move || {
        let b = bubble();
        match b.shape {
            BubbleShape::Sphere => (0.0, b.size * 0.06),
            BubbleShape::Triangle => (b.size * 0.22, 0.0),
            _ => (0.0, 0.0),
        }
    };
    view()
        .w(size)
        .h(size)
        .flex_shrink_0()
        .child(dynamic(move || {
            shape_svg(&bubble()).map(|markup| svg(ImageSrc::svg_markup(&markup)).absolute().inset_0().w(size).h(size))
        }))
        .child(
            view()
                .absolute()
                .inset_0()
                .items_center()
                .justify_center()
                .pt(move || px(nudge().0))
                .pb(move || px(nudge().1))
                .pointer_events(PointerEvents::None)
                .child(
                    text(move || meta.with(|m| m.bubble.label.clone()))
                        .text_size(move || px(label_size(&bubble())))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || color(meta.with(|m| m.bubble.label_color), 1.0)),
                ),
        )
        // The badge: a small dark pill in the top right corner.
        .child(dynamic(move || {
            let label = badge()?;
            Some(
                view()
                    .absolute()
                    .top(px(0.0))
                    .right(px(0.0))
                    .h(px(15.0))
                    .min_w(px(15.0))
                    .px(px(4.0))
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(nsh_core::Color(0x18181bf2))
                    .border_1()
                    .border_color(nsh_core::Color(0xffffffcc))
                    .pointer_events(PointerEvents::None)
                    .child(text(label).text_size(px(9.0)).font_weight(FontWeight::SEMIBOLD).text_color(nsh_core::Color(0xffffffff))),
            )
        }))
}

/// What a bubble's badge says: the time left to the note's alarm, else how
/// many of its checklist items (`tasks`: done, all) are done. `None` for no
/// badge.
pub fn badge_text(meta: &NoteMeta, now: f64, tasks: (usize, usize)) -> Option<String> {
    if !meta.bubble.badge {
        return None;
    }
    if let Some(alarm) = meta.alarm {
        let minutes = ((alarm.at - now) / 60_000.0).ceil().max(0.0);
        return Some(match minutes {
            m if m < 1.0 => "now".into(),
            m if m < 60.0 => format!("{m:.0}m"),
            m if m < 48.0 * 60.0 => format!("{:.0}h", (m / 60.0).round()),
            m => format!("{:.0}d", (m / 1440.0).round()),
        });
    }
    (tasks.1 > 0).then(|| format!("{}/{}", tasks.0, tasks.1))
}

/// The note's checklist items: how many are checked, how many there are.
fn tasks(note: &Note) -> (usize, usize) {
    let (mut done, mut all) = (0, 0);
    note.editor.doc().descendants(&mut |node, _, _, _| {
        if node.ty().name() == "taskItem" {
            all += 1;
            if node.attrs().get("checked").and_then(|c| c.as_bool()).unwrap_or(false) {
                done += 1;
            }
        }
        true
    });
    (done, all)
}

// ----- Opening and shutting ------------------------------------------------------------

/// What a bubble keeps across rebuilds of its view (`note_window` makes the
/// view again on every change of the note's data), under the app's owner.
#[derive(Clone, Copy)]
struct Live {
    /// How far the note is rolled out: 0 shut, 1 open.
    phase: Signal<f32>,
    /// Open, or on its way there.
    goal: Signal<bool>,
    /// The pointer is over the window.
    inside: Signal<bool>,
    /// Bumped by every roll, so an older one stops.
    run: Signal<u64>,
}

thread_local! {
    static LIVE: RefCell<HashMap<String, Live>> = RefCell::new(HashMap::new());
}

fn live(app: App, id: &str) -> Live {
    LIVE.with(|all| {
        *all.borrow_mut().entry(id.to_string()).or_insert_with(|| {
            app.owner.run(|| Live { phase: signal(0.0), goal: signal(false), inside: signal(false), run: signal(0) })
        })
    })
}

/// Opens (`want`) or shuts the note, rolling it when the bubble animates.
fn set_open(app: App, live: Live, open: Signal<bool>, meta: Signal<NoteMeta>, want: bool) {
    if live.goal.get_untracked() == want {
        return;
    }
    live.goal.set(want);
    let run = live.run.get_untracked() + 1;
    live.run.set(run);
    let (animate, ms) = meta.with_untracked(|m| (m.bubble.animate, m.bubble.animate_ms.max(1)));
    if !animate {
        live.phase.set(if want { 1.0 } else { 0.0 });
        open.set(want);
        return;
    }
    if want && !open.get_untracked() {
        live.phase.set(0.0);
        open.set(true);
    }
    let (from, to) = (live.phase.get_untracked(), if want { 1.0 } else { 0.0 });
    // A roll part done takes its part of the time.
    let span = Duration::from_millis(ms as u64).mul_f32((to - from).abs().max(0.05));
    app.owner.run(|| {
        let _ = spawn(async move {
            let start = Instant::now();
            loop {
                timer(Duration::from_millis(8)).await;
                if live.run.get_untracked() != run {
                    return;
                }
                let t = (start.elapsed().as_secs_f32() / span.as_secs_f32()).min(1.0);
                let eased = 1.0 - (1.0 - t).powi(3);
                live.phase.set(from + (to - from) * eased);
                if t >= 1.0 {
                    break;
                }
            }
            if !want {
                open.set(false);
            }
        });
    });
}

/// Shuts the note after the grace delay, unless the pointer came back.
fn close_after_leave(app: App, live: Live, open: Signal<bool>, meta: Signal<NoteMeta>) {
    app.owner.run(|| {
        let _ = spawn(async move {
            timer(LEAVE_GRACE).await;
            if !live.inside.get_untracked() && !win::primary_button_down() {
                set_open(app, live, open, meta, false);
            }
        });
    });
}

/// The bubble, and its note (`card`) while `open`.
pub fn bubble_view(app: App, note: Note, open: Signal<bool>, card: impl Fn() -> AnyElement + 'static) -> impl IntoElement {
    let meta = note.meta;
    let live = live(app, &note.id);
    // A new window starts shut (its `open` is new too).
    if !open.get_untracked() {
        live.goal.set(false);
        live.phase.set(0.0);
    } else if !live.goal.get_untracked() && live.run.get_untracked() == 0 {
        live.goal.set(true);
        live.phase.set(1.0);
    }
    let window = current_window();
    let direction = meta.with_untracked(|m| m.bubble.direction);
    // On top of the bubble the note hides it, so only leaving shuts it.
    let closes_on_leave = move || meta.with_untracked(|m| m.bubble.close_on_leave || m.bubble.direction == BubbleDirection::OnTop);

    let menu_note = note.clone();
    let drag_id = note.id.clone();
    let badge_note = stored(note.clone());
    let badge = move || {
        // Follows the clock (the countdown) and the text (the checklist).
        let now = app.now.get();
        app.revision.get();
        let note = badge_note.get();
        meta.with(|m| badge_text(m, now, tasks(&note)))
    };
    let bubble = bubble_shape(meta, badge)
        .cursor(Cursor::Default)
        .style_when(move || open.get() && direction == BubbleDirection::OnTop, |s| s.hidden())
        .on_press_down(move || {
            // A right press opens the menu instead (and must not toggle).
            if !win::primary_button_down() {
                return;
            }
            // The panel beside a note follows the bubble's own corner.
            let (offset_x, offset_y) = meta.with_untracked(|m| {
                let r = window_rect(m, open.get_untracked());
                (m.bubble.x - r.x, m.bubble.y - r.y)
            });
            let id = drag_id.clone();
            drag::set_neighbours(app.other_note_windows(&id));
            drag::drag_window(
                app.owner,
                window.clone(),
                move |x, y| app.dragging.set(Some((id.clone(), x + offset_x, y + offset_y))),
                move |moved| {
                    app.dragging.set(None);
                    // A press that didn't move is a click.
                    if !moved {
                        set_open(app, live, open, meta, !live.goal.get_untracked());
                    }
                },
            );
        })
        .on_context_menu(move |_: &ContextMenuEvent| note_menu::show(app, &menu_note))
        .on_wheel(move |e: &WheelEvent| {
            if !meta.with_untracked(|m| m.bubble.wheel) {
                return;
            }
            // Windows reports a notch away from the user (the wheel "up")
            // as a negative y: nsh-platform-windows hands on `WM_MOUSEWHEEL`'s
            // delta negated, as the DOM's `deltaY` is. So up opens.
            let (_, y) = e.delta.xy();
            if y < 0.0 {
                set_open(app, live, open, meta, true);
            } else if y > 0.0 {
                set_open(app, live, open, meta, false);
            }
        });

    // The note, rolled out of the bubble by `phase` along its side.
    let card = Rc::new(card);
    // The note's size, set only when it changes: the note changes on every
    // save, and nothing here may be made again for that (the text would
    // lose the keyboard focus).
    let size = signal(meta.with_untracked(note_size));
    effect(move || {
        let now = meta.with(note_size);
        if size.get_untracked() != now {
            size.set(now);
        }
    });
    let (w, h) = (move || size.get().0, move || size.get().1);
    let note_part = move || {
        let card = card.clone();
        dynamic(move || {
            open.get().then(|| {
                let rolled = move |full: f32| px(full * live.phase.get());
                let holder = view().flex_shrink_0().overflow_hidden();
                let holder = match direction {
                    BubbleDirection::Bottom => holder.w(move || px(w())).h(move || rolled(h())),
                    BubbleDirection::Top => holder.w(move || px(w())).h(move || rolled(h())).justify_end(),
                    BubbleDirection::Right => holder.h(move || px(h())).w(move || rolled(w())),
                    BubbleDirection::Left => holder.h(move || px(h())).w(move || rolled(w())).items_end(),
                    // Over the bubble it grows from the middle.
                    BubbleDirection::OnTop => holder
                        .items_center()
                        .justify_center()
                        .w(move || px(w() * (0.25 + 0.75 * live.phase.get())))
                        .h(move || px(h() * (0.25 + 0.75 * live.phase.get())))
                        .opacity(move || live.phase.get()),
                };
                holder.child(view().w(move || px(w())).h(move || px(h())).flex_shrink_0().child(untracked(|| card())))
            })
        })
    };
    let gap = move || view().flex_shrink_0().w(px(GAP)).h(px(GAP)).style_when(move || !open.get(), |s| s.hidden());

    let root = view()
        .flex_1()
        .overflow_hidden()
        .on_mouse_enter(move |_: &MouseEvent| {
            live.inside.set(true);
            if meta.with_untracked(|m| m.bubble.open_on == BubbleOpenOn::Hover) {
                set_open(app, live, open, meta, true);
            }
        })
        .on_mouse_leave(move |_: &MouseEvent| {
            live.inside.set(false);
            if live.goal.get_untracked() && closes_on_leave() {
                close_after_leave(app, live, open, meta);
            }
        });
    match direction {
        BubbleDirection::Bottom => root.items_start().child(bubble).child(gap()).child(note_part()),
        BubbleDirection::Top => root.items_start().justify_end().child(note_part()).child(gap()).child(bubble),
        BubbleDirection::Right => root.flex_row().items_start().child(bubble).child(gap()).child(note_part()),
        BubbleDirection::Left => root.flex_row().items_start().justify_end().child(note_part()).child(gap()).child(bubble),
        BubbleDirection::OnTop => root.items_center().justify_center().child(bubble).child(note_part()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_badge_counts_down_to_the_alarm_or_counts_the_checklist() {
        let mut m = NoteMeta::default();
        assert_eq!(badge_text(&m, 0.0, (0, 0)), None);
        assert_eq!(badge_text(&m, 0.0, (2, 5)).as_deref(), Some("2/5"));
        let alarm = |minutes: f64| Some(crate::model::Alarm { at: minutes * 60_000.0, repeat: Default::default() });
        m.alarm = alarm(0.5);
        assert_eq!(badge_text(&m, 0.0, (2, 5)).as_deref(), Some("1m"));
        m.alarm = alarm(45.0);
        assert_eq!(badge_text(&m, 0.0, (0, 0)).as_deref(), Some("45m"));
        m.alarm = alarm(150.0);
        assert_eq!(badge_text(&m, 0.0, (0, 0)).as_deref(), Some("3h"));
        m.alarm = alarm(4.0 * 1440.0);
        assert_eq!(badge_text(&m, 0.0, (0, 0)).as_deref(), Some("4d"));
        m.alarm = alarm(-3.0);
        assert_eq!(badge_text(&m, 0.0, (0, 0)).as_deref(), Some("now"));
        m.bubble.badge = false;
        assert_eq!(badge_text(&m, 0.0, (2, 5)), None);
    }

    const DIRECTIONS: [BubbleDirection; 5] =
        [BubbleDirection::Bottom, BubbleDirection::Right, BubbleDirection::Top, BubbleDirection::Left, BubbleDirection::OnTop];

    fn meta(direction: BubbleDirection, rolled_up: bool) -> NoteMeta {
        let mut m = NoteMeta { rolled_up, rect: Rect { x: 0.0, y: 0.0, w: 240.0, h: 200.0 }, ..NoteMeta::default() };
        m.bubble.enabled = true;
        m.bubble.direction = direction;
        m.bubble.x = 500.0;
        m.bubble.y = 400.0;
        m.bubble.size = 64.0;
        m
    }

    #[test]
    fn shut_it_is_the_bubble() {
        let m = meta(BubbleDirection::Top, false);
        assert_eq!(window_rect(&m, false), Rect { x: 500.0, y: 400.0, w: 64.0, h: 64.0 });
    }

    #[test]
    fn the_bubble_stays_put_whichever_side_the_note_opens_to() {
        for direction in DIRECTIONS {
            for rolled_up in [false, true] {
                let m = meta(direction, rolled_up);
                let r = window_rect(&m, true);
                let (w, h) = note_size(&m);
                let l = layout(direction, 64.0, w, h);
                assert_eq!((r.x + l.bubble.0, r.y + l.bubble.1), (500.0, 400.0), "{direction:?}");
                // And the window holds the bubble.
                assert!(l.bubble.0 >= 0.0 && l.bubble.0 + 64.0 <= r.w + 0.01, "{direction:?}");
            }
        }
    }

    #[test]
    fn the_note_opens_on_its_side() {
        let r = |d| window_rect(&meta(d, false), true);
        assert_eq!(r(BubbleDirection::Bottom), Rect { x: 500.0, y: 400.0, w: 240.0, h: 64.0 + GAP + 200.0 });
        assert_eq!(r(BubbleDirection::Top), Rect { x: 500.0, y: 400.0 - 200.0 - GAP, w: 240.0, h: 64.0 + GAP + 200.0 });
        assert_eq!(r(BubbleDirection::Right), Rect { x: 500.0, y: 400.0, w: 64.0 + GAP + 240.0, h: 200.0 });
        assert_eq!(r(BubbleDirection::Left), Rect { x: 500.0 - 240.0 - GAP, y: 400.0, w: 64.0 + GAP + 240.0, h: 200.0 });
        assert_eq!(r(BubbleDirection::OnTop), Rect { x: 500.0 + 32.0 - 120.0, y: 400.0 + 32.0 - 100.0, w: 240.0, h: 200.0 });
    }

    #[test]
    fn reporting_the_same_window_changes_nothing() {
        for direction in DIRECTIONS {
            for rolled_up in [false, true] {
                for open in [false, true] {
                    let before = meta(direction, rolled_up);
                    let mut after = before.clone();
                    window_moved(&mut after, open, window_rect(&before, open));
                    assert_eq!(after, before, "{direction:?} rolled up {rolled_up} open {open}");
                }
            }
        }
    }

    #[test]
    fn a_move_moves_the_bubble_by_as_much() {
        for direction in DIRECTIONS {
            for open in [false, true] {
                let before = meta(direction, false);
                let mut after = before.clone();
                let r = window_rect(&before, open);
                window_moved(&mut after, open, Rect { x: r.x + 30.0, y: r.y - 12.0, ..r });
                assert_eq!((after.bubble.x, after.bubble.y), (530.0, 388.0), "{direction:?}");
                assert_eq!(after.rect, before.rect);
            }
        }
    }

    #[test]
    fn the_grip_resizes_the_note_only() {
        for direction in DIRECTIONS {
            let before = meta(direction, false);
            let mut after = before.clone();
            let r = window_rect(&before, true);
            window_moved(&mut after, true, Rect { w: r.w + 20.0, h: r.h + 10.0, ..r });
            assert_eq!((after.rect.w, after.rect.h), (260.0, 210.0), "{direction:?}");
            // The window it makes now is the one reported.
            assert_eq!(window_rect(&after, true), Rect { w: r.w + 20.0, h: r.h + 10.0, ..r }, "{direction:?}");
        }
    }

    #[test]
    fn a_rolled_up_note_keeps_its_height() {
        let before = meta(BubbleDirection::Bottom, true);
        let mut after = before.clone();
        let r = window_rect(&before, true);
        window_moved(&mut after, true, Rect { h: r.h + 50.0, ..r });
        assert_eq!(after.rect.h, 200.0);
    }

    #[test]
    fn every_shape_but_the_emoji_is_drawn() {
        let mut b = Bubble::default();
        for shape in [BubbleShape::Sphere, BubbleShape::Circle, BubbleShape::Square, BubbleShape::Triangle] {
            b.shape = shape;
            b.border_width = 2.0;
            b.color_to = Some(0x112233);
            let svg = shape_svg(&b).expect("a shape");
            assert!(svg.starts_with("<svg") && svg.contains("#e0457b"), "{shape:?}");
        }
        b.shape = BubbleShape::Emoji;
        assert!(shape_svg(&b).is_none());
    }

    #[test]
    fn colours_mix() {
        assert_eq!(mix(0x000000, 0xFFFFFF, 0.5), 0x808080);
        assert_eq!(mix(0xE0457B, 0xE0457B, 0.3), 0xE0457B);
        assert_eq!(hex(0x0A0B0C), "#0a0b0c");
    }
}
