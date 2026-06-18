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

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::mcp::{McpClient, ToolInfo};

/// Robinhood's Agentic trading MCP endpoint (Streamable HTTP).
pub const ENDPOINT: &str = "https://agent.robinhood.com/mcp/trading";

/// A single equity position held in the connected account.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Position {
    pub ticker: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub quantity: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub market_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_buy_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unrealized_plpc: Option<f64>,
    pub currency: String,
}

/// Account-level money summary (best-effort; every field optional).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccountSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub portfolio_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buying_power: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cash: Option<f64>,
    pub currency: String,
}

/// The read-only snapshot returned to the frontend.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Portfolio {
    pub positions: Vec<Position>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<AccountSummary>,
    /// RFC3339 time the snapshot was taken.
    pub as_of: String,
    /// Which MCP tools the data came from (provenance shown in the UI).
    pub tools_used: Vec<String>,
}

impl Portfolio {
    /// Uppercased tickers the user actually holds (quantity > 0). Used to badge
    /// research picks the user already owns.
    pub fn owned_tickers(&self) -> BTreeSet<String> {
        self.positions
            .iter()
            .filter(|p| p.quantity > 0.0 && !p.ticker.is_empty())
            .map(|p| p.ticker.to_ascii_uppercase())
            .collect()
    }
}

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

/// Pick the first read-only tool whose name matches one of `nouns`, in priority
/// order. Tools that fail the read-only gate (name allow-list or destructive
/// annotation) are never considered.
pub fn select_tool<'a>(tools: &'a [ToolInfo], nouns: &[&str]) -> Option<&'a ToolInfo> {
    for noun in nouns {
        if let Some(t) = tools
            .iter()
            .filter(|t| is_safe_read_tool(t))
            .find(|t| t.name.to_ascii_lowercase().contains(noun))
        {
            return Some(t);
        }
    }
    None
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

/// Reduce a `tools/call` result to the JSON we can parse: prefer the typed
/// `structuredContent`, otherwise JSON-decode the joined text content, otherwise
/// fall back to the raw text as a string.
fn tool_result_value(result: &Value) -> Value {
    if let Some(sc) = result.get("structuredContent") {
        if !sc.is_null() {
            return sc.clone();
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

/// Find the array of records inside an arbitrarily-shaped tool result. Handles a
/// bare array, or an object that wraps the array under a common key.
fn find_records(v: &Value) -> Vec<Value> {
    if let Some(arr) = v.as_array() {
        return arr.clone();
    }
    if let Some(obj) = v.as_object() {
        const KEYS: &[&str] = &["positions", "holdings", "results", "data", "items", "equities"];
        for k in KEYS {
            if let Some(arr) = obj.get(*k).and_then(Value::as_array) {
                return arr.clone();
            }
        }
        // Single object that itself looks like a position.
        if obj.contains_key("symbol") || obj.contains_key("ticker") {
            return vec![v.clone()];
        }
    }
    Vec::new()
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
        let ticker = text(&rec, &["symbol", "ticker", "instrument_symbol", "instrument"])
            .unwrap_or_default()
            .to_ascii_uppercase();
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
        });
    }
    out
}

/// Normalize an arbitrary account payload into an `AccountSummary`, if any of the
/// money fields are present.
pub fn parse_account(v: &Value) -> Option<AccountSummary> {
    // Unwrap a single-element wrapper array or a `{ "account": {…} }` shape.
    let obj = if let Some(a) = v.as_array().and_then(|a| a.first()) {
        a.clone()
    } else if let Some(inner) = v.get("account") {
        inner.clone()
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

/// A connected, read-only Robinhood session built on the MCP client.
pub struct RobinhoodClient {
    mcp: McpClient,
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

        let positions = if let Some(t) = select_tool(&tools, &["position", "holding", "portfolio"]) {
            tools_used.push(t.name.clone());
            let res = self.mcp.call_tool(&t.name, json!({})).await?;
            parse_positions(&tool_result_value(&res))
        } else {
            Vec::new()
        };

        let account = if let Some(t) = select_tool(&tools, &["account", "balance"]) {
            // Don't fail the whole snapshot if the account tool errors.
            if let Ok(res) = self.mcp.call_tool(&t.name, json!({})).await {
                let parsed = parse_account(&tool_result_value(&res));
                if parsed.is_some() {
                    tools_used.push(t.name.clone());
                }
                parsed
            } else {
                None
            }
        } else {
            None
        };

        if positions.is_empty() && account.is_none() {
            return Err(AppError::Robinhood(
                "Connected, but no read-only positions/account tools were available.".into(),
            ));
        }

        Ok(Portfolio {
            positions,
            account,
            as_of: chrono::Utc::now().to_rfc3339(),
            tools_used,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::{ToolAnnotations, ToolInfo};

    fn tool(name: &str) -> ToolInfo {
        ToolInfo { name: name.into(), annotations: None }
    }

    fn tool_annotated(name: &str, read_only: Option<bool>, destructive: Option<bool>) -> ToolInfo {
        ToolInfo {
            name: name.into(),
            annotations: Some(ToolAnnotations {
                read_only_hint: read_only,
                destructive_hint: destructive,
            }),
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
    fn select_tool_prefers_positions_then_account() {
        let tools = vec![tool("get_account"), tool("list_positions"), tool("place_order")];
        assert_eq!(select_tool(&tools, &["position", "holding"]).unwrap().name, "list_positions");
        assert_eq!(select_tool(&tools, &["account"]).unwrap().name, "get_account");
        // A mutating tool is never selected even if the noun matches.
        assert!(select_tool(&vec![tool("place_order")], &["order"]).is_none());
    }

    #[test]
    fn select_tool_skips_destructive_annotation() {
        // Name looks like a benign read, but the server flags it destructive.
        let tools = vec![tool_annotated("get_positions", None, Some(true))];
        assert!(select_tool(&tools, &["position"]).is_none());
        // Explicitly non-read-only is likewise refused.
        let tools = vec![tool_annotated("portfolio_view", Some(false), None)];
        assert!(select_tool(&tools, &["portfolio"]).is_none());
        // A clean read-only annotation is fine.
        let tools = vec![tool_annotated("list_positions", Some(true), Some(false))];
        assert_eq!(select_tool(&tools, &["position"]).unwrap().name, "list_positions");
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
    fn tool_result_value_prefers_structured_then_text_json() {
        let structured = json!({ "structuredContent": { "positions": [] }, "content": [] });
        assert!(tool_result_value(&structured).get("positions").is_some());

        let text = json!({ "content": [{ "type": "text", "text": "[{\"symbol\":\"AAPL\",\"quantity\":1}]" }] });
        let parsed = tool_result_value(&text);
        assert!(parsed.is_array());
    }
}
