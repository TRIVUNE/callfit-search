//! The list filter (`/`): words, "phrases" and -exclusions, matched at word starts, across the
//! 8 crawl languages, with one typo forgiven when nothing matches exactly.
//! Skips filler words, matches word forms, and offers a loose mode that matches only some words.

use crate::live::fold;

/// Words that mean the same in EN DE FR ES IT NL PT PL (plus Nordic where common). A search word
/// that finds one member also finds the others. Written plain; `fold` runs on them.
const SYNONYMS: &[&[&str]] = &[
    &["grant", "funding", "förderung", "förderprogramm", "zuschuss", "subvention", "financement", "subvencion", "ayuda", "sovvenzione", "finanziamento", "contributo", "subsidie", "dofinansowanie", "financiamento", "fomento", "money", "cash", "geld", "argent", "dinero", "soldi", "pieniądze"],
    &["stipend", "scholarship", "stipendium", "bourse", "beca", "borsa", "beurs", "stypendium", "bolsa"],
    &["residency", "residence", "residencies", "residenz", "residencia", "residenza", "residentie", "rezydencja", "residência", "artist in residence"],
    &["prize", "award", "preis", "auszeichnung", "prix", "premio", "prijs", "nagroda", "prémio"],
    &["competition", "contest", "wettbewerb", "concours", "concurso", "concorso", "wedstrijd", "konkurs"],
    &["open call", "call for entries", "call for submissions", "call for works", "call for artists", "ausschreibung", "appel à candidatures", "appel à projets", "convocatoria", "bando", "open oproep", "oproep", "nabór", "edital", "convocatória"],
    &["call for papers", "cfp", "call for abstracts", "call for contributions", "appel à communications", "appel à contributions", "call for proposals"],
    &["fellowship", "fellow", "forschungsstipendium", "beca de investigacion", "borsa di ricerca"],
    &["hackathon", "hackaton", "game jam", "codefest"],
    &["conference", "konferenz", "tagung", "kongress", "congrès", "conferencia", "congreso", "conferenza", "convegno", "congresso", "conferentie", "konferencja", "symposium"],
    &["festival", "festiwal", "festivals"],
    &["workshop", "atelier", "taller", "laboratorio", "warsztaty"],
    &["summer school", "sommerakademie", "sommerschule", "école d'été", "escuela de verano", "scuola estiva", "zomerschool", "szkoła letnia", "escola de verão"],
    &["accelerator", "incubator", "inkubator", "accélérateur", "aceleradora", "incubadora", "acceleratore", "incubatore", "akcelerator"],
    &["art", "arts", "kunst", "arte", "sztuka", "konst", "kunsten"],
    &["artist", "künstler", "künstlerin", "artiste", "artista", "kunstenaar", "artysta", "konstnär", "kunstner"],
    &["sound", "klang", "sonido", "suono", "geluid", "dźwięk", "ljud", "lyd", "sonore", "sonoro", "sonora"],
    &["sound art", "klangkunst", "art sonore", "arte sonoro", "arte sonora", "geluidskunst", "sztuka dźwięku", "ljudkonst", "lydkunst", "audio art"],
    &["music", "musik", "musique", "musica", "música", "muziek", "muzyka", "musikk", "musician", "musicians", "musiker", "musicien", "músico", "musicista", "muzikant", "muzyk"],
    &["composition", "composer", "komposition", "komponist", "compositeur", "compositor", "compositore", "componist", "kompozytor", "kompozycja", "composicion", "composizione", "compositie", "call for scores"],
    &["electronic music", "elektronische musik", "musique électronique", "música electrónica", "musica elettronica", "elektronische muziek", "muzyka elektroniczna", "electroacoustic", "elektroakustisch", "électroacoustique", "electroacústica", "elettroacustica"],
    &["media art", "medienkunst", "new media", "digital art", "arts numériques", "art numérique", "arte digital", "arte digitale", "digitale kunst", "sztuka cyfrowa", "sztuka mediów", "mediakunst"],
    &["installation", "installazione", "instalación", "installatie", "instalacja", "instalação"],
    &["science", "wissenschaft", "ciencia", "scienza", "wetenschap", "nauka", "ciência"],
    &["research", "forschung", "recherche", "investigacion", "ricerca", "onderzoek", "badania", "pesquisa"],
    &["technology", "technologie", "tecnologia", "technologia", "tecnología", "technik"],
    &["mathematics", "math", "maths", "mathematik", "mathématiques", "matemáticas", "matematica", "wiskunde", "matematyka"],
    &["quantum", "quanten", "quantique", "cuántica", "quantistica", "kwantum", "kwantowy", "quântica"],
    &["university", "universität", "hochschule", "université", "universidad", "università", "universiteit", "uniwersytet", "universidade"],
    &["conservatory", "musikhochschule", "konservatorium", "conservatoire", "conservatorio", "conservatorium", "konserwatorium"],
    &["game", "games", "spiel", "spiele", "jeu", "jeux", "videojuego", "videogioco", "gra", "jogo", "videogame"],
    &["exhibition", "ausstellung", "exposition", "exposición", "mostra", "tentoonstelling", "wystawa", "exposição"],
    &["concert", "konzert", "concierto", "concerto", "koncert"],
    &["artificial intelligence", "ai", "ki", "künstliche intelligenz", "intelligence artificielle", "inteligencia artificial", "intelligenza artificiale", "sztuczna inteligencja", "machine learning"],
    &["virtual reality", "vr", "xr", "virtuelle realität", "réalité virtuelle", "realidad virtual", "realtà virtuale", "immersive", "immersiv", "immersif", "inmersivo", "immersivo"],
    &["signal processing", "dsp", "signalverarbeitung", "traitement du signal", "procesamiento de señales", "elaborazione del segnale"],
    &["open source", "quelloffen", "código abierto", "logiciel libre", "software libre"],
    &["germany", "deutschland", "allemagne", "alemania", "germania", "duitsland", "niemcy", "alemanha"],
    &["europe", "europa", "europäisch", "européen", "europeo", "europese", "europejski", "european"],
    &["young", "emerging", "nachwuchs", "émergent", "emergente", "jong", "młody"],
];

