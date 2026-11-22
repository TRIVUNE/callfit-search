//! callfit - find funding, open calls and residencies that fit Trivune.

mod live;
mod model;

use chrono::{Local, NaiveDate};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use model::*;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, HighlightSpacing, List, ListItem, ListState, Padding, Paragraph, Row, Table, TableState, Wrap},
    DefaultTerminal, Frame,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

/// trivune.com palette: ink on charcoal (dark) or light grey (light), one red accent.
struct Theme {
    bg: Color,
    ink: Color,
    muted: Color,
    faint: Color,
    line: Color,
    sel: Color,
    /// the red of the TRIVUNE wordmark's V: bars, marks, the mode chip
    red: Color,
    /// red that stays readable as text on `bg`
    red_text: Color,
}
const DARK: Theme = Theme {
    bg: Color::Rgb(0x1f, 0x1f, 0x1f),
    ink: Color::Rgb(0xf2, 0xf2, 0xf2),
    muted: Color::Rgb(0xa8, 0xa8, 0xa8),
    faint: Color::Rgb(0x7a, 0x7a, 0x7a),
    line: Color::Rgb(0x3a, 0x3a, 0x3a),
    sel: Color::Rgb(0x2e, 0x2e, 0x2e),
    red: Color::Rgb(0xb3, 0x12, 0x1b),
    red_text: Color::Rgb(0xf2, 0x55, 0x5a),
};
const LIGHT: Theme = Theme {
    bg: Color::Rgb(0xec, 0xec, 0xec),
    ink: Color::Rgb(0x11, 0x11, 0x11),
    muted: Color::Rgb(0x55, 0x55, 0x55),
    faint: Color::Rgb(0x73, 0x73, 0x73),
    line: Color::Rgb(0xc8, 0xc8, 0xc8),
    sel: Color::Rgb(0xdc, 0xdc, 0xdc),
    red: Color::Rgb(0xb3, 0x12, 0x1b),
    red_text: Color::Rgb(0xb3, 0x12, 0x1b),
};
static LIGHT_MODE: AtomicBool = AtomicBool::new(false);

fn th() -> &'static Theme {
    if LIGHT_MODE.load(Ordering::Relaxed) { &LIGHT } else { &DARK }
}

fn fg(c: Color) -> Style {
    Style::default().fg(c)
}

/// data_dir/theme.txt holds "light" or "dark"
fn load_theme() {
    let light = std::fs::read_to_string(data_dir().join("theme.txt")).map(|s| s.trim() == "light").unwrap_or(false);
    LIGHT_MODE.store(light, Ordering::Relaxed);
}

fn toggle_theme() -> &'static str {
    let light = !LIGHT_MODE.load(Ordering::Relaxed);
    LIGHT_MODE.store(light, Ordering::Relaxed);
    let name = if light { "light" } else { "dark" };
    let _ = std::fs::write(data_dir().join("theme.txt"), name);
    name
}

const KINDS: &[&str] = &["All", "Grant", "Prize", "Stipend", "Residency", "Fellowship", "Call for papers", "Open call", "Competition", "Hackathon", "Conference", "Accelerator", "Live"];
const REGIONS: &[&str] = &["All", "Berlin", "DE", "EU", "World"];
const FITS: &[&str] = &["Eligible", "Perfect+Good", "All"];
const STATUSES: &[&str] = &["All", "Open now", "Upcoming"];
const SORTS: &[&str] = &["Score", "Match", "Deadline", "Amount", "Name"];
const VIEWS: &[&str] = &["All", "Tracked"];

#[derive(PartialEq)]
enum Mode {
    Normal,
    /// `/`: filter the list
    Search,
    /// `w`: typing a web search
    Web,
    /// web search results popup
    WebResults,
}

enum WebMsg {
    /// (query, results or why there are none)
    Results(String, Result<Vec<Opportunity>, String>),
    Added(Opportunity),
}

struct App {
    today: NaiveDate,
    profile: Profile,
    all: Vec<Scored>,
    shown: Vec<usize>,
    table: TableState,
    query: String,
    mode: Mode,
    kind: usize,
    region: usize,
    fit: usize,
    status: usize,
    sort: usize,
    view: usize,
    tracker: HashMap<String, Track>,
    live_rx: Option<Receiver<live::Msg>>,
    live_status: String,
    live_count: usize,
    toast: Option<(String, std::time::Instant)>,
    detail_scroll: u16,
    #[allow(dead_code)]
    user_entries: usize,
    web_query: String,
    web_results: Vec<Opportunity>,
    web_list: ListState,
    web_status: String,
    web_tx: Sender<WebMsg>,
    web_rx: Receiver<WebMsg>,
}

impl App {
    fn new() -> Self {
        let today = Local::now().date_naive();
        let profile = load_profile();
        let (mut opps, user_entries) = load_opportunities();
        let cached = live::load_cache(today);
        let live_count = cached.len();
        opps.extend(cached);
        let all = opps.iter().map(|o| score(o, &profile, today)).collect();
        let (web_tx, web_rx) = channel();
        let mut app = App {
            today,
            profile,
            all,
            shown: vec![],
            table: TableState::default(),
            query: String::new(),
            mode: Mode::Normal,
            kind: 0,
            region: 0,
            fit: 0,
            status: 0,
            sort: 0,
            view: 0,
            tracker: load_tracker(),
            live_rx: None,
            live_status: if live_count > 0 { format!("{} saved, press u", live_count) } else { "press u".into() },
            live_count,
            toast: None,
            detail_scroll: 0,
            user_entries,
            web_query: String::new(),
            web_results: vec![],
            web_list: ListState::default(),
            web_status: String::new(),
            web_tx,
            web_rx,
        };
        app.refilter();
        app.table.select(Some(0));
        app
    }

