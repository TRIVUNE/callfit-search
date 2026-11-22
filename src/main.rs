//! callfit - find funding, open calls and residencies that fit Trivune.

mod live;
mod model;

use chrono::{Local, NaiveDate};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use model::*;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap},
    DefaultTerminal, Frame,
};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

// palette
const BORDER: Color = Color::Rgb(0x3b, 0x42, 0x61);
const DIM: Color = Color::Rgb(0x63, 0x6d, 0xa6);
const FG: Color = Color::Rgb(0xc8, 0xd3, 0xf5);
const GREEN: Color = Color::Rgb(0xc3, 0xe8, 0x8d);
const BLUE: Color = Color::Rgb(0x82, 0xaa, 0xff);
const CYAN: Color = Color::Rgb(0x86, 0xe1, 0xfc);
const YELLOW: Color = Color::Rgb(0xff, 0xc7, 0x77);
const RED: Color = Color::Rgb(0xff, 0x75, 0x7f);
const SEL: Color = Color::Rgb(0x2f, 0x33, 0x4d);

const KINDS: &[&str] = &["All", "Grant", "Prize", "Stipend", "Residency", "Fellowship", "Call for papers", "Open call", "Accelerator", "Live"];
const REGIONS: &[&str] = &["All", "Berlin", "DE", "EU", "World"];
const FITS: &[&str] = &["Eligible", "Perfect+Good", "All"];
const STATUSES: &[&str] = &["All", "Open now", "Upcoming", "Closed"];
const SORTS: &[&str] = &["Score", "Deadline", "Amount", "Name"];
const VIEWS: &[&str] = &["All", "Tracked"];

#[derive(PartialEq)]
enum Mode {
    Normal,
    Search,
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
}

