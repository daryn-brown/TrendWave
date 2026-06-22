//! Phase 2 — candidate **discovery**, independent of the local LLM.
//!
//! The bottleneck model can only name tickers it was trained on, so post-cutoff
//! names (e.g. SanDisk / `SNDK`, relisted Feb 2025) are invisible to it. This
//! module adds two **free** discovery sources that surface live candidates the
//! prompt never mentioned:
//!
//!   * **SEC EDGAR full-text search** on the bottleneck's own vocabulary (e.g.
//!     `"NAND"`, `"HBM"`) → operating companies whose filings discuss the
//!     chokepoint. An EDGAR FTS for `"NAND"` returns Micron, Western Digital,
//!     Intel, ASML… the dominant beneficiaries, on-thesis.
//!   * **Yahoo predefined screeners** (e.g. `growth_technology_stocks`,
//!     `day_gainers`) → a live momentum/growth universe that includes names the
//!     model has never heard of (SanDisk shows up here once it is trading).
//!
//! Everything is best-effort: any network or parse failure yields an empty list,
//! never an error, so discovery can only *add* breadth — it never breaks a run.
//! The pipeline scores discovered names with the same signals as model-named
//! ones and trims to `max_results`, so low-quality finds simply fall away.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::Semaphore;

use crate::fundamentals::EDGAR_UA;

/// A candidate surfaced by a discovery source rather than the LLM plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    /// Normalized upper-case ticker.
    pub ticker: String,
    /// Best-known company name (from the source).
    pub company: String,
    /// Bottleneck bucket this attaches to (a real bottleneck title for EDGAR
    /// hits, or a synthetic screener bucket for Yahoo hits).
    pub bottleneck: String,
    /// Provenance tag mirrored into `Candidate.discovery`
    /// (`"screener:edgar-fts"` | `"screener:yahoo"`).
    pub source: &'static str,
    /// Short human context (matched term or screener theme) for the thesis line.
    pub context: String,
}

pub const SRC_EDGAR: &str = "screener:edgar-fts";
pub const SRC_YAHOO: &str = "screener:yahoo";

/// Yahoo predefined screeners we pull, as `(scrId, human title)`. Chosen to
/// favor growth / cyclical-recovery / momentum names without being so broad they
/// drown the thesis: tech growth, under-valued growth, and the day's movers
/// (which is how brand-new relistings first become visible).
const YAHOO_THEMES: &[(&str, &str)] = &[
    ("growth_technology_stocks", "Growth technology stocks"),
    ("undervalued_growth_stocks", "Undervalued growth stocks"),
    ("day_gainers", "Day's gainers"),
];

/// Synthetic bottleneck bucket for thesis-agnostic Yahoo finds.
const YAHOO_BUCKET: &str = "Market momentum screener";

const EDGAR_PER_TERM: usize = 6;
const YAHOO_PER_THEME: usize = 6;
const TERMS_PER_BOTTLENECK: usize = 2;
const HTTP_CONCURRENCY: usize = 4;

/// All-caps tokens that are ordinary English / boilerplate, not bottleneck
/// vocabulary — excluded so we don't run a full-text search for `"THE"`.
const ACRONYM_STOP: &[&str] = &[
    "AND", "THE", "FOR", "WITH", "FROM", "THIS", "THAT", "ARE", "USA", "INC",
    "LLC", "LTD", "CEO", "CFO", "CTO", "USD", "GAAP", "SEC", "ESG", "IPO", "ETF",
    "API", "FAQ", "NYSE", "OTC", "ALL", "NEW", "KEY", "PLC", "LP",
];

// ---------------------------------------------------------------------------
// Pure parsing / term extraction (unit-tested)
// ---------------------------------------------------------------------------

