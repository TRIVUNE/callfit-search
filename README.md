# callfit

Find funding, open calls, residencies and paper calls that fit **Trivune**. A terminal app in the style of [llmfit](https://www.llmfit.org/), written in Rust.

callfit scores each opportunity against your profile and shows a fit level, a score breakdown, the money, the deadline and the apply link.

## Install

| Method | Command |
|---|---|
| Scoop (Windows) | `scoop bucket add trivune https://github.com/TRIVUNE/scoop-bucket` then `scoop install callfit` |
| Cargo (any OS) | `cargo install --git https://github.com/TRIVUNE/callfit-search` |
| Download | Get the zip from the GitHub Releases page and run `callfit.exe` |

Use Windows Terminal for the best look.

## Keys

| Key | Action |
|---|---|
| `↑` `↓` / `j` `k` | Move |
| `Enter` | Open the apply link |
| `i` | Open the info page |
| `p` | Track: Watching → Drafting → Submitted → Won → Rejected |
| `/` | Search (name, org, tags, notes) |
| `t` | Filter by type |
| `r` | Filter by region (Berlin, DE, EU, World) |
| `f` | Filter by fit (Eligible, Perfect+Good, All) |
| `o` | Filter by status (Open now, Upcoming, Closed) |
| `s` | Sort (Score, Deadline, Amount, Name) |
| `v` | Show tracked only |
| `u` | Load live listings from 16 call pages |
| `w` | Web search for the current query |
| `x` | Export the current list to CSV |
| `e` | Open the data folder |
| `J` `K` | Scroll the detail panel |
| `q` | Quit |

Plain list for scripts: `callfit --list`

## How the score works

| Part | Weight | What it measures |
|---|---|---|
| Eligibility | 40% | Region, applicant type, and requirements you meet |
| Relevance | 30% | Overlap between the call's tags and your interests |
| Money | 15% | Amount, on a log scale |
| Timing | 15% | Open with time left scores high; closed scores low |

Fit levels: **Perfect** (80+, no missing requirement, not closed), **Good** (62+), **Marginal** (45+), **Low**, **Not eligible** (wrong region or applicant type).

## Your data

The data folder (`e` opens it) is `%APPDATA%\callfit` on Windows.

- `profile.json` — who you are. Edit `has` to list requirements you meet, and `interests` for relevance.
- `opportunities.json` — add or override entries (same format as `data/opportunities.json`, matched by `id`).
- `feeds.json` — extra listing pages for live mode: `[["Name", "https://..."]]`.
- `tracker.json` — your tracking status per call.

Requirement keys for `has`: `open_source`, `germany_based`, `berlin_residence`, `berlin_brandenburg`, `artist_track_record`, `cofinance_10`, `finished_work`, `venue_partner`, `education_partner`, `international_partner`, `consortium`, `legal_entity`, `institution`, `startup`, `incubator`, `product_idea`, `critical_infrastructure`.

## Data quality

Every entry shows how it was checked:

- **verified** — dates and amounts taken from the official page or press release.
- **dates from yearly pattern** — next dates guessed from last year's dates. Confirm before you plan.
- **check on site** — no reliable date or amount found.
- **live listing** — pulled from a listing page; eligibility not checked.

Always confirm the deadline on the official page before you apply.

## Release (maintainers)

1. Push a tag: `git tag v0.1.0 && git push --tags`. GitHub Actions builds Windows, macOS and Linux binaries and attaches them to the release.
2. Download `callfit.json` from the release. It already has the version, URL and hash. Put it in the root of the `TRIVUNE/scoop-bucket` repo.
3. Later releases update through Scoop's `autoupdate`.
