//! Real growth fundamentals.
//!
//! Two tiers, both keyless:
//!  * **SEC EDGAR** — the reliable backbone. We map a ticker to its CIK and pull
//!    annual revenue + net-income history from XBRL `companyconcept`, then derive
//!    real growth (latest YoY, multi-year CAGR, profitability). EDGAR is stable
//!    and built for programmatic access, so this is what we depend on.
//!  * **Yahoo `quoteSummary`** — opportunistic enrichment (forward P/E, analyst
//!    target upside). It now requires a cookie+crumb handshake and rate-limits
//!    hard, so every step is best-effort: any failure degrades to EDGAR-only.
//!
//! Parsing and scoring are split into pure functions so they unit-test without
//! the network.

use std::collections::HashMap;

use chrono::{Datelike, NaiveDate};
use reqwest::header::USER_AGENT;
use serde::Deserialize;
use tokio::sync::OnceCell;

use crate::model::GrowthData;
use crate::providers::EstimateRevisions;

/// SEC fair-access requires a descriptive User-Agent that includes a contact
/// email; Akamai 403s generic tool UAs. Operators should substitute their own
/// address. See https://www.sec.gov/os/webmaster-faq#developers.
pub(crate) const EDGAR_UA: &str = "TrendWave/0.1 (admin@trendwave.app)";
/// Yahoo blocks obvious bots; present as a normal browser for the crumb dance.
const BROWSER_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// Revenue is reported under different us-gaap tags depending on the filer and
/// era; try the common ones in order and take the first with a usable series.
const REVENUE_CONCEPTS: &[&str] = &[
    "RevenueFromContractWithCustomerExcludingAssessedTax",
    "Revenues",
    "SalesRevenueNet",
    "RevenueFromContractWithCustomerIncludingAssessedTax",
];

// ---- Public entry point -----------------------------------------------------

/// Best-effort growth lookup for one candidate. Returns `None` only when neither
/// EDGAR nor Yahoo yields anything, in which case the caller scores it neutral.
pub async fn fetch_growth(
    http: &reqwest::Client,
    ticker: &str,
    yahoo: Option<&YahooEnrich>,
) -> Option<GrowthData> {
    let edgar = edgar_growth(http, ticker).await;
    let yahoo_metrics = match yahoo {
        Some(y) => y.fetch(ticker).await,
        None => None,
    };
    build_growth(edgar, yahoo_metrics)
}

/// Map the raw EDGAR + Yahoo signals into a single `GrowthData`, preferring
/// EDGAR's audited numbers and filling gaps with Yahoo. Pure for testability.
fn build_growth(edgar: Option<EdgarOut>, yahoo: Option<YahooMetrics>) -> Option<GrowthData> {
    if edgar.is_none() && yahoo.is_none() {
        return None;
    }
    let mut g = GrowthData::default();
    let has_edgar = edgar.is_some();
    let has_yahoo = yahoo.is_some();

    if let Some(e) = edgar {
        g.revenue_growth_yoy = e.revenue_yoy;
        g.revenue_cagr = e.revenue_cagr;
        g.earnings_growth_yoy = e.earnings_yoy;
        g.profitable = e.profitable;
        g.years = e.years;
    }
    // The YoY figures are audited *annual* numbers only when EDGAR supplied the
    // revenue series; capture that before the Yahoo fallback below, which fills
    // gaps with most-recent-quarter growth.
    g.annual_growth = g.revenue_growth_yoy.is_some();

    if let Some(y) = yahoo {
        g.forward_pe = y.forward_pe;
        g.analyst_upside = y.analyst_upside;
        g.market_cap = y.market_cap;
        // Only fall back to Yahoo's growth figures where EDGAR was silent.
        if g.revenue_growth_yoy.is_none() {
            g.revenue_growth_yoy = y.revenue_growth;
        }
        if g.earnings_growth_yoy.is_none() {
            g.earnings_growth_yoy = y.earnings_growth;
        }
    }
    g.source = match (has_edgar, has_yahoo) {
        (true, true) => "SEC EDGAR + Yahoo",
        (true, false) => "SEC EDGAR",
        (false, true) => "Yahoo Finance",
        (false, false) => "",
    }
    .to_string();
    Some(g)
}

