use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use rusqlite::Connection;
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::biometric;
use crate::db::{self, Watchlist};
use crate::error::{AppError, AppResult};
use crate::feeds;
use crate::model::{Listing, ListingInfo, Portfolio, ProgressEvent, ResearchResult};
use crate::oauth;
use crate::ollama::OllamaClient;
use crate::questrade::{self, QuestradeClient};
use crate::research;
use crate::robinhood::{self, RobinhoodClient};
use crate::settings::Settings;

/// Shared application state managed by Tauri. The SQLite connection lives behind
/// a `Mutex` (rusqlite's `Connection` is not `Sync`); we only ever hold the lock
/// for short, synchronous queries and never across an `.await`. The HTTP client
/// is internally ref-counted and cheap to clone for each async task. `robinhood`
/// and `questrade` each cache the last read-only portfolio snapshot so research
/// can badge owned tickers without a network round-trip. `unlocked` tracks
/// whether the user has passed the biometric gate this run (in-memory only, so
/// it resets each launch).
pub struct AppState {
    pub db: Mutex<Connection>,
    pub http: reqwest::Client,
    pub robinhood: Mutex<Option<Portfolio>>,
    pub questrade: Mutex<Option<Portfolio>>,
    pub unlocked: AtomicBool,
}

impl AppState {
    fn lock_db(&self) -> AppResult<MutexGuard<'_, Connection>> {
        self.db
            .lock()
            .map_err(|_| AppError::Database("database lock was poisoned".into()))
    }

    /// True when a connected Robinhood session is currently gated behind an
    /// unmet biometric unlock. Degrades to unlocked when biometrics are
    /// unavailable or the user hasn't opted in, so the gate can never strand a
    /// session the user can't get back into.
    fn portfolio_locked(&self) -> AppResult<bool> {
        if !biometric::is_available() {
            return Ok(false);
        }
        let (require, connected) = {
            let conn = self.lock_db()?;
            (
                db::load_settings(&conn)?.require_biometric_unlock,
                db::get_flag(&conn, db::FLAG_ROBINHOOD_CONNECTED)?,
            )
        };
        if !require || !connected {
            return Ok(false);
        }
        Ok(!self.unlocked.load(Ordering::Relaxed))
    }
}

#[tauri::command]
pub async fn run_research(
    state: State<'_, AppState>,
    prompt: String,
    on_event: Channel<ProgressEvent>,
) -> AppResult<ResearchResult> {
    execute(&state, &prompt, &on_event).await
}

