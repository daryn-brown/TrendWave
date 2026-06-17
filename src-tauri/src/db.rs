use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::model::ResearchResult;
use crate::settings::Settings;

/// A saved query the user can re-run with one click. We cache the last result
/// JSON so reopening a watchlist shows something instantly before any re-run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watchlist {
    pub id: i64,
    pub name: String,
    pub prompt: String,
    pub last_result: Option<ResearchResult>,
    pub last_run_at: Option<String>,
    pub created_at: String,
}

/// Create the schema if needed. Idempotent, so it is safe to call on every boot.
pub fn init(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS settings (
            id   INTEGER PRIMARY KEY CHECK (id = 1),
            data TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS watchlists (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            name        TEXT NOT NULL,
            prompt      TEXT NOT NULL,
            last_result TEXT,
            last_run_at TEXT,
            created_at  TEXT NOT NULL
        );",
    )?;
    Ok(())
}

pub fn load_settings(conn: &Connection) -> AppResult<Settings> {
    let mut stmt = conn.prepare("SELECT data FROM settings WHERE id = 1")?;
    let mut rows = stmt.query([])?;
    match rows.next()? {
        Some(row) => {
            let data: String = row.get(0)?;
            Ok(serde_json::from_str(&data).unwrap_or_default())
        }
        None => Ok(Settings::default()),
    }
}

pub fn save_settings(conn: &Connection, settings: &Settings) -> AppResult<()> {
    let data = serde_json::to_string(settings)?;
    conn.execute(
        "INSERT INTO settings (id, data) VALUES (1, ?1)
         ON CONFLICT(id) DO UPDATE SET data = excluded.data",
        [data],
    )?;
    Ok(())
}

pub fn list_watchlists(conn: &Connection) -> AppResult<Vec<Watchlist>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, prompt, last_result, last_run_at, created_at
         FROM watchlists ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map([], row_to_watchlist)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn create_watchlist(conn: &Connection, name: &str, prompt: &str) -> AppResult<Watchlist> {
    let created_at = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO watchlists (name, prompt, created_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![name, prompt, created_at],
    )?;
    let id = conn.last_insert_rowid();
    Ok(Watchlist {
        id,
        name: name.to_string(),
        prompt: prompt.to_string(),
        last_result: None,
        last_run_at: None,
        created_at,
    })
}

pub fn delete_watchlist(conn: &Connection, id: i64) -> AppResult<()> {
    conn.execute("DELETE FROM watchlists WHERE id = ?1", [id])?;
    Ok(())
}

/// Cache the latest research output against a watchlist after a re-run.
pub fn update_watchlist_result(
    conn: &Connection,
    id: i64,
    result: &ResearchResult,
) -> AppResult<()> {
    let data = serde_json::to_string(result)?;
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE watchlists SET last_result = ?1, last_run_at = ?2 WHERE id = ?3",
        rusqlite::params![data, now, id],
    )?;
    Ok(())
}

pub fn get_watchlist(conn: &Connection, id: i64) -> AppResult<Watchlist> {
    let mut stmt = conn.prepare(
        "SELECT id, name, prompt, last_result, last_run_at, created_at
         FROM watchlists WHERE id = ?1",
    )?;
    let mut rows = stmt.query([id])?;
    match rows.next()? {
        Some(row) => Ok(row_to_watchlist(row)?),
        None => Err(AppError::Other(format!("watchlist {id} not found"))),
    }
}

fn row_to_watchlist(row: &rusqlite::Row<'_>) -> rusqlite::Result<Watchlist> {
    let last_result: Option<String> = row.get(3)?;
    Ok(Watchlist {
        id: row.get(0)?,
        name: row.get(1)?,
        prompt: row.get(2)?,
        last_result: last_result.and_then(|s| serde_json::from_str(&s).ok()),
        last_run_at: row.get(4)?,
        created_at: row.get(5)?,
    })
}
