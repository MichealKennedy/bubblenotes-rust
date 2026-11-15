//! "Edit configuration": a small panel beside a note with that note's own
//! settings.
//!
//! It's a frameless window that sits to the right of the note (to the left
//! when that display has no room) and follows it, live while it's dragged.
//! Every control changes the note at once (`App::update`); there's no OK.
//! The everyday things come first, the rest is in sections that start shut.
//!
//! Pickers (colours, emoji) open inline, inside the panel: a popup is drawn
//! in its window and this window is small. Only the selects pop up, and
//! their lists are short enough to fit.
//!
//! The panel's controls (`switch_row`, `slider_row`, `color_row`,
//! `theme_swatches`...) are also what the Options window is made of.

use std::rc::Rc;

use chrono::{Duration as Days, Local, TimeZone};
use nsh_base::{
    ColorChannel, ColorPickerEvent, ColorSpace, EmojiPickerEvent, SelectItem, SliderValue, collapsible, color_area, color_field,
    color_picker, color_slider, emoji_picker, number_input, radio, radio_group, select, slider, switch, theme, toggle, toggle_group,
};
use nsh_core::Color;
use nsh_core::desktop::{WindowAttributes, WindowFrame, WindowFrameStyle, WindowLevel, WindowPosition};
use nsh_core::element::desktop::{OpenChange, current_window, displays, window};
use nsh_core::element::{ImageSrc, Input, input};
use nsh_core::prelude::*;

use crate::alarms;
use crate::app::{App, Note};
use crate::model::{Alarm, BubbleDirection, BubbleOpenOn, BubbleShape, Look, NoteMeta, Rect, Repeat, Rgb, Theme, builtin_themes};
use crate::note_window::{color, shape_rect};

const PANEL_WIDTH: f32 = 300.0;
const PANEL_HEIGHT: f32 = 560.0;
/// Between the note and the panel, and the panel and the screen's edges.
const GAP: f32 = 10.0;
/// The width of what's inside the panel's sections.
const INNER_WIDTH: f32 = PANEL_WIDTH - 2.0 * 14.0 - 6.0;

const FONTS: [&str; 6] = ["Segoe UI", "Calibri", "Georgia", "Consolas", "Comic Sans MS", "Segoe Print"];

// ----- Placement ---------------------------------------------------------------------

/// Where the panel goes for a note whose window is `anchor`: to its right,
/// else to its left, inside the work area of the display the note is on
/// (`areas`, the work areas of every display).
fn place(anchor: Rect, areas: &[Rect], w: f32, h: f32) -> (f32, f32) {
    let (cx, cy) = (anchor.x + anchor.w / 2.0, anchor.y + anchor.h / 2.0);
    let inside = |a: &&Rect| cx >= a.x && cx < a.x + a.w && cy >= a.y && cy < a.y + a.h;
    let distance = |a: &Rect| {
        let dx = (a.x - cx).max(cx - (a.x + a.w)).max(0.0);
        let dy = (a.y - cy).max(cy - (a.y + a.h)).max(0.0);
        dx * dx + dy * dy
    };
    let area = areas
        .iter()
        .find(inside)
        .or_else(|| areas.iter().min_by(|a, b| distance(a).total_cmp(&distance(b))))
        .copied()
        .unwrap_or(Rect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 });
    let right = anchor.x + anchor.w + GAP;
    let x = if right + w <= area.x + area.w { right } else { anchor.x - GAP - w };
    let x = x.min(area.x + area.w - w).max(area.x);
    let y = anchor.y.min(area.y + area.h - h).max(area.y);
    (x, y)
}

fn work_areas() -> Vec<Rect> {
    displays()
        .iter()
        .map(|d| {
            let b = d.visible_bounds;
            Rect { x: b.origin.x.0, y: b.origin.y.0, w: b.size.width.0, h: b.size.height.0 }
        })
        .collect()
}

fn attributes() -> WindowAttributes {
    let mut a = WindowAttributes::default();
    a.frame = Some(WindowFrameStyle::None);
    a.transparent = Some(true);
    a.taskbar = Some(false);
    a.shadow = Some(false);
    a.resizable = Some(false);
    a.level = Some(WindowLevel::Floating);
    a
}

/// The panel's window, open while a note is being configured
/// (`App::config_for`).
pub fn config_window(app: App) -> impl IntoElement {
    window()
        .id("config")
        .title("Edit configuration")
        .default_size(size(px(PANEL_WIDTH), px(PANEL_HEIGHT)))
        .default_position(WindowPosition::At(point(px(200.0), px(120.0))))
        .attributes(attributes())
        .open(move || app.config_for.get().is_some())
        .on_open_change(move |c: &OpenChange| {
            if !c.open {
                app.config_for.set(None);
            }
        })
        .content(move || {
            let window = current_window();
            // Beside the note, wherever it goes.
            let last = stored(None::<WindowFrame>);
            effect(move || {
                // Bounds asked of a window that isn't up yet are lost: this
                // runs again once it is.
                if window.state().is_none() {
                    last.replace(None);
                    return;
                }
                let Some(note) = app.config_for.get().and_then(|id| app.note(&id)) else { return };
                let mut anchor = note.meta.with(|m| shape_rect(m, false));
                if let Some((id, x, y)) = app.dragging.get()
                    && id == note.id
                {
                    anchor.x = x;
                    anchor.y = y;
                }
                let areas = work_areas();
                let tallest = areas.iter().map(|a| a.h).fold(0.0, f32::max);
                let h = if tallest > 0.0 { PANEL_HEIGHT.min(tallest - 2.0 * GAP) } else { PANEL_HEIGHT };
                let (x, y) = place(anchor, &areas, PANEL_WIDTH, h);
                let frame = WindowFrame { x: Some(px(x)), y: Some(px(y)), width: Some(px(PANEL_WIDTH)), height: Some(px(h)), animated: None };
                if last.get() != Some(frame.clone()) {
                    last.replace(Some(frame.clone()));
                    window.set_bounds(frame);
                }
            });
            dynamic(move || app.config_for.get().and_then(|id| app.note(&id)).map(|note| panel(app, note)))
        })
}