// ---- Growth score (pure) ----------------------------------------------------

/// Collapse `GrowthData` into a 0..1 score emphasizing *potential for growth*:
/// revenue growth dominates, earnings trend refines, analyst upside nudges.
/// Unknown sub-signals drop out (weights renormalize); a fully empty record
/// sits neutral at 0.5 so missing data never silently tanks a pick.
pub fn growth_score(g: &GrowthData) -> f64 {
    let mut weighted = 0.0;
    let mut total = 0.0;

    if let Some(rev) = revenue_component(g) {
        weighted += 0.55 * rev;
        total += 0.55;
    }
    if let Some(earn) = earnings_component(g) {
        weighted += 0.25 * earn;
        total += 0.25;
    }
    if let Some(analyst) = g.analyst_upside.map(|u| normalize(u, -0.10, 0.40)) {
        weighted += 0.20 * analyst;
        total += 0.20;
    }

    if total == 0.0 {
        0.5
    } else {
        weighted / total
    }
}

/// Blend YoY and CAGR revenue growth onto 0..1 (-10% → 0, +40% → 1).
fn revenue_component(g: &GrowthData) -> Option<f64> {
    let parts: Vec<f64> = [g.revenue_growth_yoy, g.revenue_cagr]
        .into_iter()
        .flatten()
        .map(|x| normalize(x, -0.10, 0.40))
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.iter().sum::<f64>() / parts.len() as f64)
    }
}

/// Profitability gates the band; earnings growth positions within it.
fn earnings_component(g: &GrowthData) -> Option<f64> {
    match (g.profitable, g.earnings_growth_yoy) {
        (Some(true), Some(y)) => Some((0.6 + y.clamp(-0.5, 0.5) * 0.8).clamp(0.0, 1.0)),
        (Some(true), None) => Some(0.6),
        (Some(false), _) => Some(0.3),
        (None, Some(y)) => Some(normalize(y, -0.20, 0.40)),
        (None, None) => None,
    }
}

/// Linear map of `x` from [`lo`, `hi`] onto [0, 1], clamped.
fn normalize(x: f64, lo: f64, hi: f64) -> f64 {
    ((x - lo) / (hi - lo)).clamp(0.0, 1.0)
}

// ---- SEC EDGAR --------------------------------------------------------------

struct EdgarOut {
    revenue_yoy: Option<f64>,
    revenue_cagr: Option<f64>,
    earnings_yoy: Option<f64>,
    profitable: Option<bool>,
    years: Option<u32>,
}

/// Process-wide cache of the ~800KB ticker→CIK map; fetched at most once.
static CIK_MAP: OnceCell<HashMap<String, String>> = OnceCell::const_new();

#[derive(Deserialize)]
struct TickerRow {
    cik_str: u64,
    ticker: String,
}

#[derive(Deserialize)]
struct ConceptResponse {
    units: HashMap<String, Vec<ConceptPoint>>,
}

#[derive(Deserialize, Clone)]
pub(crate) struct ConceptPoint {
    #[serde(default)]
    pub(crate) start: Option<String>,
    #[serde(default)]
    pub(crate) end: Option<String>,
    pub(crate) val: f64,
    #[serde(default)]
    pub(crate) form: Option<String>,
    #[serde(default)]
    pub(crate) frame: Option<String>,
}