    fn track(&self, id: &str) -> Track {
        *self.tracker.get(id).unwrap_or(&Track::None)
    }

    fn refilter(&mut self) {
        let keep_id = self.current().map(|s| s.opp.id.clone());
        let q = self.query.clone();
        let mut idx: Vec<usize> = (0..self.all.len())
            .filter(|&i| {
                let s = &self.all[i];
                let o = &s.opp;
                match KINDS[self.kind] {
                    "All" => {}
                    "Live" if o.confidence != "live" => return false,
                    "Live" => {}
                    k if o.kind != k => return false,
                    _ => {}
                }
                if REGIONS[self.region] != "All" && o.scope != REGIONS[self.region] {
                    return false;
                }
                match FITS[self.fit] {
                    "Eligible" if s.fit == FitLevel::NotEligible => return false,
                    "Perfect+Good" if !matches!(s.fit, FitLevel::Perfect | FitLevel::Good) => return false,
                    _ => {}
                }
                match STATUSES[self.status] {
                    "Open now" if !matches!(s.status, Status::Open | Status::Rolling) => return false,
                    "Upcoming" if s.status != Status::Upcoming => return false,
                    _ if s.status == Status::Closed => return false, // passed deadlines are never shown
                    _ => {}
                }
                if VIEWS[self.view] == "Tracked" && self.track(&o.id) == Track::None {
                    return false;
                }
                if !q.is_empty() {
                    // every word must match (accent-insensitive); "-word" excludes
                    let hay = live::fold(&format!(
                        "{} {} {} {} {} {} {} {} {} {} {}",
                        o.name, o.org, o.kind, o.city, o.country, o.scope, o.summary, o.tags.join(" "),
                        o.amount_note, o.cadence, o.apply_url
                    ));
                    for w in q.split_whitespace() {
                        let (neg, w) = match w.strip_prefix('-') { Some(x) if !x.is_empty() => (true, x), _ => (false, w) };
                        if hay.contains(&live::fold(w)) == neg {
                            return false;
                        }
                    }
                }
                true
            })
            .collect();
        let all = &self.all;
        match SORTS[self.sort] {
            "Deadline" => idx.sort_by_key(|&i| {
                let s = &all[i];
                (
                    match s.status {
                        Status::Open => 0,
                        Status::Upcoming => 1,
                        Status::Rolling => 2,
                        Status::Unknown => 3,
                        Status::Closed => 4,
                    },
                    s.days_left.or(s.days_to_open).unwrap_or(99999),
                )
            }),
            "Amount" => idx.sort_by(|&a, &b| {
                let x = all[a].opp.amount_max.max(all[a].opp.amount_min);
                let y = all[b].opp.amount_max.max(all[b].opp.amount_min);
                y.cmp(&x)
            }),
            "Match" => idx.sort_by(|&a, &b| all[b].match10.cmp(&all[a].match10).then(all[b].overall.partial_cmp(&all[a].overall).unwrap())),
            "Name" => idx.sort_by(|&a, &b| all[a].opp.name.to_lowercase().cmp(&all[b].opp.name.to_lowercase())),
            _ => idx.sort_by(|&a, &b| all[b].overall.partial_cmp(&all[a].overall).unwrap()),
        }
        self.shown = idx;
        let pos = keep_id
            .and_then(|id| self.shown.iter().position(|&i| self.all[i].opp.id == id))
            .unwrap_or(0);
        self.table.select(if self.shown.is_empty() { None } else { Some(pos) });
    }

    fn current(&self) -> Option<&Scored> {
        self.table.selected().and_then(|i| self.shown.get(i)).map(|&i| &self.all[i])
    }

    fn toast(&mut self, s: impl Into<String>) {
        self.toast = Some((s.into(), std::time::Instant::now()));
    }

    fn move_sel(&mut self, d: i64) {
        if self.shown.is_empty() {
            return;
        }
        let cur = self.table.selected().unwrap_or(0) as i64;
        let n = (cur + d).clamp(0, self.shown.len() as i64 - 1) as usize;
        self.table.select(Some(n));
        self.detail_scroll = 0;
    }

    fn start_live(&mut self) {
        if self.live_rx.is_some() {
            return;
        }
        let (tx, rx) = channel();
        live::spawn(tx, self.today);
        self.live_rx = Some(rx);
        self.live_status = "loading…".into();
        // old live rows stay until this run replaces them
    }

