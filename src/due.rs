//! Dates in the text: typing `@friday`, `@tomorrow 3pm`, `@20 nov` or
//! `@in 2h` puts a date there (a chip), and at that time the app reminds of
//! the line it's on, unless that's a checklist item already ticked.
//!
//! A date is an inline `due` node holding its time as `yyyy-mm-dd hh:mm`
//! (local time) and whether its reminder was given. In a note's file it's
//! `<time data-due="2026-11-20 09:00">Fri 20 Nov 09:00</time>`; in Markdown
//! and plain text, `@2026-11-20 09:00`.
//!
//! While something that may become a date is typed after `@`, a menu
//! offers what it means now (Enter or Tab takes it). Typing on does too: as
//! soon as what follows can't be part of the date (`@friday call…`), the
//! date before it becomes the chip. Something that never was a date
//! (`@john`) is left alone.

use std::sync::Arc;

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use nsh_base::rich_editor::{
    extensions::{
        Extension,
        suggestion::{SuggestionOptions, SuggestionState, SuggestionTrigger, exit_suggestion, replace_suggestion, suggestion, suggestion_state},
    },
    model::{AttrParse, AttrRender, AttrSpec, DomOutputSpec, Fragment, Node, NodeSpec, ParseRule, Schema, TextRender},
    state::{EditorState, Plugin},
    view::{NodeView, NodeViewCx, NodeViews},
};
use nsh_base::theme;
use nsh_core::prelude::*;
use serde_json::{Value, json};

use crate::alarms;
use crate::app::{App, Note};
use crate::kit::Editor;
use crate::ui::Ctx;

/// The node type, and the plugin's key.
pub const NODE: &str = "due";
const KEY: &str = "due";

// ----- Reading what's typed -------------------------------------------------------

const WEEKDAYS: [(&str, Weekday); 7] = [
    ("monday", Weekday::Mon),
    ("tuesday", Weekday::Tue),
    ("wednesday", Weekday::Wed),
    ("thursday", Weekday::Thu),
    ("friday", Weekday::Fri),
    ("saturday", Weekday::Sat),
    ("sunday", Weekday::Sun),
];
const MONTHS: [&str; 12] = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
/// The words for a day that the menu completes.
const DAY_WORDS: [&str; 10] = ["today", "tonight", "tomorrow", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];
const OTHER_WORDS: [&str; 5] = ["at", "in", "noon", "tmr", "tmrw"];
const UNITS: [(&str, i64); 14] = [
    ("m", 1),
    ("min", 1),
    ("mins", 1),
    ("minute", 1),
    ("minutes", 1),
    ("h", 60),
    ("hr", 60),
    ("hrs", 60),
    ("hour", 60),
    ("hours", 60),
    ("d", 1440),
    ("day", 1440),
    ("days", 1440),
    ("week", 10080),
];

/// Whether `token` is `word`, or a short form of it (three letters or
/// more: `fri`, `tom`, `nov`).
fn is_word(token: &str, word: &str) -> bool {
    token == word || (token.len() >= 3 && word.starts_with(token))
}

fn time(h: u32, m: u32) -> Option<NaiveTime> {
    NaiveTime::from_hms_opt(h, m, 0)
}

/// `3pm`, `3:30pm`, `9am`, `15:00`, `noon`. A bare number isn't a time.
fn parse_time(token: &str) -> Option<NaiveTime> {
    if token == "noon" {
        return time(12, 0);
    }
    let (digits, half) = match token.strip_suffix("am").map(|d| (d, Some(false))).or_else(|| token.strip_suffix("pm").map(|d| (d, Some(true)))) {
        Some((digits, half)) => (digits, half),
        None => (token, None),
    };
    let (h, m) = match digits.split_once(':') {
        Some((h, m)) if m.len() == 2 => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        Some(_) => return None,
        None if half.is_some() => (digits.parse::<u32>().ok()?, 0),
        None => return None,
    };
    match half {
        Some(pm) if (1..=12).contains(&h) => time(h % 12 + if pm { 12 } else { 0 }, m),
        Some(_) => None,
        None => time(h, m),
    }
}

/// A day typed, before it's given a date.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Day {
    Today,
    Tonight,
    Tomorrow,
    Weekday(Weekday),
    /// A day of a month, of this year or the next.
    MonthDay(u32, u32),
    Date(NaiveDate),
}

