//! Read-only Robinhood Agentic (MCP) integration.
//!
//! Scope is deliberately **read-only**: we pull the user's positions and account
//! summary to enrich research ("you already hold this"), and we never place,
//! modify, or cancel orders. Two layers enforce that:
//!
//! 1. `is_read_only_tool` — a conservative allow-list. A tool must name a known
//!    read concept (position/account/balance/…) *and* contain no mutating verb
//!    (buy/sell/place/cancel/…). Anything ambiguous is refused. Over-blocking is
//!    the safe direction; we would rather skip a readable field than risk firing
//!    a state-changing tool.
//! 2. We only ever call tools that pass that gate.
//!
//! The JSON normalizers are intentionally schema-tolerant: Robinhood's exact tool
//! output isn't pinned here, so we look for the common key spellings and parse
//! numbers whether they arrive as numbers or strings.

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::mcp::{McpClient, ToolInfo};
use crate::model::{AccountSummary, Portfolio, Position};

/// Robinhood's Agentic trading MCP endpoint (Streamable HTTP).
pub const ENDPOINT: &str = "https://agent.robinhood.com/mcp/trading";

/// Whether a tool is safe to call in read-only mode. Deny wins over allow.
pub fn is_read_only_tool(name: &str) -> bool {
    let n = name.to_ascii_lowercase();

    // Any hint of a state change disqualifies the tool outright. Substring match
    // is intentional: "place_order", "cancel_orders", "buying"/"buy", "sell" all
    // trip it. This can over-block (e.g. "buying_power"), which is acceptable —
    // we never want a false negative here.
    const MUTATING: &[&str] = &[
        "place", "submit", "create", "cancel", "buy", "sell", "trade", "order",
        "modify", "update", "delete", "close", "execute", "transfer", "deposit",
        "withdraw", "sign", "approve", "set_", "enable", "disable", "liquidat",
    ];
    if MUTATING.iter().any(|m| n.contains(m)) {
        return false;
    }

    // Must positively name a read concept we understand.
    const READ: &[&str] = &[
        "position", "holding", "portfolio", "account", "balance", "equity",
        "quote", "price", "history", "summary", "watchlist", "instrument",
        "fundamental", "dividend", "list", "get", "read", "fetch", "view",
    ];
    READ.iter().any(|r| n.contains(r))
}

/// A tool is safe to call read-only when its name passes the allow-list AND its
/// server-supplied annotations don't mark it destructive or non-read-only.
fn is_safe_read_tool(t: &ToolInfo) -> bool {
    if !is_read_only_tool(&t.name) {
        return false;
    }
    match &t.annotations {
        Some(a) if a.destructive_hint == Some(true) || a.read_only_hint == Some(false) => false,
        _ => true,
    }
}

/// Robinhood scopes its reads to an account; tools name that argument with one of
/// these spellings. Returns the one this tool requires, if any.
fn account_arg_key(t: &ToolInfo) -> Option<&'static str> {
    ["account_number", "accountNumber", "account_id", "accountId", "account"]
        .into_iter()
        .find(|k| t.requires(k))
}

/// Build the argument set(s) needed to call a read tool, given the account
/// numbers we discovered. Returns one call per account when the tool is
/// account-scoped, a single empty-args call when it needs nothing, or an error
/// describing the required args we can't satisfy.
fn call_args_for(t: &ToolInfo, account_numbers: &[String]) -> Result<Vec<Value>, String> {
    let required = t.required_params();
    if required.is_empty() {
        return Ok(vec![json!({})]);
    }
    // The only required argument we know how to provide is the account id.
    if let Some(key) = account_arg_key(t) {
        let others: Vec<&str> = required
            .iter()
            .map(String::as_str)
            .filter(|p| *p != key)
            .collect();
        if !others.is_empty() {
            return Err(format!("needs {}", required.join("+")));
        }
        if account_numbers.is_empty() {
            return Err(format!("needs {key}, no account number discovered"));
        }
        return Ok(account_numbers
            .iter()
            .map(|n| json!({ key: n }))
            .collect());
    }
    Err(format!("needs {}", required.join("+")))
}

/// All read-only tools we can actually call (given the account numbers in hand)
/// whose name matches any of `nouns`, in tools-list order.
fn callable_matching<'a>(
    tools: &'a [ToolInfo],
    nouns: &[&str],
    account_numbers: &[String],
) -> Vec<&'a ToolInfo> {
    tools
        .iter()
        .filter(|t| is_safe_read_tool(t) && call_args_for(t, account_numbers).is_ok())
        .filter(|t| {
            let n = t.name.to_ascii_lowercase();
            nouns.iter().any(|noun| n.contains(noun))
        })
        .collect()
}

/// Pick the first read-only tool matching a noun that takes no required args, so
/// it can be called to bootstrap (e.g. to discover account numbers).
fn pick_no_arg_tool<'a>(tools: &'a [ToolInfo], nouns: &[&str]) -> Option<&'a ToolInfo> {
    for noun in nouns {
        if let Some(t) = tools
            .iter()
            .filter(|t| is_safe_read_tool(t) && t.required_params().is_empty())
            .find(|t| t.name.to_ascii_lowercase().contains(noun))
        {
            return Some(t);
        }
    }
    None
}

/// Deep-scan any JSON for account-number-like fields, preserving order and
/// de-duplicating. Robinhood returns these from its no-arg accounts read.
fn collect_account_numbers(v: &Value) -> Vec<String> {
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(map) => {
                for (k, val) in map {
                    let kl = k.to_ascii_lowercase();
                    if kl == "account_number" || kl == "accountnumber" || kl == "account_id" {
                        let found = match val {
                            Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
                            Value::Number(n) => Some(n.to_string()),
                            _ => None,
                        };
                        if let Some(s) = found {
                            if !out.contains(&s) {
                                out.push(s);
                            }
                        }
                    }
                    walk(val, out);
                }
            }
            Value::Array(arr) => arr.iter().for_each(|item| walk(item, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(v, &mut out);
    out
}

/// One-line description of a tool for diagnostics: name plus any required args.
fn describe_tool(t: &ToolInfo) -> String {
    let req = t.required_params();
    if req.is_empty() {
        t.name.clone()
    } else {
        format!("{}(needs {})", t.name, req.join("+"))
    }
}

/// Describe the *shape* of a tool response — its keys only, never its values —
/// so a parsing miss can be diagnosed without leaking holdings or balances.
fn shape_of(v: &Value) -> String {
    shape_of_depth(v, 4)
}

/// Structural description of a JSON value to `depth` levels: object keys, array
/// lengths, and scalar kinds only — never any value. Objects are capped to a
/// handful of keys so the string stays bounded.
fn shape_of_depth(v: &Value, depth: u8) -> String {
    match v {
        Value::Object(map) if depth > 0 => {
            let mut parts: Vec<String> = map
                .iter()
                .take(10)
                .map(|(k, val)| format!("{k}:{}", shape_of_depth(val, depth - 1)))
                .collect();
            if map.len() > 10 {
                parts.push("…".into());
            }
            format!("object{{{}}}", parts.join(","))
        }
        Value::Object(map) => format!("object{{{}}}", map.keys().take(10).cloned().collect::<Vec<_>>().join(",")),
        Value::Array(arr) => match arr.first() {
            Some(first) if depth > 0 => format!("array[{}] of {}", arr.len(), shape_of_depth(first, depth - 1)),
            _ => format!("array[{}]", arr.len()),
        },
        Value::String(_) => "string".into(),
        Value::Null => "null".into(),
        _ => "scalar".into(),
    }
}

/// Like `shape_of`, but for a *string* payload it appends a short, number-redacted
/// preview so an unrecognized text format (CSV, NDJSON, …) can be identified
/// without leaking any price, quantity, or balance.
fn shape_preview(v: &Value) -> String {
    match v {
        Value::String(s) => {
            let head: String = s.trim().chars().take(140).collect();
            format!("string «{}»", redact_nums(&head))
        }
        _ => shape_of(v),
    }
}

/// Reduce a diagnostic string to something safe to surface in the UI: every run
/// of digits collapses to a single `#`, so no price, quantity, balance, or
/// account number can survive even if an upstream error echoes a raw response
/// body. Text (tickers, error reasons) is preserved; output is length-bounded.
fn redact_nums(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_digits = false;
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            if !in_digits {
                out.push('#');
                in_digits = true;
            }
        } else {
            out.push(ch);
            in_digits = false;
        }
        if out.len() >= 160 {
            out.push('…');
            break;
        }
    }
    out
}