    fn poll_live(&mut self) {
        let mut changed = false;
        let mut finished = false;
        if let Some(rx) = &self.live_rx {
            while let Ok(m) = rx.try_recv() {
                match m {
                    live::Msg::Items(items) => {
                        for o in items {
                            changed = true;
                            if let Some(s) = self.all.iter_mut().find(|s| s.opp.id == o.id) {
                                *s = score(&o, &self.profile, self.today);
                                continue;
                            }
                            // a built-in entry with the same link wins over the live copy
                            if self.all.iter().any(|s| s.opp.confidence != "live" && !s.opp.apply_url.is_empty() && s.opp.apply_url == o.apply_url) {
                                continue;
                            }
                            self.all.push(score(&o, &self.profile, self.today));
                            self.live_count += 1;
                        }
                    }
                    live::Msg::Gone(ids) => {
                        let before = self.all.len();
                        self.all.retain(|s| !ids.contains(&s.opp.id));
                        self.live_count -= before - self.all.len();
                        changed = true;
                    }
                    live::Msg::Progress { stage, done, total, .. } => {
                        self.live_status = format!("{} {}/{} · {} found", stage, done, total, self.live_count);
                    }
                    live::Msg::Done { failed } => {
                        let live: Vec<Opportunity> = self.all.iter().filter(|s| s.opp.confidence == "live").map(|s| s.opp.clone()).collect();
                        self.live_count = live.len();
                        live::save_cache(&live, self.today);
                        self.live_status = if failed.is_empty() {
                            format!("{} live", self.live_count)
                        } else {
                            format!("{} live, {} sources failed", self.live_count, failed.len())
                        };
                        finished = true;
                    }
                }
            }
        }
        if finished {
            self.live_rx = None;
        }
        if changed {
            self.refilter();
        }
    }

    /// The computer's date changed (app open past midnight): score everything again.
    fn check_day(&mut self) {
        let now = Local::now().date_naive();
        if now != self.today {
            self.today = now;
            self.all = self.all.iter().map(|s| score(&s.opp, &self.profile, now)).collect();
            self.refilter();
        }
    }

    fn start_web(&mut self) {
        let q = self.web_query.trim().to_string();
        if q.is_empty() {
            return;
        }
        let (tx, today) = (self.web_tx.clone(), self.today);
        self.mode = Mode::WebResults;
        self.web_results.clear();
        self.web_list.select(None);
        if q.starts_with("http://") || q.starts_with("https://") {
            // a pasted link: add that page directly
            self.web_status = format!("Adding {}: reading the page…", q);
            std::thread::spawn(move || {
                let _ = tx.send(WebMsg::Added(added(live::link_entry(&q, today))));
            });
            return;
        }
        self.web_status = format!("Searching the web for \"{}\"…", q);
        std::thread::spawn(move || {
            let res = live::web_search(&q, today);
            let _ = tx.send(WebMsg::Results(q, res));
        });
    }

    fn add_web(&mut self) {
        let Some(o) = self.web_list.selected().and_then(|i| self.web_results.get(i)).cloned() else { return };
        let (tx, today) = (self.web_tx.clone(), self.today);
        self.web_status = format!("Adding \"{}\": reading its page…", o.name);
        std::thread::spawn(move || {
            let _ = tx.send(WebMsg::Added(added(live::read_page(o, false, today))));
        });
    }

    fn poll_web(&mut self) {
        while let Ok(m) = self.web_rx.try_recv() {
            match m {
                WebMsg::Results(q, _) if q != self.web_query.trim() => {} // an older search
                WebMsg::Results(_, Ok(v)) => {
                    self.web_status = format!("{} results {} · Enter: open · a: add to your list", v.len(), v[0].cadence);
                    self.web_results = v;
                    self.web_list.select(Some(0));
                }
                WebMsg::Results(_, Err(e)) => {
                    self.web_status = format!("Nothing found ({}). o: search in your browser, then paste a link with w to add it", e)
                }
                WebMsg::Added(o) => match save_user_opportunity(&o) {
                    Err(e) => self.web_status = format!("Could not save: {}", e),
                    Ok(()) => {
                        let s = score(&o, &self.profile, self.today);
                        match self.all.iter_mut().find(|x| x.opp.id == o.id) {
                            Some(x) => *x = s,
                            None => self.all.push(s),
                        }
                        self.refilter();
                        let at = self.shown.iter().position(|&i| self.all[i].opp.id == o.id);
                        if at.is_some() {
                            self.table.select(at);
                        }
                        let hidden = if at.is_none() { " (hidden by the current filters)" } else { "" };
                        self.web_status = format!("Added \"{}\" to your list{}", o.name, hidden);
                        self.toast(self.web_status.clone());
                    }
                },
            }
        }
    }

    fn export(&mut self) {
        let name = format!("callfit_{}.csv", Local::now().format("%Y%m%d_%H%M"));
        let path = std::env::current_dir().unwrap_or_default().join(&name);
        let esc = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
        let mut out = String::from("fit,match,score,name,org,type,where,amount,deadline,event,status,tracker,apply_url,info_url\n");
        for &i in &self.shown {
            let s = &self.all[i];
            let o = &s.opp;
            out.push_str(&format!(
                "{},{},{:.0},{},{},{},{},{},{},{},{},{},{},{}\n",
                s.fit.label(),
                s.match10,
                s.overall,
                esc(&o.name),
                esc(&o.org),
                esc(&o.kind),
                esc(&where_text(o)),
                esc(&fmt_amount(o)),
                o.deadline.clone().unwrap_or_default(),
                esc(&o.event),
                s.status.label(),
                self.track(&o.id).label(),
                esc(&o.apply_url),
                esc(&o.info_url)
            ));
        }
        match std::fs::write(&path, "\u{feff}".to_string() + &out) {
            Ok(_) => self.toast(format!("Exported {} rows → {}", self.shown.len(), path.display())),
            Err(e) => self.toast(format!("Export failed: {}", e)),
        }
    }
}

/// A web result the user adds: own id and confidence, so a live run never replaces it and the
/// live cache never duplicates it.
fn added(mut o: Opportunity) -> Opportunity {
    o.id = o.id.replacen("live:", "web:", 1);
    o.confidence = "added".into();
    o
}

