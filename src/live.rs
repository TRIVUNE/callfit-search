//! Live listings: pull call links from listing pages in a background thread.

use crate::model::{data_dir, Opportunity};
use chrono::NaiveDate;
use regex::Regex;
use std::sync::mpsc::Sender;
use std::time::Duration;

pub const FEEDS: &[(&str, &str)] = &[
    ("opencalls.ai", "https://opencalls.ai/"),
    ("field notes Berlin", "https://field-notes.berlin/en/funding"),
    ("On the Move", "https://on-the-move.org/news"),
    ("TransArtists", "https://www.transartists.org/en/call-for-artists"),
    ("Res Artis", "https://resartis.org/open-calls/"),
    ("ArtConnect", "https://www.artconnect.com/opportunities"),
    ("CTM Festival", "https://www.ctm-festival.de/festival-2027/open-calls"),
    ("Kreativ Kultur Berlin", "https://www.kreativkultur.berlin/en/funding/"),
    ("Goethe-Institut", "https://www.goethe.de/en/kul/foe.html"),
    ("ZKM", "https://zkm.de/en/open-calls"),
    ("NLnet news", "https://nlnet.nl/news/"),
    ("Sovereign Tech Agency", "https://www.sovereign.tech/news"),
    ("Unitary Foundation", "https://unitary.foundation"),
    ("Berlin Senate culture deadlines", "https://www.berlin.de/sen/kultur/foerderung/antragsfristen/"),
    ("Musikfonds", "https://www.musikfonds.de"),
    ("Akademie der Kuenste", "https://www.adk.de/en/"),
];

pub enum Msg {
    #[allow(dead_code)]
    Progress { done: usize, total: usize, source: String, ok: bool },
    Items(Vec<Opportunity>),
    Done { failed: Vec<String> },
}

fn feeds() -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = FEEDS.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    // user feeds: data_dir/feeds.json = [["Name","https://..."], ...]
    if let Ok(s) = std::fs::read_to_string(data_dir().join("feeds.json")) {
        if let Ok(extra) = serde_json::from_str::<Vec<(String, String)>>(&s) {
            v.extend(extra);
        }
    }
    v
}

pub fn spawn(tx: Sender<Msg>, today: NaiveDate) {
    std::thread::spawn(move || {
        let list = feeds();
        let total = list.len();
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(15))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) callfit/0.1")
            .build();
        let mut failed = Vec::new();
        let (rtx, rrx) = std::sync::mpsc::channel();
        // 6 workers
        let chunks: Vec<Vec<(String, String)>> = list.chunks((total + 5) / 6).map(|c| c.to_vec()).collect();
        for chunk in chunks {
            let rtx = rtx.clone();
            let agent = agent.clone();
            std::thread::spawn(move || {
                for (name, url) in chunk {
                    let res = agent
                        .get(&url)
                        .call()
                        .ok()
                        .and_then(|r| {
                            let final_url = r.get_url().to_string();
                            r.into_string().ok().map(|b| (final_url, b))
                        });
                    let _ = rtx.send((name, res));
                }
            });
        }
        drop(rtx);
        let mut done = 0;
        for (name, res) in rrx {
            done += 1;
            match res {
                Some((final_url, body)) => {
                    let items = harvest(&name, &final_url, &body, today);
                    let _ = tx.send(Msg::Items(items));
                    let _ = tx.send(Msg::Progress { done, total, source: name, ok: true });
                }
                None => {
                    failed.push(name.clone());
                    let _ = tx.send(Msg::Progress { done, total, source: name, ok: false });
                }
            }
        }
        let _ = tx.send(Msg::Done { failed });
    });
}

fn strip_tags(s: &str) -> String {
    let re = Regex::new(r"(?s)<script.*?</script>|<style.*?</style>|<[^>]+>").unwrap();
    let t = re.replace_all(s, " ");
    let t = t
        .replace("&amp;", "&")
        .replace("&nbsp;", " ")
        .replace("&#8211;", "–")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&rsquo;", "'");
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn host(url: &str) -> String {
    url.split("://").nth(1).unwrap_or(url).split('/').next().unwrap_or("").to_lowercase()
}

fn join_url(base: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }
    let scheme_host = {
        let mut parts = base.splitn(4, '/');
        let s = parts.next().unwrap_or("https:");
        let _ = parts.next();
        let h = parts.next().unwrap_or("");
        format!("{}//{}", s, h)
    };
    if href.starts_with("//") {
        return format!("https:{}", href);
    }
    if href.starts_with('/') {
        return format!("{}{}", scheme_host, href);
    }
    let dir = match base.rfind('/') {
        Some(i) if i > scheme_host.len() => &base[..=i],
        _ => return format!("{}/{}", scheme_host, href),
    };
    format!("{}{}", dir, href)
}

