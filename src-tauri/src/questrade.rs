//! Read-only Questrade REST integration.
//!
//! Questrade does not expose an agentic/MCP endpoint, so this is a plain REST
//! client rather than a reuse of the Robinhood MCP plumbing. The shape of the
//! data it returns (`Portfolio`) is identical, so the same frontend panel renders
//! either broker.
//!
//! Scope is deliberately **read-only**: personal (retail) Questrade apps cannot
//! place trades at all — only Questrade *partners* get order endpoints — so the
//! read-only guarantee is enforced by the platform itself. We only ever call
//! account/positions/balances and market-data endpoints.
//!
//! ## Auth model (very different from Robinhood)
//! There is no browser OAuth flow. The user generates a **manual authorization
//! refresh token** in Questrade's API centre and pastes it in. We exchange it at
//! the login server for a short-lived access token plus the per-session
//! `api_server` base URL. Questrade rotates the refresh token on every exchange
//! (single-use), so we always persist the newest one in the OS keychain.

use std::collections::{BTreeMap, HashMap};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::model::{AccountSummary, Listing, Portfolio, Position};

/// Questrade's OAuth token endpoint. The refresh-token grant is a GET with the
/// parameters in the query string; it returns the access token and the
/// `api_server` host to use for every subsequent data call.
const TOKEN_URL: &str = "https://login.questrade.com/oauth2/token";

/// Keychain coordinates for the stored Questrade token blob. Same service as the
/// Robinhood integration but a distinct account so the two never collide.
const KEYRING_SERVICE: &str = "com.trendwave.app";
const KEYRING_ACCOUNT: &str = "questrade-api";

/// Refresh a little before the access token actually expires to avoid races.
const EXPIRY_SLACK_SECS: i64 = 30;

/// Questrade's login server returns sporadic 5xx responses for tokens that are
/// actually fine, so retry a few times before giving up — most blips clear on
/// the second try.
const EXCHANGE_MAX_ATTEMPTS: u32 = 3;
const EXCHANGE_RETRY_BACKOFF_MS: u64 = 500;

/// Most intraday points we keep for a row's sparkline (newest retained).
const SPARK_MAX_POINTS: usize = 48;

/// Cap on how many held symbols get a (best-effort) intraday candle fetch, so a
/// large portfolio doesn't fan out into dozens of market-data calls.
const SPARK_MAX_SYMBOLS: usize = 12;

// ---------------------------------------------------------------------------
// Token storage + refresh
// ---------------------------------------------------------------------------

/// Everything needed to call the API and later refresh the session. Persisted as
/// JSON in the OS keychain — never in SQLite or any plaintext file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAuth {
    pub access_token: String,
    /// Per-session API host returned by the token endpoint, e.g.
    /// `https://api01.iq.questrade.com/` (note the trailing slash).
    pub api_server: String,
    /// Questrade rotates this on every exchange; we always keep the latest.
    pub refresh_token: String,
    /// Unix epoch seconds when the access token expires, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

impl StoredAuth {
    fn is_access_valid(&self) -> bool {
        match self.expires_at {
            Some(exp) => now_secs() + EXPIRY_SLACK_SECS < exp,
            None => false,
        }
    }
}

/// Raw token endpoint response.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    api_server: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

/// Why a token exchange failed — so a *rejected* token forces a reconnect while a
/// transient network blip is surfaced as a plain error (and doesn't wrongly tell
/// the user their connection is gone).
enum ExchangeError {
    /// The login server rejected the refresh token with a 4xx (expired / already used).
    Rejected,
    /// The login server returned persistent 5xx responses. Questrade does this both
    /// for genuinely bad tokens (a known quirk) and during real outages.
    ServerError,
    /// Network or parsing failure talking to the login server.
    Transport(String),
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

fn keyring_entry() -> AppResult<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .map_err(|e| AppError::Questrade(format!("keychain unavailable: {e}")))
}

pub fn load_auth() -> Option<StoredAuth> {
    let entry = keyring_entry().ok()?;
    let json = entry.get_password().ok()?;
    serde_json::from_str(&json).ok()
}