/// Reduce a `tools/call` result to the JSON we can parse: prefer the typed
/// `structuredContent`, otherwise JSON-decode the joined text content, otherwise
/// fall back to the raw text as a string.
fn tool_result_value(result: &Value) -> Value {
    if let Some(sc) = result.get("structuredContent") {
        if !sc.is_null() {
            return reparse_json_string(sc);
        }
    }
    if let Some(items) = result.get("content").and_then(Value::as_array) {
        let mut text = String::new();
        for item in items {
            if item.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(t) = item.get("text").and_then(Value::as_str) {
                    text.push_str(t);
                }
            }
        }
        if let Ok(v) = serde_json::from_str::<Value>(text.trim()) {
            return v;
        }
        if !text.is_empty() {
            return Value::String(text);
        }
    }
    result.clone()
}

/// Some servers return JSON as a *string* (a stringified array/object) under
/// `structuredContent`. If `v` is such a string, decode it; otherwise return it
/// unchanged.
fn reparse_json_string(v: &Value) -> Value {
    if let Some(s) = v.as_str() {
        let t = s.trim();
        if t.starts_with('{') || t.starts_with('[') {
            if let Ok(parsed) = serde_json::from_str::<Value>(t) {
                return parsed;
            }
        }
    }
    v.clone()
}

/// Descend through a wrapper envelope like `{ "data": … }` or `{ "results": … }`
/// to the object/array that actually holds the records, so a `{data, guide}`-style
/// response is parsed by the same logic as a bare payload. Reparses any
/// stringified-JSON it meets on the way down. Bounded to a few hops.
fn unwrap_envelope(v: &Value) -> Value {
    const ENVELOPE_KEYS: &[&str] = &["data", "results", "result", "payload", "response", "quotes"];
    let mut cur = reparse_json_string(v);
    for _ in 0..4 {
        let Some(obj) = cur.as_object() else { break };
        let mut advanced = false;
        for k in ENVELOPE_KEYS {
            if let Some(raw) = obj.get(*k) {
                let inner = reparse_json_string(raw);
                if inner.is_object() || inner.is_array() {
                    cur = inner;
                    advanced = true;
                    break;
                }
            }
        }
        if !advanced {
            break;
        }
    }
    cur
}

/// Find the array of records inside an arbitrarily-shaped tool result. Handles a
/// bare array, an object that wraps the array under a common key, or — as a last
/// resort — the first nested array of objects anywhere in the payload.
fn find_records(v: &Value) -> Vec<Value> {
    if let Some(arr) = v.as_array() {
        return arr.clone();
    }
    if let Some(obj) = v.as_object() {
        const KEYS: &[&str] = &[
            "positions", "holdings", "results", "data", "items", "equities",
            "equity_positions", "option_positions",
        ];
        for k in KEYS {
            if let Some(arr) = obj.get(*k).and_then(Value::as_array) {
                return arr.clone();
            }
        }
        // Single object that itself looks like a position.
        if obj.contains_key("symbol") || obj.contains_key("ticker") {
            return vec![v.clone()];
        }
        // Fallback: the first nested array of objects found anywhere below.
        if let Some(arr) = first_object_array(v) {
            return arr;
        }
    }
    Vec::new()
}

/// Depth-first search for the first array whose elements are objects.
fn first_object_array(v: &Value) -> Option<Vec<Value>> {
    match v {
        Value::Array(arr) if arr.first().is_some_and(Value::is_object) => Some(arr.clone()),
        Value::Object(obj) => obj.values().find_map(first_object_array),
        Value::Array(arr) => arr.iter().find_map(first_object_array),
        _ => None,
    }
}

/// Guard against mistaking a non-symbol field (e.g. an `instrument` URL) for a
/// ticker: real tickers are short and alphanumeric.
fn looks_like_ticker(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 8
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// Read a numeric field that may be encoded as a JSON number or a numeric string.
fn num(obj: &Value, keys: &[&str]) -> Option<f64> {
    for k in keys {
        match obj.get(*k) {
            Some(Value::Number(n)) => return n.as_f64(),
            Some(Value::String(s)) => {
                if let Ok(f) = s.trim().trim_start_matches('$').replace(',', "").parse::<f64>() {
                    return Some(f);
                }
            }
            _ => {}
        }
    }
    None
}

fn text(obj: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = obj.get(*k).and_then(Value::as_str) {
            if !s.trim().is_empty() {
                return Some(s.trim().to_string());
            }
        }
    }
    None
}

/// Normalize an arbitrary positions payload into typed `Position`s. Tolerant of
/// the common key spellings across brokerage APIs.
pub fn parse_positions(v: &Value) -> Vec<Position> {
    let mut out = Vec::new();
    for rec in find_records(v) {
        let ticker = text(&rec, &["symbol", "ticker", "instrument_symbol", "chain_symbol", "instrument"])
            .map(|s| s.to_ascii_uppercase())
            .filter(|s| looks_like_ticker(s))
            .unwrap_or_default();
        if ticker.is_empty() {
            continue;
        }
        let quantity = num(&rec, &["quantity", "shares", "qty", "quantity_held", "units"]).unwrap_or(0.0);
        out.push(Position {
            ticker,
            name: text(&rec, &["name", "company", "instrument_name", "long_name", "description"]),
            quantity,
            market_value: num(&rec, &["market_value", "value", "equity", "total_value", "market_val"]),
            average_buy_price: num(
                &rec,
                &["average_buy_price", "avg_cost", "average_price", "cost_basis_per_share"],
            ),
            unrealized_plpc: num(&rec, &["unrealized_plpc", "total_return_pct", "gain_loss_pct"]),
            currency: text(&rec, &["currency"]).unwrap_or_else(|| "USD".to_string()),
            ..Default::default()
        });
    }
    out
}