// ----- The panel ---------------------------------------------------------------------

fn panel(app: App, note: Note) -> impl IntoElement {
    let meta = note.meta;
    view()
        .flex_1()
        .rounded(px(14.0))
        .overflow_hidden()
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().background)
        .child(header(app, meta))
        .child(
            scroll().flex_1().child(
                view()
                    .px(px(14.0))
                    .pb(px(14.0))
                    .gap(px(4.0))
                    .child(theme_part(app, note.clone()))
                    .child(behaviour_part(app, note.clone()))
                    .child(section("Background", signal(false), None, background_part(app, note.clone())))
                    .child(section("Title", signal(false), None, title_part(app, note.clone())))
                    .child(section("Text", signal(false), None, text_part(app, note.clone())))
                    .child(section("Sleep", signal(false), None, sleep_part(app, note.clone())))
                    .child(section("Alarm", signal(false), None, alarm_part(app, note.clone())))
                    .child(section(
                        "Bubble",
                        signal(false),
                        Some(bubble_switch(app, note.clone()).into_element()),
                        bubble_part(app, note.clone()),
                    )),
            ),
        )
}

/// The panel's title: the note's name and a close button.
fn header(app: App, meta: Signal<NoteMeta>) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .h(px(40.0))
        .pl(px(14.0))
        .pr(px(6.0))
        .gap(px(8.0))
        .border_b_1()
        .border_color(move || theme().border)
        .child(
            view()
                .w(px(12.0))
                .h(px(12.0))
                .rounded_full()
                .border_1()
                .border_color(move || theme().border)
                .bg(move || color(meta.with(|m| m.look.background), 1.0)),
        )
        .child(
            view().flex_1().child(
                text(move || meta.with(|m| m.title.clone()))
                    .truncate()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || theme().foreground),
            ),
        )
        .child(icon_button("✕", "Close", move || app.config_for.set(None)))
}

// ----- Sections ----------------------------------------------------------------------

/// A section that opens and shuts under its title; `extra` sits at the
/// right of the title (a switch for the whole section).
fn section(title: &'static str, open: Signal<bool>, extra: Option<AnyElement>, content: impl IntoElement + 'static) -> impl IntoElement {
    let chevron = svg(ImageSrc::svg_markup(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 12 12"><path d="M4.5 2.5 8 6l-3.5 3.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>"#,
    ))
    .w(px(12.0))
    .h(px(12.0))
    .text_color(move || theme().muted_foreground)
    .transition_all()
    .duration(150.0)
    .style_when(move || open.get(), |s| s.rotate(deg(90.0)));
    let mut title_row = view()
        .flex_row()
        .items_center()
        .h(px(34.0))
        .gap(px(6.0))
        .child(
            view()
                .flex_row()
                .flex_1()
                .items_center()
                .gap(px(6.0))
                .h_full()
                .cursor(Cursor::Pointer)
                .on_press(move || open.update(|o| *o = !*o))
                .child(chevron)
                .child(text(title).text_size(px(13.0)).font_weight(FontWeight::SEMIBOLD).text_color(move || theme().foreground)),
        );
    if let Some(extra) = extra {
        title_row = title_row.child(extra);
    }
    view()
        .border_t_1()
        .border_color(move || theme().border)
        .child(collapsible().open(open).reveal(true).content(view().pb(px(10.0)).gap(px(6.0)).child(content)).child(title_row))
}