fn where_text(o: &Opportunity) -> String {
    match (o.city.is_empty(), o.country.is_empty()) {
        (false, false) => format!("{}, {}", o.city, o.country),
        (true, false) => o.country.clone(),
        (false, true) => o.city.clone(),
        _ => match o.scope.as_str() {
            "World" => "Online / varies".into(),
            s => s.into(),
        },
    }
}

fn short_kind(k: &str) -> String {
    match k {
        "Call for papers" => "Papers".into(),
        "Accelerator" => "Founders".into(),
        other => other.into(),
    }
}

fn match_color(m: u8) -> Color {
    match m {
        5..=10 => th().ink,
        2..=4 => th().muted,
        _ => th().faint,
    }
}

fn fit_color(f: FitLevel) -> Color {
    match f {
        FitLevel::Perfect | FitLevel::Good => th().ink,
        FitLevel::Marginal => th().muted,
        FitLevel::Low => th().faint,
        FitLevel::NotEligible => th().red_text,
    }
}

/// guessed from last year's dates, not announced yet
fn guessed(s: &Scored) -> bool {
    s.opp.confidence == "pattern" && matches!(s.status, Status::Open | Status::Upcoming)
}

fn deadline_cell(s: &Scored) -> (String, String, Color) {
    let t = th();
    if guessed(s) {
        let month = parse_date(&s.opp.deadline).map(|d| d.format("~%b %Y").to_string()).unwrap_or_default();
        return (month, "expected".into(), t.faint);
    }
    match s.status {
        Status::Open => {
            let d = s.days_left.unwrap_or(0);
            (s.opp.deadline.clone().unwrap_or_default(), format!("{}d", d), if d <= 7 { t.red_text } else { t.ink })
        }
        Status::Upcoming => (
            s.opp.deadline.clone().unwrap_or_else(|| "upcoming".into()),
            format!("opens {}d", s.days_to_open.unwrap_or(0)),
            t.muted,
        ),
        Status::Rolling => ("rolling".into(), "anytime".into(), t.ink),
        Status::Closed => ("closed".into(), "see next".into(), t.faint),
        Status::Unknown => ("check site".into(), "—".into(), t.faint),
    }
}

fn bold(c: Color) -> Style {
    Style::default().fg(c).add_modifier(Modifier::BOLD)
}

/// One text input: red bar when active, placeholder when empty.
fn input(label: &str, key: &str, text: &str, hint: &str, active: bool) -> Line<'static> {
    let t = th();
    let mut v = vec![
        Span::styled(if active { " ▍" } else { "  " }, fg(t.red)),
        Span::styled(format!("{} ", label), if active { bold(t.ink) } else { fg(t.muted) }),
        Span::styled(format!("{}  ", key), fg(t.faint)),
    ];
    if text.is_empty() && !active {
        v.push(Span::styled(hint.to_string(), fg(t.faint)));
    } else {
        v.push(Span::styled(format!("{}{}", text, if active { "▏" } else { "" }), fg(t.ink)));
    }
    Line::from(v)
}

