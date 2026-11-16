//! Sleep and alarms: a sleeping note is hidden until its time; an alarm
//! shows its note with a notification and a sound, and may repeat.
//!
//! Times are saved as absolute moments (`NoteMeta::sleep_until`,
//! `NoteMeta::alarm`), so they survive a restart; what came due while the
//! app wasn't running fires when it starts.
//!
//! The calendar arithmetic works on local wall-clock times
//! (`NaiveDateTime`), so "daily at 09:00" stays at 09:00 across daylight
//! saving changes; only the edges convert to and from milliseconds.

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Weekday};
use nsh_core::notifications::{Notification, NotificationPermission};
use nsh_core::prelude::*;

use crate::app::{App, Note, NoteCommand};
use crate::model::{Alarm, NoteMeta, Repeat};

/// How the user reads and types a moment.
const FORMAT: &str = "%Y-%m-%d %H:%M";

thread_local! {
    /// The user was asked to allow notifications (once is enough).
    static ASKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Starts watching the clock (`App::now`) for alarms. Call once.
pub fn install(app: App) {
    // Only the clock is tracked: what firing changes mustn't run this again.
    effect(move || {
        let now = app.now.get();
        untracked(|| tick(app, now));
    });
    // A notification the user presses brings its note forward.
    app.owner.run(|| {
        let _ = spawn(async move {
            loop {
                let id = next_notification_press().await;
                if let Some(note) = app.note(&id) {
                    app.show_on_desktop(&note);
                    app.command.set(Some((note.id.clone(), NoteCommand::Focus)));
                }
            }
        });
    });
}

/// Fires the alarms that are due and forgets sleeps that are over.
fn tick(app: App, now: f64) {
    for note in app.notes.get_untracked() {
        let (alarm, sleep_until, trashed) = note.meta.with_untracked(|m| (m.alarm, m.sleep_until, m.trashed.is_some()));
        if trashed {
            continue;
        }
        if let Some(alarm) = alarm
            && alarm.at <= now
        {
            fire(app, &note, alarm, now);
        } else if sleep_until.is_some_and(|until| until <= now) {
            app.update(&note, |m| m.sleep_until = None);
        }
    }
}

/// Shows the note, tells the user, and sets the alarm's next time.
fn fire(app: App, note: &Note, alarm: Alarm, now: f64) {
    app.show_on_desktop(note);
    app.set_alarm(note, next_alarm(alarm, now));
    app.command.set(Some((note.id.clone(), NoteCommand::Focus)));
    let beeped = beep();

    let title = note.meta.with_untracked(|m| m.title.clone());
    let text = note.plain_text();
    let body = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default().to_string();
    let notification = Notification::new(note.id.to_string(), title, body).silent(beeped);
    app.owner.run(|| {
        let _ = spawn(async move {
            let mut permission = notification_permission().await;
            if permission == NotificationPermission::Default && !ASKED.replace(true) {
                permission = request_notification_permission().await;
            }
            if permission != NotificationPermission::Granted {
                eprintln!("alarm: notifications not allowed ({})", permission.as_str());
                return;
            }
            let shown = show_notification(notification).await;
            eprintln!("alarm: notification shown: {shown}");
        });
    });
}

/// The alarm after this one fired at `now`, or `None` when it doesn't repeat.
fn next_alarm(alarm: Alarm, now: f64) -> Option<Alarm> {
    let at = if alarm.repeat == Repeat::Hourly {
        // By the clock, not the calendar: an hour is an hour.
        let hour = 3_600_000.0;
        let steps = ((now - alarm.at) / hour).floor().max(0.0) + 1.0;
        alarm.at + steps * hour
    } else {
        from_local(next_occurrence(to_local(alarm.at), alarm.repeat, to_local(now))?)
    };
    Some(Alarm { at, repeat: alarm.repeat })
}

/// The alarm's sound: the system's "asterisk" sound. Whether it played.
#[cfg(target_os = "windows")]
fn beep() -> bool {
    use nsh_platform_windows::windows::Win32::{System::Diagnostics::Debug::MessageBeep, UI::WindowsAndMessaging::MB_ICONASTERISK};
    // SAFETY: plays a system sound; no pointers involved.
    unsafe { MessageBeep(MB_ICONASTERISK) }.is_ok()
}

/// Elsewhere the notification's own sound plays.
#[cfg(not(target_os = "windows"))]
fn beep() -> bool {
    false
}

/// A moment as the user reads and types it: `yyyy-mm-dd hh:mm`, local time.
pub fn format_at(ms: f64) -> String {
    to_local(ms).format(FORMAT).to_string()
}

/// A past moment in words: "Today 14:32", "Yesterday 09:10",
/// "Sat 14 Nov 14:32", with the year when it isn't this one.
pub fn describe_past(ms: f64, now: f64) -> String {
    describe_past_local(to_local(ms), to_local(now))
}

fn describe_past_local(at: NaiveDateTime, now: NaiveDateTime) -> String {
    let days = (now.date() - at.date()).num_days();
    let time = at.format("%H:%M");
    match days {
        0 => format!("Today {time}"),
        1 => format!("Yesterday {time}"),
        _ if at.date().year() == now.date().year() => format!("{} {time}", at.format("%a %-d %b")),
        _ => format!("{} {time}", at.format("%-d %b %Y")),
    }
}

/// The moment typed as `yyyy-mm-dd hh:mm` (local time), in milliseconds
/// since the Unix epoch.
pub fn parse_at(text: &str) -> Option<f64> {
    parse_local(text).map(from_local)
}

/// What's set on a note, in words ("Alarm tomorrow at 09:00, repeats daily",
/// "Asleep for 12 min"), or `None` when nothing is.
pub fn describe(meta: &NoteMeta, now: f64) -> Option<String> {
    let sleep = meta.sleep_until.filter(|&until| until > now).map(|until| (to_local(until), until - now));
    let alarm = meta.alarm.map(|a| (to_local(a.at), a.repeat));
    describe_local(sleep, alarm, to_local(now))
}

/// When "until tomorrow 9:00" ends, from `now` (milliseconds).
pub fn tomorrow_morning(now: f64) -> f64 {
    from_local(tomorrow_at_9(to_local(now)))
}

/// When "until Monday 9:00" ends, from `now` (milliseconds).
pub fn monday_morning(now: f64) -> f64 {
    from_local(monday_at_9(to_local(now)))
}

// ----- Local time ----------------------------------------------------------------

/// `ms` as this machine's local wall-clock time.
fn to_local(ms: f64) -> NaiveDateTime {
    DateTime::from_timestamp_millis(ms as i64).unwrap_or_default().with_timezone(&Local).naive_local()
}

/// A local wall-clock time in milliseconds. A time the clocks skip (moving
/// forward for summer time) counts as an hour later.
fn from_local(time: NaiveDateTime) -> f64 {
    let moment = Local
        .from_local_datetime(&time)
        .earliest()
        .or_else(|| Local.from_local_datetime(&(time + Duration::hours(1))).earliest())
        .map(|t| t.timestamp_millis())
        .unwrap_or_else(|| time.and_utc().timestamp_millis());
    moment as f64
}

fn parse_local(text: &str) -> Option<NaiveDateTime> {
    let text = text.trim();
    NaiveDateTime::parse_from_str(text, FORMAT)
        .ok()
        .or_else(|| NaiveDate::parse_from_str(text, "%Y-%m-%d").ok().map(|d| d.and_time(nine())))
}

fn nine() -> NaiveTime {
    NaiveTime::from_hms_opt(9, 0, 0).unwrap_or_default()
}

// ----- Calendar arithmetic ---------------------------------------------------------

/// Tomorrow at 09:00.
fn tomorrow_at_9(now: NaiveDateTime) -> NaiveDateTime {
    (now.date() + Duration::days(1)).and_time(nine())
}

/// The next Monday 09:00 after `now` (today's, on a Monday morning).
fn monday_at_9(now: NaiveDateTime) -> NaiveDateTime {
    let mut day = now.date();
    loop {
        let candidate = day.and_time(nine());
        if day.weekday() == Weekday::Mon && candidate > now {
            return candidate;
        }
        day += Duration::days(1);
    }
}

/// The first time a `repeat` alarm set for `at` comes after `now` (and
/// after `at`), in one go however long it was missed; `None` when it
/// doesn't repeat.
fn next_occurrence(at: NaiveDateTime, repeat: Repeat, now: NaiveDateTime) -> Option<NaiveDateTime> {
    let every = |period: Duration| {
        let steps = ((now - at).num_seconds().div_euclid(period.num_seconds()) + 1).max(1);
        at + period * steps as i32
    };
    match repeat {
        Repeat::None => None,
        Repeat::Hourly => Some(every(Duration::hours(1))),
        Repeat::Daily => Some(every(Duration::days(1))),
        Repeat::Weekly => Some(every(Duration::weeks(1))),
        Repeat::Weekdays => {
            let mut next = every(Duration::days(1));
            while matches!(next.weekday(), Weekday::Sat | Weekday::Sun) {
                next += Duration::days(1);
            }
            Some(next)
        }
        Repeat::Monthly => Some(every_months(at, 1, now)),
        Repeat::Yearly => Some(every_months(at, 12, now)),
    }
}

/// The first of `at` plus a multiple of `months` months that's after `now`
/// and after `at`. The day stays the same, clamped to a shorter month.
fn every_months(at: NaiveDateTime, months: i32, now: NaiveDateTime) -> NaiveDateTime {
    let behind = (now.year() - at.year()) * 12 + now.month() as i32 - at.month() as i32;
    // One step before the estimate, then forward: the clamped day can land
    // either side of `now` within its month.
    let mut steps = (behind.div_euclid(months) - 1).max(1);
    loop {
        let next = add_months(at, steps * months);
        if next > now {
            return next;
        }
        steps += 1;
    }
}

fn add_months(at: NaiveDateTime, months: i32) -> NaiveDateTime {
    let index = at.year() * 12 + at.month0() as i32 + months;
    let (year, month) = (index.div_euclid(12), index.rem_euclid(12) as u32 + 1);
    let day = at.day().min(days_in_month(year, month));
    NaiveDate::from_ymd_opt(year, month, day).map(|d| d.and_time(at.time())).unwrap_or(at)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let next = if month == 12 { NaiveDate::from_ymd_opt(year + 1, 1, 1) } else { NaiveDate::from_ymd_opt(year, month + 1, 1) };
    next.and_then(|d| d.pred_opt()).map_or(28, |d| d.day())
}

// ----- In words ------------------------------------------------------------------

/// `describe` on wall-clock times: `sleep` is when it ends and the
/// milliseconds left, `alarm` when it rings and how it repeats.
fn describe_local(sleep: Option<(NaiveDateTime, f64)>, alarm: Option<(NaiveDateTime, Repeat)>, now: NaiveDateTime) -> Option<String> {
    let mut parts = Vec::new();
    if let Some((until, left)) = sleep {
        let minutes = (left / 60_000.0).ceil().max(1.0);
        parts.push(if minutes < 60.0 { format!("Asleep for {minutes} min") } else { format!("Asleep until {}", sleep_end(until, now)) });
    }
    if let Some((at, repeat)) = alarm {
        let days = (at.date() - now.date()).num_days();
        let time = at.format("%H:%M");
        let when = match days {
            0 => format!("today at {time}"),
            1 => format!("tomorrow at {time}"),
            _ => format!("on {} at {time}", at.format("%Y-%m-%d")),
        };
        let repeats = match repeat {
            Repeat::None => "",
            Repeat::Hourly => ", repeats hourly",
            Repeat::Daily => ", repeats daily",
            Repeat::Weekdays => ", repeats on weekdays",
            Repeat::Weekly => ", repeats weekly",
            Repeat::Monthly => ", repeats monthly",
            Repeat::Yearly => ", repeats yearly",
        };
        parts.push(format!("Alarm {when}{repeats}"));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// When a long sleep ends: "21:48", "tomorrow 09:00", "Monday 09:00" within
/// the week, else the date.
fn sleep_end(until: NaiveDateTime, now: NaiveDateTime) -> String {
    let time = until.format("%H:%M");
    match (until.date() - now.date()).num_days() {
        0 => time.to_string(),
        1 => format!("tomorrow {time}"),
        2..=6 => format!("{} {time}", until.format("%A")),
        _ => until.format(FORMAT).to_string(),
    }
}

impl App {
    /// Hides the note for `ms` milliseconds.
    pub fn sleep_for(self, note: &Note, ms: f64) {
        let until = crate::store::now_ms() + ms;
        self.update(note, |m| m.sleep_until = Some(until));
    }

    pub fn wake(self, note: &Note) {
        self.update(note, |m| m.sleep_until = None);
    }

    /// Sets the note's alarm, or clears it.
    pub fn set_alarm(self, note: &Note, alarm: Option<Alarm>) {
        self.update(note, |m| m.alarm = alarm);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, FORMAT).unwrap()
    }

    #[test]
    fn reads_what_the_user_types() {
        assert_eq!(parse_local("2026-11-20 08:05"), Some(t("2026-11-20 08:05")));
        assert_eq!(parse_local(" 2026-11-20 8:05 "), Some(t("2026-11-20 08:05")));
        assert_eq!(parse_local("2026-11-20"), Some(t("2026-11-20 09:00")));
        assert_eq!(parse_local("2026-11-20 25:00"), None);
        assert_eq!(parse_local("tomorrow"), None);
    }

    #[test]
    fn formats_and_parses_back() {
        let ms = parse_at("2026-03-07 21:48").unwrap();
        assert_eq!(format_at(ms), "2026-03-07 21:48");
    }

    #[test]
    fn mornings() {
        // 2026-11-14 is a Saturday.
        assert_eq!(tomorrow_at_9(t("2026-11-14 23:30")), t("2026-11-15 09:00"));
        assert_eq!(tomorrow_at_9(t("2026-12-31 08:00")), t("2027-01-01 09:00"));
        assert_eq!(monday_at_9(t("2026-11-14 10:00")), t("2026-11-16 09:00"));
        assert_eq!(monday_at_9(t("2026-11-16 08:00")), t("2026-11-16 09:00"));
        assert_eq!(monday_at_9(t("2026-11-16 09:00")), t("2026-11-23 09:00"));
    }

    #[test]
    fn simple_repeats() {
        let at = t("2026-11-14 09:00");
        assert_eq!(next_occurrence(at, Repeat::None, at), None);
        assert_eq!(next_occurrence(at, Repeat::Hourly, at), Some(t("2026-11-14 10:00")));
        assert_eq!(next_occurrence(at, Repeat::Daily, at), Some(t("2026-11-15 09:00")));
        assert_eq!(next_occurrence(at, Repeat::Weekly, at), Some(t("2026-11-21 09:00")));
        assert_eq!(next_occurrence(at, Repeat::Monthly, at), Some(t("2026-12-14 09:00")));
        assert_eq!(next_occurrence(at, Repeat::Yearly, at), Some(t("2027-11-14 09:00")));
    }

    #[test]
    fn a_repeat_missed_for_days_jumps_past_now_at_once() {
        let at = t("2026-11-01 09:00");
        let now = t("2026-11-14 10:30");
        assert_eq!(next_occurrence(at, Repeat::Daily, now), Some(t("2026-11-15 09:00")));
        assert_eq!(next_occurrence(at, Repeat::Daily, t("2026-11-14 08:59")), Some(t("2026-11-14 09:00")));
        assert_eq!(next_occurrence(at, Repeat::Hourly, now), Some(t("2026-11-14 11:00")));
        assert_eq!(next_occurrence(at, Repeat::Weekly, now), Some(t("2026-11-15 09:00")));
        assert_eq!(next_occurrence(t("2024-11-20 08:00"), Repeat::Monthly, now), Some(t("2026-11-20 08:00")));
        assert_eq!(next_occurrence(t("2020-02-29 08:00"), Repeat::Yearly, now), Some(t("2027-02-28 08:00")));
    }

    #[test]
    fn weekdays_skip_the_weekend() {
        // Friday 2026-11-13 fired; Saturday and Sunday are skipped.
        assert_eq!(next_occurrence(t("2026-11-13 09:00"), Repeat::Weekdays, t("2026-11-13 09:00")), Some(t("2026-11-16 09:00")));
        // Missed since Wednesday, now Saturday: Monday.
        assert_eq!(next_occurrence(t("2026-11-11 07:00"), Repeat::Weekdays, t("2026-11-14 12:00")), Some(t("2026-11-16 07:00")));
        // Tuesday to Wednesday.
        assert_eq!(next_occurrence(t("2026-11-17 07:00"), Repeat::Weekdays, t("2026-11-17 07:00")), Some(t("2026-11-18 07:00")));
    }

    #[test]
    fn month_ends_clamp() {
        let at = t("2026-01-31 08:00");
        assert_eq!(next_occurrence(at, Repeat::Monthly, at), Some(t("2026-02-28 08:00")));
        assert_eq!(next_occurrence(t("2028-01-31 08:00"), Repeat::Monthly, t("2028-01-31 08:00")), Some(t("2028-02-29 08:00")));
        // From the 31st straight to a month that has one, past the short ones.
        assert_eq!(next_occurrence(at, Repeat::Monthly, t("2026-04-15 00:00")), Some(t("2026-04-30 08:00")));
        assert_eq!(next_occurrence(at, Repeat::Monthly, t("2026-04-30 09:00")), Some(t("2026-05-31 08:00")));
        assert_eq!(next_occurrence(t("2026-12-15 08:00"), Repeat::Monthly, t("2026-12-15 08:00")), Some(t("2027-01-15 08:00")));
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2026, 12), 31);
    }

    #[test]
    fn hourly_by_the_clock() {
        let alarm = Alarm { at: 1_000_000.0, repeat: Repeat::Hourly };
        let next = next_alarm(alarm, 1_000_000.0 + 3.5 * 3_600_000.0).unwrap();
        assert_eq!(next.at, 1_000_000.0 + 4.0 * 3_600_000.0);
        assert_eq!(next_alarm(Alarm { at: 5.0, repeat: Repeat::None }, 10.0), None);
    }

    #[test]
    fn in_words() {
        let now = t("2026-11-14 20:00");
        assert_eq!(describe_local(None, None, now), None);
        assert_eq!(describe_local(None, Some((t("2026-11-14 21:48"), Repeat::None)), now).as_deref(), Some("Alarm today at 21:48"));
        assert_eq!(
            describe_local(None, Some((t("2026-11-15 09:00"), Repeat::Daily)), now).as_deref(),
            Some("Alarm tomorrow at 09:00, repeats daily")
        );
        assert_eq!(describe_local(None, Some((t("2026-11-20 08:00"), Repeat::None)), now).as_deref(), Some("Alarm on 2026-11-20 at 08:00"));
        assert_eq!(describe_local(Some((t("2026-11-14 20:12"), 11.5 * 60_000.0)), None, now).as_deref(), Some("Asleep for 12 min"));
        assert_eq!(
            describe_local(Some((t("2026-11-15 09:00"), 13.0 * 3_600_000.0)), Some((t("2026-11-16 09:00"), Repeat::Weekdays)), now).as_deref(),
            Some("Asleep until tomorrow 09:00 · Alarm on 2026-11-16 at 09:00, repeats on weekdays")
        );
        assert_eq!(describe_local(Some((t("2026-11-16 09:00"), 37.0 * 3_600_000.0)), None, now).as_deref(), Some("Asleep until Monday 09:00"));
    }
}