impl App {
    fn new() -> Self {
        let today = Local::now().date_naive();
        let profile = load_profile();
        let (opps, user_entries) = load_opportunities();
        let all = opps.iter().map(|o| score(o, &profile, today)).collect();
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
            live_status: "press u".into(),
            live_count: 0,
            toast: None,
            detail_scroll: 0,
            user_entries,
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
        let q = self.query.to_lowercase();
        let mut idx: Vec<usize> = (0..self.all.len())
            .filter(|&i| {
                let s = &self.all[i];
                let o = &s.opp;
                if KINDS[self.kind] != "All" && o.kind != KINDS[self.kind] {
                    return false;
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
                    "Closed" if s.status != Status::Closed => return false,
                    _ => {}
                }
                if VIEWS[self.view] == "Tracked" && self.track(&o.id) == Track::None {
                    return false;
                }
                if !q.is_empty() {
                    let hay = format!(
                        "{} {} {} {} {} {} {}",
                        o.name, o.org, o.kind, o.city, o.country, o.summary, o.tags.join(" ")
                    )
                    .to_lowercase();
                    if !q.split_whitespace().all(|w| hay.contains(w)) {
                        return false;
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
        // drop old live rows
        self.all.retain(|s| s.opp.confidence != "live");
        self.live_count = 0;
        self.refilter();
    }

    fn poll_live(&mut self) {
        let mut changed = false;
        let mut finished = false;
        if let Some(rx) = &self.live_rx {
            while let Ok(m) = rx.try_recv() {
                match m {
                    live::Msg::Items(items) => {
                        for o in items {
                            if self.all.iter().any(|s| s.opp.apply_url == o.apply_url) {
                                continue;
                            }
                            self.all.push(score(&o, &self.profile, self.today));
                            self.live_count += 1;
                            changed = true;
                        }
                    }
                    live::Msg::Progress { done, total, .. } => {
                        self.live_status = format!("loading {}/{}", done, total);
                    }
                    live::Msg::Done { failed } => {
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

    fn export(&mut self) {
        let name = format!("callfit_{}.csv", Local::now().format("%Y%m%d_%H%M"));
        let path = std::env::current_dir().unwrap_or_default().join(&name);
        let esc = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
        let mut out = String::from("fit,score,name,org,type,where,amount,deadline,status,tracker,apply_url,info_url\n");
        for &i in &self.shown {
            let s = &self.all[i];
            let o = &s.opp;
            out.push_str(&format!(
                "{},{:.0},{},{},{},{},{},{},{},{},{},{}\n",
                s.fit.label(),
                s.overall,
                esc(&o.name),
                esc(&o.org),
                esc(&o.kind),
                esc(&where_text(o)),
                esc(&fmt_amount(o)),
                o.deadline.clone().unwrap_or_default(),
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

fn fit_color(f: FitLevel) -> Color {
    match f {
        FitLevel::Perfect => GREEN,
        FitLevel::Good => CYAN,
        FitLevel::Marginal => YELLOW,
        FitLevel::Low => DIM,
        FitLevel::NotEligible => RED,
    }
}

fn deadline_cell(s: &Scored) -> (String, String, Color) {
    match s.status {
        Status::Open => {
            let d = s.days_left.unwrap_or(0);
            let c = if d <= 7 { RED } else if d <= 30 { YELLOW } else { GREEN };
            (s.opp.deadline.clone().unwrap_or_default(), format!("{}d", d), c)
        }
        Status::Upcoming => (
            s.opp.deadline.clone().map(|d| format!("~{}", d)).unwrap_or_else(|| "upcoming".into()),
            format!("opens {}d", s.days_to_open.unwrap_or(0)),
            BLUE,
        ),
        Status::Rolling => ("rolling".into(), "anytime".into(), GREEN),
        Status::Closed => ("closed".into(), "see next".into(), DIM),
        Status::Unknown => ("check site".into(), "—".into(), DIM),
    }
}

fn boxed<'a>(title: &'a str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(format!(" {} ", title), Style::default().fg(DIM)))
}

fn ui(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(3), Constraint::Min(5), Constraint::Length(1)])
        .split(area);

    // ---- header
    let open_now = app.all.iter().filter(|s| matches!(s.status, Status::Open | Status::Rolling) && s.fit != FitLevel::NotEligible).count();
    let eligible = app.all.iter().filter(|s| s.fit != FitLevel::NotEligible).count();
    let tracked = app.tracker.values().filter(|t| **t != Track::None).count();
    let sep = Span::styled("  │  ", Style::default().fg(BORDER));
    let header = Line::from(vec![
        Span::styled("Profile: ", Style::default().fg(DIM)),
        Span::styled(format!("{} · {}, {}", app.profile.name, app.profile.city, app.profile.country), Style::default().fg(FG)),
        sep.clone(),
        Span::styled("Entries: ", Style::default().fg(DIM)),
        Span::styled(format!("{}", app.all.len()), Style::default().fg(CYAN)),
        sep.clone(),
        Span::styled("Eligible: ", Style::default().fg(DIM)),
        Span::styled(format!("{}", eligible), Style::default().fg(CYAN)),
        sep.clone(),
        Span::styled("Apply now: ", Style::default().fg(DIM)),
        Span::styled(format!("{}", open_now), Style::default().fg(GREEN)),
        sep.clone(),
        Span::styled("Tracked: ", Style::default().fg(DIM)),
        Span::styled(format!("{}", tracked), Style::default().fg(YELLOW)),
        sep.clone(),
        Span::styled("Live: ", Style::default().fg(DIM)),
        Span::styled(app.live_status.clone(), Style::default().fg(YELLOW)),
        sep,
        Span::styled(app.today.format("%d %b %Y").to_string(), Style::default().fg(DIM)),
    ]);
    let hb = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(" callfit ", Style::default().fg(GREEN).add_modifier(Modifier::BOLD)));
    f.render_widget(Paragraph::new(header).block(hb), rows[0]);

    // ---- filters
    let fc = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(20),
            Constraint::Length(17),
            Constraint::Length(12),
            Constraint::Length(16),
            Constraint::Length(13),
            Constraint::Length(12),
            Constraint::Length(12),
        ])
        .split(rows[1]);
    let search_style = if app.mode == Mode::Search { Style::default().fg(GREEN) } else { Style::default().fg(BORDER) };
    let qtext = if app.query.is_empty() && app.mode != Mode::Search {
        Span::styled("sound art, quantum, residency…", Style::default().fg(DIM))
    } else {
        Span::styled(
            format!("{}{}", app.query, if app.mode == Mode::Search { "▏" } else { "" }),
            Style::default().fg(FG),
        )
    };
    f.render_widget(Paragraph::new(qtext).block(boxed("Search [/]").border_style(search_style)), fc[0]);
    let vals = [
        ("Type [t]", KINDS[app.kind]),
        ("Region [r]", REGIONS[app.region]),
        ("Fit [f]", FITS[app.fit]),
        ("Status [o]", STATUSES[app.status]),
        ("Sort [s]", SORTS[app.sort]),
        ("View [v]", VIEWS[app.view]),
    ];
    for (i, (t, v)) in vals.iter().enumerate() {
        f.render_widget(
            Paragraph::new(Span::styled(*v, Style::default().fg(GREEN))).block(boxed(t)),
            fc[i + 1],
        );
    }

    // ---- main
    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(57), Constraint::Percentage(43)])
        .split(rows[2]);

    let header_row = Row::new(["", "Opportunity", "Type", "Amount", "Deadline", "Left", "Where", "Score"])
        .style(Style::default().fg(DIM).add_modifier(Modifier::BOLD));
    let rows_v: Vec<Row> = app
        .shown
        .iter()
        .map(|&i| {
            let s = &app.all[i];
            let o = &s.opp;
            let (dl, left, dc) = deadline_cell(s);
            let tr = app.track(&o.id);
            let mark = if tr != Track::None { "★" } else { "●" };
            Row::new(vec![
                Cell::from(Span::styled(mark, Style::default().fg(fit_color(s.fit)))),
                Cell::from(Span::styled(o.name.clone(), Style::default().fg(FG))),
                Cell::from(Span::styled(short_kind(&o.kind), Style::default().fg(CYAN))),
                Cell::from(Span::styled(fmt_amount(o), Style::default().fg(YELLOW))),
                Cell::from(Span::styled(dl, Style::default().fg(dc))),
                Cell::from(Span::styled(left, Style::default().fg(dc))),
                Cell::from(Span::styled(where_text(o), Style::default().fg(DIM))),
                Cell::from(Span::styled(format!("{:.0}", s.overall), Style::default().fg(fit_color(s.fit)))),
            ])
        })
        .collect();
    let title = format!("Opportunities ({}/{})", app.shown.len(), app.all.len());
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
        ],
    )
    .header(header_row)
    .row_highlight_style(Style::default().bg(SEL).add_modifier(Modifier::BOLD))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(BORDER))
            .title(Span::styled(format!(" {} ", title), Style::default().fg(BLUE))),
    );
    f.render_stateful_widget(table, main[0], &mut app.table);

    // ---- detail
    let detail = detail_lines(app);
    let db = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(
            format!(" {} ", app.current().map(|s| s.opp.name.as_str()).unwrap_or("Details")),
            Style::default().fg(FG).add_modifier(Modifier::BOLD),
        ));
    f.render_widget(
        Paragraph::new(detail).block(db).wrap(Wrap { trim: false }).scroll((app.detail_scroll, 0)),
        main[1],
    );

