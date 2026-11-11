//! Nyx Bar clock: the date and time, the next event beside them, and a calendar popover.
//! The wide layout shows the month with a dot per calendar, the selected day's agenda
//! (a line at the current time, Join for meetings, Open in Calendar), the next days and
//! the reminders due today, which tick off in place; the compact one stacks the month,
//! the day and its reminders in one column.
//!
//! Events come from Nyx Calendar's agenda (`agenda.sqlite`, which `nyx-calendard`
//! writes) and reminders from Nyx Reminders' `tasks.sqlite`; both are published read-only
//! files. Ticking a reminder runs `nyx-remindersd --complete`.

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone};
use nyx_plugin_sdk::protocol::{EventKind, MonitorInfo};
use nyx_plugin_sdk::view::*;
use nyx_plugin_sdk::{Input, Plugin, spawn};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

const ITEM: &str = "time";

#[derive(Debug, Clone, PartialEq)]
struct Event {
    calendar: String,
    color: Option<String>,
    title: String,
    location: String,
    description: String,
    url: Option<String>,
    starts: i64,
    ends: i64,
    all_day: bool,
}

impl Event {
    fn holiday(&self) -> bool {
        self.calendar.to_lowercase().contains("holiday")
    }

    fn paint(&self) -> String {
        self.color.clone().unwrap_or_else(|| "accent".into())
    }

    /// A meeting link in the event's URL, location or notes.
    fn meeting(&self) -> Option<String> {
        const HOSTS: [&str; 7] = ["meet.google.com", "zoom.us", "teams.microsoft.com", "teams.live.com", "whereby.com", "meet.jit.si", "webex.com"];
        let fields = [self.url.as_deref().unwrap_or(""), &self.location, &self.description];
        for f in fields {
            for word in f.split(|c: char| c.is_whitespace() || matches!(c, '"' | '<' | '>' | '(' | ')')) {
                if word.starts_with("https://") && HOSTS.iter().any(|h| word.contains(h)) {
                    return Some(word.trim_end_matches(['.', ',', ';']).to_string());
                }
            }
        }
        None
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Task {
    list: String,
    list_name: String,
    uid: String,
    title: String,
    due_day: Option<NaiveDate>,
    due_at: Option<i64>,
    overdue: bool,
}

fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"))
}

fn agenda_path() -> PathBuf {
    data_home().join("nyx-calendar/agenda.sqlite")
}

fn tasks_path() -> PathBuf {
    data_home().join("nyx-reminders/tasks.sqlite")
}

fn open_ro(path: &Path) -> Option<rusqlite::Connection> {
    if !path.exists() {
        return None;
    }
    rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()
}

/// Events overlapping `from..to`, each once (holidays come from several calendars).
fn read_events(path: &Path, from: i64, to: i64) -> Vec<Event> {
    let Some(db) = open_ro(path) else { return vec![] };
    let Ok(mut stmt) = db.prepare(
        "SELECT calendar_name, color, title, location, description, url, starts, ends, all_day FROM events \
         WHERE ends > ?1 AND starts < ?2 ORDER BY all_day DESC, starts, title",
    ) else {
        return vec![];
    };
    let rows = stmt.query_map([from, to], |r| {
        Ok(Event {
            calendar: r.get(0)?,
            color: r.get::<_, Option<String>>(1)?.filter(|c| c.len() == 7 && c.starts_with('#')),
            title: r.get(2)?,
            location: r.get(3)?,
            description: r.get(4)?,
            url: r.get(5)?,
            starts: r.get(6)?,
            ends: r.get(7)?,
            all_day: r.get::<_, i64>(8)? != 0,
        })
    });
    let mut out: Vec<Event> = vec![];
    for e in rows.into_iter().flatten().flatten() {
        if !out.iter().any(|p| p.title == e.title && p.starts == e.starts) {
            out.push(e);
        }
    }
    out
}

/// Open tasks due by `day` (overdue ones included).
fn read_tasks(path: &Path, day: NaiveDate) -> Vec<Task> {
    let Some(db) = open_ro(path) else { return vec![] };
    let Ok(mut stmt) = db.prepare(
        "SELECT list, list_name, uid, title, due_day, due_at, overdue FROM tasks \
         WHERE parent IS NULL AND due_day IS NOT NULL AND due_day <= ?1 ORDER BY overdue DESC, due_day, due_at IS NULL, due_at, title",
    ) else {
        return vec![];
    };
    let rows = stmt.query_map([day.format("%Y-%m-%d").to_string()], |r| {
        Ok(Task {
            list: r.get(0)?,
            list_name: r.get(1)?,
            uid: r.get(2)?,
            title: r.get(3)?,
            due_day: r.get::<_, Option<String>>(4)?.and_then(|d| NaiveDate::parse_from_str(&d, "%Y-%m-%d").ok()),
            due_at: r.get(5)?,
            overdue: r.get::<_, i64>(6)? != 0,
        })
    });
    rows.into_iter().flatten().flatten().collect()
}

#[derive(Debug, Clone)]
struct Settings {
    /// The bar's date (dim, may be empty) and time (bold), as chrono formats.
    date: String,
    time: String,
    /// The bar's time has seconds, so it ticks every second.
    seconds: bool,
    /// Times everywhere else: the tooltip, agenda, upcoming days and reminders.
    twelve: bool,
    wide: bool,
    sunday_first: bool,
    next_event: bool,
    calendars: Vec<String>,
    holidays: bool,
    reminder_lists: Option<Vec<String>>,
}

impl Settings {
    fn from(v: &serde_json::Value) -> Settings {
        let s = |k: &str, d: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or(d).to_string();
        let b = |k: &str, d: bool| v.get(k).and_then(|x| x.as_bool()).unwrap_or(d);
        let list = |k: &str| -> Option<Vec<String>> {
            v.get(k).and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|i| i.as_str().map(String::from)).collect())
        };
        let twelve = match v.get("hours").map(|h| h.as_str().map(String::from).unwrap_or_else(|| h.to_string())).as_deref() {
            Some("12") => true,
            Some("24") => false,
            _ => twelve_hour(),
        };
        let seconds = b("seconds", false);
        let time = match (twelve, seconds) {
            (true, true) => "%-I:%M:%S %p",
            (true, false) => "%-I:%M %p",
            (false, true) => "%H:%M:%S",
            (false, false) => "%H:%M",
        };
        let date = match s("date", "short").as_str() {
            "long" => "%A, %B %-d",
            "numeric" => numeric_date(),
            "iso" => "%Y-%m-%d",
            "none" => "",
            _ => "%a %b %-d",
        };
        // A format of your own: a double space splits the date from the time.
        let (date, time) = match Some(s("format", "")).filter(|f| valid_format(f)) {
            Some(f) => match f.rsplit_once("  ") {
                Some((d, t)) => (d.to_string(), t.to_string()),
                None => (String::new(), f),
            },
            None => (date.to_string(), time.to_string()),
        };
        let seconds = ["%S", "%-S", "%T", "%X", "%r", "%s"].iter().any(|p| time.contains(p));
        Settings {
            date,
            time,
            seconds,
            twelve,
            wide: s("layout", "wide") != "compact",
            sunday_first: s("week_start", "monday") == "sunday",
            next_event: b("next_event", true),
            calendars: list("calendars").unwrap_or_default(),
            holidays: b("holidays", true),
            reminder_lists: list("reminder_lists"),
        }
    }

    /// A time of day outside the bar's clock.
    fn hm(&self, t: DateTime<Local>) -> String {
        fmt(t, if self.twelve { "%-I:%M %p" } else { "%H:%M" })
    }

    fn shows(&self, e: &Event) -> bool {
        if e.holiday() {
            return self.holidays;
        }
        self.calendars.is_empty() || self.calendars.iter().any(|c| c.eq_ignore_ascii_case(&e.calendar))
    }

    fn shows_task(&self, t: &Task) -> bool {
        self.reminder_lists.as_ref().is_none_or(|l| l.is_empty() || l.iter().any(|n| n.eq_ignore_ascii_case(&t.list_name)))
    }
}

