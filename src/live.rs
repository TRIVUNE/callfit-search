//! Live search: crawl listing sites, feeds, APIs and search engines in parallel,
//! then read each result's own page for the real deadline and amount.
//! Text matching covers EN, DE, FR, ES, IT, NL, PT, PL and the Nordic languages.

use crate::model::{data_dir, Opportunity};
use chrono::{Datelike, NaiveDate};
use regex::Regex;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

const WORKERS: usize = 64;
/// Parallel requests to one host. Higher gets the IP blocked and returns fewer results.
const PER_HOST: usize = 6;
/// Listing pages followed per source (page 1 + next pages).
const MAX_PAGES: usize = 3;
const MAX_PER_PAGE: usize = 80;
const MAX_DETAIL: usize = 2500;

/// Listing pages: aggregators, funders, festivals, conferences, hackathon boards.
pub const FEEDS: &[(&str, &str)] = &[
    // open-call aggregators
    ("opencalls.ai", "https://opencalls.ai/"),
    ("ArtConnect", "https://www.artconnect.com/opportunities"),
    ("ArtRabbit", "https://www.artrabbit.com/artist-opportunities"),
    ("CuratorSpace", "https://www.curatorspace.com/opportunities"),
    ("ArtJobs", "https://www.artjobs.com/open-calls"),
    ("Art Deadlines List", "https://www.artdeadlineslist.com/"),
    ("Call for Curators", "https://www.callforcurators.com/"),
    ("Reviewed by Artists", "https://www.reviewedbyartists.com/open-calls/"),
    ("XHBT", "https://www.xhbt.org/open-calls"),
    ("e-flux", "https://www.e-flux.com/announcements/"),
    ("Art & Education", "https://www.artandeducation.net/announcements"),
    ("TransArtists", "https://www.transartists.org/en/call-for-artists"),
    ("Alliance of Artists Communities", "https://www.artistcommunities.org/directory/open-calls"),
    ("Residency Unlimited", "https://www.residencyunlimited.org/"),
    ("On the Move", "https://on-the-move.org/news"),
    ("Sound and Music", "https://www.soundandmusic.org/opportunities/"),
    ("Music in Africa", "https://www.musicinafrica.net/opportunities"),
    ("profilculture", "https://www.profilculture.com/annonce/appels-a-projets/"),
    ("CIPAC", "https://www.cipac.net/"),
    ("field notes Berlin", "https://field-notes.berlin/en/funding"),
    // funders: Germany, Berlin, Austria, Netherlands, EU
    ("Kreativ Kultur Berlin", "https://www.kreativkultur.berlin/"),
    ("Berlin Senate culture deadlines", "https://www.berlin.de/sen/kultur/foerderung/antragsfristen/"),
    ("Berlin Senate programmes", "https://www.berlin.de/sen/kultur/foerderung/foerderprogramme/"),
    ("Musikfonds", "https://www.musikfonds.de"),
    ("Initiative Musik", "https://www.initiative-musik.de/foerderprogramme/"),
    ("Stiftung Kunstfonds", "https://www.kunstfonds.de/foerderung"),
    ("Fonds Soziokultur", "https://www.fonds-soziokultur.de/"),
    ("Kulturstiftung des Bundes", "https://www.kulturstiftung-des-bundes.de/de/foerderung"),
    ("Akademie der Kuenste", "https://www.adk.de/en/"),
    ("Akademie Schloss Solitude", "https://www.akademie-solitude.de/en/"),
    ("Creative Europe Desk", "https://www.creative-europe-desk.de/"),
    ("Prototype Fund", "https://www.prototypefund.de/"),
    ("FFG", "https://www.ffg.at/ausschreibungen"),
    ("BMKOES", "https://www.bmkoes.gv.at/"),
    ("Stimuleringsfonds", "https://stimuleringsfonds.nl/"),
    ("Mondriaan Fonds", "https://www.mondriaanfonds.nl/"),
    // open source, quantum, research
    ("NLnet news", "https://nlnet.nl/news/"),
    ("NLnet calls", "https://nlnet.nl/propose/"),
    ("Sovereign Tech Agency", "https://www.sovereign.tech/news"),
    ("Unitary Foundation", "https://unitary.foundation"),
    ("QIP", "https://qip2027.org"),
    ("Bridges math art", "https://bridgesmathart.org/"),
    ("IMAGINARY", "https://www.imaginary.org/"),
    // festivals, media art, prizes
    ("CTM Festival", "https://www.ctm-festival.de/"),
    ("CTM open calls", "https://www.ctm-festival.de/festival-2027/open-calls"),
    ("transmediale", "https://www.transmediale.de/"),
    ("ZKM", "https://zkm.de/en/open-calls"),
    ("Prix Ars Electronica", "https://ars.electronica.art/prix/en/"),
    ("STARTS", "https://starts.eu/"),
    ("ISCM", "https://iscm.org/news/"),
    ("CreativeApplications", "https://www.creativeapplications.net/"),
    // audio, DSP, music tech conferences and societies
    ("ADC Audio Developer Conference", "https://audio.dev/"),
    ("DAFx", "https://www.dafx.de/"),
    ("NIME", "https://www.nime.org/"),
    ("ICMC", "https://www.icmc2026.org/"),
    ("ISMIR", "https://ismir.net/conferences/"),
    ("DCASE", "https://dcase.community/challenge2026/"),
    ("SEAMUS", "https://www.seamusonline.org/"),
    ("EURASIP", "https://www.eurasip.org/"),
    ("IEEE Signal Processing Society", "https://signalprocessingsociety.org/"),
    ("KVR Audio", "https://www.kvraudio.com/news"),
    // hackathons and challenges
    ("MLH", "https://mlh.io/seasons/2027/events"),
    ("hackathon.com", "https://www.hackathon.com/online"),
    ("HackerEarth", "https://www.hackerearth.com/challenges/hackathon/"),
    ("Devfolio", "https://devfolio.co/hackathons"),
    ("Hackaday contests", "https://hackaday.io/contests"),
    ("Eventbrite Berlin hackathons", "https://www.eventbrite.de/d/germany--berlin/hackathon/"),
    ("itch.io game jams", "https://itch.io/jams/upcoming"),
    ("Taikai hackathons", "https://www.taikai.network/hackathons"),
    ("DoraHacks", "https://dorahacks.io/hackathon"),
    ("HackQuest", "https://www.hackquest.io/"),
    ("lablab.ai events", "https://lablab.ai/event"),
    ("ETHGlobal", "https://ethglobal.com/events"),
    ("MLH", "https://www.mlh.io/"),
    ("AIcrowd challenges", "https://www.aicrowd.com/challenges"),
    ("DrivenData competitions", "https://www.drivendata.org/competitions/"),
    ("Zindi competitions", "https://zindi.africa/competitions"),
    ("CodaLab competitions", "https://codalab.lisn.upsaclay.fr/"),
    ("Kaggle competitions", "https://www.kaggle.com/competitions"),
    ("KVR Developer Challenge", "https://www.kvraudio.com/kvr-developer-challenge"),
    ("Deutscher Computerspielpreis", "https://www.deutscher-computerspielpreis.de/"),
    // Berlin universities, research funders, Berlin funding
    ("HTW IUNO open call AI XR", "https://iuno.fki.htw-berlin.de/en/open-call-ai-xr-2027/"),
    ("HTW IUNO", "https://iuno.fki.htw-berlin.de/en/"),
    ("HTW Berlin", "https://www.htw-berlin.de/"),
    ("BHT Berlin", "https://www.bht-berlin.de/"),
    ("TU Berlin", "https://www.tu.berlin/"),
    ("UdK Berlin", "https://www.udk-berlin.de/"),
    ("FU Berlin", "https://www.fu-berlin.de/"),
    ("HU Berlin", "https://www.hu-berlin.de/"),
    ("Berlin University Alliance", "https://www.berlin-university-alliance.de/en/"),
    ("Einstein Foundation", "https://www.einsteinfoundation.de/en/"),
    ("Weizenbaum Institute", "https://www.weizenbaum-institut.de/"),
    ("Technologiestiftung Berlin", "https://www.technologiestiftung-berlin.de/"),
    ("Medienboard Berlin-Brandenburg", "https://www.medienboard.de/"),
    ("Musicboard Berlin", "https://www.musicboard-berlin.de/"),
    ("IBB", "https://www.ibb.de/"),
    ("Creative City Berlin", "https://www.creative-city-berlin.de/"),
    ("EXIST", "https://www.exist.de/"),
    ("DFG funding", "https://www.dfg.de/en/research-funding"),
    ("VolkswagenStiftung", "https://www.volkswagenstiftung.de/en/funding"),
    ("Humboldt Foundation", "https://www.humboldt-foundation.de/en/apply"),
    ("Studienstiftung", "https://www.studienstiftung.de/"),
    // universities, music schools, sound/media labs (Germany and abroad)
    ("UdK Berlin Hanns Eisler", "https://www.hfm-berlin.de/"),
    ("HfMT Hamburg", "https://www.hfmt-hamburg.de/"),
    ("HfM Weimar", "https://www.hfm-weimar.de/"),
    ("HfM Karlsruhe", "https://www.hfm-karlsruhe.de/"),
    ("HfG Karlsruhe", "https://hfg-karlsruhe.de/"),
    ("HMTM Munich", "https://www.hmtm.de/"),
    ("HfMDK Frankfurt", "https://www.hfmdk-frankfurt.de/"),
    ("HMDK Stuttgart", "https://www.hmdk-stuttgart.de/"),
    ("Folkwang", "https://www.folkwang-uni.de/"),
    ("MH Freiburg", "https://www.mh-freiburg.de/"),
    ("KHM Cologne", "https://www.khm.de/"),
    ("Bauhaus-Uni Weimar", "https://www.uni-weimar.de/"),
    ("TU Berlin Audio Communication", "https://www.ak.tu-berlin.de/"),
    ("mdw Vienna", "https://www.mdw.ac.at/"),
    ("Mozarteum", "https://www.uni-mozarteum.at/"),
    ("IEM Graz", "https://iem.kug.ac.at/"),
    ("Kunstuni Linz", "https://www.kunstuni-linz.at/"),
    ("ZHdK", "https://www.zhdk.ch/"),
    ("ICST Zurich", "https://www.icst.net/"),
    ("Royal Conservatoire The Hague", "https://www.koncon.nl/"),
    ("IRCAM agenda", "https://www.ircam.fr/agenda"),
    ("IRCAM transmission", "https://www.ircam.fr/transmission"),
    ("EMS Stockholm", "https://elektronmusikstudion.se/"),
    ("Aalto University", "https://www.aalto.fi/en"),
    ("UAL", "https://www.arts.ac.uk/"),
    ("RCA", "https://www.rca.ac.uk/"),
    ("CCRMA Stanford", "https://ccrma.stanford.edu/"),
    ("MIT Media Lab", "https://www.media.mit.edu/"),
    // math and quantum institutes
    ("MATH+ Berlin", "https://mathplus.de/"),
    ("Berlin Mathematical School", "https://www.math-berlin.de/"),
    ("WIAS Berlin", "https://www.wias-berlin.de/"),
    ("Zuse Institute Berlin", "https://www.zib.de/"),
    ("MPI Mathematics Bonn", "https://www.mpim-bonn.mpg.de/"),
    ("Hausdorff Center", "https://www.hcm.uni-bonn.de/"),
    ("MPI Mathematics in the Sciences", "https://www.mis.mpg.de/"),
    ("Oberwolfach MFO", "https://www.mfo.de/"),
    ("IHES", "https://www.ihes.fr/"),
    ("Isaac Newton Institute", "https://www.newton.ac.uk/"),
    ("ICERM", "https://icerm.brown.edu/"),
    ("IPAM", "https://www.ipam.ucla.edu/"),
    ("Fields Institute", "http://www.fields.utoronto.ca/"),
    ("Simons Foundation funding", "https://www.simonsfoundation.org/funding-opportunities/"),
    ("Perimeter Institute", "https://perimeterinstitute.ca/"),
    ("IQC Waterloo", "https://uwaterloo.ca/institute-for-quantum-computing/"),
    // communities (often need a login: kept and marked)
    ("Airwiggles conference", "https://www.airwiggles.com/c/conference"),
    ("Airwiggles", "https://www.airwiggles.com/"),
];

/// WikiCFP categories: every feed item is a call for papers.
const CFP_TOPICS: &[&str] = &[
    "signal processing", "audio", "music", "computer music", "acoustics", "speech", "multimedia",
    "quantum", "quantum computing", "quantum information", "mathematics", "applied mathematics",
    "algebra", "number theory", "combinatorics", "graph theory", "geometry", "topology", "analysis",
    "numerical analysis", "optimization", "probability", "statistics", "logic", "cryptography",
    "algorithms", "theoretical computer science", "machine learning", "artificial intelligence",
    "human computer interaction", "computer graphics", "virtual reality", "art", "digital humanities",
    "physics", "image processing", "pattern recognition", "embedded systems", "open source",
];

/// Devpost hackathon searches.
const DEVPOST_TOPICS: &[&str] = &[
    "audio", "music", "sound", "quantum", "math", "signal", "dsp", "art", "creative", "ai",
    "hardware", "open source", "accessibility", "education", "game", "game audio", "xr", "vr", "ar",
    "computing", "music technology", "quantum computing", "signal processing", "spatial",
];

/// EU Funding & Tenders searches (open and forthcoming calls only).
const EU_TOPICS: &[&str] = &[
    "music", "culture", "art", "audio", "sound", "creative", "media", "digital", "quantum",
    "mathematics", "signal processing", "open source", "software", "artificial intelligence",
    "research", "innovation", "startup", "photonics", "acoustics",
];

/// confs.tech topic files (conference talk calls with CFP end dates).
const CONFS_TOPICS: &[&str] = &[
    "accessibility", "android", "api", "data", "devops", "dotnet", "general", "golang", "ios", "iot",
    "java", "javascript", "kotlin", "leadership", "networking", "opensource", "php", "product",
    "python", "security", "sre", "tech-comm", "testing", "typescript", "ux",
];

