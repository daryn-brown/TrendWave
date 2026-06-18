use feed_rs::parser;
use serde::Deserialize;

use crate::error::{AppError, AppResult};
use crate::model::{Listing, ListingInfo, NewsItem, PriceData};

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
    #[serde(rename = "longName")]
    long_name: Option<String>,
    #[serde(rename = "shortName")]
    short_name: Option<String>,
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

/// Some Yahoo quotes are denominated in a currency *subunit* (London in pence
/// "GBp", Johannesburg in cents "ZAc", Tel Aviv in agorot "ILA") rather than the
/// major unit. Convert those to the major unit and canonical ISO-4217 code so
/// the price and any derived ratios are consistent and format correctly (e.g.
/// 2164 GBp → 21.64 GBP, not £2,164). Unknown codes pass through unchanged.
fn to_major_units(price: f64, currency: &str) -> (f64, String) {
    match currency {
        "GBp" | "GBX" => (price / 100.0, "GBP".to_string()),
        "ZAc" | "ZAX" => (price / 100.0, "ZAR".to_string()),
        "ILA" | "ILa" => (price / 100.0, "ILS".to_string()),
        other => (price, other.to_string()),
    }
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

    let raw_price = result
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

    let raw_currency = result.meta.currency.unwrap_or_else(|| "USD".to_string());
    let (price, currency) = to_major_units(raw_price, &raw_currency);
    let name = result
        .meta
        .long_name
        .or(result.meta.short_name)
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty());

    Ok(PriceData {
        price,
        currency,
        name,
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
    #[serde(rename = "shortname")]
    short_name: Option<String>,
    #[serde(rename = "longname")]
    long_name: Option<String>,
    exchange: Option<String>,
    #[serde(rename = "exchDisp")]
    exch_disp: Option<String>,
    #[serde(rename = "quoteType")]
    quote_type: Option<String>,
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

/// Whether a Yahoo symbol is a Canadian listing (TSX `.TO`, TSXV `.V`,
/// Cboe Canada/NEO `.NE`, CSE `.CN`).
fn is_canadian_symbol(sym: &str) -> bool {
    let s = sym.to_ascii_uppercase();
    s.ends_with(".TO") || s.ends_with(".V") || s.ends_with(".NE") || s.ends_with(".CN")
}

/// The root ticker before any exchange suffix (`SHOP.TO` -> `SHOP`).
fn symbol_root(sym: &str) -> &str {
    sym.split('.').next().unwrap_or(sym)
}

/// Normalize a company name for conservative same-security matching: lowercased,
/// punctuation removed, and common corporate/structure tokens dropped so
/// "Shopify Inc." and "Shopify" compare equal.
fn normalize_company(name: &str) -> String {
    const STOP: &[&str] = &[
        "inc", "incorporated", "corp", "corporation", "ltd", "limited", "plc", "co",
        "company", "the", "holdings", "holding", "group", "sa", "nv", "ag", "class",
        "cls", "common", "stock", "shares", "a", "b",
    ];
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .filter(|w| !STOP.contains(w))
        .collect::<Vec<_>>()
        .join("")
}

/// True when two company names match strongly enough to treat two listings as the
/// same security (exact normalized equality, or one is a prefix of the other).
fn names_match(a: &str, b: &str) -> bool {
    let (a, b) = (normalize_company(a), normalize_company(b));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    a == b || a.starts_with(&b) || b.starts_with(&a)
}

/// Pick the best same-security Canadian interlisting from Yahoo search results.
/// Conservative on purpose: only accepts a Canadian-listed *equity* whose root
/// ticker matches the US symbol or whose name strongly matches the company.
fn best_canadian_listing(quotes: &[SearchQuote], us_symbol: &str, company: &str) -> Option<Listing> {
    let mut matches: Vec<&SearchQuote> = quotes
        .iter()
        .filter(|q| {
            q.quote_type
                .as_deref()
                .map(|t| t.eq_ignore_ascii_case("EQUITY"))
                .unwrap_or(false)
        })
        .filter(|q| q.symbol.as_deref().map(is_canadian_symbol).unwrap_or(false))
        .filter(|q| {
            let sym = q.symbol.as_deref().unwrap_or("");
            let root_match = symbol_root(sym).eq_ignore_ascii_case(us_symbol);
            let name = q.long_name.as_deref().or(q.short_name.as_deref()).unwrap_or("");
            root_match || names_match(name, company)
        })
        .collect();

    // Prefer a matching root, then TSX (`.TO`) over the other Canadian venues.
    matches.sort_by_key(|q| {
        let sym = q.symbol.as_deref().unwrap_or("").to_ascii_uppercase();
        let root_match = !symbol_root(&sym).eq_ignore_ascii_case(us_symbol);
        let not_tsx = !sym.ends_with(".TO");
        (root_match, not_tsx)
    });

    matches.first().map(|q| Listing {
        symbol: q.symbol.clone().unwrap_or_default(),
        exchange: q.exch_disp.clone().or_else(|| q.exchange.clone()),
        currency: Some("CAD".to_string()),
    })
}

/// Resolve the listings the Buy action needs for a pick: the US/base symbol with
/// its exchange (for brokers whose deep-link needs an exchange prefix) and a
/// same-security Canadian interlisting when one exists (so Canadian brokers can
/// trade it in CAD without an FX conversion). Best-effort via Yahoo search.
pub async fn resolve_listings(
    http: &reqwest::Client,
    symbol: &str,
    company: &str,
) -> AppResult<ListingInfo> {
    let us_symbol = symbol_root(symbol.trim()).to_ascii_uppercase();
    // Searching by company name surfaces cross-listings (e.g. SHOP and SHOP.TO);
    // fall back to the symbol when no company name is available.
    let query = if company.trim().is_empty() { us_symbol.as_str() } else { company.trim() };

    let envelope: SearchEnvelope = http
        .get(SEARCH_URL)
        .query(&[("q", query), ("quotesCount", "15"), ("newsCount", "0")])
        .send()
        .await?
        .json()
        .await?;

    let us_exchange = envelope
        .quotes
        .iter()
        .find(|q| {
            q.symbol
                .as_deref()
                .map(|s| s.eq_ignore_ascii_case(&us_symbol))
                .unwrap_or(false)
        })
        .and_then(|q| q.exch_disp.clone().or_else(|| q.exchange.clone()));

    let canadian = best_canadian_listing(&envelope.quotes, &us_symbol, company);

    Ok(ListingInfo {
        us_symbol,
        us_exchange,
        canadian,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_pence_quotes_to_pounds() {
        let (price, currency) = to_major_units(2164.0, "GBp");
        assert!((price - 21.64).abs() < 1e-9);
        assert_eq!(currency, "GBP");
    }

    #[test]
    fn passes_through_major_currencies() {
        let (price, currency) = to_major_units(150.0, "USD");
        assert!((price - 150.0).abs() < 1e-9);
        assert_eq!(currency, "USD");
    }

    fn quote(symbol: &str, name: &str, quote_type: &str) -> SearchQuote {
        SearchQuote {
            symbol: Some(symbol.to_string()),
            short_name: Some(name.to_string()),
            long_name: Some(name.to_string()),
            exchange: None,
            exch_disp: Some("Toronto".to_string()),
            quote_type: Some(quote_type.to_string()),
        }
    }

    #[test]
    fn finds_interlisted_canadian_share_by_root() {
        let quotes = vec![
            quote("SHOP", "Shopify Inc.", "EQUITY"),
            quote("SHOP.TO", "Shopify Inc.", "EQUITY"),
        ];
        let found = best_canadian_listing(&quotes, "SHOP", "Shopify Inc.").unwrap();
        assert_eq!(found.symbol, "SHOP.TO");
        assert_eq!(found.currency.as_deref(), Some("CAD"));
    }

    #[test]
    fn matches_interlisting_by_company_name_when_root_differs() {
        // Barrick trades as GOLD on NYSE but ABX on the TSX.
        let quotes = vec![quote("ABX.TO", "Barrick Gold Corporation", "EQUITY")];
        let found = best_canadian_listing(&quotes, "GOLD", "Barrick Gold").unwrap();
        assert_eq!(found.symbol, "ABX.TO");
    }

    #[test]
    fn ignores_unrelated_canadian_listings() {
        let quotes = vec![
            quote("RY.TO", "Royal Bank of Canada", "EQUITY"),
            quote("ETF.TO", "Some Index Fund", "ETF"),
        ];
        assert!(best_canadian_listing(&quotes, "AAPL", "Apple Inc.").is_none());
    }

    #[test]
    fn prefers_tsx_over_other_canadian_venues() {
        let quotes = vec![
            quote("SHOP.NE", "Shopify Inc.", "EQUITY"),
            quote("SHOP.TO", "Shopify Inc.", "EQUITY"),
        ];
        let found = best_canadian_listing(&quotes, "SHOP", "Shopify Inc.").unwrap();
        assert_eq!(found.symbol, "SHOP.TO");
    }
}
