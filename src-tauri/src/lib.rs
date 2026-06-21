mod biometric;
mod commands;
mod db;
mod error;
mod feeds;
mod fundamentals;
mod mcp;
mod model;
mod oauth;
mod ollama;
mod onboarding;
mod questrade;
mod research;
mod robinhood;
mod settings;

use std::sync::Mutex;
use std::time::Duration;

use tauri::Manager;

use commands::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            // Keep all state local-first: the SQLite file lives in the OS app
            // data directory for this bundle identifier.
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let conn = rusqlite::Connection::open(data_dir.join("trendwave.db"))?;
            db::init(&conn)?;

            // One-time migration: default the biometric gate ON for existing
            // installs (fresh installs already default it on). Tracked by a flag
            // outside the Settings blob so a later opt-out via Settings isn't
            // re-forced on the next launch.
            if !db::get_flag(&conn, db::FLAG_BIO_DEFAULT_MIGRATED)? {
                let mut settings = db::load_settings(&conn)?;
                settings.require_biometric_unlock = true;
                db::save_settings(&conn, &settings)?;
                db::set_flag(&conn, db::FLAG_BIO_DEFAULT_MIGRATED, true)?;
            }

            // One-time marker backfill: broker status now reads SQLite connection
            // markers (set only on connect/disconnect) instead of the keychain.
            // Installs that were already connected before that change have a stored
            // token but no marker, which left the portfolio sidebar empty. Reconcile
            // once from whether a credential exists. `load_auth()` returns `None`
            // without prompting when there's no keychain entry, so never-connected
            // and fresh installs stay prompt-free; only a genuinely connected user
            // sees a single one-time keychain prompt here, after which status is
            // marker-only again.
            if !db::get_flag(&conn, db::FLAG_MARKERS_BACKFILLED)? {
                db::set_flag(
                    &conn,
                    db::FLAG_ROBINHOOD_CONNECTED,
                    oauth::load_auth().is_some(),
                )?;
                db::set_flag(
                    &conn,
                    db::FLAG_QUESTRADE_CONNECTED,
                    questrade::load_auth().is_some(),
                )?;
                db::set_flag(&conn, db::FLAG_MARKERS_BACKFILLED, true)?;
            }

            // One shared HTTP client. A short connect timeout makes a dead Ollama
            // (or offline machine) fail fast, while a long overall timeout leaves
            // room for slow local-model generation.
            let http = reqwest::Client::builder()
                .user_agent("TrendWave/0.1 (local research tool)")
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(180))
                .build()?;

            app.manage(AppState {
                db: Mutex::new(conn),
                http,
                robinhood: Mutex::new(None),
                unlocked: std::sync::atomic::AtomicBool::new(false),
                questrade: Mutex::new(None),
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::run_research,
            commands::run_watchlist,
            commands::get_settings,
            commands::save_settings,
            commands::list_watchlists,
            commands::create_watchlist,
            commands::delete_watchlist,
            commands::robinhood_status,
            commands::robinhood_connect,
            commands::robinhood_disconnect,
            commands::robinhood_portfolio,
            commands::biometric_available,
            commands::biometric_unlock,
            commands::questrade_status,
            commands::questrade_connect,
            commands::questrade_disconnect,
            commands::questrade_portfolio,
            commands::resolve_listings,
            commands::robinhood_symbol_available,
            commands::questrade_find_listing,
        ])
        .run(tauri::generate_context!());

    if let Err(error) = result {
        eprintln!("TrendWave failed to start: {error}");
        std::process::exit(1);
    }
}