/// Web search queries. {Y} becomes the coming season's year.
const QUERIES: &[&str] = &[
    "open call sound art {Y}", "call for papers audio signal processing {Y}", "music technology hackathon {Y}",
    "Ausschreibung Klangkunst {Y}", "KVR Developer Challenge {Y}",
    // games, XR, showcases, online competitions
    "game audio jam {Y}", "game jam {Y} sound", "XR hackathon {Y}", "AR VR hackathon Berlin {Y}", "open call AI XR {Y}",
    "call for participation XR art {Y}", "audio plugin developer challenge {Y}", "online composition competition {Y}",
    "sound design competition {Y}", "music production contest {Y}", "call for demos audio {Y}",
    "showcase call for submissions music technology {Y}", "creative technology showcase open call {Y}",
    "AI music competition {Y}", "MIDI innovation award {Y}", "music tech startup competition {Y}",
    "computing hackathon {Y}", "math hackathon {Y}", "quantum hackathon Berlin {Y}", "airwiggles conference",
    // universities, conservatories, institutes anywhere
    "university artist in residence open call {Y}", "conservatory call for scores {Y}",
    "university call for compositions electronic music {Y}", "music university composition competition {Y}",
    "university sound art fellowship {Y}", "artist in residence physics mathematics department {Y}",
    "university creative technology fellowship {Y}", "visiting researcher program audio {Y}",
    "postdoc fellowship music technology {Y}", "summer school audio signal processing {Y}",
    "summer school quantum computing {Y}", "summer school mathematics {Y}", "workshop call for participation mathematics {Y}",
    "Musikhochschule Kompositionswettbewerb {Y}", "Hochschule Ausschreibung Artist in Residence {Y}",
    "Universität Ausschreibung Stipendium Kunst {Y}", "Kunsthochschule open call {Y}", "Sommerakademie elektronische Musik {Y}",
    "université appel à candidatures résidence artiste {Y}", "conservatoire appel à partitions {Y}",
    "universidad convocatoria residencia artística {Y}", "università bando artista in residenza {Y}",
    "universiteit open call kunstenaar {Y}", "uczelnia nabór rezydencja artystyczna {Y}",
    // Berlin
    "Berlin open call {Y}", "Hochschule Berlin open call {Y}", "Förderung Berlin Kreativwirtschaft {Y}",
    "Klangkunst Wettbewerb {Y}", "Berlin Stipendium Kunst Technologie {Y}",
    // English: art, sound, music
    "open call sound art {Y}", "call for works electroacoustic {Y}", "call for scores electronic music {Y}",
    "artist residency sound art {Y}", "media art prize {Y}", "call for installations sound {Y}",
    "composition competition electronic music {Y}", "art science residency {Y}", "spatial audio open call {Y}",
    "immersive audio commission {Y}", "music technology fellowship {Y}", "generative art open call {Y}",
    "creative coding grant {Y}", "audiovisual open call {Y}", "call for artists new media {Y}",
    // English: research, DSP, audio dev, quantum, math
    "call for papers audio signal processing {Y}", "call for papers music technology {Y}",
    "audio developer conference call for speakers {Y}", "audio plugin competition {Y}",
    "DSP challenge {Y}", "audio machine learning challenge {Y}", "music hackathon {Y}",
    "quantum computing hackathon {Y}", "quantum computing grant {Y}", "quantum software prize {Y}",
    "quantum music {Y}", "mathematics conference call for papers {Y}", "mathematics prize {Y}",
    "mathematics research grant {Y}", "number theory conference {Y}", "combinatorics conference {Y}",
    "applied mathematics workshop call for abstracts {Y}", "math competition {Y}",
    "open source software grant {Y}", "open source fund call for proposals {Y}",
    "acoustics conference call for papers {Y}", "speech and audio challenge {Y}",
    "game audio competition {Y}", "open hardware prize {Y}", "synthesizer design competition {Y}",
    "hackathon Europe {Y}", "hackathon Berlin {Y}", "call for speakers creative technology {Y}",
    "residency for artists and scientists {Y}", "stipend for artists working with technology {Y}",
    // German
    "Ausschreibung Klangkunst {Y}", "Open Call Medienkunst {Y}", "Stipendium Klangkunst {Y}",
    "Förderung Musiktechnologie {Y}", "Residenz Klangkunst {Y}", "Kompositionswettbewerb elektronische Musik {Y}",
    "Förderprogramm Open Source Software {Y}", "Hackathon Berlin {Y}", "Ausschreibung Kunst und Wissenschaft {Y}",
    "Mathematik Preis Ausschreibung {Y}", "Quantencomputing Förderung {Y}", "Stipendium Medienkunst Berlin {Y}",
    // French
    "appel à candidatures art sonore {Y}", "résidence art sonore {Y}", "appel à projets arts numériques {Y}",
    "concours composition électroacoustique {Y}", "appel à communications traitement du signal {Y}",
    "appel à candidatures mathématiques {Y}",
    // Spanish
    "convocatoria arte sonoro {Y}", "residencia artística arte sonoro {Y}", "convocatoria arte y tecnología {Y}",
    "convocatoria música electrónica {Y}", "beca arte digital {Y}",
    // Italian
    "bando arte sonora {Y}", "residenza artistica sound art {Y}", "call per artisti arte digitale {Y}",
    "concorso composizione musica elettronica {Y}",
    // Dutch
    "open oproep geluidskunst {Y}", "subsidie digitale cultuur {Y}",
    // Portuguese
    "convocatória arte sonora {Y}", "edital arte sonora {Y}", "residência artística arte sonora {Y}",
    // Polish
    "nabór sztuka dźwięku {Y}", "rezydencja artystyczna sztuka dźwięku {Y}", "konkurs kompozytorski muzyka elektroniczna {Y}",
    // Nordic
    "open call ljudkonst {Y}", "residency lydkunst {Y}",
];

const ENGINES: &[(&str, &str)] = &[
    ("Seznam", "https://search.seznam.cz/?q="),
    ("DuckDuckGo", "https://html.duckduckgo.com/html/?q="),
    ("Brave", "https://search.brave.com/search?q="),
    // the old /do/search address answers bots with a CAPTCHA
    ("Startpage", "https://www.startpage.com/sp/search?query="),
];