enum Tick {
    Second,
    Minute,
    Files,
}

/// The locale's region ("US" from `en_US.UTF-8`).
fn region() -> String {
    let locale =
        ["LC_ALL", "LC_TIME", "LANG"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty())).unwrap_or_default();
    locale.split(['.', '@']).next().unwrap_or("").rsplit('_').next().unwrap_or("").to_string()
}

/// A 12-hour clock where the locale uses one, as Nyx Calendar does.
fn twelve_hour() -> bool {
    ["US", "CA", "AU", "NZ", "IN", "PH", "PK", "EG", "SA"].contains(&region().as_str())
}

/// The day and month in numbers, in the locale's order.
fn numeric_date() -> &'static str {
    let r = region();
    if ["US", "PH", "BZ", "FM", "MH", "PW"].contains(&r.as_str()) {
        "%-m/%-d"
    } else if ["DE", "AT", "CH", "PL", "RU", "UA", "CZ", "SK", "FI", "NO", "DK", "TR"].contains(&r.as_str()) {
        "%-d.%-m."
    } else {
        "%-d/%-m"
    }
}

/// A strftime format chrono can use; a bad one falls back to the built-in styles.
fn valid_format(f: &str) -> bool {
    !f.trim().is_empty() && !chrono::format::StrftimeItems::new(f).any(|i| matches!(i, chrono::format::Item::Error))
}

/// `t` formatted, or empty if the format can't be shown (it never panics).
fn fmt(t: DateTime<Local>, f: &str) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    if write!(out, "{}", t.format(f)).is_err() {
        out.clear();
    }
    out
}

struct State {
    settings: Settings,
    /// The first day of the month shown.
    month: NaiveDate,
    selected: NaiveDate,
    events: Vec<Event>,
    tasks: Vec<Task>,
    /// Ticked off this session, until `tasks.sqlite` drops them.
    done: HashSet<String>,
    /// The reminders the popover shows, in order: `done:<n>` indexes them.
    shown_tasks: Vec<Task>,
    monitors: Vec<MonitorInfo>,
}

/// Below this width (logical pixels) the next event's chip shows only the time left.
const NARROW: i32 = 1440;