fn ui(f: &mut Frame, app: &mut App) {
    let t = th();
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(t.bg).fg(t.ink)), area);
    let rows = Layout::vertical([Constraint::Length(2), Constraint::Length(3), Constraint::Min(5), Constraint::Length(1)]).split(area);
    let hairline = Block::default().borders(Borders::BOTTOM).border_style(fg(t.line));

    // ---- header: wordmark, then counts
    let open_now = app.all.iter().filter(|s| matches!(s.status, Status::Open | Status::Rolling) && s.fit != FitLevel::NotEligible).count();
    let eligible = app.all.iter().filter(|s| s.fit != FitLevel::NotEligible).count();
    let tracked = app.tracker.values().filter(|t| **t != Track::None).count();
    let mut head = vec![
        Span::styled(" TRI", bold(t.ink)),
        Span::styled("V", bold(t.red)),
        Span::styled("UNE", bold(t.ink)),
        Span::styled("  callfit", fg(t.muted)),
        Span::raw("      "),
    ];
    for (n, label) in [(app.all.len(), "entries"), (eligible, "eligible"), (open_now, "apply now"), (tracked, "tracked")] {
        head.push(Span::styled(format!("{} ", n), fg(t.ink)));
        head.push(Span::styled(format!("{}    ", label), fg(t.muted)));
    }
    head.push(Span::styled("live ", fg(t.muted)));
    head.push(Span::styled(format!("{}    ", app.live_status), fg(t.ink)));
    head.push(Span::styled(app.today.format("%a %d %b %Y").to_string(), fg(t.muted)));
    f.render_widget(Paragraph::new(Line::from(head)).block(hairline.clone()), rows[0]);

    // ---- the two searches, then the filters (a changed filter shows in red)
    let lines = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(hairline.inner(rows[1]));
    f.render_widget(hairline.clone(), rows[1]);
    let halves = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(lines[0]);
    f.render_widget(Paragraph::new(input("Filter the list", "/", &app.query, "sound art, quantum, residency…", app.mode == Mode::Search)), halves[0]);
    f.render_widget(
        Paragraph::new(input("Search the web", "w", &app.web_query, "anything not in the list, or paste a link", app.mode == Mode::Web)),
        halves[1],
    );
    let mut opts = vec![Span::raw("  ")];
    for (label, key, i, vals) in [
        ("Type", "t", app.kind, KINDS),
        ("Region", "r", app.region, REGIONS),
        ("Fit", "f", app.fit, FITS),
        ("Status", "o", app.status, STATUSES),
        ("Sort", "s", app.sort, SORTS),
        ("View", "v", app.view, VIEWS),
    ] {
        opts.push(Span::styled(format!("{} ", label), fg(t.muted)));
        opts.push(Span::styled(format!("{} ", key), fg(t.faint)));
        opts.push(Span::styled(vals[i], if i == 0 { fg(t.ink) } else { bold(t.red_text) }));
        opts.push(Span::raw("     "));
    }
    f.render_widget(Paragraph::new(Line::from(opts)), lines[1]);

    // ---- main
    let main = Layout::horizontal([Constraint::Percentage(57), Constraint::Percentage(43)]).split(rows[2]);

    let header_row = Row::new(["", "Opportunity", "Type", "Amount", "Deadline", "Left", "Where", "Match", "Score"]).style(bold(t.muted));
    let rows_v: Vec<Row> = app
        .shown
        .iter()
        .map(|&i| {
            let s = &app.all[i];
            let o = &s.opp;
            let (dl, left, dc) = deadline_cell(s);
            let mark = if app.track(&o.id) != Track::None { Span::styled("★", fg(t.red_text)) } else { Span::styled("●", fg(fit_color(s.fit))) };
            let mut name = vec![];
            if !o.warnings.is_empty() {
                name.push(Span::styled("⚠ ", fg(t.red_text)));
            }
            name.push(Span::styled(o.name.clone(), fg(t.ink)));
            Row::new(vec![
                Cell::from(mark),
                Cell::from(Line::from(name)),
                Cell::from(Span::styled(short_kind(&o.kind), fg(t.muted))),
                Cell::from(Span::styled(fmt_amount(o), fg(t.ink))),
                Cell::from(Span::styled(dl, fg(dc))),
                Cell::from(Span::styled(left, fg(dc))),
                Cell::from(Span::styled(where_text(o), fg(t.muted))),
                Cell::from(Span::styled(format!("{:>2}/10", s.match10), fg(match_color(s.match10)))),
                Cell::from(Span::styled(format!("{:.0}", s.overall), fg(fit_color(s.fit)))),
            ])
        })
        .collect();
    let table = Table::new(
        rows_v,
        [
            Constraint::Length(1),
            Constraint::Min(26),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(11),
            Constraint::Length(10),
            Constraint::Length(12),
            Constraint::Length(5),
            Constraint::Length(5),
        ],
    )
    .header(header_row)
    .row_highlight_style(Style::default().bg(t.sel).add_modifier(Modifier::BOLD))
    .highlight_symbol(Span::styled("▌", fg(t.red)))
    .highlight_spacing(HighlightSpacing::Always)
    .block(Block::default().title(Line::from(vec![
        Span::styled(" Opportunities  ", bold(t.ink)),
        Span::styled(format!("{} of {}", app.shown.len(), app.all.len()), fg(t.muted)),
    ])));
    f.render_stateful_widget(table, main[0], &mut app.table);

    // ---- detail
    let db = Block::default().borders(Borders::LEFT).border_style(fg(t.line)).padding(Padding::horizontal(1));
    f.render_widget(
        Paragraph::new(detail_lines(app)).block(db).wrap(Wrap { trim: false }).scroll((app.detail_scroll, 0)),
        main[1],
    );

    // ---- footer
    let mode = match app.mode {
        Mode::Normal => " LIST ",
        Mode::Search => " FILTER ",
        Mode::Web | Mode::WebResults => " WEB ",
    };
    let mut foot = vec![Span::styled(mode, Style::default().fg(Color::White).bg(t.red).add_modifier(Modifier::BOLD)), Span::raw(" ")];
    if let Some((msg, at)) = &app.toast {
        if at.elapsed() < Duration::from_secs(4) {
            foot.push(Span::styled(msg.clone(), fg(t.ink)));
        }
    }
    if foot.len() == 2 {
        let keys = match app.mode {
            Mode::Search => "type to filter · Enter: done (no matches: search the web) · Esc: done · Ctrl+U: clear",
            Mode::Web => "type what to find, or paste a link to add it · Enter: go · Esc: cancel · Ctrl+U: clear",
            Mode::WebResults => "↑↓: pick · Enter: open · a: add to your list · o: search in your browser · w: new search · Esc: close",
            Mode::Normal => "↑↓/jk:nav  Enter:apply  i:info  p:track  /:filter  w:web search  t r f o s v:filters  u:live  x:export  e:data  m:theme  J/K:scroll  q:quit",
        };
        foot.push(Span::styled(keys, fg(t.muted)));
    }
    f.render_widget(Paragraph::new(Line::from(foot)), rows[3]);

    // ---- web search results
    if app.mode == Mode::WebResults {
        let r = centered(area, area.width.saturating_sub(8).min(110), area.height.saturating_sub(6).min(32));
        f.render_widget(Clear, r);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(fg(t.line))
            .style(Style::default().bg(t.bg))
            .padding(Padding::horizontal(1))
            .title(Line::from(vec![Span::styled(" ▍", fg(t.red)), Span::styled(format!("Web search: {} ", app.web_query.trim()), bold(t.ink))]))
            .title_bottom(Line::from(Span::styled(format!(" {} ", app.web_status), fg(t.muted))));
        let items: Vec<ListItem> = app
            .web_results
            .iter()
            .map(|o| {
                ListItem::new(vec![
                    Line::from(Span::styled(o.name.clone(), bold(t.ink))),
                    Line::from(Span::styled(format!("{} · {}", o.org, o.summary), fg(t.muted))),
                    Line::raw(""),
                ])
            })
            .collect();
        // highlight_style's red only reaches the bar: the spans keep their own colours
        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().bg(t.sel).fg(t.red))
            .highlight_symbol("▌ ")
            .highlight_spacing(HighlightSpacing::Always);
        f.render_stateful_widget(list, r, &mut app.web_list);
    }

    if app.all.is_empty() {
        let r = centered(area, 40, 3);
        f.render_widget(Clear, r);
        f.render_widget(Paragraph::new(" No data").style(Style::default().bg(t.bg)).block(Block::default().borders(Borders::ALL).border_style(fg(t.line))), r);
    }
}