pub enum Msg {
    /// `note`: why a source failed (HTTP status, timeout...)
    Progress { stage: &'static str, done: usize, total: usize, source: String, ok: bool, found: usize, note: String },
    /// New or updated items; the receiver upserts by id.
    Items(Vec<Opportunity>),
    /// Ids whose call page showed a passed deadline: remove them.
    Gone(Vec<String>),
    Done { failed: Vec<String> },
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum How {
    /// HTML listing page or RSS/Atom feed; keyword-filtered.
    Page,
    /// Feed where every item counts (WikiCFP).
    Cfp,
    /// Search engine result page.
    Search,
    Devpost,
    Confs,
    Eu,
}

#[derive(Clone, Debug)]
struct Job {
    source: String,
    url: String,
    how: How,
    page: usize,
    /// EU API search term (sent as POST)
    term: String,
}

struct Cand {
    o: Opportunity,
    /// read the item's own page for deadline/amount
    detail: bool,
    /// replace the name with the page title (search results)
    retitle: bool,
}

// ---------------------------------------------------------------- regexes

macro_rules! re {
    ($name:ident, $pat:expr) => {
        fn $name() -> &'static Regex {
            static R: OnceLock<Regex> = OnceLock::new();
            R.get_or_init(|| Regex::new($pat).unwrap())
        }
    };
}

re!(re_anchor, r#"(?is)<a\b([^>]*)>(.*?)</a>"#);
re!(re_href, r#"(?is)\bhref\s*=\s*["']([^"']+)["']"#);
re!(re_strip, r"(?is)<!--.*?-->|<script\b.*?</script>|<style\b.*?</style>|<noscript\b.*?</noscript>|<svg\b.*?</svg>|<[^>]+>");
re!(re_chrome, r"(?is)<nav\b.*?</nav>|<footer\b.*?</footer>");
re!(re_entity, r"&(#[xX][0-9a-fA-F]+|#\d+|[a-zA-Z]+\d*);");
re!(
    re_kw,
    r"(?i)open[- ]?call|call[- ]for[- ](?:papers|proposals|works|entries|artists|applications|submissions|participation|speakers|talks|abstracts|scores|projects|contributions|posters|demos|music|pieces)|\bcfp\b|residenc|residen[zt]|résidence|residencia|residenza|residência|rezydencj|\bgrants?\b|fellowship|scholarship|bursary|stipend|\bawards?\b|\bprizes?\b|competition|contest|challenge|hackathon|hack ?day|game ?jam|datathon|commission|funding|\bfund\b|\bapply\b|deadline|ausschreibung|wettbewerb|stipendi|förder|foerder|bewerb|einreich|aufruf|appel à|appel a |candidature|bourse|\bprix\b|concours|subvention|convocatoria|\bbeca|premio|concurso|subvención|\bbando|\bbandi\b|call per|borsa di studio|concorso|scadenza|oproep|\bbeurs|subsidie|regeling|wedstrijd|convocatória|edital|\bbolsa|prémio|prêmio|chamada|nabór|konkurs|stypendi|dotacj|nagrod|utlysning|tävling|\blegat\b|ansøgning|ansökan|søknad"
);
re!(
    re_skip,
    r"(?i)login|log-in|signin|sign-in|sign-up|signup|register\b|privacy|datenschutz|impressum|imprint|cookie|newsletter|facebook\.|instagram\.|twitter\.|x\.com/|linkedin\.|youtube\.|youtu\.be|tiktok\.|pinterest\.|wa\.me|whatsapp|t\.me/|mailto:|tel:|javascript:|^#|/tag/|/tags/|/category/|/author/|share=|/wp-login|/cart|/account|\.(?:jpg|jpeg|png|gif|webp|svg|zip|mp3|mp4)(?:\?|$)"
);
re!(
    re_engine_host,
    r"(?i)(?:^|\.)(?:duckduckgo\.com|brave\.com|startpage\.com|(?:o-)?seznam\.cz|google\.[a-z.]+|bing\.com|microsoft\.com|yahoo\.com|yandex\.[a-z]+|wikipedia\.org|wikimedia\.org|facebook\.com|instagram\.com|twitter\.com|x\.com|linkedin\.com|youtube\.com|tiktok\.com|pinterest\.[a-z.]+|reddit\.com|amazon\.[a-z.]+)$"
);
re!(
    re_next,
    r"(?i)^(?:next|next page|more|load more|older|older posts|older entries|weiter|nächste|nächste seite|vor|mehr|ältere|suivant|page suivante|siguiente|successiv[oa]|avanti|volgende|następna|próxima|seguinte|›|»|>|→|>>)$"
);
re!(re_rel_next, r#"(?is)<(?:a|link)\b[^>]*rel=["']next["'][^>]*>"#);
re!(re_feed_link, r#"(?is)<link\b[^>]*type=["']application/(?:rss|atom)\+xml["'][^>]*>"#);
re!(re_item, r"(?is)<item\b.*?</item>|<entry\b.*?</entry>");
re!(re_title_tag, r"(?is)<title[^>]*>(.*?)</title>");
re!(re_link_text, r"(?is)<link[^>]*>([^<]+)</link>");
re!(re_link_href, r#"(?is)<link\b[^>]*href=["']([^"']+)["']"#);
re!(re_desc_tag, r"(?is)<(?:description|summary|content:encoded|content)[^>]*>(.*?)</(?:description|summary|content:encoded|content)>");
re!(
    re_nav_title,
    r"(?i)^(?:explore|view|see|show|browse|discover|find|search|load|all)\b.{0,30}\b(?:calls|competitions|opportunities|events|hackathons|grants|jams|challenges|listings|results|programmes|programs|funding)\b|competition search|^apply for funding$|^(?:more|next|previous)\b"
);
// news about winners or past rounds, not a call
re!(re_past, r"(?i)\bwins\b|\bwinners?\b|\bawarded to\b|\bannounced\b|\brecipients\b|gewinner|preisträger|lauréat|ganador|vincitor|congratulat|\bcongrats\b|\bopen role\b|we.re hiring|\bcareers\b|stellenangebot");
// the call page itself says the round is over
re!(
    re_closed,
    r"(?i)no longer (?:being )?accept|(?:submissions?|applications?|entries|registrations?|nominations|the call|this call)(?: (?:are|is|has|have))? (?:now )?(?:been )?closed\b|(?:call|competition|submission period|application period) (?:has )?(?:ended|closed)\b|deadline (?:has )?(?:passed|expired)|bewerbungs(?:frist|phase|zeitraum) (?:ist )?(?:abgelaufen|beendet|vorbei)|(?:ausschreibung|einreichung|bewerbung) (?:ist )?(?:beendet|abgeschlossen|geschlossen)|(?:candidatures|inscriptions) (?:sont )?(?:closes|clôturées|terminées)|appel (?:à [a-zé]+ )?(?:est )?(?:clos|clôturé|terminé)|convocatoria (?:está )?(?:cerrada|finalizada)|(?:bando|candidature) (?:è |sono )?(?:chius[oe]|scadut[oe])|inschrijving (?:is )?gesloten|zgłoszenia (?:zostały )?zamknięte"
);
re!(re_h1, r"(?is)<h1\b[^>]*>(.*?)</h1>");
re!(re_meta, r#"(?is)<meta\b[^>]*>"#);
re!(re_meta_key, r#"(?is)(?:property|name)\s*=\s*["']([^"']+)["']"#);
re!(re_meta_content, r#"(?is)content\s*=\s*["']([^"']*)["']"#);
re!(re_lang, r#"(?is)<html\b[^>]*\blang\s*=\s*["']([a-zA-Z]{2})"#);

// dates
re!(re_d_num, r"\b(\d{1,2})\s?[./-]\s?(\d{1,2})\s?[./-]\s?(20\d\d)\b");
re!(re_d_iso, r"\b(20\d\d)[-/.](\d{1,2})[-/.](\d{1,2})\b");
re!(re_d_dmy, r"(?i)\b(\d{1,2})(?:st|nd|rd|th|er|º|°|\.)?\s+(?:de\s+|of\s+)?(\p{L}{3,12})\.?,?\s+(?:de\s+)?(20\d\d)\b");
re!(re_d_mdy, r"(?i)\b(\p{L}{3,12})\.?\s+(\d{1,2})(?:st|nd|rd|th)?,?\s+(20\d\d)\b");
re!(re_d_dm, r"(?i)\b(\d{1,2})(?:st|nd|rd|th|er|\.)?\s+(?:de\s+|of\s+)?(\p{L}{3,12})\b");
re!(re_d_md, r"(?i)\b(\p{L}{3,12})\.?\s+(\d{1,2})(?:st|nd|rd|th)?\b");
re!(re_d_numdm, r"\b(\d{1,2})\.(\d{1,2})\.(?:\s|$)");
re!(
    re_deadline_kw,
    r"(?i)deadline|due date|due by|closing date|\bcloses\b|close on|apply by|apply before|apply until|submissions? (?:due|close)|applications? (?:close|due)|entries close|no later than|\buntil\b|\bfrist|bewerbungsschluss|einsendeschluss|einreichungsschluss|abgabeschluss|anmeldeschluss|antragsfrist|bewerbungsfrist|bis zum|bis spätestens|date limite|date-limite|clôture|jusqu.au|avant le|au plus tard|fecha límite|fecha limite|\bplazo|hasta el|scadenza|entro il|entro e non oltre|sluitingsdatum|uiterlijk|tot en met|\bprazo|data limite|até o dia|até dia|termin (?:zgłoszeń|nadsyłania|składania)|do dnia|sista ansökningsdag|ansøgningsfrist|søknadsfrist|uzávěrka"
);
re!(
    re_event_kw,
    r"(?i)takes place|will take place|taking place|will be held|to be held|held (?:on|in|from)|event dates?|festival dates?|conference dates?|exhibition dates?|dates of the (?:event|conference|festival)|\bwhen\b|\bfindet\b|veranstaltungszeitraum|\bzeitraum\b|aura lieu|se tiendra|se déroulera|se celebrará|tendrá lugar|si terrà|si svolgerà|avrà luogo|vindt plaats|odbędzie się|\bacontece|decorrerá|äger rum|finder sted|foregår"
);
// "2–4 July 2027", "2. bis 4. Juli 2027"
re!(re_d_range_dm, r"(?i)\b(\d{1,2})\.?\s?(?:[–-]|to|bis|au|al|tot)\s?(\d{1,2})(?:st|nd|rd|th|er|\.)?\s+(?:de\s+)?(\p{L}{3,12})\.?,?\s+(?:de\s+)?(20\d\d)\b");
// "July 2-4, 2027"
re!(re_d_range_md, r"(?i)\b(\p{L}{3,12})\.?\s+(\d{1,2})\s?[–-]\s?(\d{1,2}),?\s+(20\d\d)\b");
re!(
    re_money_kw,
    r"(?i)prize|award|grant|\bfee\b|honorari|stipend|scholarship|funding|up to|worth|endowed|dotiert|preisgeld|fördersumme|förderung|zuschuss|\bprix\b|dotation|bourse|cachet|premio|dotación|\bbeca|importe|borsa|compenso|montepremi|prijzengeld|vergoeding|\bbeurs|prémio|prêmio|\bbolsa|nagrod|stypendium|\bcash\b"
);
re!(
    re_money,
    r"(?i)(€|\beur(?:os?)?\b|\$|\busd\b|£|\bgbp\b|\bchf\b)\s?(\d{1,3}(?:[.,' \u{a0}]\d{3})+(?:[.,]\d{1,2})?|\d+(?:[.,]\d+)?)\s?(k|tsd|mio|mln|millions?|millionen|m)?\b|(\d{1,3}(?:[.,' \u{a0}]\d{3})+(?:[.,]\d{1,2})?|\d+(?:[.,]\d+)?)\s?(k|tsd|mio|mln|millions?|millionen|m)?\s?(€|\beur(?:os?)?\b|\$|\busd\b|£|\bgbp\b|\bchf\b)"
);
re!(re_grouped, r"^\d{1,3}(?:[.,' \u{a0}]\d{3})+");

// ---------------------------------------------------------------- text helpers

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    re_entity()
        .replace_all(s, |c: &regex::Captures| {
            let e = &c[1];
            let ch = if let Some(h) = e.strip_prefix("#x").or_else(|| e.strip_prefix("#X")) {
                u32::from_str_radix(h, 16).ok().and_then(char::from_u32)
            } else if let Some(d) = e.strip_prefix('#') {
                d.parse().ok().and_then(char::from_u32)
            } else {
                Some(match e {
                    "amp" => '&', "nbsp" => ' ', "quot" => '"', "apos" => '\'', "lt" => '<', "gt" => '>',
                    "ndash" => '–', "mdash" => '—', "rsquo" | "lsquo" => '\'', "ldquo" | "rdquo" | "bdquo" => '"',
                    "hellip" => '…', "euro" => '€', "pound" => '£', "laquo" => '«', "raquo" => '»', "middot" => '·',
                    "auml" => 'ä', "ouml" => 'ö', "uuml" => 'ü', "Auml" => 'Ä', "Ouml" => 'Ö', "Uuml" => 'Ü', "szlig" => 'ß',
                    "eacute" => 'é', "egrave" => 'è', "ecirc" => 'ê', "euml" => 'ë', "Eacute" => 'É',
                    "aacute" => 'á', "agrave" => 'à', "acirc" => 'â', "atilde" => 'ã', "aring" => 'å',
                    "iacute" => 'í', "igrave" => 'ì', "icirc" => 'î', "oacute" => 'ó', "ograve" => 'ò', "ocirc" => 'ô',
                    "otilde" => 'õ', "oslash" => 'ø', "uacute" => 'ú', "ugrave" => 'ù', "ucirc" => 'û',
                    "ccedil" => 'ç', "ntilde" => 'ñ', "aelig" => 'æ', "Aring" => 'Å', "Oslash" => 'Ø',
                    _ => return c[0].to_string(),
                })
            };
            ch.map(|c| c.to_string()).unwrap_or_default()
        })
        .into_owned()
}

fn strip_tags(s: &str) -> String {
    let t = re_strip().replace_all(s, " ");
    decode_entities(&t).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Visible text of a whole page, without menus and footers.
fn page_text(html: &str) -> String {
    strip_tags(&re_chrome().replace_all(html, " "))
}

fn truncate_chars(s: &mut String, n: usize) {
    if let Some((i, _)) = s.char_indices().nth(n) {
        s.truncate(i);
    }
}

/// Slice by byte offsets, moved onto char boundaries.
fn window(text: &str, from: usize, to: usize) -> &str {
    let mut a = from.min(text.len());
    while !text.is_char_boundary(a) {
        a -= 1;
    }
    let mut b = to.min(text.len());
    while !text.is_char_boundary(b) {
        b += 1;
    }
    &text[a..b]
}

/// Lowercase, accents removed, umlauts folded, so "Förderung", "Foerderung" and "forderung" match.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.to_lowercase().chars() {
        match ch {
            'ä' | 'á' | 'à' | 'â' | 'ã' | 'å' | 'ą' => out.push('a'),
            'æ' => out.push_str("ae"),
            'ö' | 'ó' | 'ò' | 'ô' | 'õ' | 'ø' => out.push('o'),
            'ü' | 'ú' | 'ù' | 'û' => out.push('u'),
            'é' | 'è' | 'ê' | 'ë' | 'ę' | 'ě' => out.push('e'),
            'í' | 'ì' | 'î' | 'ï' => out.push('i'),
            'ç' | 'ć' | 'č' => out.push('c'),
            'ñ' | 'ń' | 'ň' => out.push('n'),
            'ś' | 'š' => out.push('s'),
            'ź' | 'ż' | 'ž' => out.push('z'),
            'ł' => out.push('l'),
            'ř' => out.push('r'),
            'ý' => out.push('y'),
            'ß' => out.push_str("ss"),
            c => out.push(c),
        }
    }
    out.replace("ae", "a").replace("oe", "o").replace("ue", "u")
}

fn host(url: &str) -> String {
    let h = url.split("://").nth(1).unwrap_or(url).split(['/', '?', '#']).next().unwrap_or("").to_lowercase();
    h.strip_prefix("www.").map(str::to_string).unwrap_or(h)
}

/// URL without scheme, www, fragment, tracking params and trailing slash: for de-duplication.
pub fn url_key(url: &str) -> String {
    let u = url.split('#').next().unwrap_or(url);
    let (path, query) = u.split_once('?').unwrap_or((u, ""));
    let q: Vec<&str> = query.split('&').filter(|p| !p.is_empty() && !p.starts_with("utm_") && !p.starts_with("ref=")).collect();
    let mut k = format!("{}{}", host(path), path.split("://").nth(1).unwrap_or(path).splitn(2, '/').nth(1).map(|p| format!("/{}", p)).unwrap_or_default());
    while k.ends_with('/') {
        k.pop();
    }
    if !q.is_empty() {
        k.push('?');
        k.push_str(&q.join("&"));
    }
    k.to_lowercase()
}

fn name_key(name: &str) -> String {
    let mut k: String = fold(name).chars().filter(|c| c.is_alphanumeric()).collect();
    truncate_chars(&mut k, 60);
    k
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".into(),
            _ => format!("%{:02X}", b),
        })
        .collect()
}

fn join_url(base: &str, href: &str) -> String {
    let href = href.trim();
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }
    let scheme = if base.starts_with("http://") { "http:" } else { "https:" };
    if let Some(rest) = href.strip_prefix("//") {
        return format!("{}//{}", scheme, rest);
    }
    let after = base.split("://").nth(1).unwrap_or(base);
    let base_host = after.split(['/', '?', '#']).next().unwrap_or("");
    let scheme_host = format!("{}//{}", scheme, base_host);
    if href.starts_with('/') {
        return format!("{}{}", scheme_host, href);
    }
    let base_path = base.split(['?', '#']).next().unwrap_or(base);
    if href.starts_with('?') {
        return format!("{}{}", base_path, href);
    }
    match base_path.rfind('/') {
        Some(i) if i > scheme_host.len() => format!("{}{}", &base_path[..=i], href),
        _ => format!("{}/{}", scheme_host, href),
    }
}

fn attr<'a>(re: &Regex, tag: &'a str) -> Option<&'a str> {
    re.captures(tag).and_then(|c| c.get(1)).map(|m| m.as_str())
}

fn meta(html: &str, keys: &[&str]) -> Option<String> {
    for m in re_meta().find_iter(html) {
        let tag = m.as_str();
        if let (Some(k), Some(v)) = (attr(re_meta_key(), tag), attr(re_meta_content(), tag)) {
            if keys.iter().any(|x| x.eq_ignore_ascii_case(k)) {
                let v = strip_tags(v);
                if !v.is_empty() {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// "Open Call 2027 | Some Site" -> "Open Call 2027"
fn clean_title(t: &str) -> String {
    let t = strip_tags(t);
    for sep in [" | ", " – ", " — ", " - ", " :: ", " · ", " » "] {
        if let Some((a, _)) = t.split_once(sep) {
            if a.chars().count() >= 12 {
                return a.trim().to_string();
            }
        }
    }
    t
}

/// Navigation links, winner news, and titles that are mostly numbers ("58.2k 177 29 ...").
fn junk_title(title: &str) -> bool {
    let digits = title.chars().filter(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '(' | ')')).count();
    re_nav_title().is_match(title.trim()) || re_past().is_match(title) || digits * 10 > title.chars().count() * 3
}

re!(re_year, r"\b(20\d\d)\b");

/// A title that names only past years ("unitaryHACK 2022", "ISQCMC 2023") is an old round.
fn stale(title: &str, today: NaiveDate) -> bool {
    let years: Vec<i32> = re_year().captures_iter(title).filter_map(|c| c[1].parse().ok()).collect();
    !years.is_empty() && years.iter().all(|y| *y < today.year())
}

fn is_generic(title: &str) -> bool {
    let f = fold(title);
    f.chars().count() < 16
        || ["read more", "more info", "learn more", "details", "mehr", "weiterlesen", "en savoir plus", "leer mas", "scopri", "lees meer", "apply now", "view", "click here"]
            .iter()
            .any(|g| f.starts_with(g))
}

// ---------------------------------------------------------------- dates

fn months() -> &'static HashMap<String, u32> {
    static M: OnceLock<HashMap<String, u32>> = OnceLock::new();
    M.get_or_init(|| {
        let table = [
            "january jan januar jänner jaenner janvier janv enero ene gennaio gen januari janeiro stycznia styczeń styczen ledna",
            "february feb februar février fevrier févr fevr febrero febbraio februari fevereiro fev lutego luty února",
            "march mar märz maerz mrz mär mars marzo maart março marco marca marzec března",
            "april apr avril avr abril abr aprile kwietnia kwiecień kwiecien dubna",
            "may mai mayo maggio mag mei maio maja maj května",
            "june jun juni juin junio giugno giu junho czerwca czerwiec června",
            "july jul juli juillet juil julio luglio lug julho lipca lipiec července",
            "august aug août aout agosto ago augustus augusti sierpnia sierpień sierpien srpna",
            "september sep sept septembre septiembre setiembre settembre set setembro września wrzesień wrzesien září",
            "october oct oktober okt octobre octubre ottobre ott outubro out października październik pazdziernik října",
            "november nov novembre noviembre novembro listopada listopad listopadu",
            "december dec dezember dez décembre decembre déc diciembre dic dicembre dezembro grudnia grudzień grudzien prosince",
        ];
        let mut m = HashMap::new();
        for (i, line) in table.iter().enumerate() {
            for w in line.split_whitespace() {
                m.insert(w.to_string(), i as u32 + 1);
            }
        }
        m
    })
}

fn month_num(word: &str) -> Option<u32> {
    months().get(&word.to_lowercase()).copied()
}

fn ymd(y: i32, m: u32, d: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(y, m, d)
}

/// All dates with a year, as (byte position, date), in text order.
fn dates_in(text: &str) -> Vec<(usize, NaiveDate)> {
    let mut v = Vec::new();
    for c in re_d_num().captures_iter(text) {
        let (a, b, y): (u32, u32, i32) = (c[1].parse().unwrap_or(0), c[2].parse().unwrap_or(0), c[3].parse().unwrap_or(0));
        // European day-first, unless that is impossible (US month-first)
        let d = if b > 12 && a <= 12 { ymd(y, a, b) } else { ymd(y, b, a) };
        v.extend(d.map(|d| (c.get(0).unwrap().start(), d)));
    }
    for c in re_d_iso().captures_iter(text) {
        v.extend(ymd(c[1].parse().unwrap_or(0), c[2].parse().unwrap_or(0), c[3].parse().unwrap_or(0)).map(|d| (c.get(0).unwrap().start(), d)));
    }
    for c in re_d_dmy().captures_iter(text) {
        if let Some(m) = month_num(&c[2]) {
            v.extend(ymd(c[3].parse().unwrap_or(0), m, c[1].parse().unwrap_or(0)).map(|d| (c.get(0).unwrap().start(), d)));
        }
    }
    for c in re_d_mdy().captures_iter(text) {
        if let Some(m) = month_num(&c[1]) {
            v.extend(ymd(c[3].parse().unwrap_or(0), m, c[2].parse().unwrap_or(0)).map(|d| (c.get(0).unwrap().start(), d)));
        }
    }
    v.sort();
    v.dedup();
    v
}

/// Dates without a year ("15 January", "15.01."): the next such day from today.
fn yearless_dates_in(text: &str, today: NaiveDate) -> Vec<(usize, NaiveDate)> {
    let next = |m: u32, d: u32| -> Option<NaiveDate> {
        let this = ymd(today.year(), m, d)?;
        if this >= today { Some(this) } else { ymd(today.year() + 1, m, d) }
    };
    let mut v = Vec::new();
    for c in re_d_dm().captures_iter(text) {
        if let Some(m) = month_num(&c[2]) {
            v.extend(next(m, c[1].parse().unwrap_or(0)).map(|d| (c.get(0).unwrap().start(), d)));
        }
    }
    for c in re_d_md().captures_iter(text) {
        if let Some(m) = month_num(&c[1]) {
            v.extend(next(m, c[2].parse().unwrap_or(0)).map(|d| (c.get(0).unwrap().start(), d)));
        }
    }
    for c in re_d_numdm().captures_iter(text) {
        let (d, m): (u32, u32) = (c[1].parse().unwrap_or(0), c[2].parse().unwrap_or(0));
        v.extend(next(m, d).map(|x| (c.get(0).unwrap().start(), x)));
    }
    v.sort();
    v
}

/// Deadline: the first date right after a deadline word ("Deadline:", "Frist", "date limite", "scadenza"...).
/// Among those, the earliest future one wins. Falls back to the page's only future date.
pub fn find_deadline(text: &str, today: NaiveDate) -> Option<NaiveDate> {
    let mut found = Vec::new();
    for m in re_deadline_kw().find_iter(text) {
        let w = window(text, m.end(), m.end() + 140);
        let mut d = dates_in(w);
        if d.is_empty() {
            d = yearless_dates_in(w, today);
        }
        if let Some((_, first)) = d.first() {
            found.push(*first);
        }
    }
    found.sort();
    if let Some(d) = found.iter().find(|d| **d >= today).or(found.last()) {
        return Some(*d);
    }
    let horizon = today + chrono::Duration::days(730);
    let mut future: Vec<NaiveDate> = dates_in(text).into_iter().map(|x| x.1).filter(|d| *d >= today && *d <= horizon).collect();
    future.sort();
    future.dedup();
    if future.len() == 1 { Some(future[0]) } else { None }
}

fn fmt_range(a: NaiveDate, b: Option<NaiveDate>) -> String {
    match b {
        Some(b) if b > a => format!("{} – {}", a.format("%Y-%m-%d"), b.format("%Y-%m-%d")),
        _ => a.format("%Y-%m-%d").to_string(),
    }
}

/// A day range like "2–4 July 2027" or "July 2-4, 2027" at the start of the text.
fn range_in(text: &str) -> Option<(usize, NaiveDate, NaiveDate)> {
    let mut best: Option<(usize, NaiveDate, NaiveDate)> = None;
    for c in re_d_range_dm().captures_iter(text) {
        if let (Some(m), Ok(y)) = (month_num(&c[3]), c[4].parse()) {
            if let (Some(a), Some(b)) = (ymd(y, m, c[1].parse().unwrap_or(0)), ymd(y, m, c[2].parse().unwrap_or(0))) {
                best = best.filter(|x| x.0 < c.get(0).unwrap().start()).or(Some((c.get(0).unwrap().start(), a, b)));
            }
        }
    }
    for c in re_d_range_md().captures_iter(text) {
        if let (Some(m), Ok(y)) = (month_num(&c[1]), c[4].parse()) {
            if let (Some(a), Some(b)) = (ymd(y, m, c[2].parse().unwrap_or(0)), ymd(y, m, c[3].parse().unwrap_or(0))) {
                best = best.filter(|x| x.0 < c.get(0).unwrap().start()).or(Some((c.get(0).unwrap().start(), a, b)));
            }
        }
    }
    best
}

/// When the event itself happens: dates right after "takes place", "When", "findet ... statt",
/// "aura lieu", "si terrà"... A second date close behind the first makes a range. Not the deadline.
pub fn find_event(text: &str, deadline: Option<NaiveDate>, today: NaiveDate) -> Option<String> {
    for m in re_event_kw().find_iter(text) {
        let w = window(text, m.end(), m.end() + 160);
        let d = dates_in(w);
        let r = range_in(w);
        let start = match (r, d.first()) {
            (Some(r), Some(f)) if r.0 <= f.0 => Some((r.1, Some(r.2))),
            (Some(r), None) => Some((r.1, Some(r.2))),
            (_, Some(f)) => {
                let second = d.get(1).filter(|x| x.0 - f.0 < 40 && x.1 > f.1 && (x.1 - f.1).num_days() <= 90).map(|x| x.1);
                Some((f.1, second))
            }
            _ => None,
        };
        if let Some((a, b)) = start {
            if Some(a) == deadline || b.unwrap_or(a) < today {
                continue;
            }
            return Some(fmt_range(a, b));
        }
    }
    None
}

// ---------------------------------------------------------------- money

fn parse_num(n: &str, mult: Option<&str>) -> Option<u64> {
    let m = match mult.map(|s| s.to_lowercase()).as_deref() {
        Some("k") | Some("tsd") => 1_000.0,
        Some("mio") | Some("mln") | Some("m") | Some("million") | Some("millions") | Some("millionen") => 1_000_000.0,
        _ => 1.0,
    };
    // "10.000,00" -> 10000: thousands groups, decimals dropped
    let v: f64 = if let Some(g) = re_grouped().find(n) {
        g.as_str().chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse().ok()?
    } else {
        n.replace(',', ".").parse().ok()?
    };
    Some((v * m) as u64)
}

/// Money amounts in text order: (value, currency).
fn amounts_in(text: &str) -> Vec<(u64, String)> {
    let mut v = Vec::new();
    for c in re_money().captures_iter(text) {
        let (sym, num, mult) = if let Some(n) = c.get(2) {
            (c.get(1).unwrap().as_str(), n.as_str(), c.get(3).map(|m| m.as_str()))
        } else {
            (c.get(6).unwrap().as_str(), c.get(4).unwrap().as_str(), c.get(5).map(|m| m.as_str()))
        };
        if let Some(x) = parse_num(num, mult).filter(|x| *x > 0 && *x < 10_000_000) {
            let cur = match sym.to_lowercase().as_str() {
                "$" | "usd" => "USD",
                "£" | "gbp" => "GBP",
                "chf" => "CHF",
                _ => "EUR",
            };
            v.push((x, cur.to_string()));
        }
    }
    v
}

/// Largest money amount in the text: (min, max, currency).
pub fn find_amount(text: &str) -> (u64, u64, String) {
    amounts_in(text).into_iter().max_by_key(|a| a.0).map(|(x, c)| (0, x, c)).unwrap_or((0, 0, "EUR".into()))
}

/// Amount tied to a money word (prize, Preisgeld, dotation, premio...): the first amount after it,
/// else the last one just before it. The largest such amount wins; budgets elsewhere are ignored.
fn amount_near(text: &str) -> (u64, u64, String) {
    let mut best = (0, 0, "EUR".to_string());
    for m in re_money_kw().find_iter(text) {
        if m.as_str().eq_ignore_ascii_case("fee") && re_fee().is_match(window(text, m.start().saturating_sub(16), m.end())) {
            continue;
        }
        let after = amounts_in(window(text, m.end(), m.end() + 120)).into_iter().next();
        let a = after.or_else(|| amounts_in(window(text, m.start().saturating_sub(40), m.start())).pop());
        if let Some((x, c)) = a {
            if x > best.1 {
                best = (0, x, c);
            }
        }
    }
    best
}

// ---------------------------------------------------------------- classification

fn guess_tags(text: &str) -> Vec<String> {
    let t = fold(text);
    let table: &[(&str, &[&str])] = &[
        ("sound art", &["sound art", "klangkunst", "art sonore", "arte sonoro", "arte sonora", "geluidskunst", "sztuka dzwieku", "ljudkonst", "lydkunst"]),
        ("audio", &["audio"]),
        ("music", &["music", "musik", "musique", "musica", "muziek", "muzyk"]),
        ("electroacoustic", &["electroacoustic", "elektroakust", "electroacoust", "elettroacust", "acousmat"]),
        ("computer music", &["computer music", "computermusik", "informatique musicale"]),
        ("music technology", &["music technology", "music tech", "musiktechnolog", "music informatics", "music information retrieval", "ismir"]),
        ("spatial audio", &["spatial audio", "ambisonic", "wave field", "binaural", "3d audio", "immersive audio"]),
        ("immersive", &["immersive", "virtual reality", " vr ", " xr ", "augmented reality"]),
        ("signal processing", &["signal processing", "signalverarbeitung", "traitement du signal", "procesamiento de senales", " dsp", "dafx"]),
        ("acoustics", &["acoustic", "akustik", "acoustique"]),
        ("speech", &["speech", "sprachverarbeitung"]),
        ("media art", &["media art", "medienkunst", "new media", "digital art", "arts numeriques", "arte digital", "arte digitale", "digitale kunst", "net art"]),
        ("audiovisual", &["audiovisual", "audiovisuell", "audiovisuel"]),
        ("installation", &["installation", "instalacion", "installazione"]),
        ("art science", &["art science", "art and science", "art & science", "kunst und wissenschaft", "art et science", "sciart", "s+t+arts", "starts prize"]),
        ("research", &["research", "forschung", "recherche", "investigacion", "ricerca", "onderzoek", "badani", "pesquisa"]),
        ("quantum computing", &["quantum", "quanten", "quantique", "cuantic", "quantistic", "kwantow"]),
        ("mathematics", &["mathemat", "mathematik", "matematic", "matematyk", "wiskunde", "algebra", "geometr", "topolog", "number theory", "combinatori", "probability", "real analysis", "functional analysis", "harmonic analysis"]),
        ("algorithms", &["algorithm", "algorithmus", "algorithme"]),
        ("machine learning", &["machine learning", "deep learning", "neural", "artificial intelligence", "kunstliche intelligenz"]),
        ("open source", &["open source", "open-source", "free software", "quelloffen", " foss"]),
        ("software", &["software", "programming"]),
        ("tools", &["plugin", "plug-in", " vst", "toolkit"]),
        ("instruments", &["musical instrument", "instrument building", "instrument design", "synthesizer", "eurorack", "modular synth"]),
        ("interfaces", &["musical interface", "musical expression", "nime", "midi controller", "human computer interaction", " hci "]),
        ("game audio", &["game audio", "game sound", "wwise", "fmod", "game jam", "gamejam"]),
        ("creative coding", &["creative coding", "openframeworks", "processing.org", "generative"]),
        ("open hardware", &["open hardware", "open-source hardware", "arduino", "eurorack", "diy electronics"]),
        ("workshops", &["workshop"]),
        ("education", &["education", "bildung", "teaching"]),
        ("residency", &["residenc", "residenz", "residence"]),
    ];
    table
        .iter()
        .filter(|(_, words)| words.iter().any(|w| t.contains(w)))
        .map(|(tag, _)| tag.to_string())
        .collect()
}

fn guess_kind(name: &str, text: &str) -> Option<String> {
    let rules: &[(&str, &[&str])] = &[
        ("Hackathon", &["hackathon", "hack day", "hackday", "game jam", "datathon", "codefest", "hacknight"]),
        ("Conference", &["call for speakers", "call for talks", "call for presentations", "speaker application", "cfp for talks", "talk proposals"]),
        ("Call for papers", &["call for papers", "call for abstracts", "call for contributions", "call for posters", "paper submission", "appel a communications", "cfp"]),
        ("Residency", &["residenc", "residenz", "residence", "residencia", "residenza", "residentie", "rezydencj"]),
        ("Fellowship", &["fellowship"]),
        ("Stipend", &["stipend", "stipendi", "scholarship", "bourse", "beca", "borsa di studio", "bolsa", "stypendi", "beurs", "bursary"]),
        ("Prize", &["prize", "award", "preis", "prix", "premio", "premi", "prijs", "nagrod"]),
        ("Competition", &["competition", "contest", "wettbewerb", "concours", "concorso", "concurso", "konkurs", "wedstrijd", "challenge", "tavling"]),
        ("Grant", &["grant", "forder", "funding", "subvention", "subvencion", "subsidie", "dotacj", "zuschuss", "fund"]),
        ("Accelerator", &["accelerator", "incubator", "inkubator"]),
        ("Open call", &["open call", "call for", "ausschreibung", "appel a", "convocatoria", "bando", "oproep", "convocatoria", "edital", "nabor"]),
    ];
    let n = fold(name);
    let mut t = fold(text);
    truncate_chars(&mut t, 2500);
    for hay in [&n, &t] {
        for (kind, words) in rules {
            if words.iter().any(|w| hay.contains(w)) {
                return Some(kind.to_string());
            }
        }
    }
    None
}

fn live_opp(source: &str, name: String, url: &str, info: &str, snippet: String, today: NaiveDate) -> Opportunity {
    let text = format!("{} {}", name, snippet);
    let deadline = find_deadline(&text, today);
    let event = find_event(&text, deadline, today).unwrap_or_default();
    let (amin, amax, cur) = find_amount(&text);
    Opportunity {
        id: format!("live:{}", url_key(url)),
        kind: guess_kind(&name, &snippet).unwrap_or_else(|| "Open call".into()),
        name,
        org: source.to_string(),
        city: String::new(),
        country: String::new(),
        scope: "World".into(),
        amount_min: amin,
        amount_max: amax,
        currency: cur,
        amount_note: if amax > 0 { "Amount read from listing text - verify".into() } else { "Not stated in listing".into() },
        opens: None,
        deadline: deadline.map(|d| d.format("%Y-%m-%d").to_string()),
        event,
        rolling: false,
        cadence: String::new(),
        next_expected: String::new(),
        entities: vec![],
        requires: vec!["unverified".into()],
        tags: guess_tags(&text),
        apply_url: url.to_string(),
        info_url: info.to_string(),
        summary: snippet,
        checked: today.format("%Y-%m-%d").to_string(),
        confidence: "live".into(),
        warnings: vec![],
    }
}

/// live_opp plus warnings from its text.
fn live_item(source: &str, name: String, url: &str, info: &str, snippet: String, today: NaiveDate) -> Opportunity {
    let text = format!("{} {}", name, snippet);
    let mut o = live_opp(source, name, url, info, snippet, today);
    flag(&mut o, &text);
    o
}

// ---------------------------------------------------------------- warnings

pub const CLOSED: &str = "Call page says submissions are closed";
/// start of enrich's warning for a page behind JavaScript, a login or a CAPTCHA
const UNREADABLE: &str = "Page needs JavaScript or a login";

/// A search snippet that starts with its post date ("Dec 2, 2025 - ...") more than 180 days ago.
fn old_post(snippet: &str, today: NaiveDate) -> bool {
    dates_in(window(snippet, 0, 30)).first().map(|(_, d)| (today - *d).num_days() > 180).unwrap_or(false)
}

/// The page says the call is over and names no deadline still ahead.
fn says_closed(text: &str, today: NaiveDate) -> bool {
    re_closed().is_match(text) && find_deadline(text, today).map(|d| d < today).unwrap_or(true)
}

pub fn warn(o: &mut Opportunity, w: &str) {
    if !o.warnings.iter().any(|x| x == w) {
        o.warnings.push(w.to_string());
    }
}

re!(
    re_fee,
    r"(?i)(?:entry|application|submission|registration|processing|reading|jury|participation|administration|admin)\s+fees?|teilnahmegebühr|bewerbungsgebühr|anmeldegebühr|einreichungsgebühr|frais d.(?:inscription|participation|dossier)|cuota de (?:inscripción|participación)|tasa de inscripción|quota di (?:iscrizione|partecipazione)|inschrijfgeld|taxa de inscrição|opłata (?:rejestracyjna|zgłoszeniowa)"
);
re!(
    re_no_fee,
    r"(?i)\bno (?:entry |application |submission |registration )?fees?\b|free (?:to apply|to enter|of charge|entry)|without (?:a |any )?fee|kostenlos|gebührenfrei|keine (?:teilnahme|bewerbungs|anmelde)?gebühr|gratuit|sin costo|gratuita|\bgratis\b"
);
re!(
    re_restrict,
    r"(?i)only (?:open )?(?:to|for)|open only to|exclusively (?:for|to)|restricted to|limited to|must be (?:a )?(?:legal )?(?:residents?|citizens?|nationals?|based) (?:of|in)|(?:residents|citizens|nationals) of|(?:artists|applicants|participants|candidates|individuals|organi[sz]ations|those) (?:who are |that are )?(?:based|living|residing) in|nur für|ausschließlich für|mit (?:erstem )?wohnsitz in|réservé aux?|exclusivamente para|riservat[oa] a"
);
re!(
    re_predatory,
    r"(?i)waset|world academy of science|scirp|scientific research publishing|conferenceindex|allconferencealert|conferencealerts|internationalconferencealerts|omicsonline|researchfora|conferenceworld|article processing charge|publication fee"
);
re!(
    re_unreadable,
    r"(?i)enable javascript|javascript (?:is )?required|please (?:log|sign) in|(?:sign|log) in to (?:view|continue|see)|members only|access denied|just a moment\.\.\.|checking your browser|verify you are human"
);

/// Words after "only open to ..." that include Trivune (Berlin, Europe, worldwide...).
const REGION_OK: &[&str] = &["berlin", "brandenburg", "europe", "european", " eu", "eu ", "eu-", "schengen", "eea", "international", "worldwide", "all countries", "any country", "anywhere", "global", "all nationalities"];
/// "Germany" alone includes Trivune, but not a German state or another city.
const REGION_DE: &[&str] = &["germany", "german", "deutschland", "bundesweit"];
const REGION_DE_PART: &[&str] = &[
    "bavaria", "bayern", "hamburg", "bremen", "nrw", "nordrhein", "westfalen", "sachsen", "saxony", "baden", "wurttemberg", "hessen", "hesse",
    "niedersachsen", "lower saxony", "thuring", "rheinland", "saarland", "schleswig", "mecklenburg", "munich", "munchen", "cologne", "koln",
    "frankfurt", "stuttgart", "leipzig", "dresden", "dusseldorf", "hannover", "nurnberg", "karlsruhe",
];
const REGION_ABROAD: &[&str] = &[
    "canad", "united states", "u.s.", " usa", "american", "united kingdom", " uk", "british", "england", "scotland", "wales", "ireland", "irish",
    "australia", "new zealand", "india", "nigeria", "south africa", "kenya", "ghana", "brazil", "mexic", "argentin", "chile", "colombia", "japan",
    "korea", "china", "chinese", "singapore", "hong kong", "taiwan", "israel", "turkey", "france", "french", "netherlands", "dutch", "belgi",
    "switzerland", "swiss", "austria", "austrian", "italy", "italian", "spain", "spanish", "portugal", "portuguese", "poland", "polish",
    "sweden", "swedish", "norway", "norwegian", "denmark", "danish", "finland", "finnish", "iceland", "czech", "slovak", "hungar", "romania",
    "bulgaria", "greece", "greek", "croatia", "sloven", "serbia", "ukrain", "estonia", "latvia", "lithuania", "luxembourg", "new york",
    "california", "texas", "london", "toronto", "montreal", "quebec", "ontario", "vancouver", "sydney", "melbourne", "paris", "vienna",
    "wien", "zurich", "amsterdam", "flanders", "wallonia", "scandinavia", "nordic", "latin america", "africa", "asia",
];

/// The text after a "only open to ..." phrase, when it names a region that excludes Trivune.
fn region_limit(text: &str) -> Option<String> {
    for m in re_restrict().find_iter(text) {
        let w = window(text, m.end(), m.end() + 70);
        let f = format!(" {} ", fold(w));
        // list words folded like the text ("Québec" -> "qubec")
        let has = |list: &[&str]| list.iter().any(|x| f.contains(&fold(x)));
        if has(REGION_OK) {
            continue;
        }
        if has(REGION_DE_PART) || (has(REGION_ABROAD) && !has(REGION_DE)) {
            return Some(format!("{} {}", m.as_str(), w.trim()));
        }
    }
    None
}

/// Broad fields in one conference title: 3 or more is a pay-to-publish pattern.
fn broad_fields(title: &str) -> usize {
    let t = fold(title);
    [
        "engineering", "management", "business", "economic", "finance", "social science", "humanities", "education", "medicine", "medical",
        "health", "agricultur", "environment", " law", "tourism", "marketing", "psycholog", "biolog", "chemi", "material", "energy",
        "civil", "mechanical", "electrical", "information technology", "communication", "architecture", "nursing", "pharma", "sport",
    ]
    .iter()
    .filter(|w| t.contains(*w))
    .count()
}

fn fee_text(text: &str) -> Option<String> {
    for m in re_fee().find_iter(text) {
        if re_no_fee().is_match(window(text, m.start().saturating_sub(40), m.end() + 40)) {
            continue;
        }
        let after = window(text, m.end(), m.end() + 80);
        let amt = amounts_in(after).into_iter().next().or_else(|| amounts_in(window(text, m.start().saturating_sub(30), m.start())).pop());
        return Some(match amt {
            Some((v, cur)) => {
                let sym = match cur.as_str() {
                    "USD" => "$",
                    "GBP" => "£",
                    "CHF" => "CHF ",
                    _ => "€",
                };
                format!("Entry fee: {}{}", sym, v)
            }
            None => "Entry fee mentioned: check the page".to_string(),
        });
    }
    None
}

/// Warnings from text: region limit (hides it as not eligible), entry fee, pay-to-publish signals.
fn flag(o: &mut Opportunity, text: &str) {
    if let Some(r) = region_limit(text) {
        o.scope = "Restricted".into();
        warn(o, &format!("Limited to: \"{}\"", r));
    }
    if let Some(f) = fee_text(text) {
        warn(o, &f);
    }
    if re_predatory().is_match(&o.apply_url) || re_predatory().is_match(text) || broad_fields(&o.name) >= 3 {
        warn(o, "Possibly predatory (pay-to-publish signals): check before you submit");
    }
}

// ---------------------------------------------------------------- parsers

/// Text between the end of an anchor and the next anchor, max ~300 chars.
fn snippet_after(page: &str, end: usize) -> String {
    let next = page[end..].find("<a ").map(|i| end + i).unwrap_or(page.len());
    let stop = next.min(end + 900);
    let mut s = strip_tags(window(page, end, stop));
    truncate_chars(&mut s, 300);
    s
}

/// Opportunity links on an HTML listing page. Same-site links pass on a keyword in the title,
/// path or nearby text; links to other sites need the keyword in the title or path.
pub fn harvest(source: &str, base: &str, page: &str, today: NaiveDate) -> Vec<Opportunity> {
    let base_host = host(base);
    let mut out: Vec<Opportunity> = Vec::new();
    let mut seen = HashSet::new();
    for c in re_anchor().captures_iter(page) {
        let Some(href) = attr(re_href(), c.get(1).unwrap().as_str()) else { continue };
        let href = decode_entities(href);
        if re_skip().is_match(&href) {
            continue;
        }
        let title = strip_tags(c.get(2).unwrap().as_str());
        let tl = title.chars().count();
        if !(12..=200).contains(&tl) || junk_title(&title) {
            continue;
        }
        let url = join_url(base, &href);
        if !url.starts_with("http") {
            continue;
        }
        let h = host(&url);
        let path = url.split_once(&h).map(|x| x.1).unwrap_or("");
        if path.trim_matches('/').is_empty() || url_key(&url) == url_key(base) {
            continue;
        }
        let snippet = snippet_after(page, c.get(0).unwrap().end());
        let strong = re_kw().is_match(&title) || re_kw().is_match(path);
        let ok = if h == base_host { strong || re_kw().is_match(&snippet) } else { strong && !re_engine_host().is_match(&h) };
        if !ok || !seen.insert(url_key(&url)) {
            continue;
        }
        let org = if h == base_host { source.to_string() } else { h.clone() };
        out.push(live_item(&org, title, &url, base, snippet, today));
        if out.len() >= MAX_PER_PAGE {
            break;
        }
    }
    out
}

/// Next-page link of a listing (rel=next or a "next / weiter / suivant / ›" link).
fn next_page(base: &str, page: &str) -> Option<String> {
    if let Some(m) = re_rel_next().find(page) {
        if let Some(h) = attr(re_href(), m.as_str()) {
            return Some(join_url(base, &decode_entities(h)));
        }
    }
    for c in re_anchor().captures_iter(page) {
        let text = strip_tags(c.get(2).unwrap().as_str());
        if re_next().is_match(text.trim()) {
            if let Some(h) = attr(re_href(), c.get(1).unwrap().as_str()) {
                let h = decode_entities(h);
                if !h.starts_with('#') && !h.starts_with("javascript") {
                    return Some(join_url(base, &h));
                }
            }
        }
    }
    None
}

fn feed_links(base: &str, page: &str) -> Vec<String> {
    re_feed_link()
        .find_iter(page)
        .filter_map(|m| attr(re_href(), m.as_str()).map(|h| join_url(base, &decode_entities(h))))
        .filter(|u| !u.contains("comments"))
        .take(2)
        .collect()
}

fn is_feed(body: &str) -> bool {
    let head = window(body, 0, 600);
    head.contains("<rss") || head.contains("<feed") || (head.starts_with("<?xml") && body.contains("<item"))
}

fn uncdata(s: &str) -> String {
    let s = s.replace("<![CDATA[", "").replace("]]>", "");
    strip_tags(&decode_entities(&s))
}

/// RSS/Atom items. `all`: every item counts, else keyword filter.
pub fn harvest_feed(source: &str, base: &str, body: &str, all: bool, today: NaiveDate) -> Vec<Opportunity> {
    let mut out = Vec::new();
    for m in re_item().find_iter(body) {
        let it = m.as_str();
        let title = re_title_tag().captures(it).map(|c| uncdata(&c[1])).unwrap_or_default();
        let link = re_link_text()
            .captures(it)
            .map(|c| uncdata(&c[1]))
            .filter(|l| l.starts_with("http"))
            .or_else(|| re_link_href().captures(it).map(|c| decode_entities(&c[1])));
        let Some(link) = link else { continue };
        let mut desc = re_desc_tag().captures(it).map(|c| uncdata(&c[1])).unwrap_or_default();
        truncate_chars(&mut desc, 400);
        if title.chars().count() < 8 || (!all && !(re_kw().is_match(&title) || re_kw().is_match(&desc))) {
            continue;
        }
        let mut o = live_item(source, title, &join_url(base, &link), base, desc.clone(), today);
        if all && o.event.is_empty() {
            // WikiCFP: "[City] [Jul 2, 2027 - Jul 4, 2027]"
            let d = dates_in(&desc);
            if let Some(f) = d.first() {
                o.event = fmt_range(f.1, d.last().map(|l| l.1));
            }
        }
        out.push(o);
        if out.len() >= MAX_PER_PAGE {
            break;
        }
    }
    out
}

/// External result links of a search engine page (DuckDuckGo redirects decoded).
/// `need_kw`: keep only results that look like a call (crawl); off for a user's own search.
pub fn harvest_search(engine: &str, page: &str, need_kw: bool, today: NaiveDate) -> Vec<Opportunity> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for c in re_anchor().captures_iter(page) {
        let attrs = c.get(1).unwrap().as_str();
        // favicon, site name and shown-URL links come before the title link (Startpage, Seznam)
        if ["favicon-link", "wgl-site-title", "wgl-display-url", "data-e-a=\"reference\""].iter().any(|k| attrs.contains(k)) {
            continue;
        }
        let Some(href) = attr(re_href(), attrs) else { continue };
        let mut url = decode_entities(href);
        if let Some(i) = url.find("uddg=") {
            url = percent_decode(url[i + 5..].split('&').next().unwrap_or(""));
        }
        if !url.starts_with("http") || re_skip().is_match(&url) {
            continue;
        }
        let h = host(&url);
        if re_engine_host().is_match(&h) {
            continue;
        }
        let mut title = strip_tags(c.get(2).unwrap().as_str());
        // Brave puts "Site host › path › slug" before the title: keep what follows the slug
        if let Some(i) = title.rfind(" › ") {
            let rest = &title[i + " › ".len()..];
            title = clean_title(rest.split_once(' ').map(|x| x.1).unwrap_or(rest));
        }
        truncate_chars(&mut title, 200);
        if title.chars().count() < 10 || junk_title(&title) {
            continue;
        }
        let snippet = snippet_after(page, c.get(0).unwrap().end());
        if need_kw && (old_post(&snippet, today) || !(re_kw().is_match(&title) || re_kw().is_match(&url) || re_kw().is_match(&snippet))) {
            continue;
        }
        if !seen.insert(url_key(&url)) {
            continue;
        }
        let mut o = live_item(&h, title, &url, &url, snippet, today);
        o.amount_note = "Found by web search - verify".into();
        o.cadence = format!("via {}", engine);
        out.push(o);
    }
    out
}

pub fn harvest_devpost(body: &str, today: NaiveDate) -> Vec<Opportunity> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else { return vec![] };
    let mut out = Vec::new();
    for h in v["hackathons"].as_array().into_iter().flatten() {
        if h["open_state"].as_str() == Some("ended") {
            continue;
        }
        let (Some(title), Some(url)) = (h["title"].as_str(), h["url"].as_str()) else { continue };
        let dates = h["submission_period_dates"].as_str().unwrap_or("");
        let prize = strip_tags(h["prize_amount"].as_str().unwrap_or(""));
        let themes: Vec<String> = h["themes"].as_array().into_iter().flatten().filter_map(|t| t["name"].as_str().map(str::to_string)).collect();
        let place = h["displayed_location"]["location"].as_str().unwrap_or("Online").to_string();
        let summary = format!("Hackathon by {}. {}. Submissions {}. Themes: {}.", h["organization_name"].as_str().unwrap_or("?"), place, dates, themes.join(", "));
        let mut o = live_opp("Devpost", title.to_string(), url, url, summary.clone(), today);
        o.kind = "Hackathon".into();
        let d = dates_in(dates);
        o.deadline = d.last().map(|x| x.1.format("%Y-%m-%d").to_string());
        o.event = d.first().map(|f| format!("hackathon {}", fmt_range(f.1, d.last().map(|l| l.1)))).unwrap_or_default();
        let (_, amt, cur) = find_amount(&prize);
        o.amount_max = amt;
        o.currency = cur;
        o.amount_note = if amt > 0 { "Total prize pool (Devpost)".into() } else { "Non-cash prizes or none".into() };
        if place != "Online" {
            o.city = place;
        }
        o.tags = guess_tags(&format!("{} {}", title, summary));
        o.tags.push("hackathon".into());
        out.push(o);
    }
    out
}

pub fn harvest_confs(body: &str, topic: &str, today: NaiveDate) -> Vec<Opportunity> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else { return vec![] };
    let mut out = Vec::new();
    for c in v.as_array().into_iter().flatten() {
        let (Some(name), Some(cfp), Some(end)) = (c["name"].as_str(), c["cfpUrl"].as_str(), c["cfpEndDate"].as_str()) else { continue };
        let Ok(end_d) = NaiveDate::parse_from_str(end, "%Y-%m-%d") else { continue };
        if end_d < today {
            continue;
        }
        let city = c["city"].as_str().unwrap_or("").to_string();
        let country = c["country"].as_str().unwrap_or("").to_string();
        let online = c["online"].as_bool().unwrap_or(false);
        let summary = format!(
            "{} conference, {} to {}, {}{}. Talk proposals until {}.",
            topic,
            c["startDate"].as_str().unwrap_or("?"),
            c["endDate"].as_str().unwrap_or("?"),
            if city.is_empty() { "location tba".to_string() } else { format!("{}, {}", city, country) },
            if online { ", also online" } else { "" },
            end
        );
        let mut o = live_opp("confs.tech", format!("{}: call for speakers", name), cfp, c["url"].as_str().unwrap_or(cfp), summary, today);
        o.kind = "Conference".into();
        o.deadline = Some(end.to_string());
        let sd = c["startDate"].as_str().and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
        let ed = c["endDate"].as_str().and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
        o.event = sd.map(|a| fmt_range(a, ed)).unwrap_or_default();
        o.city = city;
        o.country = country;
        o.amount_note = "Speaking slot; travel support varies".into();
        o.tags.push(topic.to_string());
        out.push(o);
    }
    out
}

pub fn harvest_eu(body: &str, today: NaiveDate) -> Vec<Opportunity> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else { return vec![] };
    let first = |x: &serde_json::Value| -> String {
        x.as_array().and_then(|a| a.first()).and_then(|s| s.as_str()).unwrap_or("").to_string()
    };
    let mut out = Vec::new();
    for r in v["results"].as_array().into_iter().flatten() {
        let m = &r["metadata"];
        let id = first(&m["identifier"]);
        let title = first(&m["title"]);
        if id.is_empty() || title.is_empty() {
            continue;
        }
        let deadlines: Vec<NaiveDate> = m["deadlineDate"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|d| d.as_str().and_then(|s| NaiveDate::parse_from_str(&s[..10.min(s.len())], "%Y-%m-%d").ok()))
            .collect();
        let deadline = deadlines.iter().filter(|d| **d >= today).min().copied();
        if deadline.is_none() && !deadlines.is_empty() {
            continue;
        }
        let url = format!("https://ec.europa.eu/info/funding-tenders/opportunities/portal/screen/opportunities/topic-details/{}", id);
        let call = first(&m["callTitle"]);
        let opens = first(&m["startDate"]);
        let summary = format!("EU call {} ({}). {}", id, first(&m["frameworkProgramme"]).trim(), call);
        let mut o = live_opp("EU Funding & Tenders", title, &url, &url, summary, today);
        o.kind = "Grant".into();
        o.scope = "EU".into();
        o.deadline = deadline.map(|d| d.format("%Y-%m-%d").to_string());
        o.opens = (opens.len() >= 10).then(|| opens[..10].to_string());
        o.amount_note = "See the call's budget table".into();
        o.requires = vec!["unverified".into(), "legal_entity".into()];
        out.push(o);
    }
    out
}

/// Fill in a candidate from its own page: title, summary, deadline, amount, kind, tags.
fn enrich(c: &mut Cand, body: &str, today: NaiveDate) {
    let o = &mut c.o;
    let text = page_text(body);
    if text.chars().count() < 250 || re_unreadable().is_match(window(&text, 0, 3000)) {
        warn(o, &format!("{}: details not read, open it", UNREADABLE));
    }
    if c.retitle || is_generic(&o.name) {
        // first title that says something: og:title, then <h1>, then <title>
        let bad = |t: &str| {
            let f = fold(t);
            f.chars().count() < 6 || ["home", "homepage", "start", "startseite", "accueil", "inicio", "welcome", "willkommen", "index", "news", "aktuelles", "untitled"].contains(&f.trim())
        };
        let t = [
            meta(body, &["og:title", "twitter:title"]),
            re_h1().captures(body).map(|x| strip_tags(&x[1])),
            re_title_tag().captures(body).map(|x| x[1].to_string()),
        ]
        .into_iter()
        .flatten()
        .map(|t| clean_title(&t))
        .find(|t| !bad(t));
        if let Some(t) = t {
            o.name = t;
        }
    }
    let desc = meta(body, &["og:description", "description", "twitter:description"]);
    if let Some(d) = desc {
        if d.chars().count() > o.summary.chars().count() / 2 {
            o.summary = d;
        }
    }
    if o.summary.chars().count() < 60 {
        if let Some(m) = re_kw().find(&text) {
            o.summary = window(&text, m.start().saturating_sub(80), m.start() + 320).to_string();
        }
    }
    truncate_chars(&mut o.summary, 500);
    if let Some(d) = find_deadline(&text, today) {
        o.deadline = Some(d.format("%Y-%m-%d").to_string());
    }
    if let Some(e) = find_event(&text, crate::model::parse_date(&o.deadline), today) {
        o.event = e;
    }
    let (_, amt, cur) = amount_near(&text);
    if amt > 0 {
        o.amount_max = amt;
        o.currency = cur;
        o.amount_note = "Amount read from the call page - verify".into();
    }
    if let Some(k) = guess_kind(&o.name, &format!("{} {}", o.summary, text)) {
        o.kind = k;
    }
    let mut head = text.clone();
    truncate_chars(&mut head, 1500);
    for t in guess_tags(&format!("{} {} {}", o.name, o.summary, head)) {
        if !o.tags.contains(&t) {
            o.tags.push(t);
        }
    }
    flag(o, &text);
    if says_closed(&text, today) {
        warn(o, CLOSED);
    }
    if let Some(l) = re_lang().captures(body) {
        let l = l[1].to_lowercase();
        if l != "en" {
            o.cadence = format!("{} page", l.to_uppercase()).trim().to_string();
        }
    }
}

// ---------------------------------------------------------------- fetching

/// The whole crawl stops after this many seconds; everything found by then is kept.
pub const BUDGET_SECS: u64 = 60;

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .redirects(8)
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0 Safari/537.36")
        .build()
}

fn why(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, _) => format!("HTTP {}", code),
        ureq::Error::Transport(t) => t.kind().to_string(),
    }
}