fn local(ts: i64) -> DateTime<Local> {
    Local.timestamp_opt(ts, 0).single().unwrap_or_else(Local::now)
}

fn midnight(day: NaiveDate) -> i64 {
    Local.from_local_datetime(&day.and_hms_opt(0, 0, 0).expect("midnight")).earliest().map_or(0, |d| d.timestamp())
}

fn first_of(day: NaiveDate) -> NaiveDate {
    day.with_day(1).expect("day one")
}

fn add_months(first: NaiveDate, n: i32) -> NaiveDate {
    let m = first.month0() as i32 + n;
    NaiveDate::from_ymd_opt(first.year() + m.div_euclid(12), m.rem_euclid(12) as u32 + 1, 1).expect("a month")
}

/// The six weeks the month grid shows.
fn grid_start(first: NaiveDate, sunday_first: bool) -> NaiveDate {
    let back = if sunday_first { first.weekday().num_days_from_sunday() } else { first.weekday().num_days_from_monday() };
    first - Duration::days(back as i64)
}

impl State {
    fn load(&mut self) {
        let today = Local::now().date_naive();
        let start = grid_start(self.month, self.settings.sunday_first).min(today);
        let end = (grid_start(self.month, self.settings.sunday_first) + Duration::days(42)).max(today + Duration::days(9));
        let s = &self.settings;
        self.events = read_events(&agenda_path(), midnight(start), midnight(end)).into_iter().filter(|e| s.shows(e)).collect();
        self.tasks = read_tasks(&tasks_path(), today).into_iter().filter(|t| s.shows_task(t)).collect();
        self.done.retain(|k| self.tasks.iter().any(|t| task_key(t) == *k));
    }

    fn on_day(&self, day: NaiveDate) -> Vec<&Event> {
        let (a, b) = (midnight(day), midnight(day + Duration::days(1)));
        self.events.iter().filter(|e| e.starts < b && e.ends > a && (e.all_day || e.starts >= a || e.ends > a)).collect()
    }

    /// The next timed event that hasn't started yet (within the next 12 hours), or one
    /// that started in the last 5 minutes.
    fn next_event(&self, now: i64) -> Option<&Event> {
        self.events.iter().filter(|e| !e.all_day && e.starts > now - 300 && e.starts < now + 12 * 3600).min_by_key(|e| e.starts)
    }
}

fn task_key(t: &Task) -> String {
    format!("{}\t{}", t.list, t.uid)
}

fn short(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
    t.push('…');
    t
}

/// A box whose children keep their own width, centred, instead of filling it.
fn centered(mut n: Node) -> Node {
    if let NodeKind::Box { align, .. } = &mut n.kind {
        *align = Align::Center;
    }
    n
}

/// A box whose children stretch across it.
fn filled(mut n: Node) -> Node {
    if let NodeKind::Box { align, .. } = &mut n.kind {
        *align = Align::Fill;
    }
    n
}

/// A short label that keeps its words beside an expanding one (popover labels wrap).
fn keep(text: impl Into<String>, px_per_char: i32) -> Node {
    let t: String = text.into();
    let w = t.chars().count() as i32 * px_per_char + 8;
    label(t).width(w)
}

fn dot(paint: &str, size: f64) -> Node {
    let r = size / 2.0;
    vector([0.0, 0.0, size, size], vec![Draw::Circle { cx: r, cy: r, r, fill: Some(color(paint)), stroke: None, width: None }]).size(size as i32, size as i32)
}

fn bar(paint: &str, h: i32) -> Node {
    vector([0.0, 0.0, 3.0, h as f64], vec![Draw::Rect { x: 0.0, y: 0.0, w: 3.0, h: h as f64, radius: 1.5, fill: Some(color(paint)), stroke: None, width: None }]).size(3, h)
}

fn square(paint: &str) -> Node {
    vector([0.0, 0.0, 8.0, 8.0], vec![Draw::Rect { x: 0.0, y: 0.0, w: 8.0, h: 8.0, radius: 2.0, fill: Some(color(paint)), stroke: None, width: None }]).size(8, 8)
}

/// A 26 px icon button, dim until hovered.
fn ibtn(action: &str, icon: &str, tip: &str) -> Node {
    button(action, None, Some(icon)).class("ibtn").tooltip(tip)
}

/// Open in Calendar: an arrow out of a box (no icon theme has the same one), shown on
/// the row's hover.
fn open_button(day: NaiveDate) -> Node {
    let d = "M14.5 3.5h6v6M10 14 20.5 3.5M18.5 13.5v5a2 2 0 0 1-2 2h-11a2 2 0 0 1-2-2v-11a2 2 0 0 1 2-2h5";
    let glyph = vector([0.0, 0.0, 24.0, 24.0], vec![Draw::Path { d: d.into(), fill: None, stroke: Some(color("dim_text")), width: Some(1.8) }]).size(15, 15);
    hbox([glyph]).class("ibox").class("open").action(&format!("open:{day}")).tooltip("Open in Calendar")
}