    // ---- footer
    let mode = if app.mode == Mode::Search { " SEARCH " } else { " NORMAL " };
    let mut foot = vec![
        Span::styled(mode, Style::default().fg(Color::Black).bg(GREEN).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
    ];
    if let Some((msg, at)) = &app.toast {
        if at.elapsed() < Duration::from_secs(4) {
            foot.push(Span::styled(msg.clone(), Style::default().fg(YELLOW)));
        }
    }
    if foot.len() == 2 {
        let keys = if app.mode == Mode::Search {
            "type to filter · Enter/Esc: done · Ctrl+U: clear"
        } else {
            "↑↓/jk:nav  Enter:apply  i:info  p:track  /:search  t r f o s v:filters  u:live  w:web  x:export  e:data  J/K:scroll  q:quit"
        };
        foot.push(Span::styled(keys, Style::default().fg(DIM)));
    }
    f.render_widget(Paragraph::new(Line::from(foot)), rows[3]);

    if app.all.is_empty() {
        let r = centered(area, 40, 5);
        f.render_widget(Clear, r);
        f.render_widget(Paragraph::new("No data").block(boxed("callfit")), r);
    }
}

fn centered(a: Rect, w: u16, h: u16) -> Rect {
    Rect { x: a.x + a.width.saturating_sub(w) / 2, y: a.y + a.height.saturating_sub(h) / 2, width: w.min(a.width), height: h.min(a.height) }
}

fn kv(k: &str, v: impl Into<String>, c: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!(" {:<14}", k), Style::default().fg(DIM)),
        Span::styled(v.into(), Style::default().fg(c)),
    ])
}

fn section(t: &str) -> Line<'static> {
    Line::from(Span::styled(format!(" ── {} ──", t), Style::default().fg(CYAN)))
}