/// GET (or EU POST) a page: (final url, body). Err: the reason (HTTP status, timeout, not text).
fn fetch(agent: &ureq::Agent, job: &Job) -> Result<(String, String), String> {
    let resp = if job.how == How::Eu {
        let boundary = "callfitboundary7d1";
        let query = r#"{"bool":{"must":[{"terms":{"type":["1","2","8"]}},{"terms":{"status":["31094501","31094502"]}}]}}"#;
        let body = format!(
            "--{b}\r\nContent-Disposition: form-data; name=\"query\"\r\nContent-Type: application/json\r\n\r\n{q}\r\n--{b}\r\nContent-Disposition: form-data; name=\"languages\"\r\nContent-Type: application/json\r\n\r\n[\"en\"]\r\n--{b}--\r\n",
            b = boundary,
            q = query
        );
        agent
            .post(&job.url)
            .set("Content-Type", &format!("multipart/form-data; boundary={}", boundary))
            .send_string(&body)
            .map_err(why)?
    } else {
        agent
            .get(&job.url)
            .set("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,application/json;q=0.8,*/*;q=0.5")
            .set("Accept-Language", "en,de;q=0.9,fr;q=0.8,es;q=0.7,it;q=0.6,nl;q=0.5,pt;q=0.5,pl;q=0.4")
            .call()
            .map_err(why)?
    };
    let ctype = resp.header("content-type").unwrap_or("").to_lowercase();
    if ctype.contains("pdf") || ctype.starts_with("image/") || ctype.starts_with("audio/") || ctype.starts_with("video/") {
        return Err(format!("not a web page ({})", ctype));
    }
    let final_url = resp.get_url().to_string();
    let mut bytes = Vec::new();
    // some servers (the EU API) end TLS without close_notify: keep what arrived
    if let Err(e) = resp.into_reader().take(8_000_000).read_to_end(&mut bytes) {
        if bytes.is_empty() {
            return Err(e.to_string());
        }
    }
    let latin1 = ctype.contains("iso-8859") || ctype.contains("windows-1252") || ctype.contains("latin");
    let body = match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) if latin1 => e.into_bytes().iter().map(|&b| b as char).collect(),
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    };
    Ok((final_url, body))
}