/// A reminder's check: a ring (red when overdue), filled with a tick when done.
fn check(done: bool, late: bool) -> Node {
    let mut cmds = vec![Draw::Circle {
        cx: 9.0,
        cy: 9.0,
        r: 8.0,
        fill: done.then(|| color("accent")),
        stroke: Some(color(if done { "accent" } else if late { "danger" } else { "text_tertiary" })),
        width: Some(1.6),
    }];
    if done {
        cmds.push(Draw::Path { d: "M5.2 9.3 7.8 11.8 12.8 6.6".into(), fill: None, stroke: Some(color("on_accent")), width: Some(1.9) });
    }
    vector([0.0, 0.0, 18.0, 18.0], cmds).size(18, 18)
}

fn until(mins: i64) -> String {
    match mins {
        ..=0 => "now".into(),
        1..=59 => format!("{mins} min"),
        _ if mins % 60 == 0 => format!("{} h", mins / 60),
        _ => format!("{} h {} min", mins / 60, mins % 60),
    }
}

/// The bar item: the date dim, the time bold, then the next event (on a narrow monitor,
/// only how long until it).
fn bar_item(st: &State, now: DateTime<Local>, narrow: bool) -> Item {
    let date = fmt(now, &st.settings.date);
    let mut children = vec![];
    if !date.is_empty() {
        children.push(label(date).id("date").class("dim"));
    }
    children.push(label(fmt(now, &st.settings.time)).id("time").class("strong"));
    let mut tip = now.format("%A, %B %-d, %Y").to_string();
    if st.settings.next_event
        && let Some(e) = st.next_event(now.timestamp())
    {
        let mins = (e.starts - now.timestamp()).div_euclid(60).max(0);
        let soon = mins <= 5;
        let text = match (narrow, soon && mins > 0) {
            (true, _) => until(mins),
            (false, true) => format!("{} in {}", short(&e.title, 22), until(mins)),
            (false, false) => format!("{} · {}", short(&e.title, 22), until(mins)),
        };
        let mut chip = hbox([dot(&e.paint(), 6.0), label(text).class("shrink")]).spacing(5).class("chip");
        if soon {
            chip = chip.class("warning");
        }
        children.push(chip);
        tip.push_str(&format!("\nNext: {} at {}", e.title, st.settings.hm(local(e.starts))));
    }
    item(ITEM).view(hbox(children).spacing(6)).tooltip(tip).on_right_click(&["nyx-calendar"])
}

fn month_grid(st: &State, today: NaiveDate, cell: (i32, i32)) -> Node {
    let start = grid_start(st.month, st.settings.sunday_first);
    let names: Vec<&str> = if st.settings.sunday_first { vec!["S", "M", "T", "W", "T", "F", "S"] } else { vec!["M", "T", "W", "T", "F", "S", "S"] };
    // Without a width of their own the letters centre over their columns.
    let mut rows = vec![hbox(names.into_iter().map(|n| centered(vbox([label(n).class("wd")])).width(cell.0))).spacing(0)];
    for w in 0..6 {
        let mut days = vec![];
        for d in 0..7 {
            let day = start + Duration::days(w * 7 + d);
            let events = st.on_day(day);
            let mut num = label(day.day().to_string()).class("dn").class("center");
            if day.month() != st.month.month() {
                num = num.class("out");
            }
            if day == today {
                num = num.class("today");
            } else if day == st.selected {
                num = num.class("sel");
            }
            if events.iter().any(|e| e.holiday()) {
                num = num.class("hol");
            }
            if day.weekday().num_days_from_monday() >= 5 {
                num = num.class("we");
            }
            let mut colors: Vec<String> = vec![];
            for e in events.iter().filter(|e| !e.holiday()) {
                let p = e.paint();
                if colors.len() < 3 && !colors.contains(&p) {
                    colors.push(p);
                }
            }
            let dots = hbox(colors.iter().map(|c| dot(c, 5.0))).spacing(2).height(5);
            days.push(centered(vbox([num, centered(dots)])).spacing(2).size(cell.0, cell.1).class("day").action(&format!("day:{day}")).tooltip(&day_tip(&st.settings, day, &events)));
        }
        rows.push(hbox(days).spacing(0));
    }
    vbox(rows).spacing(1).class("month")
}

fn day_tip(s: &Settings, day: NaiveDate, events: &[&Event]) -> String {
    let mut t = day.format("%A, %B %-d").to_string();
    for e in events.iter().take(6) {
        if e.all_day {
            t.push_str(&format!("\n{}", e.title));
        } else {
            t.push_str(&format!("\n{} {}", s.hm(local(e.starts)), e.title));
        }
    }
    t
}

