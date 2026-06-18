use std::collections::BTreeSet;
use std::sync::{Mutex, MutexGuard};

use rusqlite::Connection;
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::db::{self, Watchlist};
use crate::error::{AppError, AppResult};
use crate::model::{ProgressEvent, ResearchResult};
use crate::oauth;
use crate::ollama::OllamaClient;
use crate::research;
use crate::robinhood::{Portfolio, RobinhoodClient};
use crate::settings::Settings;

/// Shared application state managed by Tauri. The SQLite connection lives behind
/// a `Mutex` (rusqlite's `Connection` is not `Sync`); we only ever hold the lock
/// for short, synchronous queries and never across an `.await`. The HTTP client
/// is internally ref-counted and cheap to clone for each async task. `robinhood`
/// caches the last read-only portfolio snapshot so research can badge owned
/// tickers without a network round-trip.
pub struct AppState {
    pub db: Mutex<Connection>,
    pub http: reqwest::Client,
    pub robinhood: Mutex<Option<Portfolio>>,
}

impl AppState {
    fn lock_db(&self) -> AppResult<MutexGuard<'_, Connection>> {
        self.db
            .lock()
            .map_err(|_| AppError::Database("database lock was poisoned".into()))
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
#[derive(serde::Serialize)]
pub struct RobinhoodStatus {
    pub connected: bool,
    pub portfolio: Option<Portfolio>,
}

#[tauri::command]
pub async fn robinhood_status(state: State<'_, AppState>) -> AppResult<RobinhoodStatus> {
    let portfolio = state
        .robinhood
        .lock()
        .ok()
        .and_then(|guard| guard.clone());
    Ok(RobinhoodStatus {
        connected: oauth::is_connected(),
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

    let portfolio = fetch_and_cache(&state).await.ok();
    Ok(RobinhoodStatus {
        connected: true,
        portfolio,
    })
}

#[tauri::command]
pub async fn robinhood_disconnect(state: State<'_, AppState>) -> AppResult<()> {
    oauth::clear_auth()?;
    if let Ok(mut guard) = state.robinhood.lock() {
        *guard = None;
    }
    Ok(())
}

/// Force a fresh read-only portfolio fetch (refreshing the token if needed).
#[tauri::command]
pub async fn robinhood_portfolio(state: State<'_, AppState>) -> AppResult<Portfolio> {
    fetch_and_cache(&state).await
}

/// Pull a read-only portfolio via the MCP client and cache it for enrichment.
async fn fetch_and_cache(state: &AppState) -> AppResult<Portfolio> {
    let token = oauth::ensure_access_token(&state.http).await?;
    let client = RobinhoodClient::new(state.http.clone(), token);
    let portfolio = client.fetch_portfolio().await?;
    if let Ok(mut guard) = state.robinhood.lock() {
        *guard = Some(portfolio.clone());
    }
    Ok(portfolio)
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

    // Read-only Robinhood enrichment: badge picks the user already holds. Pulled
    // from the last cached snapshot so a research run never blocks on the broker.
    let owned: BTreeSet<String> = state
        .robinhood
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .map(|p| p.owned_tickers())
        .unwrap_or_default();

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