/// A part shown without a section: a small heading over its rows.
fn heading(title: &'static str) -> impl IntoElement {
    text(title)
        .text_size(px(11.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || theme().muted_foreground)
        .pt(px(10.0))
        .pb(px(2.0))
}

fn theme_part(app: App, note: Note) -> impl IntoElement {
    let meta = note.meta;
    let more = signal(false);
    let naming = signal(false);
    let name = signal(String::new());
    let (apply_note, save_note, default_note) = (note.clone(), note.clone(), note.clone());
    let save = move || {
        let label = name.get_untracked().trim().to_string();
        if label.is_empty() {
            return;
        }
        let look = save_note.meta.with_untracked(|m| m.look.clone());
        app.update_settings(|s| s.themes.push(Theme { name: label, look }));
        name.set(String::new());
        naming.set(false);
    };
    view()
        .gap(px(6.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .child(view().flex_1().child(heading("THEME")))
                .child(link_button(move || if more.get() { "Less".into() } else { "More".into() }, move || more.update(|m| *m = !*m))),
        )
        .child(theme_swatches(app, move || meta.with(|m| m.look.clone()), move |look| {
            app.update(&apply_note, |m| {
                let on_top = m.look.always_on_top;
                m.look = look;
                m.look.always_on_top = on_top;
            })
        }))
        .child(
            view()
                .gap(px(6.0))
                .style_when(move || !more.get(), |s| s.hidden())
                .child(
                    view()
                        .flex_row()
                        .gap(px(6.0))
                        .child(small_button(|| "Save as theme…".into(), move || naming.update(|n| *n = !*n)))
                        .child(small_button(|| "Use for new notes".into(), move || {
                            let mut look = default_note.meta.with_untracked(|m| m.look.clone());
                            look.always_on_top = false;
                            app.update_settings(|s| s.default_look = look);
                        })),
                )
                .child(
                    view()
                        .flex_row()
                        .gap(px(6.0))
                        .style_when(move || !naming.get(), |s| s.hidden())
                        .child(
                            text_input(move || name.get(), move |t| name.set(t), "Theme name")
                                .flex_1()
                                .on_submit(move |_: &str| save()),
                        )
                        .child(primary_button(|| "Save".into(), save)),
                ),
        )
}

/// A swatch for each theme, built-in then the user's: pressing one hands
/// its look to `apply`. The one `current` matches is ringed.
pub(crate) fn theme_swatches(app: App, current: impl Fn() -> Look + 'static, apply: impl Fn(Look) + 'static) -> impl IntoElement {
    let current = Rc::new(current);
    let apply = Rc::new(apply);
    dynamic(move || {
        let themes: Vec<Theme> = builtin_themes().into_iter().chain(app.settings.with(|s| s.themes.clone())).collect();
        let (current, apply) = (current.clone(), apply.clone());
        view().flex_row().flex_wrap().gap(px(8.0)).children(themes.into_iter().map(move |t| {
            let (current, apply) = (current.clone(), apply.clone());
            let look = t.look.clone();
            let chosen = {
                let look = look.clone();
                move || {
                    let c = current();
                    c.background == look.background && c.background_to == look.background_to && c.body_color == look.body_color
                }
            };
            let fill = color(look.background, 1.0);
            let swatch = view()
                .w(px(26.0))
                .h(px(26.0))
                .rounded_full()
                .border_1()
                .border_color(Color(0x0000_0024))
                .bg(fill)
                .items_center()
                .justify_center()
                .label(t.name.clone())
                .cursor(Cursor::Pointer)
                .active(|s| s.opacity(0.7))
                .on_press(move || apply(look.clone()))
                .child(text("A").text_size(px(11.0)).font_weight(FontWeight::SEMIBOLD).text_color(color(t.look.body_color, 0.85)));
            let swatch = match t.look.background_to {
                Some(to) => swatch.bg_linear_to_b().gradient_from(fill).gradient_to(color(to, 1.0)),
                None => swatch,
            };
            // The ring around the chosen one.
            view()
                .p(px(2.0))
                .rounded_full()
                .border_2()
                .border_color(move || if chosen() { theme().primary } else { Color(0).into() })
                .child(swatch)
                .into_element()
        }))
    })
}

fn behaviour_part(app: App, note: Note) -> impl IntoElement {
    let meta = note.meta;
    let (n1, n2, n3) = (note.clone(), note.clone(), note.clone());
    view()
        .gap(px(2.0))
        .pb(px(8.0))
        .child(heading("BEHAVIOUR"))
        .child(switch_row("Always on top", move || meta.with(|m| m.look.always_on_top), move |on| app.update(&n1, |m| m.look.always_on_top = on)))
        .child(switch_row("Lock note", move || meta.with(|m| m.locked), move |on| app.update(&n2, |m| m.locked = on)))
        .child(switch_row("Rolled up", move || meta.with(|m| m.rolled_up), move |on| app.update(&n3, |m| m.rolled_up = on)))
}

fn background_part(app: App, note: Note) -> impl IntoElement {
    let meta = note.meta;
    let (n1, n2, n3, n4, n5) = (note.clone(), note.clone(), note.clone(), note.clone(), note.clone());
    view()
        .gap(px(4.0))
        .child(color_row("Colour", move || meta.with(|m| m.look.background), move |c| app.update(&n1, |m| m.look.background = c)))
        .child(switch_row("Gradient", move || meta.with(|m| m.look.background_to.is_some()), move |on| {
            app.update(&n2, |m| m.look.background_to = on.then(|| shade(m.look.background)))
        }))
        .child(
            view()
                .style_when(move || meta.with(|m| m.look.background_to.is_none()), |s| s.hidden())
                .child(color_row("Second colour", move || meta.with(|m| m.look.background_to.unwrap_or(m.look.background)), move |c| {
                    app.update(&n3, |m| m.look.background_to = Some(c))
                })),
        )
        .child(slider_row(
            "Transparency",
            (0.0, 90.0, 5.0),
            move || ((1.0 - meta.with(|m| m.look.opacity)) * 100.0).round() as f64,
            |v| format!("{v:.0}%"),
            move |v| app.update(&n4, |m| m.look.opacity = 1.0 - v as f32 / 100.0),
        ))
        .child(slider_row(
            "Corners",
            (0.0, 30.0, 1.0),
            move || meta.with(|m| m.look.corner_radius) as f64,
            |v| format!("{v:.0}"),
            move |v| app.update(&n5, |m| m.look.corner_radius = v as f32),
        ))
}

/// A gradient's second colour to start from: `rgb` a little darker.
fn shade(rgb: Rgb) -> Rgb {
    let channel = |shift: u32| ((((rgb >> shift) & 0xFF) as f32 * 0.8) as u32) << shift;
    channel(16) | channel(8) | channel(0)
}

fn title_part(app: App, note: Note) -> impl IntoElement {
    let meta = note.meta;
    let (n1, n2, n3, n4, n5) = (note.clone(), note.clone(), note.clone(), note.clone(), note.clone());
    view()
        .gap(px(4.0))
        .child(switch_row("Show title", move || meta.with(|m| m.look.show_title), move |on| app.update(&n1, |m| m.look.show_title = on)))
        .child(committed_input(
            move || meta.with(|m| m.title.clone()),
            move |t| {
                let t = t.trim().to_string();
                if !t.is_empty() {
                    app.update(&n2, |m| m.title = t);
                }
            },
            "Title",
        ))
        .child(color_row("Colour", move || meta.with(|m| m.look.title_color), move |c| app.update(&n3, |m| m.look.title_color = c)))
        .child(slider_row(
            "Size",
            (10.0, 22.0, 1.0),
            move || meta.with(|m| m.look.title_size) as f64,
            |v| format!("{v:.0}"),
            move |v| app.update(&n4, |m| m.look.title_size = v as f32),
        ))
        .child(switch_row("Bold", move || meta.with(|m| m.look.title_bold), move |on| app.update(&n5, |m| m.look.title_bold = on)))
}

fn text_part(app: App, note: Note) -> impl IntoElement {
    let meta = note.meta;
    let (n1, n2, n3) = (note.clone(), note.clone(), note.clone());
    let mut fonts = vec![("", "Default")];
    fonts.extend(FONTS.iter().map(|f| (*f, *f)));
    view()
        .gap(px(4.0))
        .child(color_row("Colour", move || meta.with(|m| m.look.body_color), move |c| app.update(&n1, |m| m.look.body_color = c)))
        .child(slider_row(
            "Size",
            (10.0, 28.0, 1.0),
            move || meta.with(|m| m.look.body_size) as f64,
            |v| format!("{v:.0}"),
            move |v| app.update(&n2, |m| m.look.body_size = v as f32),
        ))
        .child(row(
            "Font",
            choice(fonts, move || meta.with(|m| m.look.body_font.clone()), move |f| app.update(&n3, |m| m.look.body_font = f)).w(px(150.0)),
        ))
}

// ----- Sleep and alarm -----------------------------------------------------------------

const UNITS: [(&str, &str, f64); 4] =
    [("seconds", "seconds", 1_000.0), ("minutes", "minutes", 60_000.0), ("hours", "hours", 3_600_000.0), ("days", "days", 86_400_000.0)];

fn unit_ms(unit: &str) -> f64 {
    UNITS.iter().find(|(id, ..)| *id == unit).map_or(60_000.0, |u| u.2)
}

fn unit_items(units: &[&str]) -> Vec<(&'static str, &'static str)> {
    UNITS.iter().filter(|u| units.contains(&u.0)).map(|u| (u.0, u.1)).collect()
}

/// Tomorrow at 9:00, local time, in milliseconds since the Unix epoch.
fn tomorrow_at_nine(now: f64) -> Option<f64> {
    let today = Local.timestamp_millis_opt(now as i64).single()?.date_naive();
    let at = (today + Days::days(1)).and_hms_opt(9, 0, 0)?;
    Some(Local.from_local_datetime(&at).earliest()?.timestamp_millis() as f64)
}

fn sleep_part(app: App, note: Note) -> impl IntoElement {
    let meta = note.meta;
    let count = signal(Some(30.0));
    let unit = signal(String::from("minutes"));
    let asleep = move || {
        let now = app.now.get();
        meta.with(|m| m.asleep(now))
    };
    let sleep = {
        let note = note.clone();
        move |ms: f64| app.sleep_for(&note, ms)
    };
    let (s1, s2, s3, s4) = (sleep.clone(), sleep.clone(), sleep.clone(), sleep);
    let wake_note = note.clone();
    view()
        .gap(px(8.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    view().flex_1().child(
                        text(move || {
                            if asleep() {
                                let until = meta.with(|m| m.sleep_until.unwrap_or_default());
                                format!("Asleep until {}", alarms::format_at(until))
                            } else {
                                "Hide the note for a while; it comes back by itself.".into()
                            }
                        })
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                    ),
                )
                .child(view().style_when(move || !asleep(), |s| s.hidden()).child(primary_button(|| "Wake now".into(), move || app.wake(&wake_note)))),
        )
        .child(
            view()
                .flex_row()
                .gap(px(6.0))
                .child(small_button(|| "15 min".into(), move || s1(15.0 * 60_000.0)))
                .child(small_button(|| "1 hour".into(), move || s2(3_600_000.0)))
                .child(small_button(|| "Tomorrow 9:00".into(), move || {
                    let now = crate::store::now_ms();
                    if let Some(at) = tomorrow_at_nine(now) {
                        s3(at - now);
                    }
                })),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(text("For").text_size(px(13.0)).text_color(move || theme().foreground))
                .child(count_input(count))
                .child(choice(unit_items(&["seconds", "minutes", "hours"]), move || unit.get(), move |u| unit.set(u)).w(px(92.0)))
                .child(primary_button(|| "Sleep".into(), move || {
                    if let Some(n) = count.get_untracked().filter(|n| *n > 0.0) {
                        s4(n * unit_ms(&unit.get_untracked()));
                    }
                })),
        )
}

const REPEATS: [(Repeat, &str, &str); 7] = [
    (Repeat::None, "none", "None"),
    (Repeat::Hourly, "hourly", "Every hour"),
    (Repeat::Daily, "daily", "Every day"),
    (Repeat::Weekdays, "weekdays", "Weekdays"),
    (Repeat::Weekly, "weekly", "Every week"),
    (Repeat::Monthly, "monthly", "Every month"),
    (Repeat::Yearly, "yearly", "Every year"),
];

fn alarm_part(app: App, note: Note) -> impl IntoElement {
    let meta = note.meta;
    // "In n units" (false) or "at" a typed moment (true).
    let at_mode = signal(false);
    let count = signal(Some(1.0));
    let unit = signal(String::from("hours"));
    let start = meta.with_untracked(|m| m.alarm.map_or(crate::store::now_ms() + 3_600_000.0, |a| a.at));
    let at_text = signal(alarms::format_at(start));
    let repeat = signal(meta.with_untracked(|m| m.alarm.map_or(Repeat::None, |a| a.repeat)));
    let typed_ok = move || at_text.with(|t| t.trim().is_empty() || alarms::parse_at(t).is_some());
    let status = move || {
        let now = app.now.get();
        meta.with(|m| {
            let alarm = m.alarm?;
            // Only the alarm: what `describe` says about sleep is shown above.
            let only_alarm = NoteMeta { sleep_until: None, ..m.clone() };
            Some(alarms::describe(&only_alarm, now).unwrap_or_else(|| format!("Alarm at {}", alarms::format_at(alarm.at))))
        })
    };
    let (set_note, clear_note) = (note.clone(), note.clone());
    let set = move || {
        let at = if at_mode.get_untracked() {
            at_text.with_untracked(|t| alarms::parse_at(t))
        } else {
            count.get_untracked().filter(|n| *n > 0.0).map(|n| crate::store::now_ms() + n * unit_ms(&unit.get_untracked()))
        };
        if let Some(at) = at {
            app.set_alarm(&set_note, Some(Alarm { at, repeat: repeat.get_untracked() }));
        }
    };
    let option = move |at: bool, content: View| {
        view().flex_row().items_center().gap(px(6.0)).min_h(px(30.0)).child(radio_mark(move || at_mode.get() == at, move || at_mode.set(at))).child(content)
    };
    view()
        .gap(px(6.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    view().flex_1().child(
                        text(move || status().unwrap_or_else(|| "No alarm set.".into())).text_size(px(12.0)).text_color(move || theme().muted_foreground),
                    ),
                )
                .child(
                    view()
                        .style_when(move || status().is_none(), |s| s.hidden())
                        .child(small_button(|| "Clear alarm".into(), move || app.set_alarm(&clear_note, None))),
                ),
        )
        .child(
            radio_group()
                .gap(px(2.0))
                .child(option(
                    false,
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .child(text("In").w(px(18.0)).text_size(px(13.0)).text_color(move || theme().foreground))
                        .child(count_input(count))
                        .child(choice(unit_items(&["minutes", "hours", "days"]), move || unit.get(), move |u| unit.set(u)).w(px(92.0))),
                ))
                .child(option(
                    true,
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .child(text("At").w(px(18.0)).text_size(px(13.0)).text_color(move || theme().foreground))
                        .child(
                            text_input(move || at_text.get(), move |t| at_text.set(t), "2026-11-14 21:48")
                                .w(px(150.0))
                                .on_focus(move || at_mode.set(true)),
                        ),
                )),
        )
        .child(
            text(move || if typed_ok() { "Date format: yyyy-mm-dd hh:mm".to_string() } else { "That's not a date: use yyyy-mm-dd hh:mm".to_string() })
                .text_size(px(11.0))
                .pl(px(24.0))
                .text_color(move || if typed_ok() { theme().muted_foreground } else { theme().destructive }),
        )
        .child(row(
            "Repeat",
            choice(
                REPEATS.iter().map(|r| (r.1, r.2)).collect(),
                move || REPEATS.iter().find(|r| r.0 == repeat.get()).map_or("none", |r| r.1).to_string(),
                move |v| repeat.set(REPEATS.iter().find(|r| r.1 == v).map_or(Repeat::None, |r| r.0)),
            )
            .w(px(150.0)),
        ))
        .child(view().flex_row().justify_end().child(primary_button(|| "Set alarm".into(), set)))
}

// ----- Bubble ------------------------------------------------------------------------

fn bubble_switch(app: App, note: Note) -> impl IntoElement {
    let meta = note.meta;
    view()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .child(text("Show as bubble").text_size(px(12.0)).text_color(move || theme().muted_foreground))
        .child(styled_switch(move || meta.with(|m| m.bubble.enabled), move |on| app.update(&note, |m| m.bubble.enabled = on), "Show as bubble"))
}

fn bubble_part(app: App, note: Note) -> impl IntoElement {
    let meta = note.meta;
    let edit = {
        let note = note.clone();
        Rc::new(move |change: &dyn Fn(&mut NoteMeta)| app.update(&note, |m| change(m)))
    };
    let e = move || edit.clone();
    let picking_emoji = signal(false);
    let (e1, e2, e3, e4, e5, e6, e7, e8, e9, e10, e11, e12, e13, e14, e15, e16) =
        (e(), e(), e(), e(), e(), e(), e(), e(), e(), e(), e(), e(), e(), e(), e(), e());
    let shapes = vec![
        (BubbleShape::Sphere, "◉", "Sphere"),
        (BubbleShape::Circle, "●", "Circle"),
        (BubbleShape::Square, "■", "Square"),
        (BubbleShape::Triangle, "▲", "Triangle"),
        (BubbleShape::Emoji, "☺", "Emoji"),
    ];
    let directions = vec![
        (BubbleDirection::Bottom, "↓", "Bottom"),
        (BubbleDirection::Right, "→", "Right"),
        (BubbleDirection::Top, "↑", "Top"),
        (BubbleDirection::Left, "←", "Left"),
        (BubbleDirection::OnTop, "◎", "On top"),
    ];
    let shape_name = move || shapes_name(meta.with(|m| m.bubble.shape));
    view()
        .gap(px(4.0))
        .child(slider_row(
            "Size",
            (32.0, 128.0, 4.0),
            move || meta.with(|m| m.bubble.size) as f64,
            |v| format!("{v:.0}"),
            move |v| e1(&|m| m.bubble.size = v as f32),
        ))
        .child(row_dyn(
            move || format!("Shape: {}", shape_name()),
            segmented(INNER_WIDTH, shapes, move || meta.with(|m| m.bubble.shape), move |s| e2(&|m| m.bubble.shape = s)),
        ))
        .child(color_row("Colour", move || meta.with(|m| m.bubble.color), move |c| e3(&|m| m.bubble.color = c)))
        .child(switch_row("Gradient", move || meta.with(|m| m.bubble.color_to.is_some()), move |on| {
            e4(&|m| m.bubble.color_to = on.then(|| shade(m.bubble.color)))
        }))
        .child(
            view()
                .style_when(move || meta.with(|m| m.bubble.color_to.is_none()), |s| s.hidden())
                .child(color_row("Second colour", move || meta.with(|m| m.bubble.color_to.unwrap_or(m.bubble.color)), move |c| {
                    e5(&|m| m.bubble.color_to = Some(c))
                })),
        )
        .child(row(
            "Label",
            view()
                .flex_row()
                .gap(px(4.0))
                .child(committed_input(move || meta.with(|m| m.bubble.label.clone()), move |t| e6(&|m| m.bubble.label = t.clone()), "Text or emoji").w(px(110.0)))
                .child(icon_button("☺", "Pick an emoji", move || picking_emoji.update(|p| *p = !*p))),
        ))
        .child(collapsible().open(picking_emoji).reveal(true).content(
            emoji_picker()
                .placeholder("Search emoji")
                .accessibility_label("Emoji")
                .columns(7)
                .search_bar_class("pb-2")
                .search_class("flex-1 h-8 px-2 rounded-md border border-input text-sm text-foreground")
                .group_class("px-1 pt-2 pb-1 text-xs text-muted-foreground")
                .item_class("flex-1 h-8 items-center justify-center rounded-md text-xl hover:bg-accent focus:bg-accent")
                .on_select(move |ev: &EmojiPickerEvent| {
                    if let EmojiPickerEvent::Select(emoji, ..) = ev {
                        let emoji = emoji.to_string();
                        e7(&|m| m.bubble.label = emoji.clone());
                        picking_emoji.set(false);
                    }
                })
                .w(px(INNER_WIDTH))
                .h(px(260.0))
                .p(px(8.0))
                .rounded(px(10.0))
                .border_1()
                .border_color(move || theme().border)
                .bg(move || theme().card),
        ))
        .child(color_row("Label colour", move || meta.with(|m| m.bubble.label_color), move |c| e8(&|m| m.bubble.label_color = c)))
        .child(color_row("Border colour", move || meta.with(|m| m.bubble.border_color), move |c| e9(&|m| m.bubble.border_color = c)))
        .child(slider_row(
            "Border",
            (0.0, 6.0, 0.5),
            move || meta.with(|m| m.bubble.border_width) as f64,
            |v| format!("{v:.1}"),
            move |v| e10(&|m| m.bubble.border_width = v as f32),
        ))
        .child(row(
            "Opens on",
            segmented(
                130.0,
                vec![(BubbleOpenOn::Hover, "Hover", "Hover"), (BubbleOpenOn::Click, "Click", "Click")],
                move || meta.with(|m| m.bubble.open_on),
                move |o| e11(&|m| m.bubble.open_on = o),
            ),
        ))
        .child(row_dyn(
            move || format!("Opens to: {}", directions_name(meta.with(|m| m.bubble.direction))),
            segmented(INNER_WIDTH, directions, move || meta.with(|m| m.bubble.direction), move |d| e12(&|m| m.bubble.direction = d)),
        ))
        .child(switch_row("Roll open", move || meta.with(|m| m.bubble.animate), move |on| e13(&|m| m.bubble.animate = on)))
        .child(
            view().style_when(move || !meta.with(|m| m.bubble.animate), |s| s.hidden()).child(slider_row(
                "Roll time",
                (80.0, 600.0, 20.0),
                move || meta.with(|m| m.bubble.animate_ms) as f64,
                |v| format!("{v:.0} ms"),
                move |v| e14(&|m| m.bubble.animate_ms = v as u32),
            )),
        )
        .child(switch_row("Close when the pointer leaves", move || meta.with(|m| m.bubble.close_on_leave), move |on| {
            e15(&|m| m.bubble.close_on_leave = on)
        }))
        .child(switch_row("Mouse wheel opens and closes", move || meta.with(|m| m.bubble.wheel), move |on| e16(&|m| m.bubble.wheel = on)))
}

fn shapes_name(shape: BubbleShape) -> &'static str {
    match shape {
        BubbleShape::Sphere => "Sphere",
        BubbleShape::Circle => "Circle",
        BubbleShape::Square => "Square",
        BubbleShape::Triangle => "Triangle",
        BubbleShape::Emoji => "Emoji",
    }
}

fn directions_name(direction: BubbleDirection) -> &'static str {
    match direction {
        BubbleDirection::Bottom => "Bottom",
        BubbleDirection::Right => "Right",
        BubbleDirection::Top => "Top",
        BubbleDirection::Left => "Left",
        BubbleDirection::OnTop => "On top",
    }
}