pub fn save_auth(auth: &StoredAuth) -> AppResult<()> {
    let json = serde_json::to_string(auth)?;
    keyring_entry()?
        .set_password(&json)
        .map_err(|e| AppError::Questrade(format!("could not save credentials: {e}")))
}

pub fn clear_auth() -> AppResult<()> {
    if let Ok(entry) = keyring_entry() {
        // Treat "no entry" as already-cleared.
        match entry.delete_credential() {
            Ok(()) => {}
            Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(AppError::Questrade(format!("could not clear credentials: {e}"))),
        }
    }
    Ok(())
}

/// Exchange a refresh token for a fresh session (access token + api_server + a
/// rotated refresh token). Used both for the initial manual-token connect and
/// for transparent refresh.
async fn exchange(http: &reqwest::Client, refresh_token: &str) -> Result<StoredAuth, ExchangeError> {
    let mut last_transport: Option<String> = None;
    let mut saw_server_error = false;

    for attempt in 0..EXCHANGE_MAX_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(
                EXCHANGE_RETRY_BACKOFF_MS * attempt as u64,
            ))
            .await;
        }

        let resp = match http
            .get(TOKEN_URL)
            .query(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ])
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                // Network blip — worth another try.
                last_transport = Some(format!("token request failed: {e}"));
                continue;
            }
        };

        let status = resp.status();

        // A 4xx means the token itself was refused; retrying won't help.
        if status.is_client_error() {
            return Err(ExchangeError::Rejected);
        }

        if !status.is_success() {
            // 5xx (Questrade's flaky auth server, or its quirky response to a
            // bad token): remember it and retry.
            saw_server_error = true;
            continue;
        }

        let token: TokenResponse = resp
            .json()
            .await
            .map_err(|e| ExchangeError::Transport(format!("malformed token response: {e}")))?;

        let api_server = token
            .api_server
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ExchangeError::Transport("token response had no api_server".into()))?;

        return Ok(StoredAuth {
            access_token: token.access_token,
            api_server,
            // Reuse the previous refresh token only if the server somehow omitted a
            // new one (it normally rotates every time).
            refresh_token: token
                .refresh_token
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| refresh_token.to_string()),
            expires_at: token.expires_in.map(|s| now_secs() + s),
        });
    }

    // Every attempt failed. A 5xx somewhere along the way is the more actionable
    // signal, so prefer it over a bare network error.
    Err(if saw_server_error {
        ExchangeError::ServerError
    } else {
        ExchangeError::Transport(
            last_transport.unwrap_or_else(|| "could not reach Questrade's login server".into()),
        )
    })
}

/// Connect using the manual authorization token the user pasted from Questrade's
/// API centre. Persists the resulting session on success.
pub async fn connect(http: &reqwest::Client, manual_token: &str) -> AppResult<StoredAuth> {
    let token = manual_token.trim();
    if token.is_empty() {
        return Err(AppError::Questrade(
            "Enter the manual authorization token from Questrade's API centre.".into(),
        ));
    }
    match exchange(http, token).await {
        Ok(auth) => {
            save_auth(&auth)?;
            Ok(auth)
        }
        Err(ExchangeError::Rejected) => Err(AppError::Questrade(
            "Authorization failed — the token may be invalid, expired, or already used. \
             Each manual token works only once, so generate a fresh one in Questrade's API \
             centre and paste it right away."
                .into(),
        )),
        Err(ExchangeError::ServerError) => Err(AppError::Questrade(
            "Questrade's login server rejected this token (HTTP 500). Manual tokens are \
             single-use and short-lived — generate a new token in Questrade's API centre and \
             paste it immediately. If it keeps failing, Questrade's API may be briefly down."
                .into(),
        )),
        Err(ExchangeError::Transport(msg)) => Err(AppError::Questrade(msg)),
    }
}