/// The day `token` names. `whole`: only written out in full (`fri`, `tom`
/// and `sat` are also words and names).
fn parse_day_word(token: &str, whole: bool) -> Option<Day> {
    let is = |word: &str| if whole { token == word } else { is_word(token, word) };
    if is("today") {
        return Some(Day::Today);
    }
    if is("tonight") {
        return Some(Day::Tonight);
    }
    if is("tomorrow") || (!whole && (token == "tmr" || token == "tmrw")) {
        return Some(Day::Tomorrow);
    }
    WEEKDAYS.iter().find(|(name, _)| is(name)).map(|(_, day)| Day::Weekday(*day))
}

fn parse_month(token: &str) -> Option<u32> {
    MONTHS.iter().position(|name| is_word(token, name)).map(|i| i as u32 + 1)
}

fn parse_day_number(token: &str) -> Option<u32> {
    let digits = token.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let suffix = &token[digits.len()..];
    if !matches!(suffix, "" | "st" | "nd" | "rd" | "th") {
        return None;
    }
    digits.parse().ok().filter(|d| (1..=31).contains(d))
}

/// The moment `text` means at `now`: `friday`, `tomorrow 3pm`, `fri at
/// 15:30`, `3pm`, `20 nov`, `nov 20 9am`, `2026-11-20`, `in 2h`, `in 30
/// minutes`. A day without a time is at 09:00 (`tonight` at 20:00, `today`
/// at 17:00 or in an hour); a time without a day is the next such time; a
/// weekday is the next one (today's only with a time still to come).
pub fn parse(text: &str, now: NaiveDateTime) -> Option<NaiveDateTime> {
    parse_as(text, now, false)
}

/// As `parse`; with `whole`, a day's name counts only written out in full.
fn parse_as(text: &str, now: NaiveDateTime, whole: bool) -> Option<NaiveDateTime> {
    let lower = text.trim().to_lowercase();
    let tokens: Vec<&str> = lower.split_whitespace().filter(|t| *t != "at").collect();
    if tokens.is_empty() {
        return None;
    }
    if tokens[0] == "in" {
        let (count, unit) = match tokens[1..] {
            [both] => {
                let digits = both.trim_end_matches(|c: char| c.is_ascii_alphabetic());
                (digits, &both[digits.len()..])
            }
            [count, unit] => (count, unit),
            _ => return None,
        };
        let count: i64 = count.parse().ok().filter(|n| (1..=10_000).contains(n))?;
        let minutes = UNITS.iter().find(|(name, _)| *name == unit || (*name == "week" && unit == "weeks") || (*name == "week" && unit == "w"))?.1;
        return Some(now + Duration::minutes(count * minutes));
    }
    let (mut day, mut at) = (None::<Day>, None::<NaiveTime>);
    let mut i = 0;
    while i < tokens.len() {
        let token = tokens[i];
        let next = tokens.get(i + 1).copied();
        if let Some(found) = parse_time(token) {
            if at.replace(found).is_some() {
                return None;
            }
        } else if let Some(found) = parse_day_word(token, whole) {
            if day.replace(found).is_some() {
                return None;
            }
        } else if let Ok(date) = NaiveDate::parse_from_str(token, "%Y-%m-%d") {
            if day.replace(Day::Date(date)).is_some() {
                return None;
            }
        } else if let (Some(d), Some(m)) = (parse_day_number(token), next.and_then(parse_month)) {
            // "20 nov"
            if day.replace(Day::MonthDay(m, d)).is_some() {
                return None;
            }
            i += 1;
        } else if let (Some(m), Some(d)) = (parse_month(token), next.and_then(parse_day_number)) {
            // "nov 20"
            if day.replace(Day::MonthDay(m, d)).is_some() {
                return None;
            }
            i += 1;
        } else {
            return None;
        }
        i += 1;
    }
    let today = now.date();
    let nine = time(9, 0)?;
    Some(match (day, at) {
        (None, None) => return None,
        (None, Some(at)) => {
            let soon = today.and_time(at);
            if soon > now { soon } else { (today + Duration::days(1)).and_time(at) }
        }
        (Some(Day::Today), at) => match at {
            Some(at) => today.and_time(at),
            None => {
                let five = today.and_time(time(17, 0)?);
                if five > now { five } else { now + Duration::hours(1) }
            }
        },
        (Some(Day::Tonight), at) => today.and_time(at.unwrap_or(time(20, 0)?)),
        (Some(Day::Tomorrow), at) => (today + Duration::days(1)).and_time(at.unwrap_or(nine)),
        (Some(Day::Weekday(weekday)), at) => {
            let ahead = (7 + weekday.num_days_from_monday() as i64 - today.weekday().num_days_from_monday() as i64) % 7;
            let same_day = today.and_time(at.unwrap_or(nine));
            if ahead == 0 && at.is_some() && same_day > now {
                same_day
            } else {
                (today + Duration::days(if ahead == 0 { 7 } else { ahead })).and_time(at.unwrap_or(nine))
            }
        }
        (Some(Day::MonthDay(m, d)), at) => {
            let this_year = NaiveDate::from_ymd_opt(today.year(), m, d);
            let date = match this_year {
                Some(date) if date >= today => date,
                _ => NaiveDate::from_ymd_opt(today.year() + 1, m, d)?,
            };
            date.and_time(at.unwrap_or(nine))
        }
        (Some(Day::Date(date)), at) => date.and_time(at.unwrap_or(nine)),
    })
}

