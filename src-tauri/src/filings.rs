//! Phase 4 — SEC-filing forward tells (free, primary-source).
//!
//! Two best-effort signals derived from a company's recent EDGAR filings, both
//! gated behind EarlyDetection upstream:
//!
//!   * **Insider buying (Form 4).** Open-market purchases by insiders
//!     (transaction code `P`) are one of the few genuinely predictive free
//!     signals. Routine sales (`S`) and grants (`A`) are noise — insiders sell
//!     for liquidity constantly — so this is *positive-when-present*: a cluster
//!     of open-market buys lifts the score and the absence of buying is simply
//!     neutral, never a penalty.
//!   * **Filing keyword signal (8-K).** Scans recent 8-K disclosures for
//!     language that confirms or contradicts a capacity / pricing-power thesis
//!     ("record demand", "raising prices", "supply agreement", "sold out"
//!     versus "oversupply", "glut", "demand weakness").
//!
//! Both reduce to a pure, unit-tested scorer; the network layer fetches the
//! submissions index once and reuses it for both signals.

use serde::Deserialize;

use crate::fundamentals::{cik_for, EDGAR_UA};
use reqwest::header::USER_AGENT;

/// Most-recent Form 4 filings to inspect per candidate (each is a ~4 KB XML).
const MAX_FORM4: usize = 10;
/// Most-recent qualifying 8-K filings to scan per candidate (each ~30–60 KB).
const MAX_8K: usize = 2;
/// Cap on text scanned for keywords, to bound work on unusually large exhibits.
const SCAN_CAP: usize = 400_000;
/// Per-phrase occurrence cap so one repeated word can't dominate the balance.
const PHRASE_CAP: usize = 3;

/// 8-K item codes worth scanning (results, regulation-FD, other events). Officer
/// changes / cover pages rarely carry demand language, so we prefer these.
const RICH_8K_ITEMS: &[&str] = &["2.02", "7.01", "8.01", "1.01"];

const POSITIVE: &[&str] = &[
    "record demand", "strong demand", "robust demand", "accelerating demand",
    "pricing power", "raise prices", "raising prices", "price increase",
    "higher prices", "increased prices", "supply agreement", "long-term agreement",
    "sold out", "allocation", "capacity expansion", "supply constrain",
    "tight supply", "lead time", "backlog", "record revenue", "record quarter",
    "demand exceeded",
];

const NEGATIVE: &[&str] = &[
    "oversupply", "glut", "weak demand", "soft demand", "demand weakness",
    "price decline", "pricing pressure", "inventory correction", "write-down",
    "writedown", "underutilization", "excess inventory", "lower prices",
    "demand softness", "reduced prices",
];

/// Both filing-derived signals for one candidate. Each is `Option` so absence
/// scores neutral upstream.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FilingSignals {
    pub insider: Option<f64>,
    pub filing: Option<f64>,
}

// ---------------------------------------------------------------------------
// Pure scoring (unit-tested)
// ---------------------------------------------------------------------------

/// Count open-market purchase transactions (code `P`) in a Form 4 XML body.
fn count_purchases(xml: &str) -> usize {
    xml.matches("<transactionCode>P</transactionCode>").count()
}

/// Map the number of recent Form-4 filings that contained an open-market
/// purchase into a 0..1 score. `None` when there were none → neutral upstream
/// (ordinary insider selling is never penalized).
pub fn insider_score(buy_filings: usize) -> Option<f64> {
    if buy_filings == 0 {
        return None;
    }
    // 1 buy → 0.6, 2 → 0.8, 3+ → 1.0 (a cluster is the strong tell).
    let scaled = ((buy_filings as f64 - 1.0) / 2.0).clamp(0.0, 1.0);
    Some(0.6 + 0.4 * scaled)
}

/// Count occurrences of `needle` in `haystack`, capped at [`PHRASE_CAP`].
fn capped_count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count().min(PHRASE_CAP)
}

/// Balance positive vs. negative thesis language in filing text into a 0..1
/// score centered on 0.5. `None` when the text contains none of either set, so
/// a filing with no relevant language stays neutral.
pub fn filing_score(text: &str) -> Option<f64> {
    let lower = text.to_ascii_lowercase();
    let pos: usize = POSITIVE.iter().map(|p| capped_count(&lower, p)).sum();
    let neg: usize = NEGATIVE.iter().map(|p| capped_count(&lower, p)).sum();
    if pos + neg == 0 {
        return None;
    }
    let bias = (pos as f64 - neg as f64) / (pos + neg) as f64;
    Some((0.5 + 0.5 * bias).clamp(0.0, 1.0))
}

/// Strip EDGAR's inline-XSLT viewer prefix (e.g. `xslF345X06/`) from a primary
/// document path to get the raw underlying file.
fn strip_xsl_prefix(doc: &str) -> &str {
    if let Some(rest) = doc.strip_prefix("xsl") {
        if let Some(idx) = rest.find('/') {
            return &rest[idx + 1..];
        }
    }
    doc
}

// ---------------------------------------------------------------------------
// Submissions index
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Submissions {
    filings: Filings,
}

#[derive(Deserialize)]
struct Filings {
    recent: Recent,
}

#[derive(Deserialize, Default)]
struct Recent {
    #[serde(default)]
    form: Vec<String>,
    #[serde(default, rename = "accessionNumber")]
    accession: Vec<String>,
    #[serde(default, rename = "primaryDocument")]
    primary: Vec<String>,
    #[serde(default)]
    items: Vec<String>,
}

/// Build an `Archives` URL for a filing's primary document. `cik_int` is the CIK
/// with leading zeros stripped (the path form EDGAR uses).
fn archive_url(cik_int: &str, accession: &str, doc: &str) -> String {
    let acc_nodash = accession.replace('-', "");
    let raw = strip_xsl_prefix(doc);
    format!("https://www.sec.gov/Archives/edgar/data/{cik_int}/{acc_nodash}/{raw}")
}