/// (parallel requests, ms between requests) for one host. Search engines block bursts.
fn pace(h: &str) -> (usize, u64) {
    match h {
        "search.brave.com" => (1, 1200),
        "html.duckduckgo.com" => (1, 1500),
        "startpage.com" => (2, 500),
        "search.seznam.cz" => (1, 1000),
        _ => (PER_HOST, 0),
    }
}

enum Tag {
    Source,
    /// listing version of the item, replace its name with the page title
    Detail(Opportunity, bool),
    /// a built-in or user entry: does its call page say "closed"?
    Check(Opportunity),
    /// search results about a site that blocked reading; the entry gets its topic from them
    SiteSearch(Opportunity),
}

type Fetched = (Job, Tag, Result<(String, String), String>);

#[derive(Default)]
struct HostState {
    busy: usize,
    next: Option<std::time::Instant>,
    fails: u32,
    /// after 2 failures in a row: one request at a time, 700 ms apart
    slow: bool,
    dead: bool,
}

struct PoolState {
    q: VecDeque<(Job, Tag, u8)>,
    hosts: HashMap<String, HostState>,
    closed: bool,
}

/// Worker pool that takes new jobs while it runs, so call pages load while sources still do.
/// Paced per host. HTTP 429/503 retries once after 8 s. A host failing twice in a row slows to
/// one request at a time; after 6 in a row (blocked, down) it is skipped for the rest of the run.
struct Pool {
    state: Arc<Mutex<PoolState>>,
}

