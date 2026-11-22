//! Data model, loading, and fit scoring.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

pub const BUILTIN_DATA: &str = include_str!("../data/opportunities.json");
pub const DEFAULT_PROFILE: &str = include_str!("../data/profile.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Opportunity {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub org: String,
    pub kind: String,
    #[serde(default)]
    pub city: String,
    #[serde(default)]
    pub country: String,
    /// Berlin | DE | EU | World
    pub scope: String,
    #[serde(default)]
    pub amount_min: u64,
    #[serde(default)]
    pub amount_max: u64,
    #[serde(default = "eur")]
    pub currency: String,
    #[serde(default)]
    pub amount_note: String,
    #[serde(default)]
    pub opens: Option<String>,
    #[serde(default)]
    pub deadline: Option<String>,
    /// When the event itself happens (festival, conference, exhibition), e.g. "2027-07-02 – 2027-07-04".
    #[serde(default)]
    pub event: String,
    #[serde(default)]
    pub rolling: bool,
    #[serde(default)]
    pub cadence: String,
    #[serde(default)]
    pub next_expected: String,
    #[serde(default)]
    pub entities: Vec<String>,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub apply_url: String,
    #[serde(default)]
    pub info_url: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub checked: String,
    /// verified | pattern | verify | live
    #[serde(default)]
    pub confidence: String,
    /// Shown in red: entry fee, region limit, pay-to-publish signals, unreadable page.
    #[serde(default)]
    pub warnings: Vec<String>,
}

fn eur() -> String {
    "EUR".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub city: String,
    pub country: String,
    /// Regions you count as: e.g. ["Berlin", "DE", "EU", "World"]
    pub regions: Vec<String>,
    /// How you can apply: individual, collective, gbr, company, nonprofit, institution
    pub entities: Vec<String>,
    /// Requirements you can meet (see README for the list)
    pub has: Vec<String>,
    /// What you do; used for relevance
    pub interests: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    Open,
    Rolling,
    Upcoming,
    Closed,
    Unknown,
}