pub fn harvest(source: &str, base: &str, page: &str, today: NaiveDate) -> Vec<Opportunity> {
    let a_re = Regex::new(r#"(?is)<a\b[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#).unwrap();
    let kw = Regex::new(r"(?i)open[- ]?call|call[- ]for|ausschreibung|residen|stipend|scholarship|grant|fellowship|funding|förder|foerder|award|prize|preis|opportunit|apply|bewerb|commission|wettbewerb|competition|microgrant").unwrap();
    let skip = Regex::new(r"(?i)login|signin|sign-up|register|privacy|datenschutz|impressum|imprint|cookie|newsletter|facebook|instagram|twitter|linkedin|mailto:|javascript:|^#").unwrap();
    let base_host = host(base);
    let mut out: Vec<Opportunity> = Vec::new();
    for c in a_re.captures_iter(page) {
        let href = c.get(1).unwrap().as_str().replace("&amp;", "&");
        if skip.is_match(&href) {
            continue;
        }
        let title = strip_tags(c.get(2).unwrap().as_str());
        if title.len() < 12 || title.len() > 180 {
            continue;
        }
        let url = join_url(base, &href);
        if host(&url) != base_host {
            continue;
        }
        let path = url.split(&base_host).nth(1).unwrap_or("");
        if path.trim_matches('/').is_empty() || url.trim_end_matches('/') == base.trim_end_matches('/') {
            continue;
        }
        let end = c.get(0).unwrap().end();
        let next = page[end..].find("<a ").map(|i| end + i).unwrap_or((end + 700).min(page.len()));
        let stop = next.min((end + 700).min(page.len()));
        let mut snippet = strip_tags(page.get(end..stop).unwrap_or(""));
        snippet.truncate(snippet.char_indices().nth(300).map(|(i, _)| i).unwrap_or(snippet.len()));
        if !(kw.is_match(&title) || kw.is_match(path) || kw.is_match(&snippet)) {
            continue;
        }
        if out.iter().any(|o| o.apply_url == url) {
            continue;
        }
        let text = format!("{} {}", title, snippet);
        let deadline = find_date(&text, today);
        let (amin, amax, cur) = find_amount(&text);
        let id = format!("live:{}", url);
        out.push(Opportunity {
            id,
            name: title,
            org: source.to_string(),
            kind: "Live".into(),
            city: String::new(),
            country: String::new(),
            scope: "World".into(),
            amount_min: amin,
            amount_max: amax,
            currency: cur,
            amount_note: if amax > 0 { "Amount read from listing text - verify".into() } else { "Not stated in listing".into() },
            opens: None,
            deadline: deadline.map(|d| d.format("%Y-%m-%d").to_string()),
            rolling: false,
            cadence: String::new(),
            next_expected: String::new(),
            entities: vec![],
            requires: vec!["unverified".into()],
            tags: guess_tags(&text),
            apply_url: url.clone(),
            info_url: base.to_string(),
            summary: snippet,
            checked: today.format("%Y-%m-%d").to_string(),
            confidence: "live".into(),
        });
        if out.len() >= 30 {
            break;
        }
    }
    out
}

fn guess_tags(text: &str) -> Vec<String> {
    let t = text.to_lowercase();
    let words = [
        "sound art", "sound", "audio", "music", "media art", "digital", "residency", "research",
        "open source", "software", "quantum", "installation", "audiovisual", "workshop",
        "art science", "spatial", "electroacoustic", "mathematics", "algorithm", "machine learning", "ai",
    ];
    words.iter().filter(|w| t.contains(*w)).map(|w| w.to_string()).collect()
}

fn month_num(m: &str) -> Option<u32> {
    let m = m.to_lowercase();
    let table = [
        ("jan", 1), ("feb", 2), ("mar", 3), ("mär", 3), ("maer", 3), ("apr", 4), ("may", 5), ("mai", 5),
        ("jun", 6), ("jul", 7), ("aug", 8), ("sep", 9), ("oct", 10), ("okt", 10), ("nov", 11), ("dec", 12), ("dez", 12),
    ];
    table.iter().find(|(k, _)| m.starts_with(k)).map(|(_, v)| *v)
}

pub fn find_date(text: &str, today: NaiveDate) -> Option<NaiveDate> {
    let mut dates = Vec::new();
    let r1 = Regex::new(r"\b(\d{1,2})\.\s?(\d{1,2})\.\s?(20\d\d)\b").unwrap();
    for c in r1.captures_iter(text) {
        if let (Ok(d), Ok(m), Ok(y)) = (c[1].parse(), c[2].parse(), c[3].parse()) {
            dates.extend(NaiveDate::from_ymd_opt(y, m, d));
        }
    }
    let r2 = Regex::new(r"\b(20\d\d)-(\d\d)-(\d\d)\b").unwrap();
    for c in r2.captures_iter(text) {
        if let (Ok(y), Ok(m), Ok(d)) = (c[1].parse(), c[2].parse(), c[3].parse()) {
            dates.extend(NaiveDate::from_ymd_opt(y, m, d));
        }
    }
    let r3 = Regex::new(r"(?i)\b(\d{1,2})\.?\s+([a-zäÄ]{3,9})\.?\s+(20\d\d)\b").unwrap();
    for c in r3.captures_iter(text) {
        if let (Ok(d), Some(m), Ok(y)) = (c[1].parse(), month_num(&c[2]), c[3].parse()) {
            dates.extend(NaiveDate::from_ymd_opt(y, m, d));
        }
    }
    let r4 = Regex::new(r"(?i)\b([a-z]{3,9})\.?\s+(\d{1,2}),?\s+(20\d\d)\b").unwrap();
    for c in r4.captures_iter(text) {
        if let (Some(m), Ok(d), Ok(y)) = (month_num(&c[1]), c[2].parse(), c[3].parse()) {
            dates.extend(NaiveDate::from_ymd_opt(y, m, d));
        }
    }
    // prefer the nearest future date; else the latest past one
    dates.sort();
    dates.iter().find(|d| **d >= today).copied().or_else(|| dates.last().copied())
}

pub fn find_amount(text: &str) -> (u64, u64, String) {
    let re = Regex::new(r"(?i)(€|eur|euro|\$|usd)\s?(\d{1,3}(?:[.,\s]\d{3})+|\d+(?:[.,]\d+)?\s?[kK]?)|(\d{1,3}(?:[.,\s]\d{3})+|\d+\s?[kK])\s?(€|eur|euro|usd|\$)").unwrap();
    let mut best = 0u64;
    let mut cur = "EUR".to_string();
    for c in re.captures_iter(text) {
        let (sym, num) = if let Some(n) = c.get(2) {
            (c.get(1).unwrap().as_str(), n.as_str())
        } else {
            (c.get(4).unwrap().as_str(), c.get(3).unwrap().as_str())
        };
        let k = num.to_lowercase().contains('k');
        let digits: String = num.chars().filter(|ch| ch.is_ascii_digit()).collect();
        if let Ok(mut v) = digits.parse::<u64>() {
            if k {
                v *= 1000;
            }
            if v > best && v < 10_000_000 {
                best = v;
                cur = if sym.contains('$') || sym.to_lowercase() == "usd" { "USD".into() } else { "EUR".into() };
            }
        }
    }
    (0, best, cur)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn harvests_calls() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 22).unwrap();
        let page = r#"<nav><a href="/login">Login to your account now</a></nav>
<div><a href="/opencalls/sound-art-residency-2027">Sound Art Residency 2027 Berlin</a><p>Deadline: 15.01.2027. Fee EUR 3,000 for sound artists.</p></div>
<div><a href="https://x.org/calls/media">Media award open call</a><p>Apply until 31 December 2026, prize €10k</p></div>
<a href="/about">About us and our team here</a>"#;
        let v = harvest("Test", "https://x.org/", page, today);
        assert_eq!(v.len(), 2, "{:?}", v.iter().map(|o| &o.name).collect::<Vec<_>>());
        assert_eq!(v[0].deadline.as_deref(), Some("2027-01-15"));
        assert_eq!(v[0].amount_max, 3000);
        assert_eq!(v[0].apply_url, "https://x.org/opencalls/sound-art-residency-2027");
        assert_eq!(v[1].deadline.as_deref(), Some("2026-12-31"));
        assert_eq!(v[1].amount_max, 10000);
    }
}