impl Pool {
    fn start(agent: &ureq::Agent, tx: Sender<Fetched>) -> Pool {
        use std::time::Instant;
        let state = Arc::new(Mutex::new(PoolState { q: VecDeque::new(), hosts: HashMap::new(), closed: false }));
        for _ in 0..WORKERS {
            let (tx, state, agent) = (tx.clone(), state.clone(), agent.clone());
            std::thread::spawn(move || loop {
                // Ok(Some): fetch it; Ok(None): nothing ready; Err: job of a dead host
                let picked = {
                    let mut g = state.lock().unwrap();
                    if g.closed {
                        break;
                    }
                    let now = Instant::now();
                    let PoolState { q, hosts, .. } = &mut *g;
                    if let Some(p) = q.iter().position(|(j, _, _)| hosts.get(&host(&j.url)).map(|h| h.dead).unwrap_or(false)) {
                        let (job, tag, _) = q.remove(p).unwrap();
                        Err((job, tag))
                    } else {
                        let pos = q.iter().position(|(j, _, _)| {
                            let h = host(&j.url);
                            let cap = pace(&h).0;
                            hosts.get(&h).map(|s| s.busy < if s.slow { 1 } else { cap } && s.next.map(|n| n <= now).unwrap_or(true)).unwrap_or(true)
                        });
                        Ok(pos.map(|p| {
                            let item = q.remove(p).unwrap();
                            let h = host(&item.0.url);
                            let s = hosts.entry(h.clone()).or_default();
                            let gap = Duration::from_millis(if s.slow { pace(&h).1.max(700) } else { pace(&h).1 });
                            s.busy += 1;
                            s.next = Some(now + gap);
                            item
                        }))
                    }
                };
                let (job, tag, tries) = match picked {
                    Err((job, tag)) => {
                        if tx.send((job, tag, Err("site blocked or unreachable, skipped".into()))).is_err() {
                            break;
                        }
                        continue;
                    }
                    Ok(None) => {
                        std::thread::sleep(Duration::from_millis(25));
                        continue;
                    }
                    Ok(Some(x)) => x,
                };
                let res = fetch(&agent, &job);
                {
                    let mut g = state.lock().unwrap();
                    let s = g.hosts.entry(host(&job.url)).or_default();
                    s.busy = s.busy.saturating_sub(1);
                    match &res {
                        Ok(_) => s.fails = 0,
                        Err(r) if r == "HTTP 429" || r == "HTTP 503" => {
                            s.fails += 1;
                            s.slow |= s.fails >= 2;
                            s.dead = s.fails >= 6;
                            if tries < 1 && !s.dead {
                                s.next = Some(Instant::now() + Duration::from_secs(8));
                                g.q.push_back((job, tag, tries + 1));
                                continue;
                            }
                        }
                        // one missing page says nothing about the site
                        Err(r) if r == "HTTP 404" || r == "HTTP 410" || r.starts_with("not a web page") => {}
                        Err(_) => {
                            s.fails += 1;
                            s.slow |= s.fails >= 2;
                            s.dead = s.fails >= 6;
                        }
                    }
                }
                if tx.send((job, tag, res)).is_err() {
                    break;
                }
            });
        }
        Pool { state }
    }

    fn push(&self, job: Job, tag: Tag) {
        self.state.lock().unwrap().q.push_back((job, tag, 0));
    }

    fn close(&self) {
        let mut g = self.state.lock().unwrap();
        g.closed = true;
        g.q.clear();
    }
}

fn season_year(today: NaiveDate) -> i32 {
    if today.month() >= 9 { today.year() + 1 } else { today.year() }
}