// ----- Controls ------------------------------------------------------------------------

fn label_text(label: impl Into<SharedString>) -> Text {
    text(label.into()).text_size(px(13.0)).text_color(move || theme().foreground)
}

/// A label on the left, its control on the right.
pub(crate) fn row(label: &'static str, control: impl IntoElement + 'static) -> View {
    view().flex_row().items_center().justify_between().gap(px(8.0)).min_h(px(32.0)).child(label_text(label)).child(control)
}

/// A row whose label changes, with its control under it.
fn row_dyn(label: impl Fn() -> String + 'static, control: impl IntoElement + 'static) -> View {
    view().gap(px(4.0)).pt(px(4.0)).child(text(label).text_size(px(13.0)).text_color(move || theme().foreground)).child(control)
}

pub(crate) fn styled_switch(checked: impl Fn() -> bool + 'static, set: impl Fn(bool) + 'static, label: &'static str) -> impl IntoElement {
    switch()
        .checked(checked)
        .accessibility_label(label)
        .hit_slop(EdgeInsets::all(px(6.0)))
        .track_class("w-9 h-5 rounded-full bg-input p-0.5 checked:bg-primary transition-colors duration-150")
        .thumb_class("size-4 rounded-full bg-background shadow-sm transition-transform duration-150 checked:translate-x-4")
        .on_change(move |on: &bool| set(*on))
}