/// Filler words ignored unless quoted or negated. Written folded; `fold` runs on query words.
const STOPWORDS: &[&str] = &[
    // en
    "a", "an", "the", "and", "or", "for", "of", "to", "in", "on", "at", "with", "by", "from", "my", "our", "your", "about",
    // de
    "der", "die", "das", "und", "oder", "fur", "von", "zu", "im", "am", "mit", "ein", "eine",
    // fr
    "le", "la", "les", "des", "du", "de", "et", "ou", "pour", "un", "une", "avec", "en", "au", "aux",
    // es
    "el", "los", "las", "y", "o", "para", "del", "con",
    // it
    "il", "lo", "gli", "e", "per", "di",
    // nl
    "het", "een", "voor", "van", "met", "op",
    // pt
    "os", "as", "do", "da", "com", "um", "uma", "em",
    // pl
    "i", "oraz", "dla", "w", "z", "na",
];

/// One way a search term can match: words in a row, or (an unquoted pair like `sound art`) each
/// anywhere.
struct Alt {
    words: Vec<String>,
    in_a_row: bool,
    /// typed by the user: short words match as a prefix and typos apply; translations need more
    typed: bool,
}

struct Term {
    alts: Vec<Alt>,
    neg: bool,
}

pub struct Query {
    terms: Vec<Term>,
    /// forgive one typo (used when nothing matches exactly)
    fuzzy: bool,
    /// match entries that contain only some of the words
    any: bool,
}

fn words(s: &str) -> Vec<String> {
    fold(s).split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

/// Do two words share a long enough prefix to be forms of the same word?
fn same_stem(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len() < 5 || b.len() < 5 {
        return false;
    }
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    common >= 5 && common * 4 >= a.len().min(b.len()) * 3
}

/// Damerau-Levenshtein from `p` to the closest prefix of `t` is at most 1.
fn close(p: &str, t: &str) -> bool {
    let p: Vec<char> = p.chars().collect();
    let t: Vec<char> = t.chars().collect();
    if t.len() + 1 < p.len() {
        return false;
    }
    let mut pp: Vec<usize> = vec![];
    let mut prev: Vec<usize> = (0..=t.len()).collect();
    for i in 1..=p.len() {
        let mut cur = vec![i; t.len() + 1];
        for j in 1..=t.len() {
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + (p[i - 1] != t[j - 1]) as usize);
            if i > 1 && j > 1 && p[i - 1] == t[j - 2] && p[i - 2] == t[j - 1] {
                cur[j] = cur[j].min(pp[j - 2] + 1);
            }
        }
        pp = std::mem::replace(&mut prev, cur);
    }
    prev.into_iter().min().unwrap_or(usize::MAX) <= 1
}