fn agenda(st: &State, day: NaiveDate, now: i64) -> Vec<Node> {
    let events = st.on_day(day);
    if events.is_empty() {
        return vec![label("Nothing on this day").class("dim").margin(6, 8, 6, 8)];
    }
    let today = day == local(now).date_naive();
    let next = events.iter().filter(|e| !e.all_day && e.ends > now).min_by_key(|e| e.starts).map(|e| (e.title.clone(), e.starts));
    let now_line = || hbox([dot("danger", 7.0), keep(st.settings.hm(local(now)), 7).class("now-t"), separator().class("now-line").expand()]).spacing(6).margin(1, 8, 1, 8);
    let time_w = if st.settings.twelve { 58 } else { 40 };
    let mut rows = vec![];
    let mut now_shown = !today;
    for e in events {
        if !now_shown && !e.all_day && e.starts > now {
            rows.push(now_line());
            now_shown = true;
        }
        let time = if e.all_day {
            vbox([label("All day").class("tm-s")]).width(time_w)
        } else {
            vbox([label(st.settings.hm(local(e.starts))).class("tm-s"), label(st.settings.hm(local(e.ends))).class("tm-e")]).spacing(0).width(time_w)
        };
        let sub = if e.location.is_empty() || e.location.starts_with("https://") { e.calendar.clone() } else { format!("{} · {}", e.calendar, e.location) };
        let mut row = vec![time, bar(&e.paint(), 32), vbox([label(&e.title).class("et"), label(sub).class("es")]).spacing(1).expand()];
        let is_next = next.as_ref().is_some_and(|(t, s)| *t == e.title && *s == e.starts);
        if let Some(link) = e.meeting().filter(|_| e.ends > now) {
            if is_next && e.starts > now && e.starts - now <= 3600 {
                row.push(keep(format!("in {}", until((e.starts - now) / 60)), 7).class("soon"));
            }
            let primary = is_next && today;
            let ink = if primary { "on_accent" } else { "text" };
            let camera = vector(
                [0.0, 0.0, 24.0, 24.0],
                vec![
                    Draw::Rect { x: 2.5, y: 6.5, w: 13.0, h: 11.0, radius: 2.5, fill: None, stroke: Some(color(ink)), width: Some(1.8) },
                    Draw::Path { d: "m15.5 10.5 6-3.5v10l-6-3.5".into(), fill: None, stroke: Some(color(ink)), width: Some(1.8) },
                ],
            )
            .size(14, 14);
            let join = hbox([camera, label("Join")]).spacing(6).class("join").action(&format!("join:{link}")).tooltip("Join the call");
            row.push(if primary { join.class("primary") } else { join });
        } else {
            row.push(open_button(local(e.starts).date_naive()));
        }
        let mut r = hbox(row).spacing(10).class("ev");
        if e.ends <= now {
            r = r.class("past");
        } else if is_next && today {
            r = r.class("next");
        }
        rows.push(r);
    }
    if !now_shown && today {
        rows.push(now_line());
    }
    vec![vbox(rows).spacing(2)]
}

fn due_text(s: &Settings, t: &Task, today: NaiveDate) -> (String, bool) {
    if t.overdue || t.due_day.is_some_and(|d| d < today) {
        let when = match t.due_day {
            Some(d) if d == today - Duration::days(1) => "yesterday".to_string(),
            Some(d) => d.format("%b %-d").to_string(),
            None => String::new(),
        };
        return (if when.is_empty() { "Overdue".into() } else { format!("Overdue · {when}") }, true);
    }
    match t.due_at {
        Some(at) => (format!("Today, {}", s.hm(local(at))), false),
        None => ("Today".into(), false),
    }
}

fn reminders(st: &mut State, today: NaiveDate, max: usize) -> (Vec<Node>, usize) {
    st.shown_tasks = st.tasks.iter().take(max).cloned().collect();
    let more = st.tasks.len().saturating_sub(max);
    let mut rows = vec![];
    for (i, t) in st.shown_tasks.iter().enumerate() {
        let done = st.done.contains(&task_key(t));
        let (due, late) = due_text(&st.settings, t, today);
        let tick = hbox([check(done, late)]).class("chk").action(&format!("done:{i}")).tooltip(if done { "Done" } else { "Mark as done" });
        let title = if done { markup(format!("<s>{}</s>", escape(&t.title))).class("tertiary").class("rt") } else { label(&t.title).class("rt") };
        let mut due = label(if done { "Done".to_string() } else { due }).class("rs");
        due = due.class(if late && !done { "danger" } else { "dim" });
        let list = short(&t.list_name, 14);
        let mut row = hbox([tick, vbox([title, due]).spacing(1).expand(), keep(list, 6).class("list")]).spacing(10).class("rem");
        if late && !done {
            row = row.class("late");
        }
        rows.push(row);
    }
    if st.tasks.is_empty() {
        rows.push(label("Nothing due today").class("dim").margin(6, 8, 6, 8));
    }
    (vec![vbox(rows).spacing(2)], more)
}

fn upcoming(st: &State, today: NaiveDate) -> Vec<Node> {
    let from = midnight(today + Duration::days(1));
    let mut list: Vec<&Event> = st.events.iter().filter(|e| e.starts >= from && e.starts < from + 7 * 86400).collect();
    list.sort_by_key(|e| (e.starts, !e.all_day));
    list.into_iter()
        .take(4)
        .map(|e| {
            let day = local(e.starts).format("%a %-d").to_string();
            let when = if e.all_day { "All day".to_string() } else { st.settings.hm(local(e.starts)) };
            hbox([label(day).class("ud").width(44), bar(&e.paint(), 16), label(short(&e.title, 28)).class("ut").expand(), keep(when, 7).class("tertiary").class("ut")])
                .spacing(9)
                .class("up")
                .action(&format!("open:{}", local(e.starts).date_naive()))
        })
        .collect()
}