/// Whether more typing could still make `text` a date: every finished word
/// belongs to one, and the word being typed starts one.
fn may_become_date(text: &str) -> bool {
    let lower = text.to_lowercase();
    let finished = lower.ends_with(char::is_whitespace);
    let tokens: Vec<&str> = lower.split_whitespace().collect();
    let numberish = |t: &str| !t.is_empty() && t.len() <= 10 && t.chars().next().is_some_and(|c| c.is_ascii_digit()) && t.chars().all(|c| c.is_ascii_alphanumeric() || c == ':' || c == '-');
    let whole = |t: &str| {
        parse_day_word(t, false).is_some() || parse_month(t).is_some() || parse_time(t).is_some() || numberish(t) || OTHER_WORDS.contains(&t) || UNITS.iter().any(|(u, _)| *u == t || format!("{u}s") == t)
    };
    let starts = |t: &str| {
        numberish(t)
            || DAY_WORDS.iter().chain(MONTHS.iter()).chain(OTHER_WORDS.iter()).any(|w| w.starts_with(t))
            || UNITS.iter().any(|(u, _)| u.starts_with(t) || format!("{u}s").starts_with(t))
    };
    // Words that are a date need a day or a time among them at some point:
    // a line of numbers isn't one. Five words is the longest a date gets.
    if tokens.len() > 5 {
        return false;
    }
    match tokens.split_last() {
        None => true,
        Some((last, before)) => before.iter().all(|t| whole(t)) && if finished { whole(last) } else { starts(last) },
    }
}

/// The longest run of words at the start of `text` that's a date, when
/// what follows it is something else: (its length in `text`, the date).
/// A day counts here only written out in full: `@tom called` and `@sat
/// down` are a name and a word, and nobody was asked.
fn date_before_other_text(text: &str, now: NaiveDateTime) -> Option<(usize, NaiveDateTime)> {
    let ends: Vec<usize> = text.char_indices().filter(|(_, c)| c.is_whitespace()).map(|(i, _)| i).collect();
    ends.into_iter().rev().find_map(|end| parse_as(&text[..end], now, true).map(|at| (text[..end].trim_end().len(), at)))
}

/// What the menu offers for what's typed after `@`: what it means, then
/// what the word being typed could become; with nothing typed, the usual
/// ones.
fn offers(text: &str, now: NaiveDateTime) -> Vec<NaiveDateTime> {
    let lower = text.trim_start().to_lowercase();
    let mut out: Vec<NaiveDateTime> = Vec::new();
    let mut add = |at: Option<NaiveDateTime>| {
        if let Some(at) = at
            && !out.contains(&at)
        {
            out.push(at);
        }
    };
    if lower.trim().is_empty() {
        for usual in ["today", "tomorrow", "monday"] {
            add(parse(usual, now));
        }
        return out;
    }
    add(parse(&lower, now));
    if !lower.ends_with(char::is_whitespace) {
        let start = lower.rfind(char::is_whitespace).map_or(0, |i| i + 1);
        let (before, last) = lower.split_at(start);
        for word in DAY_WORDS.iter().filter(|w| w.starts_with(last)) {
            add(parse(&format!("{before}{word}"), now));
        }
    }
    out.truncate(5);
    out
}