/// Plausible US-listed ticker: 1–6 chars, alphanumerics plus `.`/`-`, leading
/// letter. Deliberately strict so CIK numbers / junk never masquerade as a
/// ticker.
fn is_plausible_ticker(t: &str) -> bool {
    let len = t.chars().count();
    (1..=6).contains(&len)
        && t.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// Parse an EDGAR `display_names` entry into `(company, ticker)`.
///
/// Entries look like `"WESTERN DIGITAL CORP  (WDC)  (CIK 0000106040)"` or
/// `"Sandisk Corp  (SNDK, SNDKV)  (CIK 0002023554)"`. Filers without a ticker —
/// `"SANDISK CORP  (CIK 0001000180)"` (private/delisted) — return `None` so we
/// only ever discover listed names. When several tickers are listed we take the
/// first (primary) one.
fn parse_display_name(s: &str) -> Option<(String, String)> {
    let cik_pos = s.find("(CIK")?;
    let head = s[..cik_pos].trim_end();
    // The ticker, if any, is the last parenthetical before the CIK group.
    let open = head.rfind('(')?;
    let close_rel = head[open..].find(')')?;
    let inside = &head[open + 1..open + close_rel];
    let company = head[..open].trim();
    if company.is_empty() {
        return None;
    }
    let first = inside.split(',').next()?.trim().to_ascii_uppercase();
    if is_plausible_ticker(&first) {
        Some((company.to_string(), first))
    } else {
        None
    }
}

/// Reduce an EDGAR full-text-search payload to discovered candidates, in
/// relevance order, keeping at most [`EDGAR_PER_TERM`] unique listed tickers.
fn parse_edgar_fts(json: &Value, bottleneck: &str, term: &str) -> Vec<Discovered> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let hits = json["hits"]["hits"].as_array();
    for hit in hits.into_iter().flatten() {
        let names = hit["_source"]["display_names"].as_array();
        for name in names.into_iter().flatten() {
            let Some(name) = name.as_str() else { continue };
            let Some((company, ticker)) = parse_display_name(name) else {
                continue;
            };
            if seen.insert(ticker.clone()) {
                out.push(Discovered {
                    ticker,
                    company,
                    bottleneck: bottleneck.to_string(),
                    source: SRC_EDGAR,
                    context: term.to_string(),
                });
                if out.len() >= EDGAR_PER_TERM {
                    return out;
                }
            }
        }
    }
    out
}

/// Reduce a Yahoo predefined-screener payload to discovered candidates, keeping
/// at most [`YAHOO_PER_THEME`] names.
fn parse_yahoo_screener(json: &Value, theme_title: &str) -> Vec<Discovered> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let quotes = json["finance"]["result"][0]["quotes"].as_array();
    for q in quotes.into_iter().flatten() {
        let Some(symbol) = q["symbol"].as_str() else {
            continue;
        };
        let ticker = symbol.trim().to_ascii_uppercase();
        if !is_plausible_ticker(&ticker) || !seen.insert(ticker.clone()) {
            continue;
        }
        let company = q["shortName"]
            .as_str()
            .or_else(|| q["longName"].as_str())
            .unwrap_or(symbol)
            .trim()
            .to_string();
        out.push(Discovered {
            ticker,
            company,
            bottleneck: YAHOO_BUCKET.to_string(),
            source: SRC_YAHOO,
            context: theme_title.to_string(),
        });
        if out.len() >= YAHOO_PER_THEME {
            break;
        }
    }
    out
}

/// All-caps acronym tokens (length 2–6) that look like bottleneck vocabulary,
/// in first-seen order with stopwords removed. e.g. "High-bandwidth memory
/// (HBM) and NAND" → `["HBM", "NAND"]`.
fn acronyms(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split(|c: char| !c.is_ascii_alphanumeric()) {
        let len = raw.chars().count();
        if !(2..=6).contains(&len) {
            continue;
        }
        let all_caps_or_digit = raw
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
        let has_letter = raw.chars().any(|c| c.is_ascii_uppercase());
        if all_caps_or_digit && has_letter && !ACRONYM_STOP.contains(&raw) {
            let token = raw.to_string();
            if !out.contains(&token) {
                out.push(token);
            }
        }
    }
    out
}

/// A short fallback phrase (first few distinctive words) from a bottleneck title,
/// used when it contains no acronyms — e.g. "Constrained packaging capacity" →
/// `"constrained packaging capacity"`.
fn short_phrase(title: &str) -> Option<String> {
    let words: Vec<&str> = title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.chars().count() > 3)
        .take(4)
        .collect();
    if words.len() < 2 {
        None
    } else {
        Some(words.join(" ").to_ascii_lowercase())
    }
}