/// Normalize an arbitrary account payload into an `AccountSummary`, if any of the
/// money fields are present.
pub fn parse_account(v: &Value) -> Option<AccountSummary> {
    // Unwrap a single-element wrapper array, a `{ "account": {…} }` shape, or a
    // `{ "results": [{…}] }` list (Robinhood's get_accounts/get_portfolio shape).
    let obj = if let Some(a) = v.as_array().and_then(|a| a.first()) {
        a.clone()
    } else if let Some(inner) = v.get("account") {
        inner.clone()
    } else if let Some(first) = ["results", "data", "accounts"]
        .iter()
        .find_map(|k| v.get(*k).and_then(Value::as_array))
        .and_then(|a| a.first())
    {
        first.clone()
    } else {
        v.clone()
    };

    let portfolio_value = num(&obj, &["portfolio_value", "total_equity", "equity", "total_value", "market_value"]);
    let buying_power = num(&obj, &["buying_power", "buyingpower", "purchasing_power"]);
    let cash = num(&obj, &["cash", "cash_balance", "uninvested_cash", "available_cash"]);

    if portfolio_value.is_none() && buying_power.is_none() && cash.is_none() {
        return None;
    }
    Some(AccountSummary {
        portfolio_value,
        buying_power,
        cash,
        currency: text(&obj, &["currency"]).unwrap_or_else(|| "USD".to_string()),
    })
}

/// True when a payload carries no content at all (null, empty array/object, or a
/// wrapper whose only values are empty arrays) — a legitimately empty result we
/// should not flag as an unparsed shape.
fn is_empty_payload(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => {
            o.is_empty()
                || o.values().all(|val| {
                    matches!(val, Value::Null) || val.as_array().is_some_and(|a| a.is_empty())
                })
        }
        _ => false,
    }
}

/// Merge a freshly-parsed account summary into any existing one, filling only the
/// fields that are still missing so values from different tools can combine.
fn merge_account(existing: Option<AccountSummary>, next: AccountSummary) -> AccountSummary {
    match existing {
        None => next,
        Some(mut acc) => {
            acc.portfolio_value = acc.portfolio_value.or(next.portfolio_value);
            acc.buying_power = acc.buying_power.or(next.buying_power);
            acc.cash = acc.cash.or(next.cash);
            if acc.currency == "USD" && next.currency != "USD" {
                acc.currency = next.currency;
            }
            acc
        }
    }
}

/// A connected, read-only Robinhood session built on the MCP client.
pub struct RobinhoodClient {
    mcp: McpClient,
}

/// Most intraday points we keep for a row's sparkline (newest retained).
const SPARK_MAX_POINTS: usize = 48;

/// Symbols per `get_equity_historicals` call. The server rejects large batches
/// ("too many symbols … split your request into smaller batches"), so we keep
/// each request conservatively small and merge the results.
const HISTORICAL_BATCH: usize = 5;

/// A lightweight quote: enough to value a holding and show today's change.
#[derive(Debug, Clone, Default)]
struct Quote {
    price: Option<f64>,
    change_pct: Option<f64>,
}

/// First read-only tool whose lowercased name contains *all* of `needles`
/// (e.g. `["equit", "quote"]` → `get_equity_quotes`).
fn find_named<'a>(tools: &'a [ToolInfo], needles: &[&str]) -> Option<&'a ToolInfo> {
    tools.iter().find(|t| {
        let n = t.name.to_ascii_lowercase();
        is_safe_read_tool(t) && needles.iter().all(|needle| n.contains(needle))
    })
}

/// Whether a tool's input schema advertises a property named `key`.
fn schema_has_prop(t: &ToolInfo, key: &str) -> bool {
    t.input_schema
        .as_ref()
        .and_then(|s| s.get("properties"))
        .and_then(Value::as_object)
        .is_some_and(|props| props.contains_key(key))
}

/// Format the `symbols` argument the way a tool's schema expects it: most servers
/// want an array of strings, some want a comma-separated string.
fn symbols_value(t: &ToolInfo, symbols: &[String]) -> Value {
    let wants_string = t
        .input_schema
        .as_ref()
        .and_then(|s| s.get("properties"))
        .and_then(|p| p.get("symbols"))
        .and_then(|s| s.get("type"))
        .and_then(Value::as_str)
        == Some("string");
    if wants_string {
        json!(symbols.join(","))
    } else {
        json!(symbols)
    }
}

/// The object that actually holds quote fields. Some servers wrap it under a
/// `quote` key — `{ "quote": {…} }` — so unwrap that one level when present.
fn quote_source(rec: &Value) -> &Value {
    rec.get("quote").filter(|q| q.is_object()).unwrap_or(rec)
}

/// Extract a `Quote` (price + today's change) from one quote record.
fn quote_fields(rec: &Value) -> Quote {
    let rec = quote_source(rec);
    let price = num(
        rec,
        &[
            "last_trade_price", "last_price", "last_extended_hours_trade_price",
            "mark_price", "price", "ask_price",
        ],
    );
    let prev_close = num(
        rec,
        &["previous_close", "adjusted_previous_close", "prev_close", "previousClose"],
    );
    let change_pct = num(
        rec,
        &["change_percent_today", "percent_change", "change_pct", "todays_change_pct"],
    )
    .or_else(|| match (price, prev_close) {
        (Some(p), Some(pc)) if pc != 0.0 => Some((p - pc) / pc * 100.0),
        _ => None,
    });
    Quote { price, change_pct }
}

/// Parse a quotes payload into `ticker → quote`. Handles a list/wrapper of quote
/// objects *and* a `{ "AAPL": {…} }` symbol-keyed map, tolerant of key spellings.
fn parse_quotes(v: &Value) -> HashMap<String, Quote> {
    let v = unwrap_envelope(v);
    let mut out = HashMap::new();
    for rec in find_records(&v) {
        if let Some(sym) = text(quote_source(&rec), &["symbol", "ticker", "instrument_symbol"])
            .or_else(|| text(&rec, &["symbol", "ticker", "instrument_symbol"]))
            .map(|s| s.to_ascii_uppercase())
            .filter(|s| looks_like_ticker(s))
        {
            out.insert(sym, quote_fields(&rec));
        }
    }
    if !out.is_empty() {
        return out;
    }
    // Symbol-keyed map shape: keys are tickers, values are quote objects.
    if let Some(map) = v.as_object() {
        for (k, rec) in map {
            let sym = k.to_ascii_uppercase();
            if rec.is_object() && looks_like_ticker(&sym) {
                out.insert(sym, quote_fields(rec));
            }
        }
    }
    out
}

/// Fallback for quote responses whose records carry no usable symbol field and
/// are instead returned **positionally**, in the same order the symbols were
/// requested (Robinhood's `results` array, which keeps a slot — possibly null —
/// per requested symbol). Only zips when the counts line up, so misaligned
/// responses are left for the keyed parser rather than mislabeled.
fn parse_quotes_positional(v: &Value, symbols: &[String]) -> HashMap<String, Quote> {
    let v = unwrap_envelope(v);
    let records = find_records(&v);
    let mut out = HashMap::new();
    if records.is_empty() || records.len() != symbols.len() {
        return out;
    }
    for (sym, rec) in symbols.iter().zip(records.iter()) {
        if rec.is_object() {
            let q = quote_fields(rec);
            // Require a price: if the element uses an unrecognized price key we'd
            // rather report nothing (and surface the shape) than label a holding
            // with a blank value and no diagnostic.
            if q.price.is_some() {
                out.insert(sym.to_ascii_uppercase(), q);
            }
        }
    }
    out
}

