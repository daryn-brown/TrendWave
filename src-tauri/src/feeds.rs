use feed_rs::parser;
use serde::Deserialize;

use crate::error::{AppError, AppResult};
use crate::model::{NewsItem, PriceData};

const CHART_URL: &str = "https://query1.finance.yahoo.com/v8/finance/chart/";
const SEARCH_URL: &str = "https://query1.finance.yahoo.com/v1/finance/search";
const NEWS_URL: &str = "https://feeds.finance.yahoo.com/rss/2.0/headline";

// ---- Prices -----------------------------------------------------------------

#[derive(Deserialize)]
struct ChartEnvelope {
    chart: ChartBody,
}

#[derive(Deserialize)]
struct ChartBody {
    result: Option<Vec<ChartResult>>,
}

#[derive(Deserialize)]
struct ChartResult {
    meta: ChartMeta,
    indicators: Indicators,
}

#[derive(Deserialize)]
struct ChartMeta {
    #[serde(rename = "regularMarketPrice")]
    regular_market_price: Option<f64>,
    currency: Option<String>,
}

#[derive(Deserialize)]
struct Indicators {
    quote: Vec<Quote>,
}

#[derive(Deserialize)]
struct Quote {
    close: Option<Vec<Option<f64>>>,
    volume: Option<Vec<Option<f64>>>,
}

/// Fetch a price snapshot for `symbol` over the last month of daily candles.
/// Doubles as ticker validation: a symbol with no chart data is treated as
/// invalid and surfaces as `EmptyFeed`.
pub async fn fetch_price(http: &reqwest::Client, symbol: &str) -> AppResult<PriceData> {
    let url = format!("{CHART_URL}{symbol}?range=1mo&interval=1d");
    let envelope: ChartEnvelope = http.get(&url).send().await?.json().await?;

    let result = envelope
        .chart
        .result
        .and_then(|mut r| if r.is_empty() { None } else { Some(r.remove(0)) })
        .ok_or_else(|| AppError::EmptyFeed(format!("no price data for {symbol}")))?;

    let quote = result
        .indicators
        .quote
        .into_iter()
        .next()
        .ok_or_else(|| AppError::EmptyFeed(format!("no quote series for {symbol}")))?;

    let closes: Vec<f64> = quote
        .close
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .collect();
    let volumes: Vec<f64> = quote
        .volume
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .collect();

    let price = result
        .meta
        .regular_market_price
        .or_else(|| closes.last().copied())
        .ok_or_else(|| AppError::EmptyFeed(format!("no last price for {symbol}")))?;

    let change_pct = match (closes.first(), closes.last()) {
        (Some(first), Some(last)) if *first > 0.0 => (last - first) / first * 100.0,
        _ => 0.0,
    };
    let avg_volume = if volumes.is_empty() {
        0.0
    } else {
        volumes.iter().sum::<f64>() / volumes.len() as f64
    };

    Ok(PriceData {
        price,
        currency: result.meta.currency.unwrap_or_else(|| "USD".to_string()),
        change_pct,
        last_volume: volumes.last().copied().unwrap_or(0.0),
        avg_volume,
    })
}

#[derive(Deserialize)]
struct SearchEnvelope {
    quotes: Vec<SearchQuote>,
}

#[derive(Deserialize)]
struct SearchQuote {
    symbol: Option<String>,
}

/// Resolve a company name to its most likely ticker. Used as a fallback when
/// the model proposes a ticker that fails price validation.
pub async fn resolve_symbol(http: &reqwest::Client, company: &str) -> AppResult<String> {
    let envelope: SearchEnvelope = http
        .get(SEARCH_URL)
        .query(&[("q", company), ("quotesCount", "1"), ("newsCount", "0")])
        .send()
        .await?
        .json()
        .await?;

    envelope
        .quotes
        .into_iter()
        .find_map(|q| q.symbol)
        .ok_or_else(|| AppError::EmptyFeed(format!("no symbol match for {company}")))
}

// ---- News -------------------------------------------------------------------

/// Fetch up to `limit` recent headlines for a ticker from Yahoo's RSS feed.
/// Sentiment is left `None` here; it is filled in later by the LLM.
pub async fn fetch_news(
    http: &reqwest::Client,
    symbol: &str,
    limit: usize,
) -> AppResult<Vec<NewsItem>> {
    let bytes = http
        .get(NEWS_URL)
        .query(&[("s", symbol), ("region", "US"), ("lang", "en-US")])
        .send()
        .await?
        .bytes()
        .await?;

    let feed = parser::parse(bytes.as_ref())
        .map_err(|err| AppError::EmptyFeed(format!("could not parse news for {symbol}: {err}")))?;

    let items = feed
        .entries
        .into_iter()
        .take(limit)
        .map(|entry| NewsItem {
            title: entry
                .title
                .map(|t| t.content)
                .unwrap_or_else(|| "(untitled)".to_string()),
            url: entry
                .links
                .into_iter()
                .next()
                .map(|l| l.href)
                .unwrap_or_default(),
            source: "Yahoo Finance".to_string(),
            published: entry.published.map(|d| d.to_rfc3339()),
            sentiment: None,
        })
        .collect();

    Ok(items)
}