/// A date in words: "Today 15:00", "Tomorrow 09:00", "Fri 20 Nov 09:00",
/// with the year when it isn't this one.
fn label(at: NaiveDateTime, now: NaiveDateTime) -> String {
    let time = at.format("%H:%M");
    match (at.date() - now.date()).num_days() {
        0 => format!("Today {time}"),
        1 => format!("Tomorrow {time}"),
        -1 => format!("Yesterday {time}"),
        _ if at.date().year() == now.date().year() => format!("{} {time}", at.format("%a %-d %b")),
        _ => format!("{} {time}", at.format("%-d %b %Y")),
    }
}

/// How a date is kept in the note.
const KEPT: &str = "%Y-%m-%d %H:%M";

fn kept(at: NaiveDateTime) -> String {
    at.format(KEPT).to_string()
}

fn from_kept(text: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(text.trim(), KEPT).ok()
}

// ----- The node and the plugin -------------------------------------------------------

fn text_attr() -> AttrSpec {
    let mut attr = AttrSpec::default();
    attr.default = Some(Value::String(String::new()));
    attr.parse_html = Some(AttrParse::Skip);
    attr.render_html = Some(AttrRender::Skip);
    attr
}

/// The `due` node type: `when` (`yyyy-mm-dd hh:mm`) and `notified` (`1`
/// once its reminder was given).
pub fn extension() -> Extension {
    Extension::node(NODE, |_| {
        let mut spec = NodeSpec::default();
        spec.group = Some("inline".into());
        spec.inline = true;
        spec.atom = true;
        spec.attrs = vec![("when".into(), text_attr()), ("notified".into(), text_attr())];
        spec.parse_dom = vec![ParseRule::tag("time[data-due]").with_get_attrs(|input| {
            let element = input.element()?;
            let attrs = [("when", element.attr("data-due").unwrap_or_default().to_string()), ("notified", element.attr("data-notified").unwrap_or_default().to_string())];
            Some(attrs.into_iter().map(|(k, v)| (k.to_string(), Value::String(v))).collect())
        })];
        spec.to_dom = Some(DomOutputSpec::render(|node| {
            let get = |name: &str| node.attr(name).and_then(Value::as_str).unwrap_or_default().to_string();
            let when = get("when");
            // What a browser shows: the date as it's kept (it reads the
            // same whenever the file is opened).
            let shown = from_kept(&when).map_or_else(|| when.clone(), |at| at.format("%a %-d %b %Y %H:%M").to_string());
            let time = DomOutputSpec::tag("time").attr("data-due", when);
            let time = if get("notified").is_empty() { time } else { time.attr("data-notified", "1") };
            time.child(DomOutputSpec::Text(shown))
        }));
        spec.render_text = Some(TextRender::Template("@{when}".into()));
        spec
    })
}

/// The text after the `@` at `from..to`.
fn typed(state: &EditorState, from: usize, to: usize) -> String {
    state.doc().text_between(from, to, None, Some("\u{fffc}")).chars().skip(1).collect()
}

/// The plugin following a typed `@` while what follows may be a date.
pub fn plugin() -> Plugin {
    let mut trigger = SuggestionTrigger::default();
    trigger.char = "@".into();
    trigger.allow_spaces = true;
    let mut options = SuggestionOptions::default();
    options.key = KEY.into();
    options.trigger = trigger;
    options.capture_keys = ["ArrowUp", "ArrowDown", "Enter", "Tab"].map(String::from).to_vec();
    options.allow = Some(Arc::new(|state: &EditorState, range, _| {
        if !state.doc().resolve(range.from).is_ok_and(|at| at.parent().ty().name() != "codeBlock") {
            return false;
        }
        let text = typed(state, range.from, range.to);
        may_become_date(&text) || date_before_other_text(&text, alarms::local_now()).is_some()
    }));
    suggestion(options)
}

pub fn with_node_views(views: NodeViews) -> NodeViews {
    views.with(NODE, Arc::new(DueView))
}

fn state(editor: &Editor) -> Option<Arc<SuggestionState>> {
    suggestion_state(&editor.state(), KEY)
}