/// Derive up to [`TERMS_PER_BOTTLENECK`] EDGAR search terms for one bottleneck:
/// prefer acronyms (highest signal), else fall back to a short title phrase.
pub fn bottleneck_terms(title: &str, description: &str) -> Vec<String> {
    let mut terms = acronyms(title);
    for a in acronyms(description) {
        if !terms.contains(&a) {
            terms.push(a);
        }
    }
    terms.truncate(TERMS_PER_BOTTLENECK);
    if terms.is_empty() {
        if let Some(phrase) = short_phrase(title) {
            terms.push(phrase);
        }
    }
    terms
}

// ---------------------------------------------------------------------------
// Best-effort network fetches
// ---------------------------------------------------------------------------

/// One EDGAR full-text search for an exact `term`, returning discovered listed
/// companies. `None`/empty on any failure.
async fn fetch_edgar_fts(
    http: &reqwest::Client,
    bottleneck: &str,
    term: &str,
) -> Vec<Discovered> {
    let quoted = format!("\"{term}\"");
    let resp = http
        .get("https://efts.sec.gov/LATEST/search-index")
        .query(&[("q", quoted.as_str())])
        .header(reqwest::header::USER_AGENT, EDGAR_UA)
        .send()
        .await;
    let Ok(resp) = resp else { return Vec::new() };
    if !resp.status().is_success() {
        return Vec::new();
    }
    match resp.json::<Value>().await {
        Ok(json) => parse_edgar_fts(&json, bottleneck, term),
        Err(_) => Vec::new(),
    }
}

/// One Yahoo predefined-screener pull. `None`/empty on any failure. The
/// predefined `saved` endpoint does not require the cookie+crumb handshake.
async fn fetch_yahoo_screener(http: &reqwest::Client, scr_id: &str, title: &str) -> Vec<Discovered> {
    let resp = http
        .get("https://query1.finance.yahoo.com/v1/finance/screener/predefined/saved")
        .query(&[("count", "50"), ("scrIds", scr_id)])
        .header(reqwest::header::USER_AGENT, "Mozilla/5.0")
        .send()
        .await;
    let Ok(resp) = resp else { return Vec::new() };
    if !resp.status().is_success() {
        return Vec::new();
    }
    match resp.json::<Value>().await {
        Ok(json) => parse_yahoo_screener(&json, title),
        Err(_) => Vec::new(),
    }
}