async fn edgar_growth(http: &reqwest::Client, ticker: &str) -> Option<EdgarOut> {
    let cik = cik_for(http, ticker).await?;

    let mut revenue: Vec<(i32, f64)> = Vec::new();
    for concept in REVENUE_CONCEPTS {
        if let Some(series) = fetch_concept(http, &cik, concept).await {
            // Prefer the first tag that gives us a real multi-year series.
            if series.len() >= 2 {
                revenue = series;
                break;
            }
            if revenue.is_empty() {
                revenue = series;
            }
        }
    }
    let net_income = fetch_concept(http, &cik, "NetIncomeLoss")
        .await
        .unwrap_or_default();

    if revenue.len() < 2 && net_income.len() < 2 {
        return None;
    }

    let years = revenue
        .first()
        .zip(revenue.last())
        .map(|((y0, _), (y1, _))| (y1 - y0) as u32 + 1);

    Some(EdgarOut {
        revenue_yoy: yoy(&revenue),
        revenue_cagr: cagr(&revenue),
        earnings_yoy: yoy(&net_income),
        profitable: net_income.last().map(|(_, v)| *v > 0.0),
        years,
    })
}

/// Shared ticker→CIK lookup (reuses the process-wide cached map). Exposed so the
/// inflection module can resolve CIKs without re-fetching the ~800KB map.
pub(crate) async fn cik_for(http: &reqwest::Client, ticker: &str) -> Option<String> {
    let map = CIK_MAP
        .get_or_try_init(|| load_cik_map(http))
        .await
        .ok()?;
    map.get(&ticker.to_ascii_uppercase()).cloned()
}

async fn load_cik_map(http: &reqwest::Client) -> Result<HashMap<String, String>, reqwest::Error> {
    let rows: HashMap<String, TickerRow> = http
        .get("https://www.sec.gov/files/company_tickers.json")
        .header(USER_AGENT, EDGAR_UA)
        .send()
        .await?
        .json()
        .await?;
    let mut map = HashMap::with_capacity(rows.len());
    for row in rows.values() {
        // CIK is zero-padded to 10 digits in the data.sec.gov path.
        map.insert(row.ticker.to_ascii_uppercase(), format!("{:010}", row.cik_str));
    }
    Ok(map)
}

async fn fetch_concept(
    http: &reqwest::Client,
    cik: &str,
    concept: &str,
) -> Option<Vec<(i32, f64)>> {
    let points = fetch_concept_points(http, cik, concept).await?;
    let series = annual_series(&points);
    if series.is_empty() {
        None
    } else {
        Some(series)
    }
}

/// Fetch the raw USD `companyconcept` points for one us-gaap tag. Returns the
/// unreduced series so callers can reduce to annual *or* quarterly cadence.
/// Shared with the inflection module. Best-effort: `None` on any HTTP/parse miss.
pub(crate) async fn fetch_concept_points(
    http: &reqwest::Client,
    cik: &str,
    concept: &str,
) -> Option<Vec<ConceptPoint>> {
    let url =
        format!("https://data.sec.gov/api/xbrl/companyconcept/CIK{cik}/us-gaap/{concept}.json");
    let resp = http.get(&url).header(USER_AGENT, EDGAR_UA).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let parsed: ConceptResponse = resp.json().await.ok()?;
    let usd = parsed.units.get("USD")?;
    if usd.is_empty() {
        None
    } else {
        Some(usd.clone())
    }
}

/// Reduce raw XBRL points to one value per fiscal year, ascending. Prefer
/// EDGAR's canonical `CY####` annual frames; if too few, fall back to detecting
/// full-year 10-K periods (~365-day spans). Keeps the latest filing per year.
fn annual_series(points: &[ConceptPoint]) -> Vec<(i32, f64)> {
    let mut by_year: HashMap<i32, (String, f64)> = HashMap::new();

    for p in points {
        if let Some(year) = p.frame.as_deref().and_then(calendar_year_frame) {
            keep_latest(&mut by_year, year, p.end.clone().unwrap_or_default(), p.val);
        }
    }

    if by_year.len() < 2 {
        for p in points {
            if p.form.as_deref() != Some("10-K") {
                continue;
            }
            if let Some(year) = full_year_period(p.start.as_deref(), p.end.as_deref()) {
                keep_latest(&mut by_year, year, p.end.clone().unwrap_or_default(), p.val);
            }
        }
    }

    let mut series: Vec<(i32, f64)> = by_year.into_iter().map(|(y, (_, v))| (y, v)).collect();
    series.sort_by_key(|(y, _)| *y);
    series
}