/// Pull a close-price series out of one symbol's record (a bare array of points,
/// or an object wrapping the points under a common key), trimmed to its tail.
fn extract_series(v: &Value) -> Vec<f64> {
    const SERIES_KEYS: &[&str] =
        &["bars", "historicals", "data_points", "points", "history", "series", "candles", "results"];
    let arr = if let Some(a) = v.as_array() {
        a.clone()
    } else {
        SERIES_KEYS
            .iter()
            .find_map(|k| v.get(*k).and_then(Value::as_array))
            .cloned()
            .unwrap_or_default()
    };
    let mut prices: Vec<f64> = arr
        .iter()
        .filter_map(|p| num(p, &["close_price", "close", "price", "last_trade_price", "open_price"]))
        .collect();
    if prices.len() > SPARK_MAX_POINTS {
        prices = prices.split_off(prices.len() - SPARK_MAX_POINTS);
    }
    prices
}

/// Parse a historicals payload into `ticker → close-price series` (oldest→newest).
/// Handles a `{ "AAPL": [...] }` symbol-keyed map as well as a list/wrapper of
/// per-symbol records.
fn parse_historicals(v: &Value) -> HashMap<String, Vec<f64>> {
    let v = unwrap_envelope(v);
    let mut out = HashMap::new();
    // Symbol-keyed map: { "AAPL": [points…] } or { "AAPL": { historicals: [...] } }.
    if let Some(map) = v.as_object() {
        let keyed_by_symbol = !map.contains_key("results")
            && !map.contains_key("symbol")
            && map
                .keys()
                .any(|k| looks_like_ticker(&k.to_ascii_uppercase()));
        if keyed_by_symbol {
            for (k, val) in map {
                let sym = k.to_ascii_uppercase();
                if !looks_like_ticker(&sym) {
                    continue;
                }
                let series = extract_series(val);
                if !series.is_empty() {
                    out.insert(sym, series);
                }
            }
            if !out.is_empty() {
                return out;
            }
        }
    }
    for rec in find_records(&v) {
        if let Some(sym) = text(&rec, &["symbol", "ticker"])
            .map(|s| s.to_ascii_uppercase())
            .filter(|s| looks_like_ticker(s))
        {
            let series = extract_series(&rec);
            if !series.is_empty() {
                out.insert(sym, series);
            }
        }
    }
    out
}

impl RobinhoodClient {
    pub fn new(http: reqwest::Client, token: impl Into<String>) -> Self {
        Self {
            mcp: McpClient::new(http, ENDPOINT, token),
        }
    }

    /// Handshake, discover the read-only tools, and pull a portfolio snapshot.
    pub async fn fetch_portfolio(&self) -> AppResult<Portfolio> {
        self.mcp.initialize().await?;
        let tools = self.mcp.list_tools().await?;

        let mut tools_used = Vec::new();
        let mut account: Option<AccountSummary> = None;

        // Phase 1 — bootstrap. Robinhood scopes position/account reads to an
        // account_number, so first call a no-argument account/accounts tool to
        // learn the account number(s) (and opportunistically the cash/equity
        // summary, which the same payload often carries).
        let mut account_numbers: Vec<String> = Vec::new();
        if let Some(t) = pick_no_arg_tool(&tools, &["accounts", "account", "balance", "portfolio"]) {
            if let Ok(res) = self.mcp.call_tool(&t.name, json!({})).await {
                let val = tool_result_value(&res);
                account_numbers = collect_account_numbers(&val);
                if let Some(a) = parse_account(&val) {
                    account = Some(a);
                    tools_used.push(t.name.clone());
                }
            }
        }

        // Phase 2 — positions. Iterate *every* callable position tool (equity and
        // option), scoped per account_number when required, and capture the shape
        // of anything that doesn't parse so a miss can be diagnosed safely.
        let mut positions: Vec<Position> = Vec::new();
        let mut diagnostics: Vec<String> = Vec::new();
        for t in callable_matching(&tools, &["position", "holding"], &account_numbers) {
            let Ok(calls) = call_args_for(t, &account_numbers) else { continue };
            let mut got = false;
            for args in calls {
                match self.mcp.call_tool(&t.name, args).await {
                    Ok(res) => {
                        let val = tool_result_value(&res);
                        let parsed = parse_positions(&val);
                        if parsed.is_empty() && !is_empty_payload(&val) {
                            diagnostics.push(format!("{} → {}", t.name, shape_of(&val)));
                        }
                        if !parsed.is_empty() {
                            positions.extend(parsed);
                            got = true;
                        }
                    }
                    Err(e) => diagnostics.push(format!("{} errored: {}", t.name, redact_nums(&e.to_string()))),
                }
            }
            if got {
                tools_used.push(t.name.clone());
            }
        }

        // Phase 3 — account summary. Merge fields across every callable
        // balance/portfolio/account tool so e.g. portfolio_value (get_portfolio)
        // and cash/buying_power (get_accounts) can come from different calls.
        for t in callable_matching(&tools, &["portfolio", "balance", "account"], &account_numbers) {
            let Ok(calls) = call_args_for(t, &account_numbers) else { continue };
            let mut used = false;
            for args in calls {
                match self.mcp.call_tool(&t.name, args).await {
                    Ok(res) => {
                        let val = tool_result_value(&res);
                        if let Some(a) = parse_account(&val) {
                            account = Some(merge_account(account.take(), a));
                            used = true;
                        } else if account.is_none() && !is_empty_payload(&val) {
                            diagnostics.push(format!("{} → {}", t.name, shape_of(&val)));
                        }
                    }
                    Err(e) => diagnostics.push(format!("{} errored: {}", t.name, redact_nums(&e.to_string()))),
                }
            }
            if used && !tools_used.contains(&t.name) {
                tools_used.push(t.name.clone());
            }
        }

        if positions.is_empty() && account.is_none() {
            // Surface what the server actually offered (names + required args) and
            // the shape of any unparsed responses (keys only — never values) so the
            // parser can be tuned to Robinhood's real API.
            let available = tools
                .iter()
                .map(describe_tool)
                .collect::<Vec<_>>()
                .join(", ");
            let hint = if account_numbers.is_empty() {
                " Couldn't discover an account number from any no-argument tool.".to_string()
            } else {
                format!(" Found {} account number(s).", account_numbers.len())
            };
            let shapes = if diagnostics.is_empty() {
                String::new()
            } else {
                format!(" Unparsed responses: [{}].", diagnostics.join("; "))
            };
            return Err(AppError::Robinhood(format!(
                "Connected, but couldn't read positions or account.{hint}{shapes} \
                 Tools the server exposed: [{available}]."
            )));
        }

        // Phase 4 — enrich held positions with a live value and a today sparkline.
        // Best-effort and read-only: any failure here leaves positions intact.
        let mut debug: Vec<String> = Vec::new();
        let symbols: Vec<String> = positions
            .iter()
            .filter(|p| p.quantity > 0.0 && !p.ticker.is_empty())
            .map(|p| p.ticker.clone())
            .collect();
        if !symbols.is_empty() {
            let notes = self
                .enrich_positions(&tools, &symbols, &mut positions, &mut tools_used)
                .await;
            debug.extend(notes);
        }

        Ok(Portfolio {
            positions,
            account,
            as_of: chrono::Utc::now().to_rfc3339(),
            tools_used,
            debug,
        })
    }

