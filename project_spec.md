# Project Specification: TrendWave

## 1. Project Overview & Philosophy
TrendWave is a background-first, cross-platform desktop application built to detect early stock momentum (volume spikes, unusual activity) in specific tech micro-sectors (e.g., photonics, solid-state batteries). 
- **The Core Philosophy:** The app should be invisible 99% of the time, sipping minimal system resources, and only surface when a distinct data anomaly is detected.
- **Pedagogical Goal:** This project is being built to learn **Rust**. The AI assistant must act as a senior Rust developer mentoring a junior developer, explaining *why* decisions are made regarding memory, ownership, and async patterns.

## 2. Tech Stack & Libraries
- **App Framework:** Tauri v2
- **Backend (Core Logic):** Rust
  - `tokio`: Async runtime for background polling.
  - `reqwest`: For making HTTP calls to financial APIs.
  - `rusqlite`: For local SQLite database operations.
  - `serde` & `serde_json`: For parsing API data and IPC payload serialization.
  - `tauri-plugin-sql`: (Optional, but prefer raw `rusqlite` for learning Rust DB management).
- **Frontend (Dashboard):** React, TypeScript, Tailwind CSS, Lucide Icons.
- **Data Source:** Free tier APIs (e.g., `yfinance` unofficial endpoints, or Alpaca free tier).

## 3. Architecture & Data Flow
The application is strictly separated into a Rust backend and a React frontend, communicating exclusively via Tauri's IPC (Inter-Process Communication) commands.

### Phase 1: The Headless Foundation
- App launches without a standard window.
- Initializes a macOS Menu Bar / Windows System Tray icon.
- Tray menu options: "Open Dashboard", "Pause Scanning", "Settings", "Quit".

### Phase 2: State & Storage (The SQLite Brain)
- Initialize a local `trendwave.db` file in the user's app data directory.
- **Schema Requirements:**
  - `tickers`: id, symbol, micro_sector, is_active.
  - `daily_metrics`: id, ticker_id, date, volume, close_price.
  - `alerts`: id, ticker_id, timestamp, trigger_reason, is_read.
- **State Management:** Use Tauri's managed state (`tauri::State`) combined with a `Mutex` or `RwLock` to share the database connection pool across background threads and IPC commands safely.

### Phase 3: The Background Worker (Tokio)
- A spawned `tokio` task that runs an infinite loop.
- **Interval:** Wakes up every 15 minutes during market hours.
- **Action:** Iterates through active tickers, fetches current volume/price via `reqwest`, and compares it to a 10-day moving average stored in SQLite.
- **Trigger:** If volume > 300% of average, insert an `alert` into the DB and trigger a native OS notification.

### Phase 4: The Frontend Dashboard
- Opened via the Tray Icon.
- **UI Components:**
  1. **Sector Heatmap:** A visual representation of micro-sectors showing aggregate volume momentum.
  2. **Alert Feed:** A chronological list of triggered alerts.
  3. **Settings:** Input fields for API keys and polling intervals.
- Fetches data by invoking Tauri commands (e.g., `invoke('get_recent_alerts')`).

## 4. Strict AI & Coding Guidelines
1. **Explain the Borrow Checker:** When writing functions that pass strings, database connections, or state, explicitly comment on why you used `.clone()`, references (`&`), or `Arc<Mutex<T>>`.
2. **Error Handling:** DO NOT use `.unwrap()` in production code. Use the `?` operator and create custom Rust Error enums (`thiserror` crate is permitted) that can be serialized to the frontend.
3. **Pacing:** Never write more than one Phase or feature at a time. Await explicit user approval before moving to the next step.
4. **Idiomatic Rust:** Prefer pattern matching (`match`, `if let`) over deeply nested conditionals.