/// How well search word `p` matches text word `t`: 3 same word, 2 word start, 1 inside or typo, 0 no.
fn word_q(p: &str, t: &str, typed: bool, fuzzy: bool) -> u32 {
    if t == p {
        3
    } else if (typed || p.len() >= 4) && t.starts_with(p) || same_stem(p, t) {
        2
    } else if p.len() >= 5 && t.contains(p) {
        1 // inside a compound: "kunst" in "klangkunst"
    } else if fuzzy && typed && p.len() >= 5 && close(p, t) {
        1
    } else {
        0
    }
}

impl Query {
    pub fn parse(q: &str, fuzzy: bool) -> Query {
        // (negated, words, quoted)
        let mut items: Vec<(bool, Vec<String>, bool)> = vec![];
        let mut rest = q.trim_start();
        while !rest.is_empty() {
            let mut neg = false;
            if let Some(r) = rest.strip_prefix('-').filter(|r| r.starts_with(|c: char| !c.is_whitespace())) {
                neg = true;
                rest = r;
            }
            let (text, quoted, after) = match rest.strip_prefix('"') {
                Some(r) => {
                    let end = r.find('"').unwrap_or(r.len());
                    (&r[..end], true, r.get(end + 1..).unwrap_or(""))
                }
                None => {
                    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                    (&rest[..end], false, &rest[end..])
                }
            };
            let w = words(text);
            if !w.is_empty() {
                items.push((neg, w, quoted));
            }
            rest = after.trim_start();
        }

        let groups: Vec<Vec<Vec<String>>> = SYNONYMS.iter().map(|g| g.iter().map(|m| words(m)).collect()).collect();
        let expand = |hit: &dyn Fn(&Vec<String>) -> bool| -> Vec<Alt> {
            groups
                .iter()
                .filter(|g| g.iter().any(|m| hit(m)))
                .flatten()
                .map(|m| Alt { words: m.clone(), in_a_row: true, typed: false })
                .collect()
        };

        let mut terms = vec![];
        let mut i = 0;
        while i < items.len() {
            let (neg, w, quoted) = &items[i];
            // unquoted words that together name a group member ("sound art", "open call"): one term
            let mut joined = None;
            for n in [3, 2] {
                let run = items.get(i..i + n).filter(|r| r.iter().all(|(ng, w, qt)| !ng && !qt && w.len() == 1));
                if let Some(run) = run {
                    let ws: Vec<String> = run.iter().map(|x| x.1[0].clone()).collect();
                    let alts = expand(&|m| *m == ws);
                    if !alts.is_empty() {
                        joined = Some((n, ws, alts));
                        break;
                    }
                }
            }
            let (n, alts) = match joined {
                Some((n, ws, mut alts)) => {
                    alts.insert(0, Alt { words: ws, in_a_row: false, typed: true });
                    (n, alts)
                }
                None => {
                    if !*neg && !*quoted && w.len() == 1 && STOPWORDS.contains(&w[0].as_str()) {
                        i += 1;
                        continue;
                    }
                    let mut alts = vec![Alt { words: w.clone(), in_a_row: true, typed: true }];
                    // a word finds a group by a member's start, a phrase only by a whole member
                    let p = &w[0];
                    alts.extend(expand(&|m| {
                        m == w || (w.len() == 1 && m.len() == 1 && (p.len() >= 4 && m[0].starts_with(p.as_str()) || fuzzy && p.len() >= 5 && close(p, &m[0]) || same_stem(p, &m[0])))
                    }));
                    (1, alts)
                }
            };
            let mut seen = std::collections::HashSet::new();
            let alts = alts.into_iter().filter(|a| seen.insert(a.words.clone())).collect();
            terms.push(Term { alts, neg: *neg });
            i += n;
        }
        Query { terms, fuzzy, any: false }
    }

    /// Match entries containing only some of the words; each match adds to the score.
    pub fn loose(mut self) -> Query {
        self.any = true;
        self
    }

