use serde::{Deserialize, Serialize};

/// A supply-chain / capacity / production chokepoint the model identified for
/// the requested industry. This is the primary signal of the whole app, so it
/// is a first-class type rather than free text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bottleneck {
    pub title: String,
    pub description: String,
    /// 1 (mild) .. 5 (severe). Weighted heavily in candidate scoring.
    pub severity: u8,
}

/// A single news headline tied to a candidate, with a locally computed
/// sentiment so the UI can show *why* a stock looks favorable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsItem {
    pub title: String,
    pub url: String,
    pub source: String,
    pub published: Option<String>,
    /// -1.0 (bearish) .. 1.0 (bullish); `None` until sentiment runs.
    pub sentiment: Option<f64>,
}

/// Market snapshot for a candidate, pulled from free price feeds.
/// `#[serde(default)]` keeps older saved results (without `name`) loadable.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PriceData {
    pub price: f64,
    pub currency: String,
    /// Authoritative instrument name from the feed (Yahoo `longName`/`shortName`).
    /// This is the *real-world identity* of the ticker, used to catch cases where
    /// the model stamped a real ticker onto an unrelated company/sector.
    pub name: Option<String>,
    /// Percent change over the recent lookback window.
    pub change_pct: f64,
    pub last_volume: f64,
    pub avg_volume: f64,
}

/// Real growth fundamentals for a candidate, sourced from SEC EDGAR (reliable
/// backbone) and opportunistically enriched by Yahoo. This is the *data* that
/// replaces the model's invented upside guess when scoring for growth potential.
/// All fields are optional so a candidate the feeds don't cover still renders.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GrowthData {
    /// Latest year-over-year revenue growth as a fraction (0.13 = +13%).
    pub revenue_growth_yoy: Option<f64>,
    /// Revenue CAGR over the available annual window, as a fraction.
    pub revenue_cagr: Option<f64>,
    /// Latest year-over-year net-income growth as a fraction.
    pub earnings_growth_yoy: Option<f64>,
    /// Whether the most recent fiscal year was profitable.
    pub profitable: Option<bool>,
    /// Forward P/E (Yahoo enrichment; absent when Yahoo is unavailable).
    pub forward_pe: Option<f64>,
    /// Analyst mean-target implied upside vs. current price, as a fraction.
    pub analyst_upside: Option<f64>,
    /// Number of fiscal years the EDGAR series spans (context for CAGR).
    pub years: Option<u32>,
    /// Whether the YoY growth figures are audited *annual* results (from EDGAR).
    /// `false` means they are Yahoo's most-recent-quarter figures, which carry a
    /// different, noisier meaning and should be labeled as such.
    #[serde(default)]
    pub annual_growth: bool,
    /// Human-readable provenance, e.g. "SEC EDGAR" or "SEC EDGAR + Yahoo".
    pub source: String,
}

/// A ranked result card: a company best positioned to solve or monopolize a
/// bottleneck, with the thesis the UI renders (positioning/moat, upside,
/// bottleneck link, sentiment). Price is shown as context, never a filter.
/// `#[serde(default)]` keeps older saved watchlist results loadable as fields evolve.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Candidate {
    pub ticker: String,
    pub company: String,
    /// Authoritative company name for `ticker`, resolved from the price feed.
    /// When present the UI shows this instead of the model's claimed name, so a
    /// hallucinated ticker (e.g. "AEM" described as a battery maker, but really
    /// Agnico Eagle Mines) reveals its true identity.
    pub verified_name: Option<String>,
    /// `true` when the model's claimed company plainly contradicts the verified
    /// registrant name for this ticker — a signal the ticker may be misattributed.
    pub identity_mismatch: bool,
    pub price: Option<PriceData>,
    /// Which bottleneck (by title) this company is positioned to win.
    pub bottleneck: String,
    /// Why this company is best positioned to solve or monopolize the bottleneck.
    pub thesis: String,
    /// 1-5: how dominant / monopoly-like the company's position is (5 = near-monopoly).
    pub moat: u8,
    /// 1-5: the model's own upside guess. Retained for context, but no longer
    /// drives ranking — `growth` + `growth_score` (real data) do.
    pub upside: u8,
    pub upside_rationale: String,
    /// Real growth fundamentals, when the feeds cover this ticker.
    pub growth: Option<GrowthData>,
    /// 0..1 data-derived growth score actually used in ranking (neutral 0.5
    /// when no fundamentals are available).
    pub growth_score: f64,
    /// Aggregate news sentiment in -1.0 .. 1.0, `None` if news disabled/empty.
    pub sentiment: Option<f64>,
    pub news: Vec<NewsItem>,
    /// Composite 0-100 score used for ranking (positioning + upside weighted).
    pub score: f64,
    /// `true` when this ticker is held in the user's connected Robinhood account.
    /// Read-only context for the UI ("In your portfolio") — never affects ranking.
    #[serde(default)]
    pub owned: bool,
}

/// The full payload returned to the frontend for one research run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchResult {
    pub industry: String,
    pub summary: String,
    pub bottlenecks: Vec<Bottleneck>,
    pub candidates: Vec<Candidate>,
    pub disclaimer: String,
}

/// Streamed to the frontend over a Tauri channel so the prompt UI can show live
/// progress ("Identifying bottlenecks…", "Pricing candidates…") like an agent.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProgressEvent {
    Stage { stage: String, message: String },
    Bottlenecks { items: Vec<Bottleneck> },
    Candidate { candidate: Candidate },
    Done { result: ResearchResult },
    Failed { kind: String, message: String },
}

pub const DISCLAIMER: &str =
    "Research tool only — not financial advice. Signals are heuristic and may be wrong. \
     Verify every thesis against the linked sources before making any decision.";