impl Status {
    pub fn label(&self) -> &'static str {
        match self {
            Status::Open => "Open",
            Status::Rolling => "Rolling",
            Status::Upcoming => "Upcoming",
            Status::Closed => "Closed",
            Status::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FitLevel {
    Perfect,
    Good,
    Marginal,
    Low,
    NotEligible,
}

impl FitLevel {
    pub fn label(&self) -> &'static str {
        match self {
            FitLevel::Perfect => "Perfect",
            FitLevel::Good => "Good",
            FitLevel::Marginal => "Marginal",
            FitLevel::Low => "Low",
            FitLevel::NotEligible => "Not eligible",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Check {
    pub ok: bool,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Scored {
    pub opp: Opportunity,
    pub status: Status,
    pub days_left: Option<i64>,
    pub days_to_open: Option<i64>,
    pub eligibility: u32,
    pub relevance: u32,
    /// 0-10: how close the topic is to the profile's interests
    pub match10: u8,
    pub money: u32,
    pub timing: u32,
    pub overall: f32,
    pub fit: FitLevel,
    pub checks: Vec<Check>,
    pub matched: Vec<String>,
}

pub fn parse_date(s: &Option<String>) -> Option<NaiveDate> {
    s.as_deref()
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
}

pub fn requirement_label(r: &str) -> &str {
    match r {
        "open_source" => "Work is released as open source",
        "germany_based" => "Based in Germany",
        "berlin_residence" => "Main residence in Berlin",
        "berlin_brandenburg" => "Based in Berlin-Brandenburg",
        "artist_track_record" => "Several years of documented artistic practice",
        "cofinance_10" => "Can co-finance ~10% of costs",
        "finished_work" => "Has a finished or advanced work to submit",
        "venue_partner" => "Has a partner venue",
        "education_partner" => "Has a school/education partner",
        "international_partner" => "Has a partner abroad",
        "consortium" => "Has a multi-country consortium",
        "legal_entity" => "Is a registered legal entity",
        "institution" => "Is attached to a university/institution",
        "startup" => "Builds a company / market product",
        "incubator" => "Is accepted by an incubator",
        "product_idea" => "Has a media product idea",
        "critical_infrastructure" => "Maintains widely used infrastructure",
        "unverified" => "Live listing: eligibility not checked yet",
        other => other,
    }
}

fn norm(s: &str) -> String {
    s.to_lowercase()
}

pub fn score(o: &Opportunity, p: &Profile, today: NaiveDate) -> Scored {
    let mut checks = Vec::new();
    let mut hard_fail = false;

    // --- region
    let region_ok = p.regions.iter().any(|r| norm(r) == norm(&o.scope));
    checks.push(Check {
        ok: region_ok,
        text: format!("Region: open to {}", scope_text(&o.scope)),
    });
    if !region_ok {
        hard_fail = true;
    }

    // --- entity
    let ent_ok = o.entities.is_empty()
        || o.entities.iter().any(|e| p.entities.iter().any(|x| norm(x) == norm(e)));
    checks.push(Check {
        ok: ent_ok,
        text: format!(
            "Applicant type: {}",
            if o.entities.is_empty() { "any".to_string() } else { o.entities.join(", ") }
        ),
    });
    if !ent_ok {
        hard_fail = true;
    }

    // --- requirements
    let mut missing = 0u32;
    for r in &o.requires {
        let ok = p.has.iter().any(|h| norm(h) == norm(r));
        if !ok {
            missing += 1;
        }
        checks.push(Check { ok, text: requirement_label(r).to_string() });
    }
    let eligibility = if hard_fail { 0 } else { 100u32.saturating_sub(missing * 35) };

    // --- relevance / match 0-10. Half points: an interest in the name = 4, in summary or tags = 2;
    // broad interests (research, software...) count half. 12 half points = 10/10.
    const BROAD: &[&str] = &["digital", "research", "software", "tools", "education", "workshops", "interfaces", "instruments", "plugin", "music", "immersive"];
    let fold = crate::live::fold;
    let name_f = fold(&o.name);
    let rest_f = fold(&format!("{} {}", o.summary, o.tags.join(" ")));
    let mut points = 0u32;
    let mut matched = Vec::new();
    for i in &p.interests {
        let f = fold(i);
        if f.is_empty() {
            continue;
        }
        let w = if BROAD.contains(&f.as_str()) { 1 } else { 2 };
        if name_f.contains(&f) {
            points += 2 * w;
        } else if rest_f.contains(&f) {
            points += w;
        } else {
            continue;
        }
        matched.push(i.clone());
    }
    let match10 = (points * 10 / 12).min(10) as u8;
    let relevance = match10 as u32 * 10;

    // --- money (rough EUR)
    let amt = o.amount_max.max(o.amount_min) as f64;
    let money = if amt <= 0.0 {
        if o.kind == "Call for papers" { 20 } else { 35 }
    } else {
        // 1k -> 30, 5k -> 60, 20k -> 85, 50k+ -> 100
        ((amt.log10() - 3.0) / (4.7 - 3.0) * 70.0 + 30.0).clamp(20.0, 100.0) as u32
    };

    // --- timing
    let dl = parse_date(&o.deadline);
    let op = parse_date(&o.opens);
    let days_left = dl.map(|d| (d - today).num_days());
    let days_to_open = op.map(|d| (d - today).num_days());
    let status = if let Some(dleft) = days_left {
        if dleft < 0 {
            if o.rolling { Status::Rolling } else { Status::Closed }
        } else if days_to_open.map(|x| x > 0).unwrap_or(false) {
            Status::Upcoming
        } else {
            Status::Open
        }
    } else if o.rolling {
        Status::Rolling
    } else if days_to_open.map(|x| x > 0).unwrap_or(false) {
        Status::Upcoming
    } else {
        Status::Unknown
    };
    let timing = match status {
        Status::Open => match days_left.unwrap_or(99) {
            d if d >= 21 => 100,
            d if d >= 7 => 80,
            d if d >= 2 => 55,
            _ => 35,
        },
        Status::Rolling => 90,
        Status::Upcoming => match days_to_open.unwrap_or(999) {
            d if d <= 60 => 85,
            d if d <= 180 => 70,
            _ => 50,
        },
        Status::Unknown => 40,
        Status::Closed => 15,
    };

    let overall = 0.40 * eligibility as f32
        + 0.30 * relevance as f32
        + 0.15 * money as f32
        + 0.15 * timing as f32;

    let fit = if hard_fail {
        FitLevel::NotEligible
    } else if overall >= 80.0 && missing == 0 && timing >= 50 {
        FitLevel::Perfect
    } else if overall >= 62.0 {
        FitLevel::Good
    } else if overall >= 45.0 {
        FitLevel::Marginal
    } else {
        FitLevel::Low
    };

    Scored {
        opp: o.clone(),
        status,
        days_left,
        days_to_open,
        eligibility,
        relevance,
        match10,
        money,
        timing,
        overall,
        fit,
        checks,
        matched,
    }
}

pub fn scope_text(s: &str) -> &str {
    match s {
        "World" => "applicants worldwide",
        "EU" => "EU-based applicants",
        "DE" => "Germany-based applicants",
        "Berlin" => "Berlin-based applicants",
        "Restricted" => "a region that excludes you",
        other => other,
    }
}

pub fn fmt_amount(o: &Opportunity) -> String {
    let sym = match o.currency.as_str() {
        "EUR" => "€",
        "USD" => "$",
        "GBP" => "£",
        _ => "",
    };
    let k = |v: u64| -> String {
        if v >= 1000 {
            let x = v as f64 / 1000.0;
            if (x - x.round()).abs() < 0.05 { format!("{}{:.0}k", sym, x) } else { format!("{}{:.1}k", sym, x) }
        } else {
            format!("{}{}", sym, v)
        }
    };
    match (o.amount_min, o.amount_max) {
        (0, 0) => if o.kind == "Call for papers" { "—".into() } else { "varies".into() },
        (a, 0) => format!("{}+", k(a)),
        (0, b) => format!("≤{}", k(b)),
        (a, b) if a == b => k(a),
        (a, b) => format!("{}–{}", k(a), k(b)),
    }
}

// ---------- persistence ----------

pub fn data_dir() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    let d = base.join("callfit");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn load_profile() -> Profile {
    let path = data_dir().join("profile.json");
    if let Ok(s) = std::fs::read_to_string(&path) {
        if let Ok(p) = serde_json::from_str(&s) {
            return p;
        }
    } else {
        let _ = std::fs::write(&path, DEFAULT_PROFILE);
    }
    serde_json::from_str(DEFAULT_PROFILE).expect("built-in profile is valid")
}

/// Built-in list, overridden/extended by data_dir/opportunities.json (merged by id).
pub fn load_opportunities() -> (Vec<Opportunity>, usize) {
    let mut list: Vec<Opportunity> = serde_json::from_str(BUILTIN_DATA).expect("built-in data is valid");
    let mut extra = 0;
    let path = data_dir().join("opportunities.json");
    if let Ok(s) = std::fs::read_to_string(&path) {
        if let Ok(user) = serde_json::from_str::<Vec<Opportunity>>(&s) {
            for o in user {
                extra += 1;
                if let Some(slot) = list.iter_mut().find(|x| x.id == o.id) {
                    *slot = o;
                } else {
                    list.push(o);
                }
            }
        }
    }
    (list, extra)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Track {
    None,
    Watching,
    Drafting,
    Submitted,
    Won,
    Rejected,
}

impl Track {
    pub fn next(self) -> Track {
        match self {
            Track::None => Track::Watching,
            Track::Watching => Track::Drafting,
            Track::Drafting => Track::Submitted,
            Track::Submitted => Track::Won,
            Track::Won => Track::Rejected,
            Track::Rejected => Track::None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Track::None => "—",
            Track::Watching => "Watching",
            Track::Drafting => "Drafting",
            Track::Submitted => "Submitted",
            Track::Won => "Won",
            Track::Rejected => "Rejected",
        }
    }
}

pub fn load_tracker() -> HashMap<String, Track> {
    std::fs::read_to_string(data_dir().join("tracker.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_tracker(t: &HashMap<String, Track>) {
    if let Ok(s) = serde_json::to_string_pretty(t) {
        let _ = std::fs::write(data_dir().join("tracker.json"), s);
    }
}