fn centered(a: Rect, w: u16, h: u16) -> Rect {
    Rect { x: a.x + a.width.saturating_sub(w) / 2, y: a.y + a.height.saturating_sub(h) / 2, width: w.min(a.width), height: h.min(a.height) }
}

fn kv(k: &str, v: impl Into<String>, c: Color) -> Line<'static> {
    Line::from(vec![Span::styled(format!(" {:<14}", k), fg(th().muted)), Span::styled(v.into(), fg(c))])
}

fn section(t: &str) -> Line<'static> {
    Line::from(vec![Span::styled(" ▍", fg(th().red)), Span::styled(t.to_string(), bold(th().ink))])
}

fn detail_lines(app: &App) -> Vec<Line<'static>> {
    let t = th();
    let Some(s) = app.current() else {
        return vec![Line::from(Span::styled(" Nothing matches these filters.", fg(t.muted)))];
    };
    let o = &s.opp;
    let mut l = vec![Line::from(Span::styled(format!(" {}", o.name), bold(t.ink))), Line::raw("")];
    l.push(kv("Organisation:", o.org.clone(), t.ink));
    l.push(kv("Type:", o.kind.clone(), t.ink));
    l.push(kv("Where:", where_text(o), t.ink));
    l.push(kv("Open to:", scope_text(&o.scope).to_string(), t.ink));
    l.push(kv("Money:", fmt_amount(o), t.ink));
    if !o.amount_note.is_empty() {
        l.push(Line::from(Span::styled(format!("                {}", o.amount_note), fg(t.muted))));
    }
    let (dl, left, dc) = deadline_cell(s);
    let deadline_txt = match s.status {
        _ if guessed(s) => format!("{}  (guessed from last year's dates: not announced yet)", dl),
        Status::Open => format!("{}  ({} left)", dl, left.replace('d', " days")),
        Status::Upcoming => format!("{}  (opens in {} days)", o.deadline.clone().unwrap_or_else(|| "date tbc".into()), s.days_to_open.unwrap_or(0)),
        _ => dl,
    };
    l.push(kv("Deadline:", format!("{}  (apply / submit)", deadline_txt), dc));
    if !o.event.is_empty() {
        l.push(kv("Event:", o.event.clone(), t.ink));
    }
    if let Some(op) = &o.opens {
        l.push(kv("Opens:", op.clone(), t.ink));
    }
    if !o.cadence.is_empty() {
        l.push(kv("Cadence:", o.cadence.clone(), t.ink));
    }
    if !o.next_expected.is_empty() {
        l.push(kv("Next:", o.next_expected.clone(), t.ink));
    }
    let conf = match o.confidence.as_str() {
        "verified" => ("verified", t.ink),
        "pattern" => ("dates guessed from last year: not announced yet", t.red_text),
        "live" => ("live listing, not checked", t.muted),
        "added" => ("added by you from a web search", t.ink),
        _ => ("check on site", t.red_text),
    };
    l.push(kv("Data:", format!("{} · checked {}", conf.0, o.checked), conf.1));
    for w in &o.warnings {
        l.push(kv("⚠", w.clone(), t.red_text));
    }

    l.push(Line::raw(""));
    l.push(section("Score"));
    l.push(Line::raw(""));
    l.push(Line::from(vec![
        Span::styled(" Overall:      ", fg(t.muted)),
        Span::styled(format!("{:.1} / 100", s.overall), bold(fit_color(s.fit))),
    ]));
    l.push(Line::from(vec![
        Span::styled(" Eligibility: ", fg(t.muted)),
        Span::styled(format!("{:<4}", s.eligibility), fg(t.ink)),
        Span::styled("Relevance: ", fg(t.muted)),
        Span::styled(format!("{:<4}", s.relevance), fg(t.ink)),
        Span::styled("Money: ", fg(t.muted)),
        Span::styled(format!("{:<4}", s.money), fg(t.ink)),
        Span::styled("Timing: ", fg(t.muted)),
        Span::styled(format!("{}", s.timing), fg(t.ink)),
    ]));
    l.push(kv(
        "Match:",
        if s.matched.is_empty() { format!("{}/10 (no overlap with your interests)", s.match10) } else { format!("{}/10 ({})", s.match10, s.matched.join(", ")) },
        match_color(s.match10),
    ));

    l.push(Line::raw(""));
    l.push(section("Trivune fit"));
    l.push(Line::raw(""));
    l.push(Line::from(vec![Span::styled(" Fit Level:    ", fg(t.muted)), Span::styled(format!("● {}", s.fit.label()), bold(fit_color(s.fit)))]));
    for c in &s.checks {
        let (m, col) = if c.ok { ("✓", t.ink) } else { ("✗", t.red_text) };
        l.push(Line::from(vec![Span::styled(format!("   {} ", m), fg(col)), Span::styled(c.text.clone(), fg(if c.ok { t.ink } else { t.muted }))]));
    }

    l.push(Line::raw(""));
    l.push(section("Links"));
    l.push(Line::raw(""));
    l.push(kv("Call page [Enter]", o.apply_url.clone(), t.ink));
    l.push(kv("Found on  [i]", o.info_url.clone(), t.ink));

    l.push(Line::raw(""));
    l.push(section("Tracker"));
    l.push(Line::raw(""));
    let tr = app.track(&o.id);
    l.push(kv("Status [p]:", tr.label().to_string(), if tr == Track::None { t.muted } else { t.red_text }));

    if !o.summary.is_empty() {
        l.push(Line::raw(""));
        l.push(section("Notes"));
        l.push(Line::raw(""));
        l.push(Line::from(Span::styled(format!(" {}", o.summary), fg(t.ink))));
    }
    l
}