    pub fn is_loose(&self) -> bool {
        self.any
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Best score of one alternative over the weighted fields; 0 = no match.
    fn alt_score(&self, a: &Alt, fields: &[(u32, Vec<String>)]) -> u32 {
        let wq = |p: &str, t: &str| word_q(p, t, a.typed, self.fuzzy);
        if a.in_a_row {
            let mut best = 0;
            for (w, toks) in fields {
                for win in toks.windows(a.words.len()) {
                    best = best.max(w * a.words.iter().zip(win).map(|(p, t)| wq(p, t)).min().unwrap_or(0));
                }
            }
            best
        } else {
            let mut worst = u32::MAX;
            for p in &a.words {
                let mut best = 0;
                for (w, toks) in fields {
                    for t in toks {
                        best = best.max(w * wq(p, t));
                    }
                }
                worst = worst.min(best);
            }
            worst
        }
    }

    /// None: no match. Some(n): how well it matches, higher is better. `fields`: (weight, text).
    pub fn score(&self, fields: &[(u32, &str)]) -> Option<u32> {
        if self.terms.is_empty() {
            return Some(0);
        }
        let fields: Vec<(u32, Vec<String>)> = fields.iter().map(|(w, s)| (*w, words(s))).collect();
        let mut total = 0;
        let mut matched = false;
        for t in &self.terms {
            let best = t.alts.iter().map(|a| self.alt_score(a, &fields)).max().unwrap_or(0);
            if t.neg {
                if best > 0 {
                    return None;
                }
                continue;
            }
            if best == 0 {
                if !self.any {
                    return None;
                }
                continue;
            }
            matched = true;
            total += if self.any { 1000 + best } else { best };
        }
        if self.any && !matched {
            return None;
        }
        Some(total)
    }

    /// Does this word of the text match a search word? For highlighting.
    pub fn hits(&self, word: &str) -> bool {
        let ws = words(word);
        self.terms
            .iter()
            .filter(|term| !term.neg)
            .flat_map(|term| &term.alts)
            .any(|a| a.words.iter().any(|p| ws.iter().any(|w| word_q(p, w, a.typed, self.fuzzy) > 0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(q: &str, text: &str) -> bool {
        Query::parse(q, false).score(&[(1, text)]).is_some()
    }

    #[test]
    fn word_starts_not_inside() {
        assert!(find("art", "Media art prize"));
        assert!(find("art", "Call for artists"));
        assert!(!find("art", "Startup programme"));
        assert!(!find("ai", "Maintained by email"));
        assert!(find("kunst", "Klangkunst Förderung")); // inside a compound: 5+ letters
    }

    #[test]
    fn phrases_and_exclusions() {
        assert!(find("\"sound art\"", "open call for sound art"));
        assert!(!find("\"sound art\"", "art about sound"));
        assert!(find("sound art", "art about sound")); // unquoted: each word anywhere
        assert!(!find("residency -fee", "Residency, entry fee 20 EUR"));
        assert!(find("residency -fee", "Residency, free"));
        assert!(find("-fee", "Residency"));
    }

    #[test]
    fn other_languages() {
        assert!(find("residency", "Artist-Residenz in Berlin"));
        assert!(find("residency", "Résidence d'artiste"));
        assert!(find("grant", "Förderprogramm Musik"));
        assert!(find("foerderung", "Förderung"));
        assert!(find("sound art", "Ausschreibung Klangkunst 2027"));
        assert!(find("open call", "Convocatoria arte sonoro"));
        assert!(find("kunst", "Media art prize")); // German word finds English
        assert!(!find("residency", "Composition prize"));
    }

    #[test]
    fn typos_only_when_fuzzy() {
        let strict = Query::parse("residncy", false);
        assert!(strict.score(&[(1, "Artist residency")]).is_none());
        let loose = Query::parse("residncy", true);
        assert!(loose.score(&[(1, "Artist residency")]).is_some());
        assert!(Query::parse("hackaton", true).score(&[(1, "Quantum hackathon")]).is_some());
        assert!(Query::parse("quantmu", true).score(&[(1, "Quantum hackathon")]).is_some()); // swapped letters
    }

    #[test]
    fn name_hits_rank_higher() {
        let q = Query::parse("quantum", false);
        let in_name = q.score(&[(6, "Quantum prize"), (2, "for researchers")]).unwrap();
        let in_summary = q.score(&[(6, "Research prize"), (2, "quantum computing")]).unwrap();
        assert!(in_name > in_summary);
        assert!(q.hits("Quantum") && !q.hits("prize"));
    }

    #[test]
    fn plain_sentences() {
        assert!(find("money for musicians", "Grant for music projects"));
        assert!(find("money for musicians", "Förderung Musik"));
        assert!(!find("\"for\"", "Grant"));
        assert!(find("artist in residence", "Residenz für Künstler"));
        assert!(find("residencies", "Artist residency"));
        assert!(find("musicians", "Music prize"));
        assert!(!find("compose", "computer lab"));
    }

    #[test]
    fn loose_mode() {
        let q = Query::parse("quantum reverb zebra", false);
        assert!(q.score(&[(1, "quantum reverb")]).is_none());
        let loose = q.loose();
        let both = loose.score(&[(1, "quantum reverb")]);
        assert!(both.is_some());
        let one = loose.score(&[(1, "quantum only")]);
        assert!(both.unwrap() > one.unwrap());
        assert!(Query::parse("quantum -zebra", false).loose().score(&[(1, "zebra crossing")]).is_none());
    }
}