fn read_user_list<T: serde::de::DeserializeOwned>(file: &str) -> Vec<T> {
    std::fs::read_to_string(data_dir().join(file)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn first_jobs(today: NaiveDate) -> Vec<Job> {
    let job = |source: &str, url: String, how: How| Job { source: source.to_string(), url, how, page: 1, term: String::new() };
    let mut v: Vec<Job> = FEEDS.iter().map(|(n, u)| job(n, u.to_string(), How::Page)).collect();
    // user feeds: data_dir/feeds.json = [["Name","https://..."], ...]
    for (n, u) in read_user_list::<(String, String)>("feeds.json") {
        v.push(job(&n, u, How::Page));
    }
    for t in CFP_TOPICS {
        v.push(job(&format!("WikiCFP {}", t), format!("http://www.wikicfp.com/cfp/rss?cat={}", urlencode(t).replace('+', "%20")), How::Cfp));
    }
    for t in DEVPOST_TOPICS {
        v.push(job(&format!("Devpost {}", t), format!("https://devpost.com/api/hackathons?status[]=open&status[]=upcoming&search={}", urlencode(t)), How::Devpost));
    }
    for t in EU_TOPICS {
        let mut j = job(&format!("EU calls {}", t), format!("https://api.tech.ec.europa.eu/search-api/prod/rest/search?apiKey=SEDIA&text={}&pageSize=100&pageNumber=1", urlencode(&format!("\"{}\"", t))), How::Eu);
        j.term = t.to_string();
        v.push(j);
    }
    let y = today.year();
    for year in [y, y + 1] {
        for t in CONFS_TOPICS {
            let mut j = job(
                &format!("confs.tech {} {}", t, year),
                format!("https://raw.githubusercontent.com/tech-conferences/conference-data/main/conferences/{}/{}.json", year, t),
                How::Confs,
            );
            j.term = t.to_string();
            v.push(j);
        }
    }
    // web search: engines block a computer that sends them many queries, so each run sends a
    // few. User queries (data_dir/queries.json) always go; built-in ones take turns by day.
    // Each query goes to one engine, in turn.
    let sy = season_year(today).to_string();
    let mut queries: Vec<String> = read_user_list::<String>("queries.json");
    let mut builtin: Vec<String> = QUERIES.iter().map(|q| q.replace("{Y}", &sy)).collect();
    if !builtin.is_empty() {
        let at = (today.ordinal() as usize * SEARCHES_PER_RUN) % builtin.len();
        builtin.rotate_left(at);
    }
    queries.extend(builtin);
    let mut seen_q = HashSet::new();
    queries.retain(|q| seen_q.insert(q.to_lowercase()));
    queries.truncate(SEARCHES_PER_RUN.max(read_user_list::<String>("queries.json").len()));
    for (i, q) in queries.iter().enumerate() {
        let (name, base) = ENGINES[i % ENGINES.len()];
        let mut j = job(&format!("{}: {}", name, q), format!("{}{}", base, urlencode(q)), How::Search);
        j.term = name.to_string();
        v.push(j);
    }
    v
}

/// Web searches per live run (more get the computer blocked by the engines).
const SEARCHES_PER_RUN: usize = 16;

/// Parse one fetched page: (candidates, follow-up jobs).
fn parse(job: &Job, final_url: &str, body: &str, today: NaiveDate) -> (Vec<Cand>, Vec<Job>) {
    let cand = |o: Opportunity, detail: bool, retitle: bool| Cand { o, detail, retitle };
    match job.how {
        How::Devpost => (harvest_devpost(body, today).into_iter().map(|o| cand(o, false, false)).collect(), vec![]),
        How::Confs => (harvest_confs(body, &job.term, today).into_iter().map(|o| cand(o, false, false)).collect(), vec![]),
        How::Eu => (harvest_eu(body, today).into_iter().map(|o| cand(o, false, false)).collect(), vec![]),
        How::Search => (harvest_search(&job.term, body, true, today).into_iter().map(|o| cand(o, true, true)).collect(), vec![]),
        How::Cfp => (harvest_feed(&job.source, final_url, body, true, today).into_iter().map(|o| cand(o, true, false)).collect(), vec![]),
        How::Page => {
            if is_feed(body) {
                return (harvest_feed(&job.source, final_url, body, false, today).into_iter().map(|o| cand(o, true, false)).collect(), vec![]);
            }
            let mut items: Vec<Cand> = harvest(&job.source, final_url, body, today).into_iter().map(|o| cand(o, true, false)).collect();
            // a conference, festival or community site with no call links is itself the entry
            if items.is_empty() && job.page == 1 {
                let mut c = cand(live_opp(&job.source, job.source.clone(), final_url, final_url, String::new(), today), false, true);
                enrich(&mut c, body, today);
                if c.o.deadline.is_none() && c.o.event.is_empty() {
                    warn(&mut c.o, "No call details found on this page: open it to check");
                }
                if !fold(&c.o.name).contains(&fold(&job.source)) {
                    c.o.name = format!("{}: {}", job.source, c.o.name);
                }
                items.push(c);
            }
            let mut more = Vec::new();
            if job.page < MAX_PAGES {
                if let Some(n) = next_page(final_url, body).filter(|n| url_key(n) != url_key(final_url)) {
                    more.push(Job { url: n, page: job.page + 1, ..job.clone() });
                }
            }
            if job.page == 1 {
                for f in feed_links(final_url, body) {
                    more.push(Job { url: f, page: MAX_PAGES, ..job.clone() });
                }
            }
            (items, more)
        }
    }
}

/// Passed deadline, an old round, or the page says it is closed.
fn is_passed(o: &Opportunity, today: NaiveDate) -> bool {
    crate::model::parse_date(&o.deadline).map(|d| d < today).unwrap_or(false) || stale(&o.name, today) || o.warnings.iter().any(|w| w == CLOSED)
}

/// One query typed by the user: every engine at once (Startpage serves Google's results; Google
/// itself answers bots with a CAPTCHA), merged in rank order. Drops duplicates and pages in
/// `known` (url keys of entries already in the list).
pub fn web_search(query: &str, known: &HashSet<String>, today: NaiveDate) -> Result<Vec<Opportunity>, String> {
    let asks: Vec<_> = ENGINES
        .iter()
        .rev()
        .map(|&(name, base)| {
            let job = Job { source: name.to_string(), url: format!("{}{}", base, urlencode(query)), how: How::Search, page: 1, term: String::new() };
            std::thread::spawn(move || match fetch(&agent(), &job) {
                Ok((_, body)) => {
                    let v = harvest_search(name, &body, false, today);
                    if v.is_empty() { Err(format!("{}: no results", name)) } else { Ok(v) }
                }
                Err(e) => Err(format!("{}: {}", name, e)),
            })
        })
        .collect();
    let (mut lists, mut why) = (vec![], vec![]);
    for a in asks {
        match a.join() {
            Ok(Ok(v)) => lists.push(v),
            Ok(Err(e)) => why.push(e),
            Err(_) => why.push("a search engine crashed".to_string()),
        }
    }
    // each engine's 1st result, then each one's 2nd...
    let mut seen = known.clone();
    let mut out = vec![];
    for i in 0..lists.iter().map(Vec::len).max().unwrap_or(0) {
        for o in lists.iter().filter_map(|l| l.get(i)) {
            if seen.insert(url_key(&o.apply_url)) {
                out.push(o.clone());
            }
        }
    }
    if out.is_empty() {
        return Err(if lists.is_empty() { why.join(", ") } else { "every result is already in your list".to_string() });
    }
    Ok(out)
}

/// Read result pages at once for deadline, dates and amount; drop closed and passed calls.
pub fn read_pages(v: Vec<Opportunity>, today: NaiveDate) -> Vec<Opportunity> {
    let reads: Vec<_> = v.into_iter().map(|o| std::thread::spawn(move || read_page(o, false, today))).collect();
    reads.into_iter().filter_map(|r| r.join().ok()).filter(|o| !is_passed(o, today)).collect()
}

/// Read one search result's own page for deadline, dates and amount, like the crawl does.
/// `retitle`: take the page's own title (a pasted link has none yet).
pub fn read_page(o: Opportunity, retitle: bool, today: NaiveDate) -> Opportunity {
    let job = Job { source: o.org.clone(), url: o.apply_url.clone(), how: How::Page, page: 0, term: String::new() };
    let mut c = Cand { o, detail: true, retitle };
    match fetch(&agent(), &job) {
        Ok((_, body)) => enrich(&mut c, &body, today),
        Err(why) => warn(&mut c.o, &format!("Page could not be read ({}): details are from the search result", why)),
    }
    c.o
}

/// An entry for a link the user pasted.
pub fn link_entry(url: &str, today: NaiveDate) -> Opportunity {
    let h = host(url);
    read_page(live_item(&h, h.clone(), url, url, String::new(), today), true, today)
}

/// Engine for searches about one site.
const SITE_ENGINE: &str = "https://www.startpage.com/sp/search?query=";

/// A site that blocks reading (login, CAPTCHA, JavaScript): search the web for it instead.
fn site_search(o: &Opportunity) -> Job {
    let q = format!("{} {}", o.org, host(&o.apply_url));
    Job { source: format!("Search about {}", o.org), url: format!("{}{}", SITE_ENGINE, urlencode(&q)), how: How::Search, page: 1, term: "Startpage".into() }
}

/// Run the whole crawl in a background thread; progress and items go to `tx`.
pub fn spawn(tx: Sender<Msg>, today: NaiveDate) {
    std::thread::spawn(move || run(tx, today));
}

fn run(tx: Sender<Msg>, today: NaiveDate) {
    let t0 = std::time::Instant::now();
    let budget = Duration::from_secs(BUDGET_SECS);
    let agent = agent();
    let (rtx, rrx) = channel();
    let pool = Pool::start(&agent, rtx);
    let passed = |o: &Opportunity| is_passed(o, today);

    let jobs = first_jobs(today);
    let mut visited: HashSet<String> = jobs.iter().map(|j| url_key(&j.url)).collect();
    let (mut src_total, mut src_done, mut det_total, mut det_done) = (jobs.len(), 0, 0, 0);
    let mut outstanding = jobs.len();
    // shortcut: the "closed" warning on a built-in entry is not saved; each live run sets it again
    for o in crate::model::load_opportunities().0 {
        if o.apply_url.starts_with("http") && !o.apply_url.to_lowercase().ends_with(".pdf") {
            let j = Job { source: o.org.clone(), url: o.apply_url.clone(), how: How::Page, page: 0, term: String::new() };
            pool.push(j, Tag::Check(o));
            det_total += 1;
            outstanding += 1;
        }
    }
    for j in jobs {
        pool.push(j, Tag::Source);
    }
    let mut failed = Vec::new();
    let mut seen_url: HashSet<String> = HashSet::new();
    // name key -> id: one entry per title across all sites
    let mut seen_name: HashMap<String, String> = HashMap::new();
    let (mut batch, mut gone): (Vec<Opportunity>, Vec<String>) = (Vec::new(), Vec::new());

    while outstanding > 0 {
        let left = budget.saturating_sub(t0.elapsed());
        let Ok((job, tag, res)) = rrx.recv_timeout(left) else {
            let _ = tx.send(Msg::Progress {
                stage: "time limit",
                done: src_done + det_done,
                total: src_total + det_total,
                source: format!("stopped after {} s, {} requests skipped", BUDGET_SECS, outstanding),
                ok: false,
                found: 0,
                note: String::new(),
            });
            break;
        };
        outstanding -= 1;
        match tag {
            Tag::Source => {
                src_done += 1;
                let (ok, found, note) = match res {
                    Ok((final_url, body)) => {
                        let (cands, more) = parse(&job, &final_url, &body, today);
                        let mut fresh = Vec::new();
                        for c in cands {
                            let nk = name_key(&c.o.name);
                            if !seen_url.insert(url_key(&c.o.apply_url)) || (nk.len() >= 20 && !c.retitle && seen_name.contains_key(&nk)) {
                                continue;
                            }
                            if nk.len() >= 20 && !c.retitle {
                                seen_name.insert(nk, c.o.id.clone());
                            }
                            if passed(&c.o) {
                                continue;
                            }
                            if c.o.warnings.iter().any(|w| w.starts_with(UNREADABLE)) {
                                pool.push(site_search(&c.o), Tag::SiteSearch(c.o.clone()));
                                src_total += 1;
                                outstanding += 1;
                            }
                            if c.detail && det_total < MAX_DETAIL && !c.o.apply_url.to_lowercase().ends_with(".pdf") {
                                let dj = Job { source: job.source.clone(), url: c.o.apply_url.clone(), how: How::Page, page: 0, term: String::new() };
                                pool.push(dj, Tag::Detail(c.o.clone(), c.retitle));
                                det_total += 1;
                                outstanding += 1;
                            }
                            fresh.push(c.o);
                        }
                        for m in more {
                            if visited.insert(url_key(&m.url)) {
                                pool.push(m, Tag::Source);
                                src_total += 1;
                                outstanding += 1;
                            }
                        }
                        let n = fresh.len();
                        if n > 0 {
                            let _ = tx.send(Msg::Items(fresh));
                        }
                        (true, n, String::new())
                    }
                    Err(why) => {
                        if job.page <= 1 {
                            failed.push(job.source.clone());
                        }
                        // a listed site that blocks reading stays in the list, marked
                        if job.how == How::Page && job.page == 1 && seen_url.insert(url_key(&job.url)) {
                            let mut o = live_opp(&job.source, job.source.clone(), &job.url, &job.url, String::new(), today);
                            warn(&mut o, &format!("Site could not be read ({}): open it to check for calls", why));
                            pool.push(site_search(&o), Tag::SiteSearch(o.clone()));
                            src_total += 1;
                            outstanding += 1;
                            let _ = tx.send(Msg::Items(vec![o]));
                        }
                        (false, 0, why)
                    }
                };
                let _ = tx.send(Msg::Progress { stage: "sources", done: src_done, total: src_total, source: job.source, ok, found, note });
            }
            Tag::Detail(o, retitle) => {
                det_done += 1;
                let ok = res.is_ok();
                let mut c = Cand { o, detail: true, retitle };
                match res {
                    Ok((_, body)) => enrich(&mut c, &body, today),
                    Err(why) => warn(&mut c.o, &format!("Call page could not be read ({}): details are from the listing", why)),
                }
                let nk = name_key(&c.o.name);
                let dup = nk.len() >= 20 && seen_name.get(&nk).map(|id| *id != c.o.id).unwrap_or(false);
                if nk.len() >= 20 && !dup {
                    seen_name.insert(nk, c.o.id.clone());
                }
                if passed(&c.o) || junk_title(&c.o.name) || dup {
                    gone.push(c.o.id);
                } else {
                    batch.push(c.o);
                }
                if batch.len() + gone.len() >= 25 {
                    let _ = tx.send(Msg::Items(std::mem::take(&mut batch)));
                    let _ = tx.send(Msg::Gone(std::mem::take(&mut gone)));
                }
                let _ = tx.send(Msg::Progress { stage: "call pages", done: det_done, total: det_total, source: job.source, ok, found: 0, note: String::new() });
            }
            Tag::SiteSearch(mut o) => {
                src_done += 1;
                let ok = res.is_ok();
                let mut fresh = Vec::new();
                if let Ok((_, body)) = res {
                    let found = harvest_search(&job.term, &body, false, today);
                    // what the site is about, from the first snippets
                    let topic: Vec<&str> = found.iter().take(3).map(|r| r.summary.as_str()).filter(|s| !s.is_empty()).collect();
                    if !topic.is_empty() {
                        o.summary = topic.join(" … ");
                        truncate_chars(&mut o.summary, 500);
                        for t in guess_tags(&format!("{} {}", o.name, o.summary)) {
                            if !o.tags.contains(&t) {
                                o.tags.push(t);
                            }
                        }
                        warn(&mut o, "Topic read from search results: the site blocks automated reading");
                        fresh.push(o);
                    }
                    // results that look like a call become entries of their own
                    for r in found {
                        let call = re_kw().is_match(&format!("{} {}", r.name, r.summary)) && !old_post(&r.summary, today);
                        if call && !passed(&r) && seen_url.insert(url_key(&r.apply_url)) {
                            fresh.push(r);
                        }
                    }
                }
                let found = fresh.len();
                if found > 0 {
                    let _ = tx.send(Msg::Items(fresh));
                }
                let _ = tx.send(Msg::Progress { stage: "sources", done: src_done, total: src_total, source: job.source, ok, found, note: String::new() });
            }
            Tag::Check(mut o) => {
                det_done += 1;
                let ok = res.is_ok();
                if let Ok((_, body)) = res {
                    if says_closed(&page_text(&body), today) {
                        warn(&mut o, CLOSED);
                        let _ = tx.send(Msg::Items(vec![o]));
                    }
                }
                let _ = tx.send(Msg::Progress { stage: "call pages", done: det_done, total: det_total, source: job.source, ok, found: 0, note: String::new() });
            }
        }
    }
    pool.close();
    let _ = tx.send(Msg::Items(batch));
    let _ = tx.send(Msg::Gone(gone));
    let _ = tx.send(Msg::Done { failed });
}

// ---------------------------------------------------------------- cache

/// Live results from earlier runs (data_dir/live.json), without passed deadlines.
pub fn load_cache(today: NaiveDate) -> Vec<Opportunity> {
    let mut v: Vec<Opportunity> = read_user_list("live.json");
    v.retain(|o| crate::model::parse_date(&o.deadline).map(|d| d >= today).unwrap_or(true));
    v
}

/// True when the saved results are missing or older than 24 hours.
pub fn cache_stale() -> bool {
    std::fs::metadata(data_dir().join("live.json"))
        .and_then(|m| m.modified())
        .map(|t| t.elapsed().map(|e| e.as_secs() > 24 * 3600).unwrap_or(true))
        .unwrap_or(true)
}

/// Keep items seen in the last 21 days whose deadline has not passed.
pub fn save_cache(items: &[Opportunity], today: NaiveDate) {
    let cutoff = today - chrono::Duration::days(21);
    let keep: Vec<&Opportunity> = items
        .iter()
        .filter(|o| crate::model::parse_date(&Some(o.checked.clone())).map(|d| d >= cutoff).unwrap_or(false))
        .filter(|o| crate::model::parse_date(&o.deadline).map(|d| d >= today).unwrap_or(true))
        .collect();
    if let Ok(s) = serde_json::to_string(&keep) {
        let _ = std::fs::write(data_dir().join("live.json"), s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 22).unwrap()
    }

    #[test]
    fn harvests_calls() {
        let page = r#"<nav><a href="/login">Login to your account now</a></nav>
<div><a href="/opencalls/sound-art-residency-2027">Sound Art Residency 2027 Berlin</a><p>Deadline: 15.01.2027. Fee EUR 3,000 for sound artists.</p></div>
<div><a href="https://x.org/calls/media">Media award open call</a><p>Apply until 31 December 2026, prize €10k</p></div>
<a href="/about">About us and our team here</a>"#;
        let v = harvest("Test", "https://x.org/", page, today());
        assert_eq!(v.len(), 2, "{:?}", v.iter().map(|o| &o.name).collect::<Vec<_>>());
        assert_eq!(v[0].deadline.as_deref(), Some("2027-01-15"));
        assert_eq!(v[0].amount_max, 3000);
        assert_eq!(v[0].apply_url, "https://x.org/opencalls/sound-art-residency-2027");
        assert_eq!(v[0].kind, "Residency");
        assert_eq!(v[1].deadline.as_deref(), Some("2026-12-31"));
        assert_eq!(v[1].amount_max, 10000);
    }

    #[test]
    fn external_links_need_keyword_in_title_or_path() {
        let page = r#"<a href="https://other.org/news/xyz">Some interesting article here</a><p>grant</p>
<a href="https://other.org/open-call-2027">Spring Programme for Makers</a>"#;
        let v = harvest("Agg", "https://agg.org/list", page, today());
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].org, "other.org");
    }

    #[test]
    fn deadlines_in_many_languages() {
        let t = today();
        let d = |s: &str| find_deadline(s, t).map(|d| d.format("%Y-%m-%d").to_string());
        assert_eq!(d("Bewerbungsschluss: 15. Januar 2027. Ausstellung bis 30.06.2027").as_deref(), Some("2027-01-15"));
        assert_eq!(d("Date limite de candidature : 1er mars 2027").as_deref(), Some("2027-03-01"));
        assert_eq!(d("Fecha límite: 15 de enero de 2027").as_deref(), Some("2027-01-15"));
        assert_eq!(d("Scadenza: 10 febbraio 2027").as_deref(), Some("2027-02-10"));
        assert_eq!(d("Termin nadsyłania zgłoszeń do 20 stycznia 2027 r.").as_deref(), Some("2027-01-20"));
        assert_eq!(d("Submission Deadline Jan 15, 2027 Notification Due Mar 1, 2027").as_deref(), Some("2027-01-15"));
        assert_eq!(d("Sluitingsdatum: 3 maart 2027").as_deref(), Some("2027-03-03"));
        assert_eq!(d("Deadline: 12/31/2026").as_deref(), Some("2026-12-31"));
        // year-less right after a deadline word: next occurrence
        assert_eq!(d("Deadline: 10 January").as_deref(), Some("2027-01-10"));
        // past deadline kept when nothing future
        assert_eq!(d("Deadline was 1 March 2026").as_deref(), Some("2026-03-01"));
        // several unrelated dates and no deadline word: unknown
        assert_eq!(d("Festival 5 June 2027 and 9 July 2027"), None);
    }

    #[test]
    fn event_dates() {
        let t = today();
        let dl = NaiveDate::from_ymd_opt(2027, 1, 15);
        let e = |s: &str| find_event(s, dl, t);
        assert_eq!(e("The festival takes place 2–4 July 2027 in Berlin.").as_deref(), Some("2027-07-02 – 2027-07-04"));
        assert_eq!(e("When Jul 2, 2027 - Jul 4, 2027 Where Venice").as_deref(), Some("2027-07-02 – 2027-07-04"));
        assert_eq!(e("Das Festival findet vom 5. Juni 2027 bis 9. Juni 2027 statt.").as_deref(), Some("2027-06-05 – 2027-06-09"));
        assert_eq!(e("Le festival aura lieu le 12 mai 2027.").as_deref(), Some("2027-05-12"));
        // the deadline is not the event
        assert_eq!(e("When: 15 January 2027 (deadline)"), None);
        assert_eq!(e("Apply by 15 January 2027"), None);
    }

    #[test]
    fn warnings() {
        let mut o = live_opp("x", "Residency for artists".into(), "https://a.org/r", "https://a.org", String::new(), NaiveDate::from_ymd_opt(2026, 11, 22).unwrap());
        flag(&mut o, "This residency is only open to artists based in Canada. Application fee: $35.");
        assert_eq!(o.scope, "Restricted");
        assert!(o.warnings.iter().any(|w| w == "Entry fee: $35"), "{:?}", o.warnings);
        let mut ok = o.clone();
        ok.scope = "World".into();
        ok.warnings.clear();
        flag(&mut ok, "Open to artists based in Germany or Austria. No application fee.");
        assert_eq!(ok.scope, "World");
        assert!(ok.warnings.is_empty(), "{:?}", ok.warnings);
        assert!(region_limit("nur für Künstler mit Wohnsitz in Bayern, Deutschland").is_some());
        assert!(region_limit("open to applicants worldwide, including Canada").is_none());
        assert_eq!(broad_fields("International Conference on Business, Economics, Management and Social Sciences"), 4);
        assert_eq!(amount_near("Application fee € 20. The prize is worth € 5.000.").1, 5000);
    }

    #[test]
    fn junk_titles() {
        assert!(junk_title("Explore all competitions"));
        assert!(junk_title("Dnsmasq wins the first Blue Prize"));
        assert!(junk_title("58.2k 177 29 10 years of it"));
        assert!(!junk_title("Win €10k: Sound Art Prize 2027"));
        assert!(!junk_title("Open Call AI and XR 2027"));
        assert!(junk_title("Member of Technical Staff (Open Role)"));
        assert!(stale("unitaryHACK 2022: quantum open source", today()));
        assert!(!stale("Hackathon 2026/2027", today()));
        assert!(!stale("Open call for sound art", today()));
    }

    #[test]
    fn amounts() {
        assert_eq!(find_amount("Preisgeld 2.500 €").1, 2500);
        assert_eq!(find_amount("dotiert mit 10.000,00 Euro").1, 10000);
        assert_eq!(find_amount("prize of £5,000").2, "GBP");
        assert_eq!(find_amount("up to 2,5 Mio €").1, 2_500_000);
        assert_eq!(find_amount("$2.50 fee").1, 2);
        assert_eq!(find_amount("a $5M global competition").1, 5_000_000);
        assert_eq!(find_amount("€1.5 million").1, 1_500_000);
        assert_eq!(amount_near("Programme budget € 4.000.000. The prize is worth € 8.000.").1, 8000);
    }

    #[test]
    fn feeds_and_search() {
        let rss = r#"<?xml version="1.0"?><rss><channel><item><title>SPML 2027 : Signal Processing Conf</title>
<link>http://www.wikicfp.com/cfp/servlet/event.showcfp?eventid=1&amp;copyownerid=2</link><description>[Xi'an] [Jul 2, 2027]</description></item></channel></rss>"#;
        assert!(is_feed(rss));
        let v = harvest_feed("WikiCFP", "http://www.wikicfp.com/", rss, true, today());
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].apply_url, "http://www.wikicfp.com/cfp/servlet/event.showcfp?eventid=1&copyownerid=2");
        let ddg = r#"<a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fbears.org%2Fopen%2Dcall%2D2027%2F&rut=x">Open Call Sound 2027</a><a href="https://duckduckgo.com/settings">Settings and more</a>"#;
        let v = harvest_search("DuckDuckGo", ddg, true, today());
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].apply_url, "https://bears.org/open-call-2027/");
        // a user's own search keeps results that don't look like a call
        let plain = r#"<a class="result__a" href="https://www.airwiggles.com/">Airwiggles community</a>"#;
        assert!(harvest_search("DuckDuckGo", plain, true, today()).is_empty());
        assert_eq!(harvest_search("DuckDuckGo", plain, false, today()).len(), 1);
        // Brave: breadcrumb before the title
        let brave = r#"<a href="https://www.airwiggles.com/c/announcements/aircon26" class="l1"><div>Airwiggles airwiggles.com › c › announcements › aircon26 AirCon26: the free online audio conference | Airwiggles</div></a>"#;
        assert_eq!(harvest_search("Brave", brave, false, today())[0].name, "AirCon26: the free online audio conference");
        let seznam = r#"<h3><a class="ef854e36a42" data-e-a="heading" href="https://soundartlab.org/residency/"><span><b class=sec>Residency</b> – <b class=sec>Sound Art</b> Lab</span></a></h3><a data-e-a="reference" href="https://soundartlab.org/residency/"><span>soundartlab.org</span><span>/<b class=sec>residency</b></span></a><a href="https://search.seznam.cz/obrazky/">Obrázky</a>"#;
        let v = harvest_search("Seznam", seznam, false, today());
        assert_eq!((v.len(), v[0].name.as_str()), (1, "Residency – Sound Art Lab"));
        let sp = r#"<a href="https://resartis.org/open-calls/" class="favicon-link css-1"><span>x</span></a><a href="https://resartis.org/open-calls/" class="wgl-site-title css-3"><span class="link-text">Res Artis</span></a><a class="result-title result-link css-11" href="https://resartis.org/open-calls/"><h2 class="wgl-title">Open Calls - Res Artis</h2></a><a href="https://o-seznam.cz/napoveda/prohlizece/">Najít jiný prohlížeč</a>"#;
        let v = harvest_search("Startpage", sp, false, today());
        assert_eq!((v.len(), v[0].name.as_str()), (1, "Open Calls - Res Artis"));
    }

    #[test]
    fn devpost_and_confs() {
        let dp = r#"{"hackathons":[{"title":"Art Hack","url":"https://a.devpost.com/","open_state":"open","submission_period_dates":"Aug 23, 2026 - Jan 02, 2027","prize_amount":"$<span data-currency-value>10,000</span>","themes":[{"name":"Music/Art"}],"displayed_location":{"location":"Online"},"organization_name":"X"}]}"#;
        let v = harvest_devpost(dp, today());
        assert_eq!(v[0].deadline.as_deref(), Some("2027-01-02"));
        assert_eq!((v[0].amount_max, v[0].currency.as_str(), v[0].kind.as_str()), (10000, "USD", "Hackathon"));
        let cf = r#"[{"name":"A","url":"https://a.io","startDate":"2027-03-01","cfpUrl":"https://a.io/cfp","cfpEndDate":"2027-01-10"},{"name":"B","url":"https://b.io","cfpUrl":"https://b.io/cfp","cfpEndDate":"2026-01-10"}]"#;
        let v = harvest_confs(cf, "general", today());
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].kind, "Conference");
    }

    #[test]
    fn enrich_reads_call_page() {
        let page = r#"<html lang="de"><head><meta property="og:title" content="Klangkunst-Stipendium 2027 | Stiftung"></head>
<body><nav>Menu 1.1.2026</nav><h1>Stipendium</h1><p>Die Stiftung vergibt ein Stipendium für Klangkunst, dotiert mit 12.000 Euro.
Bewerbungsschluss ist der 28. Februar 2027.</p><footer>© 2026</footer></body></html>"#;
        let mut c = Cand { o: live_opp("x", "Read more".into(), "https://s.de/k", "https://s.de", String::new(), today()), detail: true, retitle: false };
        enrich(&mut c, page, today());
        assert_eq!(c.o.name, "Klangkunst-Stipendium 2027");
        assert_eq!(c.o.deadline.as_deref(), Some("2027-02-28"));
        assert_eq!(c.o.amount_max, 12000);
        assert_eq!(c.o.kind, "Stipend");
        assert!(c.o.tags.contains(&"sound art".to_string()));
    }

    #[test]
    fn closed_calls() {
        let t = today();
        assert!(says_closed("Unfortunately, submissions for the S+T+ARTS Prize 2026 are no longer being accepted. The deadline for submissions was March 9, 2026.", t));
        assert!(says_closed("Die Bewerbungsfrist ist abgelaufen.", t));
        assert!(says_closed("Submissions closed", t));
        // a closed old round next to an open new one is not closed
        assert!(!says_closed("Applications are closed for 2026. The 2027 call: deadline 4 March 2027.", t));
        assert!(!says_closed("Submit your work by the deadline: 4 March 2027.", t));
        assert!(old_post("December 2, 2025 - Apply to speak at AirCon26!", t));
        assert!(!old_post("November 10, 2026 - Apply to speak at AirCon27!", t));
        assert!(!old_post("Apply to speak at AirCon27 by 2 December 2027", t));
    }

    /// Real search engines: `cargo test --release -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn web_search_live() {
        let v = web_search("sound art residency open call", &HashSet::new(), today()).unwrap();
        for o in &v {
            println!("{} | {} | {}", o.cadence, o.name, o.apply_url);
        }
        let read = read_pages(v.into_iter().take(10).collect(), today());
        for o in &read {
            println!("{:?} | {} | {:?}", o.deadline, o.name, o.warnings);
        }
        assert!(!read.is_empty());
    }

    #[test]
    fn folding_and_keys() {
        assert_eq!(fold("Förderung"), fold("Foerderung"));
        assert_eq!(url_key("https://www.a.org/x/?utm_source=y#top"), url_key("http://a.org/x"));
        assert_eq!(join_url("https://a.org/list/page", "?page=2"), "https://a.org/list/page?page=2");
    }
}