fn open_url(app: &mut App, url: &str) {
    if url.is_empty() {
        app.toast("No link for this entry");
        return;
    }
    match open::that_detached(url) {
        Ok(_) => app.toast(format!("Opened {}", url)),
        Err(e) => app.toast(format!("Could not open browser: {}", e)),
    }
}

fn run(mut term: DefaultTerminal) -> std::io::Result<()> {
    let mut app = App::new();
    if live::cache_stale() {
        app.start_live();
    }
    loop {
        app.poll_live();
        app.poll_web();
        app.check_day();
        term.draw(|f| ui(f, &mut app))?;
        if !event::poll(Duration::from_millis(150))? {
            continue;
        }
        let Event::Key(k) = event::read()? else { continue };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if app.mode == Mode::Web {
            match k.code {
                KeyCode::Enter => app.start_web(),
                KeyCode::Esc => app.mode = Mode::Normal,
                KeyCode::Backspace => {
                    app.web_query.pop();
                }
                KeyCode::Char('u') if ctrl => app.web_query.clear(),
                KeyCode::Char(c) => app.web_query.push(c),
                _ => {}
            }
            continue;
        }
        if app.mode == Mode::WebResults {
            let n = app.web_results.len();
            let at = app.web_list.selected().unwrap_or(0);
            match k.code {
                KeyCode::Esc | KeyCode::Char('q') => app.mode = Mode::Normal,
                KeyCode::Down | KeyCode::Char('j') if n > 0 => app.web_list.select(Some((at + 1).min(n - 1))),
                KeyCode::Up | KeyCode::Char('k') => app.web_list.select(Some(at.saturating_sub(1))),
                KeyCode::Enter => {
                    if let Some(u) = app.web_results.get(at).map(|o| o.apply_url.clone()) {
                        open_url(&mut app, &u);
                    }
                }
                KeyCode::Char('a') => app.add_web(),
                KeyCode::Char('w') => app.mode = Mode::Web,
                KeyCode::Char('o') => {
                    let url = format!("https://www.google.com/search?q={}", live::urlencode(app.web_query.trim()));
                    open_url(&mut app, &url);
                }
                _ => {}
            }
            continue;
        }
        if app.mode == Mode::Search {
            match k.code {
                // nothing in the list matches: look on the web instead
                KeyCode::Enter if app.shown.is_empty() && !app.query.trim().is_empty() => {
                    app.web_query = app.query.clone();
                    app.start_web();
                }
                KeyCode::Enter | KeyCode::Esc => app.mode = Mode::Normal,
                KeyCode::Backspace => {
                    app.query.pop();
                    app.refilter();
                }
                KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.query.clear();
                    app.refilter();
                }
                KeyCode::Char(c) => {
                    app.query.push(c);
                    app.refilter();
                }
                KeyCode::Down => app.move_sel(1),
                KeyCode::Up => app.move_sel(-1),
                _ => {}
            }
            continue;
        }
        match k.code {
            KeyCode::Char('q') => return Ok(()),
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
            KeyCode::Esc => {
                if !app.query.is_empty() {
                    app.query.clear();
                    app.refilter();
                } else {
                    return Ok(());
                }
            }
            KeyCode::Char('/') => app.mode = Mode::Search,
            KeyCode::Down | KeyCode::Char('j') => app.move_sel(1),
            KeyCode::Up | KeyCode::Char('k') => app.move_sel(-1),
            KeyCode::PageDown => app.move_sel(10),
            KeyCode::PageUp => app.move_sel(-10),
            KeyCode::Home | KeyCode::Char('g') => app.move_sel(-100000),
            KeyCode::End | KeyCode::Char('G') => app.move_sel(100000),
            KeyCode::Char('J') => app.detail_scroll = app.detail_scroll.saturating_add(2),
            KeyCode::Char('K') => app.detail_scroll = app.detail_scroll.saturating_sub(2),
            KeyCode::Enter => {
                if let Some(u) = app.current().map(|s| s.opp.apply_url.clone()) {
                    open_url(&mut app, &u);
                }
            }
            KeyCode::Char('i') => {
                if let Some(u) = app.current().map(|s| s.opp.info_url.clone()) {
                    open_url(&mut app, &u);
                }
            }
            KeyCode::Char('t') => { app.kind = (app.kind + 1) % KINDS.len(); app.refilter(); }
            KeyCode::Char('r') => { app.region = (app.region + 1) % REGIONS.len(); app.refilter(); }
            KeyCode::Char('f') => { app.fit = (app.fit + 1) % FITS.len(); app.refilter(); }
            KeyCode::Char('o') => { app.status = (app.status + 1) % STATUSES.len(); app.refilter(); }
            KeyCode::Char('s') => { app.sort = (app.sort + 1) % SORTS.len(); app.refilter(); }
            KeyCode::Char('v') => { app.view = (app.view + 1) % VIEWS.len(); app.refilter(); }
            KeyCode::Char('p') => {
                if let Some(id) = app.current().map(|s| s.opp.id.clone()) {
                    let n = app.track(&id).next();
                    if n == Track::None { app.tracker.remove(&id); } else { app.tracker.insert(id, n); }
                    save_tracker(&app.tracker);
                    app.toast(format!("Tracker: {}", n.label()));
                    if VIEWS[app.view] == "Tracked" { app.refilter(); }
                }
            }
            KeyCode::Char('u') => app.start_live(),
            KeyCode::Char('x') => app.export(),
            KeyCode::Char('w') => {
                if !app.query.is_empty() {
                    app.web_query = app.query.clone();
                }
                app.mode = Mode::Web;
            }
            KeyCode::Char('m') => {
                let name = toggle_theme();
                app.toast(format!("Theme: {}", name));
            }
            KeyCode::Char('e') => {
                let d = data_dir();
                let _ = open::that_detached(&d);
                app.toast(format!("Data folder: {} (edit profile.json, add opportunities.json, feeds.json)", d.display()));
            }
            _ => {}
        }
    }
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--scan") {
        // headless live search: report per source, save results for the app
        let today = Local::now().date_naive();
        let t0 = std::time::Instant::now();
        let (tx, rx) = channel();
        live::spawn(tx, today);
        let mut items: HashMap<String, Opportunity> = HashMap::new();
        for m in rx {
            match m {
                live::Msg::Items(v) => items.extend(v.into_iter().map(|o| (o.id.clone(), o))),
                live::Msg::Gone(ids) => ids.iter().for_each(|id| {
                    items.remove(id);
                }),
                live::Msg::Progress { stage, done, total, source, ok, found, note } => {
                    if stage != "call pages" {
                        println!("{:<10} {:>4}/{:<4} {} {:>3}  {}  {}", stage, done, total, if ok { "ok  " } else { "FAIL" }, found, source, note);
                    } else if done % 200 == 0 || done == total {
                        println!("call pages {}/{}", done, total);
                    }
                }
                live::Msg::Done { failed } => {
                    println!("{} sources failed: {}", failed.len(), failed.join(", "));
                    break;
                }
            }
        }
        // built-in entries come back only to carry a "closed" warning; they are not cached
        let list: Vec<Opportunity> = items.into_values().filter(|o| o.confidence == "live").collect();
        live::save_cache(&list, today);
        let future = list.iter().filter(|o| parse_date(&o.deadline).map(|d| d >= today).unwrap_or(false)).count();
        let money = list.iter().filter(|o| o.amount_max > 0).count();
        let mut kinds: HashMap<&str, usize> = HashMap::new();
        for o in &list {
            *kinds.entry(o.kind.as_str()).or_default() += 1;
        }
        let mut kinds: Vec<_> = kinds.into_iter().collect();
        kinds.sort_by(|a, b| b.1.cmp(&a.1));
        println!(
            "\n{} results in {:.0}s: {} with an upcoming deadline, {} with an amount.",
            list.len(),
            t0.elapsed().as_secs_f32(),
            future,
            money
        );
        println!("{}", kinds.iter().map(|(k, n)| format!("{} {}", n, k)).collect::<Vec<_>>().join(", "));
        println!("Saved to {}. Run callfit to browse them.", data_dir().join("live.json").display());
        return Ok(());
    }
    if args.iter().any(|a| a == "--list") {
        // plain output, handy for scripts
        let app = App::new();
        for &i in &app.shown {
            let s = &app.all[i];
            let (dl, left, _) = deadline_cell(s);
            println!("{:>5.1}  {:>2}/10  {:<12} {:<11} {:<60} {:<10} {:<12} {}", s.overall, s.match10, s.fit.label(), s.opp.kind, s.opp.name, fmt_amount(&s.opp), dl, left);
        }
        return Ok(());
    }
    if let Some(p) = args.iter().position(|a| a == "--snapshot") {
        // render one frame to stdout (for docs/tests): --snapshot 160x45 [keys]
        let dims = args.get(p + 1).cloned().unwrap_or("160x45".into());
        let (w, h) = dims.split_once('x').map(|(a, b)| (a.parse().unwrap_or(160), b.parse().unwrap_or(45))).unwrap_or((160, 45));
        let mut app = App::new();
        for c in args.get(p + 2).cloned().unwrap_or_default().chars() {
            match c {
                'j' => app.move_sel(1),
                't' => { app.kind = (app.kind + 1) % KINDS.len(); app.refilter(); }
                'o' => { app.status = (app.status + 1) % STATUSES.len(); app.refilter(); }
                's' => { app.sort = (app.sort + 1) % SORTS.len(); app.refilter(); }
                'f' => { app.fit = (app.fit + 1) % FITS.len(); app.refilter(); }
                'm' => LIGHT_MODE.store(true, Ordering::Relaxed), // not saved: a snapshot changes no settings
                '/' => app.mode = Mode::Search,
                _ => {}
            }
        }
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h))?;
        t.draw(|f| ui(f, &mut app))?;
        let buf = t.backend().buffer().clone();
        for y in 0..h {
            let line: String = (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect();
            println!("{}", line.trim_end());
        }
        return Ok(());
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("callfit {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    load_theme();
    let term = ratatui::init();
    let res = run(term);
    ratatui::restore();
    res
}