/// The calendars shown, in the order they first come, holidays last.
fn legend(st: &State) -> Node {
    let mut seen: Vec<(String, String)> = vec![];
    let mut holidays = None;
    for e in &st.events {
        if e.holiday() {
            holidays.get_or_insert_with(|| e.paint());
        } else if !seen.iter().any(|(n, _)| *n == e.calendar) {
            seen.push((e.calendar.clone(), e.paint()));
        }
    }
    seen.truncate(if holidays.is_some() { 3 } else { 4 });
    seen.extend(holidays.map(|c| ("Holidays".to_string(), c)));
    hbox(seen.into_iter().map(|(n, c)| hbox([square(&c), label(short(&n, 14)).class("leg")]).spacing(6))).spacing(14).margin(2, 4, 2, 4)
}

fn section(title: &str, extra: &str, add: Option<&str>) -> Node {
    let mut c = vec![keep(title.to_uppercase(), 8).class("caps"), label("").expand()];
    if !extra.is_empty() {
        c.push(keep(extra, 7).class("tertiary").class("xs"));
    }
    if let Some(a) = add {
        c.push(ibtn(a, "list-add-symbolic", if a == "new-event" { "New event in Calendar" } else { "New reminder" }));
    }
    hbox(c).spacing(8).class("sh")
}

fn head(st: &State, today: NaiveDate) -> Node {
    let day = st.selected;
    let (title, sub) = if day == today {
        (day.format("%A").to_string(), format!("{} · Week {}", day.format("%B %-d, %Y"), day.iso_week().week()))
    } else {
        (day.format("%A %-d").to_string(), format!("{} · today is {}", day.format("%B %Y"), today.format("%A %-d")))
    };
    let mut c = vec![
        vbox([label(title).class("cal-day"), label(sub).class("cal-date")]).spacing(1).expand(),
        button("today", Some("Today"), None).class("today-btn").tooltip("Back to today"),
        ibtn("prev", "pan-start-symbolic", "Previous month"),
        ibtn("next", "pan-end-symbolic", "Next month"),
    ];
    if st.settings.wide {
        c.push(label("").width(1).class("vsep"));
        c.push(ibtn(&format!("open:{day}"), "x-office-calendar-symbolic", "Open Calendar").class("cal-app"));
        c.push(ibtn("open-reminders", "object-select-symbolic", "Open Reminders"));
    }
    hbox(c).spacing(6).margin(2, 2, 0, 4)
}