fn chip(schema: &Schema, at: NaiveDateTime) -> Option<Node> {
    schema.node_from_json(&json!({ "type": NODE, "attrs": { "when": kept(at), "notified": "" } })).ok()
}

/// Puts the date in place of the `@` and what was typed after it, with a
/// space to type on. Whether it did.
fn insert(editor: &Editor, at: NaiveDateTime) -> bool {
    let schema = &editor.kit.schema;
    let (Some(chip), Ok(space)) = (chip(schema, at), schema.node_from_json(&json!({ "type": "text", "text": " " }))) else { return false };
    editor.session.run(&replace_suggestion(KEY, Fragment::from(vec![chip, space])))
}

/// Puts the date in place of the `@` at `from` and the `length` characters
/// after it, leaving what's typed after that. Whether it did.
fn insert_before_text(editor: &Editor, from: usize, length: usize, at: NaiveDateTime) -> bool {
    let Some(chip) = chip(&editor.kit.schema, at) else { return false };
    let state = editor.state();
    let mut tr = state.tr();
    if tr.replace_with(from, from + 1 + length, Fragment::from(chip)).is_err() {
        return false;
    }
    editor.session.dispatch(tr).is_ok()
}

// ----- The date in the text ------------------------------------------------------------

/// The checklist item around `pos` is ticked.
fn in_ticked_item(doc: &Node, pos: usize) -> bool {
    let Ok(at) = doc.resolve(pos) else { return false };
    (0..=at.depth)
        .rev()
        .map(|depth| at.node(depth))
        .find(|node| node.ty().name() == "taskItem")
        .is_some_and(|item| item.attrs().get("checked").and_then(Value::as_bool).unwrap_or(false))
}

/// A date: in words as it is from today, red once it's past, faint and
/// struck through in a ticked checklist item.
struct DueView;

impl NodeView for DueView {
    fn create(&self, cx: &NodeViewCx, _: &Node) -> AnyElement {
        let node = cx.node();
        let get_pos = cx.get_pos();
        let session = cx.session.clone();
        let app = App::current();
        let when = move || node.with(|n| n.attrs().get("when").and_then(Value::as_str).and_then(from_kept));
        // Now, to the minute (it only matters when the day or the minute
        // changes), and the document as it changes around the date.
        let now = move || app.map_or_else(alarms::local_now, |app| alarms::to_local_time(app.now.get()));
        let ticked = move || {
            if let Some(app) = app {
                app.revision.get();
            }
            get_pos().is_some_and(|pos| in_ticked_item(session.state().doc(), pos))
        };
        let (ticked_too, when_too) = (ticked.clone(), when);
        let late = move || !ticked_too() && when_too().is_some_and(|at| at <= now());
        let late_too = late.clone();
        // Small and on a tight line: the editor rests the chip's box on the
        // text's baseline, and a taller one would push the line down.
        text(move || when().map_or_else(|| "date".to_string(), |at| format!(" ◷ {} ", label(at, now()))))
            .text_size(px(12.0))
            .line_height(px(13.0))
            .translate_y(px(2.0))
            .rounded(px(4.0))
            .bg(move || if late() { nsh_core::Color(0xdc262630) } else { nsh_core::Color(0x0000001a) })
            .style_when(late_too, |s| s.text_color(nsh_core::Color(0xb91c1cff)))
            .style_when(ticked, |s| s.line_through().opacity(0.55))
            .label("Reminder date")
            .into_element()
    }

    /// The same element follows the date as its reminder is given.
    fn update(&self, _: &Node, _: &[nsh_base::rich_editor::state::Decoration]) -> bool {
        true
    }
}

// ----- Reminders -------------------------------------------------------------------------

/// A date in a note whose reminder is to give: where it is, and the line
/// it's on.
#[derive(Debug, PartialEq)]
pub struct Reminder {
    pub pos: usize,
    pub line: String,
}