/// Record `val` for `year`, keeping whichever entry was filed for the latest
/// period end (later restatements supersede earlier ones).
fn keep_latest(by_year: &mut HashMap<i32, (String, f64)>, year: i32, end: String, val: f64) {
    by_year
        .entry(year)
        .and_modify(|cur| {
            if end > cur.0 {
                *cur = (end.clone(), val);
            }
        })
        .or_insert((end, val));
}

/// Parse a clean annual `CY2024` frame (rejecting quarterly `CY2024Q1` etc.).
fn calendar_year_frame(frame: &str) -> Option<i32> {
    let digits = frame.strip_prefix("CY")?;
    if digits.len() == 4 && digits.bytes().all(|b| b.is_ascii_digit()) {
        digits.parse().ok()
    } else {
        None
    }
}

/// Year of an entry whose start..end spans roughly a full fiscal year.
fn full_year_period(start: Option<&str>, end: Option<&str>) -> Option<i32> {
    let start = NaiveDate::parse_from_str(start?, "%Y-%m-%d").ok()?;
    let end = NaiveDate::parse_from_str(end?, "%Y-%m-%d").ok()?;
    let days = (end - start).num_days();
    if (330..=400).contains(&days) {
        Some(end.year())
    } else {
        None
    }
}

/// Latest year-over-year growth from an ascending annual series.
fn yoy(series: &[(i32, f64)]) -> Option<f64> {
    if series.len() < 2 {
        return None;
    }
    let prev = series[series.len() - 2].1;
    let last = series[series.len() - 1].1;
    if prev > 0.0 {
        Some((last - prev) / prev)
    } else {
        None
    }
}

/// Compound annual growth rate across the full span of an ascending series.
fn cagr(series: &[(i32, f64)]) -> Option<f64> {
    if series.len() < 2 {
        return None;
    }
    let (y0, v0) = series[0];
    let (y1, v1) = series[series.len() - 1];
    let span = (y1 - y0) as f64;
    if v0 > 0.0 && v1 > 0.0 && span >= 1.0 {
        Some((v1 / v0).powf(1.0 / span) - 1.0)
    } else {
        None
    }
}

// ---- Yahoo enrichment (opportunistic) ---------------------------------------

/// Quote-currency subunit factor. Yahoo quotes London (pence "GBp"),
/// Johannesburg (cents "ZAc") and Tel Aviv (agorot "ILA") in 1/100 of the major
/// unit while reporting EPS in the major unit — which inflates its P/E ~100x.
fn subunit_factor(currency: &str) -> f64 {
    match currency {
        "GBp" | "GBX" | "ZAc" | "ZAX" | "ILA" | "ILa" => 100.0,
        _ => 1.0,
    }
}

/// Rescale Yahoo's forward P/E by the quote subunit, then drop values that are
/// non-positive or implausibly large — those are data errors, not real
/// multiples, and showing them is worse than showing nothing.
fn sane_forward_pe(pe: f64, subunit: f64) -> Option<f64> {
    let pe = pe / subunit;
    (pe > 0.0 && pe < 250.0).then_some(pe)
}

pub struct YahooMetrics {
    pub forward_pe: Option<f64>,
    pub analyst_upside: Option<f64>,
    pub revenue_growth: Option<f64>,
    pub earnings_growth: Option<f64>,
    pub market_cap: Option<f64>,
}

/// Read `marketCap` from a `quoteSummary` result node's `price` module. Yahoo
/// wraps numbers as `{ "raw": <f64>, "fmt": "..." }`; a positive finite raw is
/// required, otherwise the size signal stays neutral. Pure, so it is unit-tested
/// against a sample payload (the live handshake needs cookies + a crumb).
fn quote_market_cap(node: &serde_json::Value) -> Option<f64> {
    node["price"]["marketCap"]["raw"]
        .as_f64()
        .filter(|mc| mc.is_finite() && *mc > 0.0)
}