fn popover(st: &mut State, now: DateTime<Local>) -> Node {
    let today = now.date_naive();
    let ts = now.timestamp();
    let day_name = if st.selected == today { "Today".to_string() } else { st.selected.format("%A %-d").to_string() };
    let count = st.on_day(st.selected).len();
    let events_note = match count {
        0 => String::new(),
        1 => "1 event".into(),
        n => format!("{n} events"),
    };
    let month_title = markup(format!("<b>{}</b> <span alpha=\"60%\">{}</span>", st.month.format("%B"), st.month.format("%Y"))).class("mtitle");
    if st.settings.wide {
        let (rem_rows, more) = reminders(st, today, 5);
        let left_note = if st.tasks.is_empty() { String::new() } else { format!("{} left", st.tasks.iter().filter(|t| !st.done.contains(&task_key(t))).count()) };
        let mut right = vec![section(&day_name, &events_note, Some("new-event"))];
        right.extend(agenda(st, st.selected, ts));
        right.push(section("Reminders", &left_note, Some("new-reminder")));
        right.extend(rem_rows);
        if more > 0 {
            right.push(button("open-reminders", Some(&format!("{more} more in Reminders")), None).class("flat").class("link"));
        }
        let mut left = vec![month_title, month_grid(st, today, (43, 38)), legend(st)];
        let up = upcoming(st, today);
        if !up.is_empty() {
            let mut list = vec![section("Upcoming", "", None)];
            list.extend(up);
            left.push(vbox(list).spacing(1));
        }
        // Filled: the card runs as tall as the month column, and the column as the card.
        let cols = filled(hbox([vbox(left).spacing(6).width(304), vbox(right).spacing(6).expand().width(440).class("card").class("cal-right")]).spacing(20));
        vbox([head(st, today), cols]).spacing(10).class("calendar")
    } else {
        let (rem_rows, more) = reminders(st, today, 2);
        let mut col = vec![head(st, today), month_grid(st, today, (60, 34)), section(&day_name, &events_note, None)];
        col.extend(agenda(st, st.selected, ts));
        col.push(section("Reminders due today", "", None));
        col.extend(rem_rows);
        let more_text = if more > 0 { format!("{more} more in Reminders") } else { "Open Reminders".into() };
        col.push(footer(&format!("open:{}", st.selected), "Open Calendar", Some(button("open-reminders", Some(&more_text), None).class("flat").class("link"))));
        vbox(col).spacing(4).class("calendar")
    }
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn redraw(plugin: &Plugin<Tick>, st: &mut State) {
    let now = Local::now();
    for m in &st.monitors {
        plugin.item(bar_item(st, now, m.width < NARROW).monitor(&m.name));
    }
    let view = popover(st, now);
    plugin.popover(ITEM, Some(view));
}

fn main() {
    let (plugin, inputs) = Plugin::<Tick>::start();
    let settings = Settings::from(plugin.settings());
    let today = Local::now().date_naive();
    let mut st = State {
        settings,
        month: first_of(today),
        selected: today,
        events: vec![],
        tasks: vec![],
        done: HashSet::new(),
        shown_tasks: vec![],
        monitors: plugin.hello.monitors.clone(),
    };
    st.load();
    redraw(&plugin, &mut st);

    // Each second (when the bar shows them), the minute turning, and either file
    // changing (checked every other second).
    let send = plugin.data();
    let seconds = Arc::new(AtomicBool::new(st.settings.seconds));
    let ticking = seconds.clone();
    std::thread::spawn(move || {
        let mut seen = (mtime(&agenda_path()), mtime(&tasks_path()));
        let mut minute = Local::now().timestamp() / 60;
        let mut odd = false;
        loop {
            let ms = Local::now().timestamp_subsec_millis().min(999);
            std::thread::sleep(std::time::Duration::from_millis(1000 - ms as u64 + 5));
            odd = !odd;
            if odd {
                let now = (mtime(&agenda_path()), mtime(&tasks_path()));
                if now != seen {
                    seen = now;
                    send(Tick::Files);
                }
            }
            let m = Local::now().timestamp() / 60;
            if m != minute {
                minute = m;
                send(Tick::Minute);
            } else if ticking.load(Ordering::Relaxed) {
                send(Tick::Second);
            }
        }
    });

    for input in inputs {
        match input {
            Input::Data(Tick::Minute) => {
                let today = Local::now().date_naive();
                if st.selected < today && st.month <= first_of(today) {
                    // A new day: follow it unless the user is looking elsewhere.
                    st.selected = today;
                    st.month = first_of(today);
                    st.load();
                }
                redraw(&plugin, &mut st);
            }
            Input::Data(Tick::Second) => {
                plugin.patch(ITEM, "time", [("text", fmt(Local::now(), &st.settings.time).into())]);
            }
            Input::Data(Tick::Files) => {
                st.load();
                redraw(&plugin, &mut st);
            }
            Input::Monitors(m) => {
                st.monitors = m;
                redraw(&plugin, &mut st);
            }
            Input::Settings(s) => {
                st.settings = Settings::from(&s);
                seconds.store(st.settings.seconds, Ordering::Relaxed);
                st.load();
                redraw(&plugin, &mut st);
            }
            Input::Event(e) => match e.kind {
                EventKind::PopoverOpen => {
                    let today = Local::now().date_naive();
                    st.selected = today;
                    st.month = first_of(today);
                    st.load();
                    redraw(&plugin, &mut st);
                }
                EventKind::Action => {
                    let Some(c) = e.control.as_deref() else { continue };
                    let today = Local::now().date_naive();
                    if let Some(d) = c.strip_prefix("day:").and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()) {
                        st.selected = d;
                        if first_of(d) != st.month {
                            st.month = first_of(d);
                            st.load();
                        }
                    } else if let Some(link) = c.strip_prefix("join:") {
                        spawn(&["xdg-open", link]);
                        continue;
                    } else if let Some(d) = c.strip_prefix("open:") {
                        spawn(&["nyx-calendar", "--date", d]);
                        continue;
                    } else if let Some(n) = c.strip_prefix("done:").and_then(|n| n.parse::<usize>().ok()) {
                        let Some(t) = st.shown_tasks.get(n).cloned() else { continue };
                        if st.done.insert(task_key(&t)) {
                            spawn(&["nyx-remindersd", "--complete", &t.list, &t.uid]);
                        }
                    } else {
                        match c {
                            "today" => {
                                st.selected = today;
                                st.month = first_of(today);
                                st.load();
                            }
                            "prev" | "next" => {
                                st.month = add_months(st.month, if c == "prev" { -1 } else { 1 });
                                st.selected = if first_of(today) == st.month { today } else { st.month };
                                st.load();
                            }
                            "open-reminders" => {
                                spawn(&["nyx-reminders"]);
                                continue;
                            }
                            "new-reminder" => {
                                spawn(&["nyx-reminders", "--add", ""]);
                                continue;
                            }
                            "new-event" => {
                                spawn(&["nyx-calendar", "--date", &st.selected.to_string()]);
                                continue;
                            }
                            _ => continue,
                        }
                    }
                    redraw(&plugin, &mut st);
                }
                _ => {}
            },
            Input::Shutdown => break,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(title: &str, starts: i64, mins: i64) -> Event {
        Event {
            calendar: "Work".into(),
            color: Some("#3d8ae8".into()),
            title: title.into(),
            location: String::new(),
            description: String::new(),
            url: None,
            starts,
            ends: starts + mins * 60,
            all_day: false,
        }
    }

    #[test]
    fn meeting_links() {
        let mut e = ev("Review", 0, 30);
        assert_eq!(e.meeting(), None);
        e.description = "Join: <https://meet.google.com/abc-defg-hij>.".into();
        assert_eq!(e.meeting().as_deref(), Some("https://meet.google.com/abc-defg-hij"));
        e.description.clear();
        e.location = "https://example.com/room".into();
        assert_eq!(e.meeting(), None, "only known meeting hosts");
    }

    #[test]
    fn clock_settings() {
        let at = Local.with_ymd_and_hms(2026, 11, 11, 17, 8, 9).unwrap();
        let s = |v: serde_json::Value| Settings::from(&v);
        let bar = |st: &Settings| (fmt(at, &st.date), fmt(at, &st.time));
        assert_eq!(bar(&s(serde_json::json!({ "hours": "24" }))), ("Wed Nov 11".into(), "17:08".into()));
        assert_eq!(bar(&s(serde_json::json!({ "hours": 12, "date": "long" }))), ("Wednesday, November 11".into(), "5:08 PM".into()));
        let secs = s(serde_json::json!({ "hours": "24", "seconds": true, "date": "iso" }));
        assert!(secs.seconds);
        assert_eq!(bar(&secs), ("2026-11-11".into(), "17:08:09".into()));
        assert_eq!(bar(&s(serde_json::json!({ "hours": "24", "date": "none" }))), (String::new(), "17:08".into()));
        let own = s(serde_json::json!({ "format": "%d.%m  %H.%M.%S" }));
        assert_eq!(bar(&own), ("11.11".into(), "17.08.09".into()));
        assert!(own.seconds, "a format of your own with seconds ticks too");
        let bad = s(serde_json::json!({ "format": "%Q %H", "hours": "24" }));
        assert_eq!(bar(&bad), ("Wed Nov 11".into(), "17:08".into()), "a bad format falls back");
        assert_eq!(s(serde_json::json!({ "hours": "12" })).hm(at), "5:08 PM");
    }

    #[test]
    fn months_and_grids() {
        let nov = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        assert_eq!(grid_start(nov, false), NaiveDate::from_ymd_opt(2026, 10, 26).unwrap(), "a Sunday the 1st starts on Monday the 26th");
        assert_eq!(grid_start(nov, true), nov);
        assert_eq!(add_months(nov, 2), NaiveDate::from_ymd_opt(2027, 1, 1).unwrap());
        assert_eq!(add_months(nov, -11), NaiveDate::from_ymd_opt(2025, 12, 1).unwrap());
        assert_eq!(until(28), "28 min");
        assert_eq!(until(90), "1 h 30 min");
    }

    #[test]
    fn reads_the_published_files() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("agenda.sqlite");
        let db = rusqlite::Connection::open(&a).unwrap();
        db.execute_batch(
            "CREATE TABLE events (calendar TEXT, calendar_name TEXT, color TEXT, uid TEXT, rid TEXT, title TEXT, location TEXT,
               description TEXT, url TEXT, starts INTEGER, ends INTEGER, all_day INTEGER, first_day TEXT, last_day TEXT);
             INSERT INTO events VALUES ('h1', 'Holidays in United States', '#30a46c', 'a', NULL, 'Veterans Day', '', '', NULL, 100, 200, 1, '', '');
             INSERT INTO events VALUES ('h2', 'Christian Holidays', NULL, 'b', NULL, 'Veterans Day', '', '', NULL, 100, 200, 1, '', '');
             INSERT INTO events VALUES ('w', 'Work', 'bad', 'c', NULL, 'Standup', '', '', NULL, 150, 160, 0, '', '');",
        )
        .unwrap();
        let events = read_events(&a, 0, 1000);
        assert_eq!(events.len(), 2, "a holiday once");
        assert!(events[0].holiday());
        assert_eq!(events[1].color, None, "only #rrggbb colours");
        let t = d.path().join("tasks.sqlite");
        let db = rusqlite::Connection::open(&t).unwrap();
        db.execute_batch(
            "CREATE TABLE tasks (list TEXT, list_name TEXT, uid TEXT, title TEXT, notes TEXT, parent TEXT, due_day TEXT, due_at INTEGER,
               overdue INTEGER, flagged INTEGER, priority INTEGER);
             INSERT INTO tasks VALUES ('l', 'Work', 'u1', 'Invoice', '', NULL, '2026-11-09', NULL, 1, 0, NULL);
             INSERT INTO tasks VALUES ('l', 'Work', 'u2', 'Later', '', NULL, '2026-11-20', NULL, 0, 0, NULL);
             INSERT INTO tasks VALUES ('l', 'Work', 'u3', 'Sub', '', 'u1', '2026-11-09', NULL, 1, 0, NULL);",
        )
        .unwrap();
        let tasks = read_tasks(&t, NaiveDate::from_ymd_opt(2026, 11, 10).unwrap());
        assert_eq!(tasks.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), vec!["Invoice"]);
        assert_eq!(due_text(&Settings::from(&serde_json::json!({})), &tasks[0], NaiveDate::from_ymd_opt(2026, 11, 10).unwrap()), ("Overdue · yesterday".to_string(), true));
    }
}