/// Return a usable `(access_token, api_server)`, refreshing transparently if the
/// stored access token has expired. A rejected refresh surfaces as
/// `QuestradeNotConnected` so the UI prompts the user to reconnect.
pub async fn ensure_session(http: &reqwest::Client) -> AppResult<(String, String)> {
    let auth = load_auth().ok_or(AppError::QuestradeNotConnected)?;
    if auth.is_access_valid() {
        return Ok((auth.access_token, auth.api_server));
    }
    match exchange(http, &auth.refresh_token).await {
        Ok(refreshed) => {
            save_auth(&refreshed)?;
            Ok((refreshed.access_token, refreshed.api_server))
        }
        Err(ExchangeError::Rejected) => Err(AppError::QuestradeNotConnected),
        Err(ExchangeError::ServerError) => Err(AppError::Questrade(
            "Questrade's login server is returning errors (HTTP 500). This is usually \
             temporary — please try again in a moment."
                .into(),
        )),
        Err(ExchangeError::Transport(msg)) => Err(AppError::Questrade(msg)),
    }
}

// ---------------------------------------------------------------------------
// REST response shapes (only the fields we use)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct AccountsResp {
    #[serde(default)]
    accounts: Vec<Account>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Account {
    number: String,
}

#[derive(Debug, Deserialize)]
struct PositionsResp {
    #[serde(default)]
    positions: Vec<QtPosition>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QtPosition {
    symbol: String,
    symbol_id: i64,
    #[serde(default)]
    open_quantity: Option<f64>,
    #[serde(default)]
    current_market_value: Option<f64>,
    #[serde(default)]
    current_price: Option<f64>,
    #[serde(default)]
    total_cost: Option<f64>,
    #[serde(default)]
    open_pnl: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BalancesResp {
    #[serde(default)]
    combined_balances: Vec<Balance>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Balance {
    #[serde(default)]
    currency: String,
    #[serde(default)]
    cash: Option<f64>,
    #[serde(default)]
    total_equity: Option<f64>,
    #[serde(default)]
    buying_power: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct SymbolsResp {
    #[serde(default)]
    symbols: Vec<QtSymbol>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QtSymbol {
    symbol_id: i64,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    currency: Option<String>,
}

/// Result rows from `symbols/search` (a different shape than `symbols?ids=`):
/// these carry the human `symbol` string, listing exchange, and tradability.
#[derive(Debug, Deserialize)]
struct SymbolSearchResp {
    #[serde(default)]
    symbols: Vec<SymbolSearchItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SymbolSearchItem {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    listing_exchange: Option<String>,
    #[serde(default)]
    currency: Option<String>,
    #[serde(default)]
    is_tradable: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct QuotesResp {
    #[serde(default)]
    quotes: Vec<QtQuote>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QtQuote {
    symbol_id: i64,
    #[serde(default)]
    last_trade_price: Option<f64>,
    #[serde(default)]
    open_price: Option<f64>,
    #[serde(default)]
    prev_day_close_price: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct CandlesResp {
    #[serde(default)]
    candles: Vec<Candle>,
}

#[derive(Debug, Deserialize)]
struct Candle {
    #[serde(default)]
    close: Option<f64>,
}

// ---------------------------------------------------------------------------
// Per-symbol aggregation across accounts
// ---------------------------------------------------------------------------

/// The same ticker is often held across several Questrade accounts (TFSA, RRSP,
/// margin). We fold them into one row so the portfolio reads as a single book.
struct Agg {
    ticker: String,
    quantity: f64,
    market_value: f64,
    total_cost: f64,
    open_pnl: f64,
    price: Option<f64>,
}

impl Agg {
    fn new(symbol: &str) -> Self {
        Self {
            ticker: symbol.to_string(),
            quantity: 0.0,
            market_value: 0.0,
            total_cost: 0.0,
            open_pnl: 0.0,
            price: None,
        }
    }

    fn into_position(self) -> Position {
        let average_buy_price = if self.quantity != 0.0 && self.total_cost != 0.0 {
            Some(self.total_cost / self.quantity)
        } else {
            None
        };
        let unrealized_plpc = if self.total_cost.abs() > f64::EPSILON {
            Some(self.open_pnl / self.total_cost)
        } else {
            None
        };
        Position {
            ticker: self.ticker.to_ascii_uppercase(),
            name: None,
            quantity: self.quantity,
            market_value: (self.market_value != 0.0).then_some(self.market_value),
            average_buy_price,
            unrealized_plpc,
            // Refined per-symbol by the /symbols enrichment; CAD is the sane
            // default for a Canadian account when the lookup misses.
            currency: "CAD".to_string(),
            price: self.price,
            change_pct: None,
            spark: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

pub struct QuestradeClient {
    http: reqwest::Client,
    access_token: String,
    api_server: String,
}

impl QuestradeClient {
    pub fn new(
        http: reqwest::Client,
        access_token: impl Into<String>,
        api_server: impl Into<String>,
    ) -> Self {
        Self {
            http,
            access_token: access_token.into(),
            api_server: api_server.into(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/v1/{}", self.api_server.trim_end_matches('/'), path)
    }

    async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> AppResult<T> {
        let resp = self
            .http
            .get(self.url(path))
            .bearer_auth(&self.access_token)
            .query(query)
            .send()
            .await
            .map_err(|e| AppError::Questrade(format!("request to {path} failed: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(AppError::Questrade(format!("{path} returned HTTP {status}")));
        }
        resp.json::<T>()
            .await
            .map_err(|e| AppError::Questrade(format!("malformed {path} response: {e}")))
    }

    /// Find the best tradable Questrade listing for `symbol`, preferring a
    /// Canadian (CAD) listing of the same security so a Canadian account can buy
    /// it without an FX conversion. Returns `None` when Questrade lists nothing
    /// matching. Read-only: this only searches the symbol catalogue.
    pub async fn find_listing(&self, symbol: &str) -> AppResult<Option<Listing>> {
        let root = symbol
            .split('.')
            .next()
            .unwrap_or(symbol)
            .trim()
            .to_ascii_uppercase();
        if root.is_empty() {
            return Ok(None);
        }

        let resp: SymbolSearchResp = self
            .get_json("symbols/search", &[("prefix", root.as_str())])
            .await?;

        // Same-security matches only: the Questrade symbol's root equals ours.
        let matches: Vec<&SymbolSearchItem> = resp
            .symbols
            .iter()
            .filter(|s| s.is_tradable.unwrap_or(true))
            .filter(|s| {
                s.symbol
                    .as_deref()
                    .map(|sym| sym.split('.').next().unwrap_or(sym).eq_ignore_ascii_case(&root))
                    .unwrap_or(false)
            })
            .collect();

        let pick = matches
            .iter()
            .copied()
            .find(|s| s.currency.as_deref().map(|c| c.eq_ignore_ascii_case("CAD")).unwrap_or(false))
            .or_else(|| matches.first().copied());

        Ok(pick.map(|s| Listing {
            symbol: s.symbol.clone().unwrap_or_default(),
            exchange: s.listing_exchange.clone(),
            currency: s.currency.clone(),
        }))
    }

    /// Handshake-free: pull every account, its positions and balances, then
    /// enrich held symbols with currency, name, today's change, and a sparkline.
    pub async fn fetch_portfolio(&self) -> AppResult<Portfolio> {
        let mut tools_used: Vec<String> = Vec::new();
        let mut debug: Vec<String> = Vec::new();

        let accounts = self.get_json::<AccountsResp>("accounts", &[]).await?.accounts;
        if accounts.is_empty() {
            return Err(AppError::Questrade(
                "Connected, but Questrade returned no accounts for this login.".into(),
            ));
        }
        tools_used.push("accounts".into());

        // Phase 1 — positions + balances, folded across every account.
        let mut by_symbol: BTreeMap<i64, Agg> = BTreeMap::new();
        let mut order: Vec<i64> = Vec::new();
        let mut account = AccountSummary {
            currency: "CAD".into(),
            ..Default::default()
        };
        let mut got_positions = false;
        let mut got_balances = false;

        for acct in &accounts {
            match self
                .get_json::<PositionsResp>(&format!("accounts/{}/positions", acct.number), &[])
                .await
            {
                Ok(resp) => {
                    for p in resp.positions {
                        let qty = p.open_quantity.unwrap_or(0.0);
                        let mv = p.current_market_value.unwrap_or(0.0);
                        if qty == 0.0 && mv == 0.0 {
                            continue;
                        }
                        let agg = by_symbol.entry(p.symbol_id).or_insert_with(|| {
                            order.push(p.symbol_id);
                            Agg::new(&p.symbol)
                        });
                        agg.quantity += qty;
                        agg.market_value += mv;
                        agg.total_cost += p.total_cost.unwrap_or(0.0);
                        agg.open_pnl += p.open_pnl.unwrap_or(0.0);
                        if agg.price.is_none() {
                            agg.price = p.current_price;
                        }
                        got_positions = true;
                    }
                }
                Err(e) => debug.push(format!("positions[{}]: {}", redacted_account(&acct.number), e)),
            }

            match self
                .get_json::<BalancesResp>(&format!("accounts/{}/balances", acct.number), &[])
                .await
            {
                Ok(resp) => {
                    // `combinedBalances` already expresses the whole account in a
                    // single currency; prefer the CAD view and sum across accounts.
                    let pick = resp
                        .combined_balances
                        .iter()
                        .find(|b| b.currency.eq_ignore_ascii_case("CAD"))
                        .or_else(|| resp.combined_balances.first());
                    if let Some(b) = pick {
                        account.portfolio_value = Some(
                            account.portfolio_value.unwrap_or(0.0) + b.total_equity.unwrap_or(0.0),
                        );
                        account.cash = Some(account.cash.unwrap_or(0.0) + b.cash.unwrap_or(0.0));
                        account.buying_power = Some(
                            account.buying_power.unwrap_or(0.0) + b.buying_power.unwrap_or(0.0),
                        );
                        if !b.currency.is_empty() {
                            account.currency = b.currency.to_ascii_uppercase();
                        }
                        got_balances = true;
                    }
                }
                Err(e) => debug.push(format!("balances[{}]: {}", redacted_account(&acct.number), e)),
            }
        }

        if got_positions {
            tools_used.push("positions".into());
        }
        if got_balances {
            tools_used.push("balances".into());
        }

        let mut positions: Vec<Position> = order
            .iter()
            .filter_map(|id| by_symbol.remove(id).map(Agg::into_position))
            .collect();

        // Phase 2 — enrich currency + display name from the symbols endpoint, and
        // today's change from a batched quote. Best-effort: misses are noted only.
        if !order.is_empty() {
            let ids = csv(&order);
            match self.get_json::<SymbolsResp>("symbols", &[("ids", &ids)]).await {
                Ok(resp) => {
                    tools_used.push("symbols".into());
                    let map: HashMap<i64, QtSymbol> =
                        resp.symbols.into_iter().map(|s| (s.symbol_id, s)).collect();
                    for (pos, id) in positions.iter_mut().zip(order.iter()) {
                        if let Some(s) = map.get(id) {
                            if let Some(c) = s.currency.as_deref().filter(|c| !c.is_empty()) {
                                pos.currency = c.to_ascii_uppercase();
                            }
                            if pos.name.is_none() {
                                pos.name = s.description.clone().filter(|d| !d.is_empty());
                            }
                        }
                    }
                }
                Err(e) => debug.push(format!("symbols: {e}")),
            }

            match self
                .get_json::<QuotesResp>("markets/quotes", &[("ids", &ids)])
                .await
            {
                Ok(resp) => {
                    tools_used.push("quotes".into());
                    let map: HashMap<i64, QtQuote> =
                        resp.quotes.into_iter().map(|q| (q.symbol_id, q)).collect();
                    for (pos, id) in positions.iter_mut().zip(order.iter()) {
                        if let Some(q) = map.get(id) {
                            let last = q.last_trade_price.or(pos.price);
                            let prev = q.prev_day_close_price.or(q.open_price);
                            pos.change_pct = compute_change_pct(last, prev);
                            if pos.price.is_none() {
                                pos.price = q.last_trade_price;
                            }
                        }
                    }
                }
                Err(e) => debug.push(format!("quotes: {e}")),
            }

            // Phase 3 — best-effort intraday sparkline for the largest holdings.
            for (pos, id) in positions.iter_mut().zip(order.iter()).take(SPARK_MAX_SYMBOLS) {
                if let Ok(series) = self.fetch_spark(*id).await {
                    if series.len() >= 2 {
                        pos.spark = downsample(series, SPARK_MAX_POINTS);
                    }
                }
            }
        }

        if positions.is_empty() && !got_balances {
            return Err(AppError::Questrade(
                "Connected, but couldn't read any positions or balances for this account.".into(),
            ));
        }

        Ok(Portfolio {
            positions,
            account: got_balances.then_some(account),
            as_of: chrono::Utc::now().to_rfc3339(),
            tools_used,
            debug,
        })
    }

    /// Pull ~24h of one-minute closes for a symbol's sparkline. Best-effort.
    async fn fetch_spark(&self, symbol_id: i64) -> AppResult<Vec<f64>> {
        let end = chrono::Utc::now();
        let start = end - chrono::Duration::hours(24);
        let resp = self
            .get_json::<CandlesResp>(
                &format!("markets/candles/{symbol_id}"),
                &[
                    ("startTime", &start.to_rfc3339()),
                    ("endTime", &end.to_rfc3339()),
                    ("interval", "OneMinute"),
                ],
            )
            .await?;
        Ok(resp.candles.into_iter().filter_map(|c| c.close).collect())
    }
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested)
// ---------------------------------------------------------------------------

fn csv(ids: &[i64]) -> String {
    ids.iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// Today's percent change from a last price and the previous close. `None` unless
/// both are present and the previous close is non-zero.
fn compute_change_pct(last: Option<f64>, prev: Option<f64>) -> Option<f64> {
    match (last, prev) {
        (Some(last), Some(prev)) if prev != 0.0 => Some((last - prev) / prev * 100.0),
        _ => None,
    }
}

/// Keep at most `max` of the most recent points (sparklines only need the tail).
fn downsample(series: Vec<f64>, max: usize) -> Vec<f64> {
    if series.len() <= max {
        return series;
    }
    series[series.len() - max..].to_vec()
}

/// Show only the last 4 characters of an account number in diagnostics so a
/// failure can be located without logging the full identifier.
fn redacted_account(number: &str) -> String {
    let n = number.len();
    if n <= 4 {
        "••••".to_string()
    } else {
        format!("••{}", &number[n - 4..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_pct_needs_both_and_nonzero_prev() {
        assert_eq!(compute_change_pct(Some(110.0), Some(100.0)), Some(10.0));
        assert_eq!(compute_change_pct(Some(90.0), Some(100.0)), Some(-10.0));
        assert_eq!(compute_change_pct(None, Some(100.0)), None);
        assert_eq!(compute_change_pct(Some(110.0), None), None);
        assert_eq!(compute_change_pct(Some(110.0), Some(0.0)), None);
    }

    #[test]
    fn downsample_keeps_recent_tail() {
        assert_eq!(downsample(vec![1.0, 2.0, 3.0], 5), vec![1.0, 2.0, 3.0]);
        assert_eq!(downsample(vec![1.0, 2.0, 3.0, 4.0], 2), vec![3.0, 4.0]);
    }

    #[test]
    fn csv_joins_ids() {
        assert_eq!(csv(&[1, 2, 3]), "1,2,3");
        assert_eq!(csv(&[]), "");
    }

    #[test]
    fn redacts_account_to_last_four() {
        assert_eq!(redacted_account("12345678"), "••5678");
        assert_eq!(redacted_account("99"), "••••");
    }

    #[test]
    fn aggregate_folds_quantity_and_weights_cost() {
        let mut a = Agg::new("shop.to");
        a.quantity += 10.0;
        a.market_value += 1000.0;
        a.total_cost += 800.0;
        a.open_pnl += 200.0;
        a.price = Some(100.0);
        // a second lot of the same symbol from another account
        a.quantity += 10.0;
        a.market_value += 1000.0;
        a.total_cost += 1200.0;

        let p = a.into_position();
        assert_eq!(p.ticker, "SHOP.TO");
        assert_eq!(p.quantity, 20.0);
        assert_eq!(p.market_value, Some(2000.0));
        assert_eq!(p.average_buy_price, Some(100.0)); // 2000 cost / 20 shares
        assert_eq!(p.currency, "CAD");
    }
}