pub(crate) fn switch_row(label: &'static str, checked: impl Fn() -> bool + 'static, set: impl Fn(bool) + 'static) -> View {
    row(label, styled_switch(checked, set, label))
}

/// A slider over `range` (min, max, step) with its value shown by `show`.
pub(crate) fn slider_row(
    label: &'static str,
    range: (f64, f64, f64),
    value: impl Fn() -> f64 + 'static,
    show: fn(f64) -> String,
    set: impl Fn(f64) + 'static,
) -> View {
    let value = Rc::new(value);
    let shown = value.clone();
    view()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .min_h(px(32.0))
        .child(label_text(label).w(px(84.0)))
        .child(
            slider()
                .flex_1()
                .min(range.0)
                .max(range.1)
                .step(range.2)
                .accessibility_label(label)
                .value(move || Some(SliderValue::Single(value())))
                .track_class("h-5 justify-center")
                .indicator_class("h-1.5 rounded-full bg-muted")
                .range_class("rounded-full bg-primary")
                .thumb_class("size-4 rounded-full bg-background border-2 border-primary shadow-sm")
                .on_change(move |v: &SliderValue| {
                    if let SliderValue::Single(v) = v {
                        set(*v)
                    }
                }),
        )
        .child(text(move || show(shown())).w(px(44.0)).text_right().text_size(px(12.0)).text_color(move || theme().muted_foreground))
}