#[tauri::command]
pub async fn run_watchlist(
    state: State<'_, AppState>,
    id: i64,
    on_event: Channel<ProgressEvent>,
) -> AppResult<ResearchResult> {
    let prompt = {
        let conn = state.lock_db()?;
        db::get_watchlist(&conn, id)?.prompt
    };
    let result = execute(&state, &prompt, &on_event).await?;
    let conn = state.lock_db()?;
    db::update_watchlist_result(&conn, id, &result)?;
    Ok(result)
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> AppResult<Settings> {
    let conn = state.lock_db()?;
    db::load_settings(&conn)
}

#[tauri::command]
pub async fn save_settings(state: State<'_, AppState>, settings: Settings) -> AppResult<()> {
    let conn = state.lock_db()?;
    db::save_settings(&conn, &settings)
}

#[tauri::command]
pub async fn list_watchlists(state: State<'_, AppState>) -> AppResult<Vec<Watchlist>> {
    let conn = state.lock_db()?;
    db::list_watchlists(&conn)
}

#[tauri::command]
pub async fn create_watchlist(
    state: State<'_, AppState>,
    name: String,
    prompt: String,
) -> AppResult<Watchlist> {
    let conn = state.lock_db()?;
    db::create_watchlist(&conn, &name, &prompt)
}

#[tauri::command]
pub async fn delete_watchlist(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    let conn = state.lock_db()?;
    db::delete_watchlist(&conn, id)
}

/// Read-only Robinhood connection status plus the last cached portfolio snapshot.
/// `locked` is true when a connected session is hidden behind an unmet biometric
/// unlock — the frontend shows an unlock prompt rather than the portfolio.
#[derive(serde::Serialize)]
pub struct RobinhoodStatus {
    pub connected: bool,
    pub locked: bool,
    pub portfolio: Option<Portfolio>,
}

#[tauri::command]
pub async fn robinhood_status(state: State<'_, AppState>) -> AppResult<RobinhoodStatus> {
    let locked = state.portfolio_locked()?;
    let connected = {
        let conn = state.lock_db()?;
        db::get_flag(&conn, db::FLAG_ROBINHOOD_CONNECTED)?
    };
    let portfolio = if locked {
        None
    } else {
        state.robinhood.lock().ok().and_then(|guard| guard.clone())
    };
    Ok(RobinhoodStatus {
        connected,
        locked,
        portfolio,
    })
}

/// Kick off the OAuth flow (opens the system browser), then pull an initial
/// read-only snapshot. Connecting succeeds even if the first snapshot fails.
#[tauri::command]
pub async fn robinhood_connect(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<RobinhoodStatus> {
    let opener = app.clone();
    oauth::connect(&state.http, move |url| {
        opener
            .opener()
            .open_url(url, None::<&str>)
            .map_err(|e| AppError::Robinhood(format!("could not open browser: {e}")))
    })
    .await?;

    // The user just completed an interactive sign-in, so they're plainly present
    // — unlock the session now and let the biometric gate apply on later launches.
    state.unlocked.store(true, Ordering::Relaxed);
    {
        let conn = state.lock_db()?;
        db::set_flag(&conn, db::FLAG_ROBINHOOD_CONNECTED, true)?;
    }

    let portfolio = robinhood_fetch_and_cache(&state).await.ok();
    Ok(RobinhoodStatus {
        connected: true,
        locked: false,
        portfolio,
    })
}

#[tauri::command]
pub async fn robinhood_disconnect(state: State<'_, AppState>) -> AppResult<()> {
    oauth::clear_auth()?;
    state.unlocked.store(false, Ordering::Relaxed);
    {
        let conn = state.lock_db()?;
        db::set_flag(&conn, db::FLAG_ROBINHOOD_CONNECTED, false)?;
    }
    if let Ok(mut guard) = state.robinhood.lock() {
        *guard = None;
    }
    Ok(())
}

/// Force a fresh read-only portfolio fetch (refreshing the token if needed).
/// Refuses while the biometric gate is engaged.
#[tauri::command]
pub async fn robinhood_portfolio(state: State<'_, AppState>) -> AppResult<Portfolio> {
    if state.portfolio_locked()? {
        return Err(AppError::Locked);
    }
    robinhood_fetch_and_cache(&state).await
}

/// Whether this device supports a biometric / device-auth unlock prompt.
#[tauri::command]
pub async fn biometric_available() -> AppResult<bool> {
    Ok(biometric::is_available())
}

/// Prompt for Touch ID / Windows Hello and, on success, unlock the saved
/// Robinhood session for the rest of this run. Returns whether it unlocked;
/// a dismissed or failed prompt resolves to `false` (no error banner).
#[tauri::command]
pub async fn biometric_unlock(state: State<'_, AppState>) -> AppResult<bool> {
    let unlocked = biometric::authenticate("unlock your saved Robinhood session").await?;
    if unlocked {
        state.unlocked.store(true, Ordering::Relaxed);
    }
    Ok(unlocked)
}

/// Pull a read-only portfolio via the MCP client and cache it for enrichment.
async fn robinhood_fetch_and_cache(state: &AppState) -> AppResult<Portfolio> {
    let token = match oauth::ensure_access_token(&state.http).await {
        Ok(token) => token,
        Err(err @ AppError::RobinhoodNotConnected) => {
            // The keychain says we are not actually connected — clear the marker
            // so status stops advertising a session that no longer exists.
            if let Ok(conn) = state.lock_db() {
                let _ = db::set_flag(&conn, db::FLAG_ROBINHOOD_CONNECTED, false);
            }
            return Err(err);
        }
        Err(err) => return Err(err),
    };
    let client = RobinhoodClient::new(state.http.clone(), token);
    let portfolio = client.fetch_portfolio().await?;
    if let Ok(mut guard) = state.robinhood.lock() {
        *guard = Some(portfolio.clone());
    }
    Ok(portfolio)
}

/// Read-only Questrade connection status plus the last cached portfolio snapshot.
#[derive(serde::Serialize)]
pub struct QuestradeStatus {
    pub connected: bool,
    pub portfolio: Option<Portfolio>,
}

#[tauri::command]
pub async fn questrade_status(state: State<'_, AppState>) -> AppResult<QuestradeStatus> {
    let connected = {
        let conn = state.lock_db()?;
        db::get_flag(&conn, db::FLAG_QUESTRADE_CONNECTED)?
    };
    let portfolio = state
        .questrade
        .lock()
        .ok()
        .and_then(|guard| guard.clone());
    Ok(QuestradeStatus {
        connected,
        portfolio,
    })
}

/// Connect Questrade with the manual authorization token from its API centre,
/// then pull an initial read-only snapshot. Connecting succeeds even if the first
/// snapshot fails.
#[tauri::command]
pub async fn questrade_connect(
    state: State<'_, AppState>,
    token: String,
) -> AppResult<QuestradeStatus> {
    questrade::connect(&state.http, &token).await?;
    {
        let conn = state.lock_db()?;
        db::set_flag(&conn, db::FLAG_QUESTRADE_CONNECTED, true)?;
    }
    let portfolio = questrade_fetch_and_cache(&state).await.ok();
    Ok(QuestradeStatus {
        connected: true,
        portfolio,
    })
}

#[tauri::command]
pub async fn questrade_disconnect(state: State<'_, AppState>) -> AppResult<()> {
    questrade::clear_auth()?;
    {
        let conn = state.lock_db()?;
        db::set_flag(&conn, db::FLAG_QUESTRADE_CONNECTED, false)?;
    }
    if let Ok(mut guard) = state.questrade.lock() {
        *guard = None;
    }
    Ok(())
}

/// Force a fresh read-only portfolio fetch (refreshing the session if needed).
#[tauri::command]
pub async fn questrade_portfolio(state: State<'_, AppState>) -> AppResult<Portfolio> {
    questrade_fetch_and_cache(&state).await
}

/// Pull a read-only portfolio via the Questrade REST client and cache it.
async fn questrade_fetch_and_cache(state: &AppState) -> AppResult<Portfolio> {
    let (access_token, api_server) = match questrade::ensure_session(&state.http).await {
        Ok(session) => session,
        Err(err @ AppError::QuestradeNotConnected) => {
            if let Ok(conn) = state.lock_db() {
                let _ = db::set_flag(&conn, db::FLAG_QUESTRADE_CONNECTED, false);
            }
            return Err(err);
        }
        Err(err) => return Err(err),
    };
    let client = QuestradeClient::new(state.http.clone(), access_token, api_server);
    let portfolio = client.fetch_portfolio().await?;
    if let Ok(mut guard) = state.questrade.lock() {
        *guard = Some(portfolio.clone());
    }
    Ok(portfolio)
}

// --- Buy routing (read-only: resolve listings, never place orders) ----------

/// Resolve the listings a Buy action needs for a pick: the US/base symbol with
/// its exchange and, when one exists, a same-security Canadian interlisting so
/// Canadian brokers can trade it in CAD without an FX conversion.
#[tauri::command]
pub async fn resolve_listings(
    state: State<'_, AppState>,
    symbol: String,
    company: String,
) -> AppResult<ListingInfo> {
    feeds::resolve_listings(&state.http, &symbol, &company).await
}

/// Whether a ticker is an active, tradable Robinhood listing (public lookup, no
/// connected account required). Lets the UI hide Robinhood for symbols it can't
/// trade rather than deep-linking to a dead page.
#[tauri::command]
pub async fn robinhood_symbol_available(
    state: State<'_, AppState>,
    symbol: String,
) -> AppResult<bool> {
    robinhood::symbol_available(&state.http, &symbol).await
}

/// Find the best tradable Questrade listing for a ticker, preferring a Canadian
/// (CAD) listing of the same security. Errors when Questrade isn't connected so
/// the frontend can fall back to its market-based heuristic.
#[tauri::command]
pub async fn questrade_find_listing(
    state: State<'_, AppState>,
    symbol: String,
) -> AppResult<Option<Listing>> {
    {
        let conn = state.lock_db()?;
        if !db::get_flag(&conn, db::FLAG_QUESTRADE_CONNECTED)? {
            return Err(AppError::QuestradeNotConnected);
        }
    }
    let (access_token, api_server) = questrade::ensure_session(&state.http).await?;
    let client = QuestradeClient::new(state.http.clone(), access_token, api_server);
    client.find_listing(&symbol).await
}

/// Shared body for both the ad-hoc prompt and watchlist re-runs: load settings,
/// confirm Ollama is ready (so failures are one clear message), then stream the
/// pipeline's progress to the frontend channel.
async fn execute(
    state: &AppState,
    prompt: &str,
    on_event: &Channel<ProgressEvent>,
) -> AppResult<ResearchResult> {
    if prompt.trim().is_empty() {
        return Err(AppError::Other("Please enter a question first.".into()));
    }

    let settings = {
        let conn = state.lock_db()?;
        db::load_settings(&conn)?
    };

    let ollama = OllamaClient::new(
        state.http.clone(),
        settings.ollama_endpoint.clone(),
        settings.model.clone(),
    );

    if let Err(err) = ollama.ensure_ready().await {
        let _ = on_event.send(ProgressEvent::Failed {
            kind: err.kind().to_string(),
            message: err.to_string(),
        });
        return Err(err);
    }

    let emit = |event: ProgressEvent| {
        let _ = on_event.send(event);
    };

    // Read-only portfolio enrichment: badge picks the user already holds in
    // either connected broker. Pulled from the last cached snapshots so a
    // research run never blocks on a broker.
    let mut owned: BTreeSet<String> = BTreeSet::new();
    for cache in [&state.robinhood, &state.questrade] {
        if let Some(p) = cache.lock().ok().and_then(|guard| guard.clone()) {
            owned.extend(p.owned_tickers());
        }
    }

    match research::run_research(&ollama, &state.http, &settings, prompt, &owned, &emit).await {
        Ok(result) => Ok(result),
        Err(err) => {
            let _ = on_event.send(ProgressEvent::Failed {
                kind: err.kind().to_string(),
                message: err.to_string(),
            });
            Err(err)
        }
    }
}
