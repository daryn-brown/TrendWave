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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PriceData {
    pub price: f64,
    pub currency: String,
    /// Percent change over the recent lookback window.
    pub change_pct: f64,
    pub last_volume: f64,
    pub avg_volume: f64,
}

/// A ranked result card: a cheap stock exposed to a bottleneck, with the full
/// thesis the UI renders (price, why-cheap, bottleneck link, upside, sentiment).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub ticker: String,
    pub company: String,
    pub price: Option<PriceData>,
    /// Which bottleneck (by title) this company is exposed to.
    pub bottleneck: String,
    pub bottleneck_thesis: String,
    pub why_cheap: String,
    pub upside_rationale: String,
    /// Aggregate news sentiment in -1.0 .. 1.0, `None` if news disabled/empty.
    pub sentiment: Option<f64>,
    pub news: Vec<NewsItem>,
    /// Composite 0-100 score used for ranking (bottleneck-weighted).
    pub score: f64,
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