/// A colour: a swatch that opens a picker under it, in the panel.
pub(crate) fn color_row(label: &'static str, value: impl Fn() -> Rgb + 'static, set: impl Fn(Rgb) + 'static) -> View {
    let open = signal(false);
    let value = Rc::new(value);
    let (shown, picked) = (value.clone(), value.clone());
    let set = Rc::new(set);
    let on_change = set.clone();
    let hex = move || format!("#{:06X}", shown() & 0xFF_FFFF);
    let picker = color_picker()
        .value(move || Some(Hsla::from(nsh_core::color::rgb(picked()))))
        .on_change(move |e: &ColorPickerEvent| {
            if let ColorPickerEvent::Change(c, _) = e {
                on_change(Color::from(*c).0 >> 8);
            }
        })
        .gap(px(8.0))
        .p(px(10.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .child(
            color_area()
                .accessibility_label("Saturation and brightness")
                .w(px(INNER_WIDTH - 22.0))
                .h(px(110.0))
                .rounded(px(8.0))
                .thumb_class("size-3.5 rounded-full border-2 border-white shadow-md"),
        )
        .child(
            color_slider()
                .color_space(ColorSpace::Hsb)
                .channel(ColorChannel::Hue)
                .accessibility_label("Hue")
                .w(px(INNER_WIDTH - 22.0))
                .h(px(16.0))
                .justify_center()
                .track_class("h-3 rounded-full")
                .thumb_class("size-3.5 rounded-full border-2 border-white shadow-md"),
        )
        .child(
            color_field()
                .accessibility_label("Hex colour")
                .w(px(INNER_WIDTH - 22.0))
                .h(px(28.0))
                .px_2()
                .rounded(px(6.0))
                .border_1()
                .border_color(move || theme().input)
                .text_size(px(12.0)),
        );
    view()
        .child(row(
            label,
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .cursor(Cursor::Pointer)
                .on_press(move || open.update(|o| *o = !*o))
                .child(text(hex).text_size(px(12.0)).text_color(move || theme().muted_foreground))
                .child(
                    view()
                        .w(px(24.0))
                        .h(px(24.0))
                        .rounded(px(7.0))
                        .border_1()
                        .border_color(move || if open.get() { theme().ring } else { theme().border })
                        .bg(move || color(value(), 1.0)),
                ),
        ))
        .child(collapsible().open(open).reveal(true).content(view().pt(px(2.0)).pb(px(6.0)).child(picker)))
}

/// One of a few choices, as a row of segments with `glyph`s (the `name`
/// is what screen readers say).
fn segmented<T: Copy + PartialEq + 'static>(
    width: f32,
    options: Vec<(T, &'static str, &'static str)>,
    value: impl Fn() -> T + 'static,
    set: impl Fn(T) + 'static,
) -> impl IntoElement {
    let value = Rc::new(value);
    let set = Rc::new(set);
    toggle_group()
        .flex_row()
        .w(px(width))
        .p(px(2.0))
        .gap(px(2.0))
        .rounded(px(8.0))
        .bg(move || theme().muted)
        .children(options.into_iter().map(move |(option, glyph, name)| {
            let (value, set) = (value.clone(), set.clone());
            let on = value.clone();
            toggle()
                .flex_1()
                .h(px(26.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .accessibility_label(name)
                .pressed(move || value() == option)
                .pressed_style(|s| s.bg(theme().background).shadow_sm())
                .on_change(move |_: &bool| set(option))
                .child(text(glyph).text_size(px(13.0)).text_color(move || if on() == option { theme().foreground } else { theme().muted_foreground }))
                .into_element()
        }))
}

/// A select of `items` (value, title) with the current `value`.
fn choice(items: Vec<(&'static str, &'static str)>, value: impl Fn() -> String + 'static, set: impl Fn(String) + 'static) -> View {
    let value = Rc::new(value);
    let titles = items.clone();
    let current = value.clone();
    let title = move || {
        let v = current();
        titles.iter().find(|(id, _)| *id == v).map_or("", |(_, t)| t).to_string()
    };
    view().child(
        select()
            .items(items.iter().map(|(id, t)| SelectItem::new(*id, *t)).collect::<Vec<_>>())
            .value(move || Some(SharedString::from(value())))
            .margin(px(6.0))
            .offset(px(4.0))
            .popup_class("p-1 rounded-lg border border-border bg-popover shadow-lg")
            .item_class("px-2 py-1.5 rounded-md text-sm text-popover-foreground hover:bg-accent focus:bg-accent")
            .trigger(
                view()
                    .flex_row()
                    .items_center()
                    .h(px(28.0))
                    .px(px(8.0))
                    .gap(px(4.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(move || theme().input)
                    .bg(move || theme().background)
                    .child(view().flex_1().child(text(title).truncate().text_size(px(12.0)).text_color(move || theme().foreground)))
                    .child(text("▾").text_size(px(10.0)).text_color(move || theme().muted_foreground)),
            )
            .on_change(move |v: &Option<SharedString>| {
                if let Some(v) = v {
                    set(v.to_string())
                }
            }),
    )
}

/// A text field kept in `value` as it's typed.
fn text_input(value: impl Fn() -> String + 'static, set: impl Fn(String) + 'static, placeholder: &'static str) -> Input {
    input()
        .h(px(28.0))
        .px(px(8.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(move || theme().input)
        .bg(move || theme().background)
        .text_color(move || theme().foreground)
        .text_size(px(12.0))
        .placeholder(placeholder)
        .value(value)
        .on_change(move |t: &str| set(t.to_string()))
}

/// A text field whose change is made when it's left or submitted (a
/// note's title renames its file; a shortcut registers).
pub(crate) fn committed_input(value: impl Fn() -> String + 'static, commit: impl Fn(String) + 'static, placeholder: &'static str) -> Input {
    let value = Rc::new(value);
    let draft = signal(untracked(|| value()));
    // A change from elsewhere shows.
    effect(move || draft.set(value()));
    let commit = Rc::new(commit);
    let on_submit = commit.clone();
    text_input(move || draft.get(), move |t| draft.set(t), placeholder)
        .on_blur(move || commit(draft.get_untracked()))
        .on_submit(move |_: &str| on_submit(draft.get_untracked()))
}

/// A small whole number.
fn count_input(value: Signal<Option<f64>>) -> impl IntoElement {
    number_input()
        .min(1.0)
        .max(999.0)
        .step(1.0)
        .accessibility_label("How many")
        .value(move || value.get())
        .on_change(move |v: &Option<f64>| value.set(*v))
        .input_class("w-12 h-7 px-2 rounded-md border border-input bg-background text-sm text-foreground")
        .decrement_button(text("−").text_size(px(11.0)))
        .increment_button(text("+").text_size(px(11.0)))
        .decrement_button_class("hidden")
        .increment_button_class("hidden")
}

/// A round radio mark (the group gives arrow keys).
fn radio_mark(checked: impl Fn() -> bool + Clone + 'static, select: impl Fn() + 'static) -> impl IntoElement {
    let dot = checked.clone();
    let ring = checked.clone();
    radio()
        .checked(checked)
        .hit_slop(EdgeInsets::all(px(6.0)))
        .on_change(move |_: &bool| select())
        .child(
            view()
                .w(px(16.0))
                .h(px(16.0))
                .rounded_full()
                .border_2()
                .border_color(move || if ring() { theme().primary } else { theme().input })
                .items_center()
                .justify_center()
                .child(view().w(px(7.0)).h(px(7.0)).rounded_full().bg(move || theme().primary).style_when(move || !dot(), |s| s.hidden())),
        )
}

pub(crate) fn small_button(label: impl Fn() -> String + 'static, press: impl Fn() + 'static) -> impl IntoElement {
    nsh_base::button()
        .h(px(28.0))
        .px(px(10.0))
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().secondary)
        .active(|s| s.opacity(0.7))
        .on_press(move || press())
        .child(text(label).text_size(px(12.0)).text_color(move || theme().secondary_foreground))
}

pub(crate) fn primary_button(label: impl Fn() -> String + 'static, press: impl Fn() + 'static) -> impl IntoElement {
    nsh_base::button()
        .h(px(28.0))
        .px(px(12.0))
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .bg(move || theme().primary)
        .active(|s| s.opacity(0.75))
        .on_press(move || press())
        .child(text(label).text_size(px(12.0)).font_weight(FontWeight::MEDIUM).text_color(move || theme().primary_foreground))
}

fn link_button(label: impl Fn() -> String + 'static, press: impl Fn() + 'static) -> impl IntoElement {
    nsh_base::button()
        .px(px(4.0))
        .h(px(22.0))
        .justify_center()
        .active(|s| s.opacity(0.6))
        .on_press(move || press())
        .child(text(label).text_size(px(12.0)).text_color(move || theme().primary))
}

fn icon_button(glyph: &'static str, label: &'static str, press: impl Fn() + 'static) -> impl IntoElement {
    nsh_base::button()
        .w(px(28.0))
        .h(px(28.0))
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .accessibility_label(label)
        .hover(|s| s.bg(theme().accent))
        .active(|s| s.opacity(0.6))
        .on_press(move || press())
        .child(text(glyph).text_size(px(13.0)).text_color(move || theme().muted_foreground))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect { x: 0.0, y: 0.0, w: 1920.0, h: 1040.0 };

    #[test]
    fn it_goes_right_of_the_note() {
        let note = Rect { x: 100.0, y: 200.0, w: 280.0, h: 260.0 };
        assert_eq!(place(note, &[SCREEN], 300.0, 560.0), (390.0, 200.0));
    }

    #[test]
    fn it_goes_left_when_the_right_has_no_room() {
        let note = Rect { x: 1500.0, y: 200.0, w: 280.0, h: 260.0 };
        assert_eq!(place(note, &[SCREEN], 300.0, 560.0), (1190.0, 200.0));
    }

    #[test]
    fn it_stays_on_the_screen() {
        let note = Rect { x: 100.0, y: 900.0, w: 280.0, h: 100.0 };
        assert_eq!(place(note, &[SCREEN], 300.0, 560.0), (390.0, 480.0));
    }

    #[test]
    fn it_goes_on_the_display_the_note_is_on() {
        let second = Rect { x: 1920.0, y: 0.0, w: 1280.0, h: 1000.0 };
        let note = Rect { x: 2900.0, y: 100.0, w: 280.0, h: 260.0 };
        assert_eq!(place(note, &[SCREEN, second], 300.0, 560.0), (2590.0, 100.0));
    }

    #[test]
    fn tomorrow_at_nine_is_a_day_ahead_at_most() {
        let now = crate::store::now_ms();
        let at = tomorrow_at_nine(now).expect("a time");
        assert!(at > now && at - now <= 34.0 * 3_600_000.0);
    }
}