    /// Fill each held position's price/value and today's change from a read-only
    /// equity quote, and attach a recent intraday series for its sparkline from a
    /// read-only historicals call. Entirely best-effort; returns short diagnostic
    /// notes (keys/shapes only, never values) for any miss.
    async fn enrich_positions(
        &self,
        tools: &[ToolInfo],
        symbols: &[String],
        positions: &mut [Position],
        tools_used: &mut Vec<String>,
    ) -> Vec<String> {
        let mut debug = Vec::new();

        // Quotes → price, today's %, and a computed market value.
        match find_named(tools, &["equit", "quote"]) {
            None => debug.push("no read-only equity-quote tool exposed".to_string()),
            Some(t) => {
                let quotes = self.fetch_quotes(t, symbols, &mut debug).await;
                if quotes.is_empty() {
                    debug.push(format!("{} returned no usable quotes", t.name));
                } else {
                    let mut priced = 0usize;
                    for p in positions.iter_mut() {
                        if let Some(q) = quotes.get(&p.ticker) {
                            if let Some(price) = q.price {
                                p.price = Some(price);
                                if p.market_value.is_none() {
                                    p.market_value = Some(price * p.quantity);
                                }
                                priced += 1;
                            }
                            p.change_pct = q.change_pct.or(p.change_pct);
                        }
                    }
                    if priced > 0 {
                        tools_used.push(t.name.clone());
                    }
                    if priced < symbols.len() {
                        debug.push(format!("priced {}/{} holdings", priced, symbols.len()));
                    }
                }
            }
        }

        // Historicals → an intraday sparkline series. Chunked because the server
        // caps how many symbols one call may request; the window starts a few days
        // back so the most recent session is captured even on a Monday or after a
        // holiday, and each series is trimmed to its most-recent tail.
        if let Some(t) = find_named(tools, &["equit", "historical"]) {
            let series = self.fetch_historicals(t, symbols, &mut debug).await;
            if !series.is_empty() {
                for p in positions.iter_mut() {
                    if let Some(s) = series.get(&p.ticker) {
                        p.spark = s.clone();
                        if p.change_pct.is_none() {
                            if let (Some(first), Some(last)) = (s.first(), s.last()) {
                                if *first != 0.0 {
                                    p.change_pct = Some((last - first) / first * 100.0);
                                }
                            }
                        }
                    }
                }
                tools_used.push(t.name.clone());
            }
        }

        debug.truncate(8);
        debug
    }

    /// Fetch quotes resiliently: one call for the whole batch, and on a *server
    /// error* (often a single unrecognized symbol rejecting the request) retry in
    /// small chunks so the rest of the holdings still price. Records the response
    /// shape (keys only) when a well-formed reply simply doesn't parse.
    async fn fetch_quotes(
        &self,
        t: &ToolInfo,
        symbols: &[String],
        debug: &mut Vec<String>,
    ) -> HashMap<String, Quote> {
        let mut out = HashMap::new();
        match self.mcp.call_tool(&t.name, json!({ "symbols": symbols_value(t, symbols) })).await {
            Ok(res) => {
                let val = tool_result_value(&res);
                let mut quotes = parse_quotes(&val);
                if quotes.is_empty() {
                    quotes = parse_quotes_positional(&val, symbols);
                }
                if quotes.is_empty() && !is_empty_payload(&val) {
                    debug.push(format!("{} → {}", t.name, shape_preview(&val)));
                }
                out.extend(quotes);
            }
            Err(_) => {
                // The whole batch was rejected. Retry in small chunks so one bad
                // symbol only costs its chunk, not the entire portfolio.
                for chunk in symbols.chunks(8) {
                    match self.mcp.call_tool(&t.name, json!({ "symbols": symbols_value(t, chunk) })).await {
                        Ok(res) => {
                            let val = tool_result_value(&res);
                            let mut q = parse_quotes(&val);
                            if q.is_empty() {
                                q = parse_quotes_positional(&val, chunk);
                            }
                            out.extend(q);
                        }
                        Err(e) => debug.push(format!("{} rejected [{}]: {}", t.name, chunk.join(","), redact_nums(&e.to_string()))),
                    }
                }
            }
        }
        out
    }

