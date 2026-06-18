//! A small client for the Model Context Protocol (MCP) over the Streamable HTTP
//! transport — just enough to talk to Robinhood's Agentic trading server
//! (`https://agent.robinhood.com/mcp/trading`): `initialize`, `tools/list`, and
//! `tools/call`. We deliberately keep this minimal and dependency-light (it runs
//! on the same `reqwest::Client` the rest of the app uses) rather than pulling in
//! a full SDK, so the request/response framing stays auditable.
//!
//! Streamable HTTP means each JSON-RPC request is a plain HTTP `POST`. The server
//! may answer with a single `application/json` body or a short `text/event-stream`
//! (SSE) body; we handle both by extracting the JSON-RPC envelope that matches our
//! request id.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};

/// Protocol revision we advertise during `initialize`.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// One tool advertised by an MCP server (`tools/list`).
#[derive(Debug, Clone, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    #[serde(default)]
    pub annotations: Option<ToolAnnotations>,
}

/// Optional, server-supplied hints about a tool. We use these as a *second*
/// safety signal on top of the name allow-list: a tool flagged destructive (or
/// explicitly not read-only) is never called, even if its name looks benign. A
/// missing/true hint is never sufficient on its own to fire a tool.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ToolAnnotations {
    #[serde(default, rename = "readOnlyHint")]
    pub read_only_hint: Option<bool>,
    #[serde(default, rename = "destructiveHint")]
    pub destructive_hint: Option<bool>,
}

#[derive(Deserialize)]
struct ToolsListResult {
    #[serde(default)]
    tools: Vec<ToolInfo>,
}

/// A connected MCP session. Cheap to construct; holds the bearer token and the
/// negotiated session id so follow-up calls reuse it.
pub struct McpClient {
    http: reqwest::Client,
    endpoint: String,
    token: String,
    session_id: Mutex<Option<String>>,
    next_id: AtomicI64,
}

impl McpClient {
    pub fn new(http: reqwest::Client, endpoint: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            http,
            endpoint: endpoint.into(),
            token: token.into(),
            session_id: Mutex::new(None),
            next_id: AtomicI64::new(1),
        }
    }

    /// Perform the MCP handshake. Captures the `Mcp-Session-Id` response header
    /// (if the server uses one) and sends the required `initialized` notification.
    pub async fn initialize(&self) -> AppResult<()> {
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "TrendWave", "version": env!("CARGO_PKG_VERSION") },
        });
        self.request("initialize", params).await?;
        self.notify("notifications/initialized", json!({})).await?;
        Ok(())
    }

    /// List the tools the server exposes.
    pub async fn list_tools(&self) -> AppResult<Vec<ToolInfo>> {
        let result = self.request("tools/list", json!({})).await?;
        let parsed: ToolsListResult = serde_json::from_value(result)
            .map_err(|e| AppError::Robinhood(format!("could not parse tools/list: {e}")))?;
        Ok(parsed.tools)
    }

    /// Call one tool by name. Returns the raw `result` object so the caller can
    /// pull either the structured payload or the text content out of it.
    pub async fn call_tool(&self, name: &str, arguments: Value) -> AppResult<Value> {
        let params = json!({ "name": name, "arguments": arguments });
        self.request("tools/call", params).await
    }

    /// Send a JSON-RPC request and return its `result`, mapping a JSON-RPC
    /// `error` or a transport failure into `AppError`.
    async fn request(&self, method: &str, params: Value) -> AppResult<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let text = self.post(&body).await?;
        let envelope = extract_response(&text, id)
            .ok_or_else(|| AppError::Robinhood(format!("no JSON-RPC response for {method}")))?;

        if let Some(err) = envelope.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            return Err(AppError::Robinhood(format!("{method} failed: {msg}")));
        }
        Ok(envelope.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Fire a JSON-RPC notification (no id, no response expected).
    async fn notify(&self, method: &str, params: Value) -> AppResult<()> {
        let body = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        self.post(&body).await?;
        Ok(())
    }

    /// POST a JSON-RPC envelope and return the response body as text. Sets the
    /// auth, protocol, and (once negotiated) session headers, and refreshes the
    /// stored session id from the response.
    async fn post(&self, body: &Value) -> AppResult<String> {
        let mut req = self
            .http
            .post(&self.endpoint)
            .bearer_auth(&self.token)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", PROTOCOL_VERSION);

        if let Some(sid) = self.session_id.lock().ok().and_then(|g| g.clone()) {
            req = req.header("Mcp-Session-Id", sid);
        }

        let resp = req.json(body).send().await.map_err(|e| {
            if e.is_connect() || e.is_timeout() {
                AppError::Robinhood(format!("could not reach Robinhood MCP: {e}"))
            } else {
                AppError::Network(e.to_string())
            }
        })?;

        if let Some(sid) = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
        {
            if let Ok(mut guard) = self.session_id.lock() {
                *guard = Some(sid);
            }
        }

        let status = resp.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            // The brokerage token is rejected/expired — surface as "not connected"
            // so the UI prompts a fresh authorization instead of a raw error.
            return Err(AppError::RobinhoodNotConnected);
        }
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(AppError::Robinhood(format!("Robinhood MCP returned {status}: {}", truncate(&text, 200))));
        }
        Ok(text)
    }
}

/// Pull the JSON-RPC envelope matching `id` out of either a plain JSON body or an
/// SSE (`text/event-stream`) body. SSE frames look like `data: {json}` lines,
/// possibly several per response, so we scan every `data:` payload.
fn extract_response(body: &str, id: i64) -> Option<Value> {
    let trimmed = body.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
            if value_has_id(&v, id) {
                return Some(v);
            }
        }
    }
    // SSE: collect the JSON from each `data:` line and return the matching one.
    for line in body.lines() {
        let line = line.trim_start();
        if let Some(payload) = line.strip_prefix("data:") {
            if let Ok(v) = serde_json::from_str::<Value>(payload.trim()) {
                if value_has_id(&v, id) {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn value_has_id(v: &Value, id: i64) -> bool {
    v.get("id").and_then(Value::as_i64) == Some(id)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_plain_json_response() {
        let body = r#"{"jsonrpc":"2.0","id":7,"result":{"ok":true}}"#;
        let v = extract_response(body, 7).expect("should find response");
        assert_eq!(v["result"]["ok"], json!(true));
    }

    #[test]
    fn extracts_sse_framed_response() {
        let body = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{\"tools\":[]}}\n\n";
        let v = extract_response(body, 3).expect("should find SSE response");
        assert!(v["result"]["tools"].is_array());
    }

    #[test]
    fn ignores_mismatched_id() {
        let body = r#"{"jsonrpc":"2.0","id":1,"result":{}}"#;
        assert!(extract_response(body, 2).is_none());
    }

    #[test]
    fn parses_tool_annotations() {
        let t: ToolInfo = serde_json::from_value(json!({
            "name": "list_positions",
            "description": "List positions",
            "annotations": { "readOnlyHint": true, "title": "Positions" }
        }))
        .unwrap();
        assert_eq!(t.name, "list_positions");
        assert_eq!(t.annotations.unwrap().read_only_hint, Some(true));
    }}