/// The dates in `doc` that have come (`now`) and weren't reminded of, not
/// counting those in ticked checklist items.
pub fn reminders(doc: &Node, now: NaiveDateTime) -> Vec<Reminder> {
    fn walk(node: &Node, start: usize, ticked: bool, now: NaiveDateTime, out: &mut Vec<Reminder>) {
        let mut pos = start;
        for child in node.content().iter() {
            if child.ty().name() == NODE {
                let get = |name: &str| child.attrs().get(name).and_then(Value::as_str).unwrap_or_default().to_string();
                if !ticked && get("notified").is_empty() && from_kept(&get("when")).is_some_and(|at| at <= now) {
                    // The line without its dates.
                    let line: String = node.content().iter().filter(|n| n.ty().name() != NODE).map(|n| n.text_content()).collect();
                    out.push(Reminder { pos, line: line.split_whitespace().collect::<Vec<_>>().join(" ") });
                }
            } else if !child.is_text() {
                let ticked = if child.ty().name() == "taskItem" { child.attrs().get("checked").and_then(Value::as_bool).unwrap_or(false) } else { ticked };
                walk(child, pos + 1, ticked, now, out);
            }
            pos += child.node_size();
        }
    }
    let mut out = Vec::new();
    walk(doc, 0, false, now, &mut out);
    out
}

/// Marks the date at `pos` as reminded of (not a step of the undo history).
pub fn mark_reminded(note: &Note, pos: usize) {
    let state = note.editor.state();
    if !state.doc().node_at(pos).is_some_and(|n| n.ty().name() == NODE) {
        return;
    }
    let mut tr = state.tr();
    if tr.set_node_attribute(pos, "notified", Value::String("1".into())).is_ok() {
        tr.set_add_to_history(false);
        let _ = note.editor.session.dispatch(tr);
    }
}

// ----- The menu ------------------------------------------------------------------------