/// Run all discovery sources concurrently and return a de-duplicated, capped
/// list of candidates. EDGAR (on-thesis) hits win over Yahoo (breadth) hits for
/// the same ticker. Best-effort throughout.
pub async fn discover(
    http: &reqwest::Client,
    bottlenecks: &[(String, String)],
    max_total: usize,
) -> Vec<Discovered> {
    if max_total == 0 {
        return Vec::new();
    }

    // Derive a de-duplicated set of (term -> first bottleneck title) to search.
    let mut terms: Vec<(String, String)> = Vec::new();
    let mut term_seen: BTreeSet<String> = BTreeSet::new();
    for (title, description) in bottlenecks {
        for term in bottleneck_terms(title, description) {
            let key = term.to_ascii_lowercase();
            if term_seen.insert(key) {
                terms.push((term, title.clone()));
            }
        }
    }

    let limit = Arc::new(Semaphore::new(HTTP_CONCURRENCY));
    let mut set = tokio::task::JoinSet::new();

    for (term, title) in terms {
        let http = http.clone();
        let limit = limit.clone();
        set.spawn(async move {
            let _permit = limit.acquire_owned().await.ok();
            fetch_edgar_fts(&http, &title, &term).await
        });
    }
    for (scr_id, title) in YAHOO_THEMES {
        let http = http.clone();
        let limit = limit.clone();
        set.spawn(async move {
            let _permit = limit.acquire_owned().await.ok();
            fetch_yahoo_screener(&http, scr_id, title).await
        });
    }

    // Collect EDGAR and Yahoo separately so EDGAR (on-thesis) takes precedence
    // when both surface the same ticker.
    let mut edgar: Vec<Discovered> = Vec::new();
    let mut yahoo: Vec<Discovered> = Vec::new();
    while let Some(joined) = set.join_next().await {
        if let Ok(found) = joined {
            for d in found {
                if d.source == SRC_EDGAR {
                    edgar.push(d);
                } else {
                    yahoo.push(d);
                }
            }
        }
    }

    let mut out: Vec<Discovered> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for d in edgar.into_iter().chain(yahoo) {
        if out.len() >= max_total {
            break;
        }
        if seen.insert(d.ticker.clone()) {
            out.push(d);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plausible_ticker_accepts_real_symbols_rejects_junk() {
        assert!(is_plausible_ticker("MU"));
        assert!(is_plausible_ticker("SNDK"));
        assert!(is_plausible_ticker("BRK.B"));
        assert!(!is_plausible_ticker("")); // empty
        assert!(!is_plausible_ticker("0001000180")); // CIK number
        assert!(!is_plausible_ticker("TOOLONGX")); // 8 chars
        assert!(!is_plausible_ticker("123")); // leading digit
    }

    #[test]
    fn display_name_extracts_primary_ticker() {
        assert_eq!(
            parse_display_name("WESTERN DIGITAL CORP  (WDC)  (CIK 0000106040)"),
            Some(("WESTERN DIGITAL CORP".into(), "WDC".into()))
        );
        // Multiple tickers → primary (first).
        assert_eq!(
            parse_display_name("Sandisk Corp  (SNDK, SNDKV)  (CIK 0002023554)"),
            Some(("Sandisk Corp".into(), "SNDK".into()))
        );
    }

    #[test]
    fn display_name_skips_tickerless_filers() {
        // Private/delisted filer: only a CIK, no ticker parenthetical.
        assert_eq!(parse_display_name("SANDISK CORP  (CIK 0001000180)"), None);
        assert_eq!(parse_display_name("Some Law Firm LLP"), None);
    }

    #[test]
    fn edgar_fts_parses_relevant_listed_names_in_order() {
        let json = serde_json::json!({
            "hits": { "hits": [
                { "_source": { "display_names": ["WESTERN DIGITAL CORP  (WDC)  (CIK 0000106040)"] } },
                { "_source": { "display_names": ["SANDISK CORP  (CIK 0001000180)"] } }, // skipped (no ticker)
                { "_source": { "display_names": ["MICRON TECHNOLOGY INC  (MU)  (CIK 0000723125)"] } },
                { "_source": { "display_names": ["WESTERN DIGITAL CORP  (WDC)  (CIK 0000106040)"] } } // dup
            ]}
        });
        let found = parse_edgar_fts(&json, "NAND undersupply", "NAND");
        let tickers: Vec<&str> = found.iter().map(|d| d.ticker.as_str()).collect();
        assert_eq!(tickers, vec!["WDC", "MU"]); // tickerless + dup dropped, order kept
        assert_eq!(found[0].source, SRC_EDGAR);
        assert_eq!(found[0].bottleneck, "NAND undersupply");
        assert_eq!(found[0].context, "NAND");
    }

    #[test]
    fn yahoo_screener_parses_symbols_and_names() {
        let json = serde_json::json!({
            "finance": { "result": [ { "quotes": [
                { "symbol": "MU", "shortName": "Micron Technology, Inc." },
                { "symbol": "SNDK", "longName": "Sandisk Corporation" },
                { "symbol": "MU", "shortName": "dup" }, // de-duped
                { "symbol": "0001", "shortName": "junk" } // implausible ticker
            ]}]}
        });
        let found = parse_yahoo_screener(&json, "Day's gainers");
        let tickers: Vec<&str> = found.iter().map(|d| d.ticker.as_str()).collect();
        assert_eq!(tickers, vec!["MU", "SNDK"]);
        assert_eq!(found[1].company, "Sandisk Corporation"); // longName fallback
        assert_eq!(found[0].source, SRC_YAHOO);
        assert_eq!(found[0].bottleneck, YAHOO_BUCKET);
    }

    #[test]
    fn acronyms_pulls_vocabulary_drops_stopwords() {
        let got = acronyms("High-bandwidth memory (HBM) and NAND supply for the USA");
        assert_eq!(got, vec!["HBM", "NAND"]); // AND/THE/FOR/USA dropped, deduped
    }

    #[test]
    fn bottleneck_terms_prefers_acronyms_then_phrase() {
        // Acronyms present → up to two, title first.
        let t = bottleneck_terms("DRAM and HBM shortage", "Tight NAND too");
        assert_eq!(t, vec!["DRAM", "HBM"]);

        // No acronyms → short title phrase fallback.
        let t2 = bottleneck_terms("Constrained advanced packaging capacity", "");
        assert_eq!(t2, vec!["constrained advanced packaging capacity"]);

        // Too short to phrase and no acronyms → empty (skip EDGAR for it).
        assert!(bottleneck_terms("Gap", "x").is_empty());
    }
}