fn detail_lines(app: &App) -> Vec<Line<'static>> {
    let Some(s) = app.current() else {
        return vec![Line::from(Span::styled(" Nothing matches these filters.", Style::default().fg(DIM)))];
    };
    let o = &s.opp;
    let mut l = vec![Line::raw("")];
    l.push(kv("Organisation:", o.org.clone(), FG));
    l.push(kv("Type:", o.kind.clone(), CYAN));
    l.push(kv("Where:", where_text(o), FG));
    l.push(kv("Open to:", scope_text(&o.scope).to_string(), FG));
    l.push(kv("Money:", fmt_amount(o), YELLOW));
    if !o.amount_note.is_empty() {
        l.push(Line::from(Span::styled(format!("                {}", o.amount_note), Style::default().fg(DIM))));
    }
    let (dl, left, dc) = deadline_cell(s);
    let deadline_txt = match s.status {
        Status::Open => format!("{}  ({} left)", dl, left.replace('d', " days")),
        Status::Upcoming => format!(
            "{}  (opens in {} days{})",
            o.deadline.clone().unwrap_or_else(|| "date tbc".into()),
            s.days_to_open.unwrap_or(0),
            if o.confidence == "pattern" { ", expected" } else { "" }
        ),
        _ => dl,
    };
    l.push(kv("Deadline:", deadline_txt, dc));
    if let Some(op) = &o.opens {
        l.push(kv("Opens:", op.clone(), FG));
    }
    if !o.cadence.is_empty() {
        l.push(kv("Cadence:", o.cadence.clone(), FG));
    }
    if !o.next_expected.is_empty() {
        l.push(kv("Next:", o.next_expected.clone(), FG));
    }
    let conf = match o.confidence.as_str() {
        "verified" => ("verified", GREEN),
        "pattern" => ("dates from yearly pattern", YELLOW),
        "live" => ("live listing, not checked", YELLOW),
        _ => ("check on site", RED),
    };
    l.push(kv("Data:", format!("{} · checked {}", conf.0, o.checked), conf.1));

    l.push(Line::raw(""));
    l.push(section("Score Breakdown"));
    l.push(Line::raw(""));
    l.push(Line::from(vec![
        Span::styled(" Overall:      ", Style::default().fg(DIM)),
        Span::styled(format!("{:.1} / 100", s.overall), Style::default().fg(fit_color(s.fit)).add_modifier(Modifier::BOLD)),
    ]));
    l.push(Line::from(vec![
        Span::styled(" Eligibility: ", Style::default().fg(DIM)),
        Span::styled(format!("{:<4}", s.eligibility), Style::default().fg(FG)),
        Span::styled("Relevance: ", Style::default().fg(DIM)),
        Span::styled(format!("{:<4}", s.relevance), Style::default().fg(FG)),
        Span::styled("Money: ", Style::default().fg(DIM)),
        Span::styled(format!("{:<4}", s.money), Style::default().fg(FG)),
        Span::styled("Timing: ", Style::default().fg(DIM)),
        Span::styled(format!("{}", s.timing), Style::default().fg(FG)),
    ]));
    if !s.matched.is_empty() {
        l.push(kv("Matches you:", s.matched.join(", "), GREEN));
    }

    l.push(Line::raw(""));
    l.push(section("Trivune Fit"));
    l.push(Line::raw(""));
    l.push(Line::from(vec![
        Span::styled(" Fit Level:    ", Style::default().fg(DIM)),
        Span::styled(format!("● {}", s.fit.label()), Style::default().fg(fit_color(s.fit)).add_modifier(Modifier::BOLD)),
    ]));
    for c in &s.checks {
        let (m, col) = if c.ok { ("✓", GREEN) } else { ("✗", RED) };
        l.push(Line::from(vec![
            Span::styled(format!("   {} ", m), Style::default().fg(col)),
            Span::styled(c.text.clone(), Style::default().fg(if c.ok { FG } else { DIM })),
        ]));
    }

    l.push(Line::raw(""));
    l.push(section("Links"));
    l.push(Line::raw(""));
    l.push(kv("Apply [Enter]", o.apply_url.clone(), BLUE));
    l.push(kv("Info  [i]", o.info_url.clone(), BLUE));

    l.push(Line::raw(""));
    l.push(section("Tracker"));
    l.push(Line::raw(""));
    let t = app.track(&o.id);
    l.push(kv("Status [p]:", t.label().to_string(), if t == Track::None { DIM } else { YELLOW }));

    if !o.summary.is_empty() {
        l.push(Line::raw(""));
        l.push(section("Notes"));
        l.push(Line::raw(""));
        l.push(Line::from(Span::styled(format!(" {}", o.summary), Style::default().fg(FG))));
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
    loop {
        app.poll_live();
        term.draw(|f| ui(f, &mut app))?;
        if !event::poll(Duration::from_millis(150))? {
            continue;
        }
        let Event::Key(k) = event::read()? else { continue };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        if app.mode == Mode::Search {
            match k.code {
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
                let q = if app.query.is_empty() { "open call sound art research".to_string() } else { app.query.clone() };
                let url = format!("https://duckduckgo.com/?q={}", urlencode(&format!("{} open call OR grant OR residency 2027", q)));
                open_url(&mut app, &url);
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

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".into(),
            _ => format!("%{:02X}", b),
        })
        .collect()
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--list") {
        // plain output, handy for scripts
        let app = App::new();
        for &i in &app.shown {
            let s = &app.all[i];
            let (dl, left, _) = deadline_cell(s);
            println!("{:>5.1}  {:<12} {:<52} {:<10} {:<12} {}", s.overall, s.fit.label(), s.opp.name, fmt_amount(&s.opp), dl, left);
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
    let term = ratatui::init();
    let res = run(term);
    ratatui::restore();
    res
}