    /// Fetch intraday historicals in small batches because the server caps how
    /// many symbols a single call may request. Merges every batch into one
    /// `ticker → series` map; records at most one diagnostic so a systemic
    /// failure doesn't flood the debug channel.
    async fn fetch_historicals(
        &self,
        t: &ToolInfo,
        symbols: &[String],
        debug: &mut Vec<String>,
    ) -> HashMap<String, Vec<f64>> {
        let mut out = HashMap::new();
        let start = chrono::Utc::now() - chrono::Duration::days(4);
        let start_str = start.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let mut noted = false;
        for chunk in symbols.chunks(HISTORICAL_BATCH) {
            let mut args = serde_json::Map::new();
            args.insert("symbols".into(), symbols_value(t, chunk));
            args.insert("start_time".into(), json!(start_str));
            for (k, v) in [("interval", "5minute"), ("bounds", "regular"), ("span", "day")] {
                if schema_has_prop(t, k) {
                    args.insert(k.into(), json!(v));
                }
            }
            match self.mcp.call_tool(&t.name, Value::Object(args)).await {
                Err(e) => {
                    if !noted {
                        debug.push(format!("{}: {}", t.name, redact_nums(&e.to_string())));
                        noted = true;
                    }
                }
                Ok(res) => {
                    let val = tool_result_value(&res);
                    let series = parse_historicals(&val);
                    if series.is_empty() {
                        if !noted && !is_empty_payload(&val) {
                            debug.push(format!("{} → {}", t.name, shape_preview(&val)));
                            noted = true;
                        }
                    } else {
                        out.extend(series);
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::{ToolAnnotations, ToolInfo};

    fn tool(name: &str) -> ToolInfo {
        ToolInfo { name: name.into(), input_schema: None, annotations: None }
    }

    fn tool_annotated(name: &str, read_only: Option<bool>, destructive: Option<bool>) -> ToolInfo {
        ToolInfo {
            name: name.into(),
            input_schema: None,
            annotations: Some(ToolAnnotations {
                read_only_hint: read_only,
                destructive_hint: destructive,
            }),
        }
    }

    fn tool_requiring(name: &str, required: &[&str]) -> ToolInfo {
        ToolInfo {
            name: name.into(),
            input_schema: Some(json!({ "type": "object", "required": required })),
            annotations: None,
        }
    }

    #[test]
    fn read_only_allows_position_and_account_reads() {
        for n in ["list_positions", "get_account", "portfolio_summary", "get_holdings", "account_balance"] {
            assert!(is_read_only_tool(n), "{n} should be allowed");
        }
    }

    #[test]
    fn read_only_blocks_anything_that_could_trade() {
        for n in [
            "place_order", "submit_order", "cancel_order", "buy_stock", "sell_stock",
            "create_order", "execute_trade", "modify_order", "withdraw_funds", "liquidate_position",
        ] {
            assert!(!is_read_only_tool(n), "{n} must be blocked");
        }
    }

    #[test]
    fn read_only_blocks_unknown_tools() {
        // No read noun, no mutating verb — refuse by default.
        assert!(!is_read_only_tool("frobnicate"));
        assert!(!is_read_only_tool("status_ping"));
    }

    #[test]
    fn callable_matching_prefers_positions_then_account() {
        let tools = vec![tool("get_account"), tool("list_positions"), tool("place_order")];
        assert_eq!(
            callable_matching(&tools, &["position", "holding"], &[])[0].name,
            "list_positions"
        );
        assert_eq!(callable_matching(&tools, &["account"], &[])[0].name, "get_account");
        // A mutating tool is never selected even if the noun matches.
        assert!(callable_matching(&vec![tool("place_order")], &["order"], &[]).is_empty());
    }

    #[test]
    fn callable_matching_skips_destructive_annotation() {
        // Name looks like a benign read, but the server flags it destructive.
        let tools = vec![tool_annotated("get_positions", None, Some(true))];
        assert!(callable_matching(&tools, &["position"], &[]).is_empty());
        // Explicitly non-read-only is likewise refused.
        let tools = vec![tool_annotated("portfolio_view", Some(false), None)];
        assert!(callable_matching(&tools, &["portfolio"], &[]).is_empty());
        // A clean read-only annotation is fine.
        let tools = vec![tool_annotated("list_positions", Some(true), Some(false))];
        assert_eq!(
            callable_matching(&tools, &["position"], &[])[0].name,
            "list_positions"
        );
    }

    #[test]
    fn collect_account_numbers_scans_nested_and_dedupes() {
        let v = json!({
            "results": [
                { "account_number": "ABC123", "type": "brokerage" },
                { "account_number": "ABC123" },
                { "accountNumber": "XYZ789" },
            ]
        });
        assert_eq!(collect_account_numbers(&v), vec!["ABC123", "XYZ789"]);
        // Numeric account ids are stringified.
        assert_eq!(collect_account_numbers(&json!({ "account_id": 42 })), vec!["42"]);
    }

    #[test]
    fn call_args_for_handles_no_arg_and_account_scoped_tools() {
        // No required args → a single empty-args call.
        assert_eq!(call_args_for(&tool("get_positions"), &[]).unwrap(), vec![json!({})]);

        // Requires account_number → one call per discovered account, keyed right.
        let t = tool_requiring("get_positions", &["account_number"]);
        let nums = vec!["A1".to_string(), "A2".to_string()];
        assert_eq!(
            call_args_for(&t, &nums).unwrap(),
            vec![json!({ "account_number": "A1" }), json!({ "account_number": "A2" })]
        );

        // Requires account_number but none known → unsatisfiable.
        assert!(call_args_for(&t, &[]).is_err());

        // Requires something we can't provide → unsatisfiable.
        let weird = tool_requiring("get_positions", &["start_date"]);
        assert!(call_args_for(&weird, &nums).is_err());
    }

    #[test]
    fn callable_matching_skips_tools_we_cannot_satisfy() {
        // The positions tool needs an account number; with none known it's skipped.
        let tools = vec![tool_requiring("get_positions", &["account_number"])];
        assert!(callable_matching(&tools, &["position"], &[]).is_empty());
        // Once we have a number, it becomes callable.
        let nums = vec!["A1".to_string()];
        assert_eq!(
            callable_matching(&tools, &["position"], &nums)[0].name,
            "get_positions"
        );
    }

    #[test]
    fn pick_no_arg_tool_requires_empty_schema() {
        let tools = vec![
            tool_requiring("get_account", &["account_number"]),
            tool("list_accounts"),
        ];
        // Skips the account_number-scoped tool, picks the no-arg lister.
        assert_eq!(pick_no_arg_tool(&tools, &["account"]).unwrap().name, "list_accounts");
    }

    #[test]
    fn parse_positions_from_bare_array_with_mixed_number_types() {
        let v = json!([
            { "symbol": "aapl", "quantity": 10, "market_value": "1850.50" },
            { "ticker": "NVDA", "shares": "5", "value": 600.0, "name": "NVIDIA" }
        ]);
        let pos = parse_positions(&v);
        assert_eq!(pos.len(), 2);
        assert_eq!(pos[0].ticker, "AAPL");
        assert_eq!(pos[0].quantity, 10.0);
        assert_eq!(pos[0].market_value, Some(1850.50));
        assert_eq!(pos[1].ticker, "NVDA");
        assert_eq!(pos[1].quantity, 5.0);
        assert_eq!(pos[1].name.as_deref(), Some("NVIDIA"));
    }

    #[test]
    fn parse_positions_from_wrapped_object() {
        let v = json!({ "positions": [{ "symbol": "MSFT", "quantity": 3 }] });
        let pos = parse_positions(&v);
        assert_eq!(pos.len(), 1);
        assert_eq!(pos[0].ticker, "MSFT");
    }

    #[test]
    fn parse_positions_skips_records_without_ticker() {
        let v = json!([{ "quantity": 10 }, { "symbol": "TSLA", "quantity": 1 }]);
        assert_eq!(parse_positions(&v).len(), 1);
    }

    #[test]
    fn owned_tickers_only_counts_held_shares() {
        let p = Portfolio {
            positions: vec![
                Position { ticker: "AAPL".into(), quantity: 10.0, ..Default::default() },
                Position { ticker: "NVDA".into(), quantity: 0.0, ..Default::default() },
            ],
            ..Default::default()
        };
        let owned = p.owned_tickers();
        assert!(owned.contains("AAPL"));
        assert!(!owned.contains("NVDA"));
    }

    #[test]
    fn parse_account_reads_money_fields() {
        let v = json!({ "portfolio_value": "12500.00", "buying_power": 300.0, "currency": "USD" });
        let acct = parse_account(&v).expect("should parse");
        assert_eq!(acct.portfolio_value, Some(12500.0));
        assert_eq!(acct.buying_power, Some(300.0));
    }

    #[test]
    fn parse_account_none_when_no_money_fields() {
        assert!(parse_account(&json!({ "id": "abc" })).is_none());
    }

    #[test]
    fn parse_account_unwraps_results_wrapper() {
        // Robinhood's get_accounts shape: a `results` list of account objects.
        let v = json!({ "results": [{ "account_number": "X1", "buying_power": "42.50" }] });
        let acct = parse_account(&v).expect("should parse from results wrapper");
        assert_eq!(acct.buying_power, Some(42.50));
    }

    #[test]
    fn find_records_recurses_into_unknown_wrapper() {
        // No known wrapper key, but a nested array of objects exists.
        let v = json!({ "payload": { "rows": [{ "symbol": "AAPL", "quantity": 1 }] } });
        let pos = parse_positions(&v);
        assert_eq!(pos.len(), 1);
        assert_eq!(pos[0].ticker, "AAPL");
    }

    #[test]
    fn parse_positions_ignores_instrument_url_as_ticker() {
        // Robinhood raw positions reference an instrument URL, not a symbol —
        // that URL must not become a garbage ticker.
        let v = json!([{ "instrument": "https://api.robinhood.com/instruments/abc-123/", "quantity": 5 }]);
        assert!(parse_positions(&v).is_empty());
        // But a real short symbol in the same field is accepted.
        let v = json!([{ "instrument": "AAPL", "quantity": 5 }]);
        assert_eq!(parse_positions(&v)[0].ticker, "AAPL");
    }

    #[test]
    fn looks_like_ticker_rejects_urls_and_long_strings() {
        assert!(looks_like_ticker("AAPL"));
        assert!(looks_like_ticker("BRK.B"));
        assert!(!looks_like_ticker("https://example.com/x"));
        assert!(!looks_like_ticker("THIS_IS_TOO_LONG"));
        assert!(!looks_like_ticker(""));
    }

    #[test]
    fn shape_of_reports_keys_not_values() {
        let v = json!({ "buying_power": "999.99", "cash": "1.00" });
        let s = shape_of(&v);
        assert!(s.contains("buying_power") && s.contains("cash"));
        // Never leaks the actual balances.
        assert!(!s.contains("999.99"));

        let arr = json!([{ "symbol": "AAPL", "quantity": 3 }]);
        let s = shape_of(&arr);
        assert!(s.starts_with("array[1] of object"));
        assert!(s.contains("symbol") && !s.contains("AAPL"));
    }

    #[test]
    fn redact_nums_strips_every_numeric_value_but_keeps_text() {
        // A raw error body echoing prices/balances/account numbers must not
        // survive into the UI-visible debug channel.
        let raw = "Robinhood MCP returned 400: {\"last_price\":172.99,\"qty\":3,\"acct\":\"123456789\"}";
        let safe = redact_nums(raw);
        for leak in ["172.99", "172", "99", "123456789", "400"] {
            assert!(!safe.contains(leak), "leaked {leak} in {safe}");
        }
        // Tickers and the textual reason are preserved.
        let r2 = redact_nums("get_equity_quotes rejected [GEMI,AAPL]: invalid symbol GEMI");
        assert!(r2.contains("GEMI") && r2.contains("invalid symbol"));
        // Length-bounded.
        assert!(redact_nums(&"x".repeat(500)).chars().count() <= 161);
    }

    #[test]
    fn merge_account_fills_missing_fields_across_tools() {
        let from_portfolio = AccountSummary {
            portfolio_value: Some(1000.0),
            buying_power: None,
            cash: None,
            currency: "USD".into(),
        };
        let from_accounts = AccountSummary {
            portfolio_value: None,
            buying_power: Some(50.0),
            cash: Some(10.0),
            currency: "USD".into(),
        };
        let merged = merge_account(Some(from_portfolio), from_accounts);
        assert_eq!(merged.portfolio_value, Some(1000.0));
        assert_eq!(merged.buying_power, Some(50.0));
        assert_eq!(merged.cash, Some(10.0));
    }

    #[test]
    fn is_empty_payload_distinguishes_empty_from_populated() {
        assert!(is_empty_payload(&json!(null)));
        assert!(is_empty_payload(&json!([])));
        assert!(is_empty_payload(&json!({})));
        assert!(is_empty_payload(&json!({ "results": [] })));
        assert!(!is_empty_payload(&json!({ "buying_power": "10" })));
        assert!(!is_empty_payload(&json!([{ "symbol": "AAPL" }])));
    }

    #[test]
    fn tool_result_value_prefers_structured_then_text_json() {
        let structured = json!({ "structuredContent": { "positions": [] }, "content": [] });
        assert!(tool_result_value(&structured).get("positions").is_some());

        let text = json!({ "content": [{ "type": "text", "text": "[{\"symbol\":\"AAPL\",\"quantity\":1}]" }] });
        let parsed = tool_result_value(&text);
        assert!(parsed.is_array());
    }

    #[test]
    fn parse_quotes_reads_price_and_derives_change() {
        let v = json!({
            "results": [
                { "symbol": "aapl", "last_trade_price": "200.00", "previous_close": "190.00" },
                { "symbol": "MSFT", "price": 400.0, "change_percent_today": -1.5 },
            ]
        });
        let q = parse_quotes(&v);
        let aapl = q.get("AAPL").expect("aapl present");
        assert_eq!(aapl.price, Some(200.0));
        // (200 - 190) / 190 * 100 ≈ 5.263…
        assert!((aapl.change_pct.unwrap() - 5.263157).abs() < 1e-3);
        let msft = q.get("MSFT").expect("msft present");
        assert_eq!(msft.price, Some(400.0));
        assert_eq!(msft.change_pct, Some(-1.5));
    }

    #[test]
    fn parse_historicals_collects_close_series_per_symbol() {
        let v = json!({
            "results": [{
                "symbol": "AAPL",
                "historicals": [
                    { "close_price": "10.0" },
                    { "close_price": "11.0" },
                    { "close_price": "12.5" },
                ]
            }]
        });
        let h = parse_historicals(&v);
        assert_eq!(h.get("AAPL").unwrap(), &vec![10.0, 11.0, 12.5]);
    }

    #[test]
    fn parse_historicals_trims_to_spark_cap() {
        let points: Vec<Value> = (0..SPARK_MAX_POINTS + 20)
            .map(|i| json!({ "close": i as f64 }))
            .collect();
        let v = json!({ "results": [{ "symbol": "AAPL", "historicals": points }] });
        let series = parse_historicals(&v).remove("AAPL").unwrap();
        assert_eq!(series.len(), SPARK_MAX_POINTS);
        // Keeps the tail (most recent points).
        assert_eq!(*series.last().unwrap(), (SPARK_MAX_POINTS + 19) as f64);
    }

    #[test]
    fn find_named_requires_all_needles_and_read_safety() {
        let tools = vec![
            tool("get_option_quotes"),
            tool("get_equity_quotes"),
            tool("place_equity_order"),
        ];
        assert_eq!(find_named(&tools, &["equit", "quote"]).unwrap().name, "get_equity_quotes");
        // A mutating equity tool is never returned.
        assert!(find_named(&vec![tool("place_equity_order")], &["equit", "order"]).is_none());
    }

    #[test]
    fn schema_has_prop_detects_optional_params() {
        let t = ToolInfo {
            name: "get_equity_historicals".into(),
            input_schema: Some(json!({
                "type": "object",
                "properties": { "symbols": {}, "start_time": {}, "interval": {} },
                "required": ["symbols", "start_time"],
            })),
            annotations: None,
        };
        assert!(schema_has_prop(&t, "interval"));
        assert!(!schema_has_prop(&t, "bounds"));
    }

    #[test]
    fn parse_quotes_handles_symbol_keyed_map() {
        // Some servers return { "AAPL": {…}, "MSFT": {…} } rather than a list.
        let v = json!({
            "AAPL": { "last_trade_price": "200.0", "previous_close": "190.0" },
            "MSFT": { "price": 400.0, "change_pct": 2.0 },
        });
        let q = parse_quotes(&v);
        assert_eq!(q.get("AAPL").unwrap().price, Some(200.0));
        assert_eq!(q.get("MSFT").unwrap().change_pct, Some(2.0));
    }

    #[test]
    fn parse_historicals_handles_symbol_keyed_map() {
        // Map of symbol → bare points array, and symbol → { historicals: [...] }.
        let v = json!({
            "AAPL": [{ "close_price": "10.0" }, { "close_price": "12.0" }],
            "MSFT": { "historicals": [{ "close": 1.0 }, { "close": 2.0 }, { "close": 3.0 }] },
        });
        let h = parse_historicals(&v);
        assert_eq!(h.get("AAPL").unwrap(), &vec![10.0, 12.0]);
        assert_eq!(h.get("MSFT").unwrap(), &vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn parse_quotes_unwraps_data_guide_envelope() {
        // The live Robinhood shape: quotes nested under a { data, guide } envelope.
        // data as a symbol-keyed map…
        let keyed = json!({
            "data": {
                "AAPL": { "last_trade_price": "200.0", "previous_close": "190.0" },
                "BRK.B": { "last_trade_price": 500.0, "previous_close": 500.0 },
            },
            "guide": "Some descriptive text 1 2 3.",
        });
        let q = parse_quotes(&keyed);
        assert_eq!(q.get("AAPL").unwrap().price, Some(200.0));
        assert_eq!(q.get("BRK.B").unwrap().price, Some(500.0));

        // …and data as a list of quote records.
        let listed = json!({
            "data": [{ "symbol": "MSFT", "price": 400.0, "change_percent_today": 1.25 }],
            "guide": {},
        });
        let q = parse_quotes(&listed);
        assert_eq!(q.get("MSFT").unwrap().change_pct, Some(1.25));
    }

    #[test]
    fn parse_historicals_unwraps_data_envelope() {
        let v = json!({
            "data": { "AAPL": [{ "close_price": "10.0" }, { "close_price": "11.5" }] },
            "guide": "ignored",
        });
        let h = parse_historicals(&v);
        assert_eq!(h.get("AAPL").unwrap(), &vec![10.0, 11.5]);
    }

    #[test]
    fn tool_result_value_reparses_stringified_json() {
        // Historicals can arrive as a JSON *string* under structuredContent.
        let res = json!({
            "structuredContent": "{\"AAPL\":[{\"close_price\":1.0},{\"close_price\":2.0}]}"
        });
        let val = tool_result_value(&res);
        assert!(val.is_object(), "stringified JSON should be decoded to an object");
        let h = parse_historicals(&val);
        assert_eq!(h.get("AAPL").unwrap(), &vec![1.0, 2.0]);
    }

    #[test]
    fn shape_of_recurses_one_level_without_leaking_values() {
        let v = json!({ "data": { "AAPL": { "last_trade_price": "172.99" } }, "guide": "x" });
        let s = shape_of(&v);
        // Reveals the nested structure so a miss is diagnosable…
        assert!(s.contains("data:object"));
        assert!(s.contains("AAPL"));
        // …but never the actual price.
        assert!(!s.contains("172.99"));
    }

    #[test]
    fn shape_preview_redacts_numbers_in_string_payloads() {
        let v = json!("2026-06-17,172.99,170.10\n2026-06-17,173.00,171.00");
        let s = shape_preview(&v);
        assert!(s.starts_with("string «"));
        for leak in ["172.99", "170.10", "173.00"] {
            assert!(!s.contains(leak), "leaked {leak}");
        }
        // The structural commas survive so a CSV format is recognizable.
        assert!(s.contains(","));
    }


    #[test]
    fn parse_quotes_positional_zips_records_to_requested_symbols() {
        // The live shape: data.results is a 1:1 positional array with no symbol
        // field on each record (and a sibling closes_error).
        let v = json!({
            "data": {
                "closes_error": "could not compute closes",
                "results": [
                    { "last_trade_price": "100.0" },
                    { "last_trade_price": "200.0", "previous_close": "190.0" },
                ],
            },
            "guide": "text",
        });
        let symbols = vec!["AAPL".to_string(), "MSFT".to_string()];
        // The keyed parser can't find symbols on these records…
        assert!(parse_quotes(&v).is_empty());
        // …but the positional parser aligns them to the request order.
        let q = parse_quotes_positional(&v, &symbols);
        assert_eq!(q.get("AAPL").unwrap().price, Some(100.0));
        assert_eq!(q.get("MSFT").unwrap().price, Some(200.0));
        assert!((q.get("MSFT").unwrap().change_pct.unwrap() - 5.263).abs() < 1e-2);
    }

    #[test]
    fn parse_quotes_positional_refuses_on_length_mismatch() {
        let v = json!({ "data": { "results": [{ "last_trade_price": "1.0" }] } });
        // Two requested but one record back → don't risk a mislabel.
        let symbols = vec!["AAPL".to_string(), "MSFT".to_string()];
        assert!(parse_quotes_positional(&v, &symbols).is_empty());
    }

    #[test]
    fn parse_quotes_reads_records_nested_under_quote_key() {
        // The live shape: data.results is a list of { quote: { … } } wrappers,
        // each carrying its own symbol inside the nested quote object.
        let v = json!({
            "data": {
                "closes_error": "n/a",
                "results": [
                    { "quote": { "symbol": "AAPL", "last_trade_price": "200.0", "previous_close": "190.0" } },
                    { "quote": { "symbol": "BRK.B", "last_trade_price": 500.0 } },
                ],
            },
            "guide": "text",
        });
        // Keyed parsing now finds the symbol inside the nested quote…
        let q = parse_quotes(&v);
        assert_eq!(q.get("AAPL").unwrap().price, Some(200.0));
        assert_eq!(q.get("BRK.B").unwrap().price, Some(500.0));

        // …and positional zipping also reaches the nested price if symbols are absent.
        let bare = json!({ "data": { "results": [{ "quote": { "last_trade_price": "12.5" } }] } });
        let pos = parse_quotes_positional(&bare, &["XYZ".to_string()]);
        assert_eq!(pos.get("XYZ").unwrap().price, Some(12.5));
    }

    #[test]
    fn parse_historicals_reads_bars_series_with_symbol() {
        // The live chunked shape: data.results is per-symbol records whose price
        // series lives under "bars".
        let v = json!({
            "data": {
                "results": [{
                    "symbol": "AAPL",
                    "bounds": "regular",
                    "interval": "5minute",
                    "bars": [
                        { "close_price": "10.0" },
                        { "close_price": "11.0" },
                        { "close_price": "12.5" },
                    ],
                }],
            },
            "guide": "text",
        });
        let h = parse_historicals(&v);
        assert_eq!(h.get("AAPL").unwrap(), &vec![10.0, 11.0, 12.5]);
    }


    #[test]
    fn symbols_value_respects_schema_type() {
        let arr_tool = tool_requiring("get_equity_quotes", &["symbols"]);
        let syms = vec!["AAPL".to_string(), "MSFT".to_string()];
        // No explicit string type → array form.
        assert_eq!(symbols_value(&arr_tool, &syms), json!(["AAPL", "MSFT"]));

        // Schema says symbols is a string → comma-joined.
        let str_tool = ToolInfo {
            name: "get_equity_quotes".into(),
            input_schema: Some(json!({
                "type": "object",
                "properties": { "symbols": { "type": "string" } },
                "required": ["symbols"],
            })),
            annotations: None,
        };
        assert_eq!(symbols_value(&str_tool, &syms), json!("AAPL,MSFT"));
    }
}