/// A primed Yahoo session: a cookie-jar client plus a fetched crumb. Cloneable
/// so the research pipeline can hand it to each concurrent task. Construct once
/// per run via [`YahooEnrich::init`]; if the handshake fails it returns `None`
/// and the pipeline simply runs EDGAR-only.
#[derive(Clone)]
pub struct YahooEnrich {
    client: reqwest::Client,
    crumb: String,
}

impl YahooEnrich {
    pub async fn init() -> Option<Self> {
        let client = reqwest::Client::builder()
            .cookie_store(true)
            .user_agent(BROWSER_UA)
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .ok()?;

        // Prime consent cookies, then ask for a crumb bound to them.
        let _ = client.get("https://fc.yahoo.com/").send().await;
        let _ = client.get("https://finance.yahoo.com/").send().await;
        let crumb = client
            .get("https://query2.finance.yahoo.com/v1/test/getcrumb")
            .send()
            .await
            .ok()?
            .text()
            .await
            .ok()?;

        // A real crumb is a short token; reject error strings like
        // "Too Many Requests" or empty/HTML responses.
        let crumb = crumb.trim().to_string();
        if crumb.is_empty() || crumb.len() > 16 || crumb.contains(char::is_whitespace) {
            return None;
        }
        Some(Self { client, crumb })
    }

    async fn fetch(&self, symbol: &str) -> Option<YahooMetrics> {
        let safe_symbol = crate::feeds::validate_ticker(symbol).ok()?;
        let value: serde_json::Value = self
            .client
            .get(format!(
                "https://query2.finance.yahoo.com/v10/finance/quoteSummary/{safe_symbol}"
            ))
            .query(&[
                ("modules", "financialData,defaultKeyStatistics,price"),
                ("crumb", self.crumb.as_str()),
            ])
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;

        let node = value["quoteSummary"]["result"].get(0)?;
        let financial = &node["financialData"];
        let stats = &node["defaultKeyStatistics"];
        let raw = |v: &serde_json::Value| v.get("raw").and_then(serde_json::Value::as_f64);

        // London/JSE/TASE names are quoted in a subunit but report EPS in the
        // major unit, so Yahoo's P/E for them is inflated ~100x. Detect the
        // subunit from the quote currency and rescale, then guard the result.
        let quote_currency = node["price"]["currency"].as_str().unwrap_or("");
        let subunit = subunit_factor(quote_currency);
        let forward_pe = raw(&stats["forwardPE"]).and_then(|pe| sane_forward_pe(pe, subunit));

        // Derive analyst upside entirely within Yahoo's own quote units (current
        // price vs target are the same currency), so it is immune to subunit
        // scaling and to whatever units the caller's price was in.
        let current = raw(&financial["currentPrice"]);
        let target = raw(&financial["targetMeanPrice"]);
        let analyst_upside = match (target, current) {
            (Some(t), Some(c)) if c > 0.0 => Some((t - c) / c),
            _ => None,
        };

        Some(YahooMetrics {
            forward_pe,
            analyst_upside,
            revenue_growth: raw(&financial["revenueGrowth"]),
            earnings_growth: raw(&financial["earningsGrowth"]),
            // Market cap comes from the `price` module already in this payload,
            // so the room-to-run signal costs no extra request. It is reported in
            // the quote currency; the scorer is gated on a USD quote downstream.
            market_cap: quote_market_cap(node),
        })
    }

    /// Best-effort current analyst recommendation breakdown — the **free** path
    /// for the estimate-revisions signal. Returns `None` on any handshake or
    /// parse miss, so the caller degrades to no-revisions (scored neutral).
    pub async fn fetch_recommendations(&self, symbol: &str) -> Option<EstimateRevisions> {
        let safe_symbol = crate::feeds::validate_ticker(symbol).ok()?;
        let value: serde_json::Value = self
            .client
            .get(format!(
                "https://query2.finance.yahoo.com/v10/finance/quoteSummary/{safe_symbol}"
            ))
            .query(&[
                ("modules", "recommendationTrend"),
                ("crumb", self.crumb.as_str()),
            ])
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        let node = value["quoteSummary"]["result"].get(0)?;
        parse_yahoo_recommendations(&node["recommendationTrend"])
    }
}