// ---------------------------------------------------------------------------
// Best-effort network
// ---------------------------------------------------------------------------

/// Fetch both filing-derived signals for `ticker`. Fetches the submissions index
/// once, then inspects recent Form 4 (insider) and 8-K (keyword) filings.
/// Best-effort: any miss yields `None` for that signal.
pub async fn fetch_signals(http: &reqwest::Client, ticker: &str) -> FilingSignals {
    let Some(cik) = cik_for(http, ticker).await else {
        return FilingSignals::default();
    };
    let url = format!("https://data.sec.gov/submissions/CIK{cik}.json");
    let subs: Option<Submissions> = async {
        let resp = http.get(&url).header(USER_AGENT, EDGAR_UA).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        resp.json::<Submissions>().await.ok()
    }
    .await;
    let Some(subs) = subs else {
        return FilingSignals::default();
    };
    let recent = subs.filings.recent;
    let cik_int = cik.trim_start_matches('0');

    let insider = fetch_insider(http, cik_int, &recent).await;
    let filing = fetch_filing_keywords(http, cik_int, &recent).await;
    FilingSignals { insider, filing }
}

async fn fetch_insider(http: &reqwest::Client, cik_int: &str, recent: &Recent) -> Option<f64> {
    let mut buy_filings = 0usize;
    let mut inspected = 0usize;
    for i in 0..recent.form.len() {
        if recent.form.get(i).map(|f| f == "4").unwrap_or(false) {
            let (Some(acc), Some(doc)) = (recent.accession.get(i), recent.primary.get(i)) else {
                continue;
            };
            let url = archive_url(cik_int, acc, doc);
            if let Ok(resp) = http.get(&url).header(USER_AGENT, EDGAR_UA).send().await {
                if let Ok(text) = resp.text().await {
                    if count_purchases(&text) > 0 {
                        buy_filings += 1;
                    }
                }
            }
            inspected += 1;
            if inspected >= MAX_FORM4 {
                break;
            }
        }
    }
    insider_score(buy_filings)
}

async fn fetch_filing_keywords(
    http: &reqwest::Client,
    cik_int: &str,
    recent: &Recent,
) -> Option<f64> {
    // Prefer disclosure-rich 8-K items; fall back to any recent 8-K.
    let mut rich: Vec<usize> = Vec::new();
    let mut any: Vec<usize> = Vec::new();
    for i in 0..recent.form.len() {
        if recent.form.get(i).map(|f| f == "8-K").unwrap_or(false) {
            any.push(i);
            let items = recent.items.get(i).map(String::as_str).unwrap_or("");
            if RICH_8K_ITEMS.iter().any(|code| items.contains(code)) {
                rich.push(i);
            }
        }
    }
    let order = if rich.is_empty() { any } else { rich };

    let mut blob = String::new();
    for &i in order.iter().take(MAX_8K) {
        let (Some(acc), Some(doc)) = (recent.accession.get(i), recent.primary.get(i)) else {
            continue;
        };
        let url = archive_url(cik_int, acc, doc);
        if let Ok(resp) = http.get(&url).header(USER_AGENT, EDGAR_UA).send().await {
            if let Ok(text) = resp.text().await {
                blob.push_str(&text);
                blob.push(' ');
            }
        }
        if blob.len() >= SCAN_CAP {
            break;
        }
    }
    if blob.len() > SCAN_CAP {
        blob.truncate(SCAN_CAP);
    }
    filing_score(&blob)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purchases_counted_from_form4_xml() {
        let xml = "<x><transactionCode>P</transactionCode>\
                   <transactionCode>S</transactionCode>\
                   <transactionCode>P</transactionCode></x>";
        assert_eq!(count_purchases(xml), 2);
        assert_eq!(count_purchases("<transactionCode>A</transactionCode>"), 0);
    }

    #[test]
    fn insider_score_is_positive_when_present_else_none() {
        assert_eq!(insider_score(0), None); // no buys → neutral upstream
        assert_eq!(insider_score(1), Some(0.6));
        assert_eq!(insider_score(2), Some(0.8));
        assert_eq!(insider_score(3), Some(1.0));
        assert_eq!(insider_score(9), Some(1.0)); // clamped
    }

    #[test]
    fn filing_score_balances_positive_and_negative_language() {
        let bull = "Demand exceeded supply; we have pricing power and raised prices. \
                    Record demand drove a record quarter; products are sold out.";
        let s = filing_score(bull).unwrap();
        assert!(s > 0.8, "bullish filing should score high, got {s}");

        let bear = "An inventory correction and oversupply led to pricing pressure \
                    and lower prices amid weak demand.";
        let s2 = filing_score(bear).unwrap();
        assert!(s2 < 0.2, "bearish filing should score low, got {s2}");

        // No thesis language at all → neutral (None).
        assert_eq!(filing_score("The board appointed a new director."), None);
    }

    #[test]
    fn xsl_prefix_stripped_to_raw_doc() {
        assert_eq!(
            strip_xsl_prefix("xslF345X06/primarydocument.xml"),
            "primarydocument.xml"
        );
        assert_eq!(strip_xsl_prefix("tm2617112d1_8k.htm"), "tm2617112d1_8k.htm");
    }

    #[test]
    fn archive_url_builds_expected_path() {
        let url = archive_url("723125", "0001798757-26-000006", "xslF345X06/primarydocument.xml");
        assert_eq!(
            url,
            "https://www.sec.gov/Archives/edgar/data/723125/000179875726000006/primarydocument.xml"
        );
    }
}