/// The menu under a typed `@`: what the date typed after it means; ↑ ↓
/// move, Enter or Tab takes one, a click does. `window_height` keeps it
/// inside the window.
pub fn menu(ctx: Ctx, window_height: Signal<f32>) -> impl IntoElement {
    let window = nsh_core::element::desktop::current_window();
    let owner = Owner::current();
    let index = signal(0_usize);
    let keys_seen = signal(0_u64);
    let open = signal(None::<(usize, String)>);
    let rows = move || open.get().map(|(_, text)| offers(&text, alarms::local_now())).unwrap_or_default();
    let pick = move |at: NaiveDateTime| {
        ctx.editor.with_untracked(|e| insert(e, at));
        ctx.changed();
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
        // Typed on past the date: the date becomes the chip, the rest stays.
        if let Some((from, text)) = &now
            && !may_become_date(text)
        {
            if let Some((length, at)) = date_before_other_text(text, alarms::local_now()) {
                let length = text[..length].encode_utf16().count();
                ctx.editor.with_untracked(|e| insert_before_text(e, *from, length, at));
            } else {
                ctx.editor.with_untracked(|e| e.session.run(&exit_suggestion(KEY)));
            }
            return;
        }
        let before = open.get_untracked();
        if now != before {
            index.set(0);
            open.set(now.clone());
        }
        if state.key_count == keys_seen.get_untracked() {
            return;
        }
        keys_seen.set(state.key_count);
        if now.is_none() {
            return;
        }
        let found = untracked(rows);
        if found.is_empty() {
            // Nothing to take: the key was meant for the text. The menu
            // lets go, so the next one reaches it.
            ctx.editor.with_untracked(|e| e.session.run(&exit_suggestion(KEY)));
            return;
        }
        match state.last_key.as_deref() {
            Some("ArrowDown") => index.update(|i| *i = (*i + 1) % found.len()),
            Some("ArrowUp") => index.update(|i| *i = (*i + found.len() - 1) % found.len()),
            Some("Enter" | "Tab") => pick(found[index.get_untracked().min(found.len() - 1)]),
            _ => {}
        }
    });
    let pointer = stored(None::<(f32, f32)>);
    let retry = signal(0_u32);
    dynamic(move || {
        retry.get();
        let (from, _) = open.get()?;
        let Some(caret) = ctx.editor.with_untracked(|e| e.session.page_coords_at_pos(from)) else {
            let _ = set_timeout(std::time::Duration::from_millis(16), move || retry.update(|n| *n += 1));
            return None;
        };
        let found = rows();
        if found.is_empty() {
            return None;
        }
        let height = window_height.get();
        let window_width = window.state().map(|s| s.bounds.size.width.0).unwrap_or(f32::MAX);
        let menu_width = 210.0_f32.min(window_width - 8.0);
        let left = caret.origin.x.0.min(window_width - menu_width - 4.0).max(4.0);
        let below = caret.origin.y.0 + caret.size.height.0 + 4.0;
        let wanted = found.len() as f32 * 32.0 + 32.0;
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
        let menu = if below + wanted <= height || caret.origin.y.0 < height / 2.0 { menu.top(px(below)) } else { menu.bottom(px(height - caret.origin.y.0 + 4.0)) };
        let today = alarms::local_now();
        Some(
            menu.children(found.into_iter().enumerate().map(move |(i, at)| {
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .h(px(30.0))
                    .px_2()
                    .rounded(px(6.0))
                    .bg(move || if index.get() == i { theme().accent } else { theme().popover })
                    .focusable(false)
                    .on_mouse_move(move |e: &MouseEvent| {
                        let here = (e.page_position.x.0, e.page_position.y.0);
                        if pointer.get().is_some_and(|last| last != here) {
                            index.set(i);
                        }
                        pointer.replace(Some(here));
                    })
                    .cursor(Cursor::Pointer)
                    .on_press(move || pick(at))
                    .child(text("◷").text_size(px(13.0)).text_color(move || theme().muted_foreground))
                    .child(text(label(at, today)).flex_1().truncate().text_size(px(13.0)))
                    .into_element()
            }))
            .child(
                text("fri 3pm · 20 nov · in 2h")
                    .px_2()
                    .py(px(4.0))
                    .text_size(px(11.0))
                    .text_color(move || theme().muted_foreground),
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::{Format, html_fragment};
    use crate::kit::{Kit, Relay};

    /// Sunday 15 November 2026, 10:30.
    fn now() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 15).unwrap().and_hms_opt(10, 30, 0).unwrap()
    }

    fn at(text: &str) -> String {
        parse(text, now()).map_or_else(|| "-".to_string(), kept)
    }

    #[test]
    fn dates_as_typed() {
        assert_eq!(at("friday"), "2026-11-20 09:00");
        assert_eq!(at("fri"), "2026-11-20 09:00");
        assert_eq!(at("Friday 3pm"), "2026-11-20 15:00");
        assert_eq!(at("fri at 15:30"), "2026-11-20 15:30");
        assert_eq!(at("tomorrow"), "2026-11-16 09:00");
        assert_eq!(at("tom 9:15am"), "2026-11-16 09:15");
        assert_eq!(at("today"), "2026-11-15 17:00");
        assert_eq!(at("tonight"), "2026-11-15 20:00");
        assert_eq!(at("3pm"), "2026-11-15 15:00");
        // A time already past is tomorrow's.
        assert_eq!(at("9am"), "2026-11-16 09:00");
        assert_eq!(at("noon"), "2026-11-15 12:00");
        assert_eq!(at("12am"), "2026-11-16 00:00");
        assert_eq!(at("20 nov"), "2026-11-20 09:00");
        assert_eq!(at("nov 20th 4pm"), "2026-11-20 16:00");
        // A day of the year already past is next year's.
        assert_eq!(at("3 march"), "2027-03-03 09:00");
        assert_eq!(at("2026-12-01"), "2026-12-01 09:00");
        assert_eq!(at("2026-12-01 18:45"), "2026-12-01 18:45");
        assert_eq!(at("in 2h"), "2026-11-15 12:30");
        assert_eq!(at("in 45 minutes"), "2026-11-15 11:15");
        assert_eq!(at("in 3 days"), "2026-11-18 10:30");
    }

    #[test]
    fn the_weekday_it_is_today_is_next_weeks_unless_a_later_time_is_given() {
        // It's Sunday 10:30.
        assert_eq!(at("sunday"), "2026-11-22 09:00");
        assert_eq!(at("sunday 3pm"), "2026-11-15 15:00");
        assert_eq!(at("sunday 9am"), "2026-11-22 09:00");
        assert_eq!(at("monday"), "2026-11-16 09:00");
    }

    #[test]
    fn what_isnt_a_date() {
        for text in ["", "john", "friday call", "3", "fr", "to do", "friday friday", "3pm 4pm", "in", "in 2", "25:00", "13pm", "31 feb"] {
            assert_eq!(at(text), "-", "{text}");
        }
    }

    #[test]
    fn typing_on_past_a_date() {
        // Still may become one: the menu stays.
        for text in ["", "f", "fri", "friday", "friday ", "friday 3", "friday 3p", "tom", "20", "20 n", "in 2", "in 2 ho", "nov 2"] {
            assert!(may_become_date(text), "{text}");
        }
        // No more: what was a date before it is taken, or nothing is.
        for text in ["john", "jo", "friday c", "to do", "3 eggs", "email", "tomorrow 3pm x"] {
            assert!(!may_become_date(text), "{text}");
        }
        let taken = |text: &str| date_before_other_text(text, now()).map(|(length, at)| (text[..length].to_string(), kept(at)));
        assert_eq!(taken("friday call mom"), Some(("friday".to_string(), "2026-11-20 09:00".to_string())));
        assert_eq!(taken("tomorrow 3pm x"), Some(("tomorrow 3pm".to_string(), "2026-11-16 15:00".to_string())));
        assert_eq!(taken("3 eggs"), None);
        assert_eq!(taken("john said"), None);
        // Short forms are taken only from the menu: these are other words.
        for text in ["tom called", "sat down", "sun is out", "fri call", "mon ami"] {
            assert_eq!(taken(text), None, "{text}");
        }
        assert_eq!(taken("3pm call"), Some(("3pm".to_string(), "2026-11-15 15:00".to_string())));
        assert_eq!(taken("20 nov call"), Some(("20 nov".to_string(), "2026-11-20 09:00".to_string())));
    }

    #[test]
    fn the_menu_offers_what_is_meant_and_completions() {
        let shown = |text: &str| offers(text, now()).into_iter().map(|at| label(at, now())).collect::<Vec<_>>();
        assert_eq!(shown(""), ["Today 17:00", "Tomorrow 09:00"]);
        assert_eq!(shown("fri"), ["Fri 20 Nov 09:00"]);
        assert_eq!(shown("t"), ["Today 17:00", "Today 20:00", "Tomorrow 09:00", "Tue 17 Nov 09:00", "Thu 19 Nov 09:00"]);
        assert_eq!(shown("sat 8pm"), ["Sat 21 Nov 20:00"]);
        assert_eq!(shown("zzz"), Vec::<String>::new());
    }

    #[test]
    fn a_date_round_trips_through_the_note_file_and_reads_as_text() {
        let kit = Kit::new(Relay::default());
        let html = r#"<ul data-type="taskList"><li data-type="taskItem" data-checked="false"><label><input type="checkbox"><span></span></label><div><p>Call the bank <time data-due="2026-11-20 09:00">Fri 20 Nov 2026 09:00</time></p></div></li></ul>"#;
        let doc = Format::Html.parse(&kit.schema, html).unwrap();
        assert_eq!(html_fragment(&doc).unwrap(), html);
        assert!(Format::Text.serialize(&doc).unwrap().contains("Call the bank @2026-11-20 09:00"));
        assert!(Format::Markdown.serialize(&doc).unwrap().contains("Call the bank @2026-11-20 09:00"));
    }

    #[test]
    fn reminders_are_for_dates_that_came_in_unticked_items_once() {
        let kit = Kit::new(Relay::default());
        let item = |checked: bool, text: &str, when: &str, notified: &str| {
            json!({ "type": "taskItem", "attrs": { "checked": checked }, "content": [{ "type": "paragraph", "content": [
                { "type": "text", "text": text },
                { "type": NODE, "attrs": { "when": when, "notified": notified } },
            ] }] })
        };
        let doc = kit
            .schema
            .node_from_json(&json!({ "type": "doc", "content": [
                { "type": "paragraph", "content": [{ "type": "text", "text": "Plain line " }, { "type": NODE, "attrs": { "when": "2026-11-15 10:00", "notified": "" } }] },
                { "type": "taskList", "content": [
                    item(false, "Past ", "2026-11-14 09:00", ""),
                    item(true, "Ticked ", "2026-11-14 09:00", ""),
                    item(false, "Told ", "2026-11-14 09:00", "1"),
                    item(false, "Later ", "2026-11-15 10:31", ""),
                ] },
            ] }))
            .unwrap();
        let found = reminders(&doc, now());
        assert_eq!(found.iter().map(|r| r.line.as_str()).collect::<Vec<_>>(), ["Plain line", "Past"]);
        // Each is where its date is.
        for reminder in &found {
            assert_eq!(doc.node_at(reminder.pos).unwrap().ty().name(), NODE);
        }
    }
}