/// Reduce Yahoo's `recommendationTrend` to normalized revision counts, preferring
/// the current period (`"0m"`). Pure for testability. `None` when there is no
/// usable coverage so the caller degrades gracefully.
fn parse_yahoo_recommendations(trend_node: &serde_json::Value) -> Option<EstimateRevisions> {
    let trend = trend_node["trend"].as_array()?;
    let row = trend
        .iter()
        .find(|t| t["period"].as_str() == Some("0m"))
        .or_else(|| trend.first())?;
    let count = |key: &str| row[key].as_u64().unwrap_or(0) as u32;

    let up = count("strongBuy") + count("buy");
    let down = count("sell") + count("strongSell");
    let total = up + count("hold") + down;
    if total == 0 {
        None
    } else {
        Some(EstimateRevisions { up, down, total })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(frame: Option<&str>, end: &str, val: f64) -> ConceptPoint {
        ConceptPoint {
            start: None,
            end: Some(end.to_string()),
            val,
            form: Some("10-K".to_string()),
            frame: frame.map(str::to_string),
        }
    }

    #[test]
    fn annual_series_prefers_calendar_frames() {
        let points = vec![
            point(Some("CY2021"), "2021-12-31", 100.0),
            point(Some("CY2022"), "2022-12-31", 120.0),
            point(Some("CY2022Q1"), "2022-03-31", 30.0), // quarterly, must be ignored
            point(Some("CY2023"), "2023-12-31", 150.0),
        ];
        let series = annual_series(&points);
        assert_eq!(series, vec![(2021, 100.0), (2022, 120.0), (2023, 150.0)]);
    }

    #[test]
    fn annual_series_falls_back_to_full_year_periods() {
        let points = vec![
            ConceptPoint {
                start: Some("2022-01-01".into()),
                end: Some("2022-12-31".into()),
                val: 200.0,
                form: Some("10-K".into()),
                frame: None,
            },
            ConceptPoint {
                start: Some("2023-01-01".into()),
                end: Some("2023-12-31".into()),
                val: 260.0,
                form: Some("10-K".into()),
                frame: None,
            },
            // A quarter inside a 10-K should not count as a year.
            ConceptPoint {
                start: Some("2023-01-01".into()),
                end: Some("2023-03-31".into()),
                val: 60.0,
                form: Some("10-K".into()),
                frame: None,
            },
        ];
        let series = annual_series(&points);
        assert_eq!(series, vec![(2022, 200.0), (2023, 260.0)]);
    }

    #[test]
    fn yoy_and_cagr_compute_real_growth() {
        let series = vec![(2021, 100.0), (2022, 120.0), (2023, 150.0)];
        let yoy = yoy(&series).unwrap();
        let cagr = cagr(&series).unwrap();
        assert!((yoy - 0.25).abs() < 1e-9); // 120 -> 150
        assert!((cagr - 0.224744).abs() < 1e-5); // 100 -> 150 over 2 yrs
    }

    #[test]
    fn growth_score_rewards_faster_growers() {
        let fast = GrowthData {
            revenue_growth_yoy: Some(0.35),
            revenue_cagr: Some(0.30),
            profitable: Some(true),
            earnings_growth_yoy: Some(0.40),
            ..Default::default()
        };
        let slow = GrowthData {
            revenue_growth_yoy: Some(0.02),
            revenue_cagr: Some(0.01),
            profitable: Some(true),
            earnings_growth_yoy: Some(0.0),
            ..Default::default()
        };
        assert!(growth_score(&fast) > growth_score(&slow));
    }

    #[test]
    fn growth_score_penalizes_unprofitable_shrinkers() {
        let shrinking = GrowthData {
            revenue_growth_yoy: Some(-0.15),
            profitable: Some(false),
            ..Default::default()
        };
        assert!(growth_score(&shrinking) < 0.4);
    }

    #[test]
    fn growth_score_is_neutral_without_data() {
        assert_eq!(growth_score(&GrowthData::default()), 0.5);
    }

    #[test]
    fn build_growth_labels_provenance() {
        let edgar_only = build_growth(
            Some(EdgarOut {
                revenue_yoy: Some(0.1),
                revenue_cagr: Some(0.1),
                earnings_yoy: Some(0.1),
                profitable: Some(true),
                years: Some(3),
            }),
            None,
        )
        .unwrap();
        assert_eq!(edgar_only.source, "SEC EDGAR");
        assert!(edgar_only.annual_growth); // EDGAR revenue series => annual
        assert!(build_growth(None, None).is_none());
    }

    #[test]
    fn yahoo_only_growth_is_marked_quarterly() {
        let yahoo_only = build_growth(
            None,
            Some(YahooMetrics {
                forward_pe: Some(12.0),
                analyst_upside: Some(0.05),
                revenue_growth: Some(0.264),
                earnings_growth: Some(0.1),
                market_cap: Some(5_000_000_000.0),
            }),
        )
        .unwrap();
        assert_eq!(yahoo_only.source, "Yahoo Finance");
        assert!(!yahoo_only.annual_growth); // Yahoo => most-recent-quarter
        assert_eq!(yahoo_only.revenue_growth_yoy, Some(0.264));
    }

    #[test]
    fn forward_pe_rescaled_for_pence_quotes() {
        // Yahoo's inflated 1124.4 for a London pence quote is really ~11.2.
        let pe = sane_forward_pe(1124.4, subunit_factor("GBp")).unwrap();
        assert!((pe - 11.244).abs() < 1e-3);
    }

    #[test]
    fn market_cap_parsed_from_price_module() {
        // Representative `quoteSummary` result node shape (the `price` module is
        // already requested for currency + forward P/E, so this is free data).
        let node = serde_json::json!({
            "price": { "currency": "USD", "marketCap": { "raw": 6_120_000_000.0, "fmt": "6.12B" } }
        });
        assert_eq!(quote_market_cap(&node), Some(6_120_000_000.0));
    }

    #[test]
    fn market_cap_absent_or_invalid_is_neutral() {
        assert_eq!(quote_market_cap(&serde_json::json!({ "price": {} })), None);
        assert_eq!(
            quote_market_cap(&serde_json::json!({ "price": { "marketCap": { "raw": 0.0 } } })),
            None
        );
        assert_eq!(quote_market_cap(&serde_json::json!({})), None);
    }

    #[test]
    fn forward_pe_rejects_implausible_and_keeps_normal() {
        assert!(sane_forward_pe(1124.4, 1.0).is_none()); // uncorrected garbage
        assert!(sane_forward_pe(-3.0, 1.0).is_none()); // negative
        assert_eq!(sane_forward_pe(22.5, 1.0), Some(22.5)); // ordinary multiple
        assert_eq!(subunit_factor("USD"), 1.0);
    }

    #[test]
    fn parses_yahoo_recommendation_trend_current_period() {
        let node = serde_json::json!({
            "trend": [
                { "period": "0m", "strongBuy": 5, "buy": 12, "hold": 3, "sell": 1, "strongSell": 0 },
                { "period": "-1m", "strongBuy": 1, "buy": 1, "hold": 9, "sell": 4, "strongSell": 2 }
            ]
        });
        let rev = parse_yahoo_recommendations(&node).expect("usable coverage");
        assert_eq!(rev.up, 17); // 5 + 12
        assert_eq!(rev.down, 1); // 1 + 0
        assert_eq!(rev.total, 21); // 17 up + 3 hold + 1 down
        assert!(rev.net_bias() > 0.7);
    }

    #[test]
    fn yahoo_recommendation_parser_rejects_empty_coverage() {
        assert!(parse_yahoo_recommendations(&serde_json::json!({ "trend": [] })).is_none());
        assert!(parse_yahoo_recommendations(&serde_json::json!({})).is_none());
        let no_votes = serde_json::json!({ "trend": [{ "period": "0m" }] });
        assert!(parse_yahoo_recommendations(&no_votes).is_none());
    }
}
