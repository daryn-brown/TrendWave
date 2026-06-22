# Project Specification: TrendWave

## 1. Overview & philosophy
TrendWave is a **local-first, prompt-first** desktop research tool for finding the public companies
best positioned to solve or monopolize **supply-chain / capacity bottlenecks** in a given industry.

- **Bottleneck-first:** the primary signal is *where the chokepoints are* — scarce components,
  limited production capacity, single-source suppliers, permitting/logistics constraints — not
  generic momentum or news volume.
- **Local & free by default:** all reasoning runs on the user's machine via Ollama. No API keys, no
  per-token cost. Network calls are to free public price/news/filing endpoints and the local Ollama
  server. A power user may *optionally* slot in a bring-your-own-key paid data source behind an
  abstraction; the key lives in the OS keychain and the app is fully functional without it.
- **Early-detection first:** the default ranking blends competitive positioning with forward signals
  (cyclical inflection, cycle timing, estimate revisions, insider buying, filing evidence, room-to-run
  size) and a screener that discovers names the prompt never mentioned — so cyclicals are flagged near the bottom
  of their cycle. A `Legacy` mode restores the original trailing-fundamentals ranking byte-for-byte.
- **Prompt-first, calm UX:** one window. Ask a question, watch progress stream, get a ranked
  shortlist. No always-on daemon, no noisy dashboard.

> Not financial advice. Output is heuristic and must be verified against sources.

## 2. Tech stack
- **App framework:** Tauri v2
- **Backend (Rust):** `tokio` (async pipeline), `reqwest` (HTTP, rustls, cookie jar for the Yahoo
  crumb handshake), `rusqlite` (SQLite, bundled), `feed-rs` (RSS parsing), `serde`/`serde_json`,
  `thiserror` (typed errors), `chrono`.
- **Intelligence:** local Ollama (`/api/chat` with `format: "json"`), default model `llama3.1:8b`.
- **Frontend:** React, TypeScript, Tailwind CSS, Vite.
- **Data sources (free, no key):** Yahoo Finance chart (prices/volume), search (name → ticker),
  RSS (headlines), screeners (candidate discovery), and `quoteSummary` (forward P/E, analyst
  targets & estimate trends, via a cookie+crumb handshake, best-effort); **SEC EDGAR**
  `companyconcept`/`companyfacts` (audited annual **and quarterly** revenue & earnings), full-text
  search (`efts.sec.gov`), and `submissions` (insider Form 4 + 8-K/10-Q filings).
- **Optional paid provider (BYO key):** a `DataProvider` seam (free default + reference Financial
  Modeling Prep adapter) lets a user supply their own key for cleaner estimate data; the key is read
  from the OS keychain, paid calls fall back to the free path on any error, and paid data is never
  persisted to saved-watchlist caches.

## 3. Architecture & data flow
A Rust backend and a React frontend communicate exclusively over Tauri IPC. Long-running research is
an async pipeline that streams progress to the UI over a Tauri `Channel`.

```
prompt ─▶ run_research (command)
            │  load settings, ensure Ollama ready
            ▼
        research::run_research
            ├─ identify bottlenecks + companies positioned to win  (Ollama, JSON)
            ├─ discover extra candidates                    (EDGAR full-text + Yahoo screeners)
            ├─ resolve + price each ticker                  (free feeds, concurrent)
            ├─ keep every named pick (price is context, not a filter)
            ├─ research growth + inflection                 (SEC EDGAR annual & quarterly + revisions)
            ├─ cycle timing, insider buys, filing signals   (Yahoo charts + EDGAR, best-effort)
            ├─ room-to-run / convexity                      (market-cap sweet spot + liquidity)
            ├─ fetch news + score sentiment                 (RSS + Ollama)
            ├─ scoring + ranking (Legacy | Early detection)
            └─ diff vs previous run                         (what changed)
            ▼
        ResearchResult ─▶ frontend (streamed events + final payload)
```

### Backend modules
- `commands.rs` — Tauri commands and shared `AppState` (`Mutex<Connection>` + `reqwest::Client`).
  The DB lock is only held for short synchronous queries, never across `.await`.
- `research.rs` — the pipeline orchestrator (enrichment fan-out, ranking, change diff).
- `scoring.rs` — `ScoringMode` (`Legacy`/`EarlyDetection`) + `ScoringWeights` + the pure
  `composite_score`; Legacy weights reduce to the original formula exactly.
- `providers.rs` — data-provider abstraction: `Free` default + reference `Fmp` BYO-key adapter,
  OS-keychain key storage, graceful fallback to free.
- `screener.rs` — candidate discovery via EDGAR full-text search + Yahoo predefined screeners.
- `inflection.rs` — quarterly-EDGAR cyclical inflection (trough/acceleration/margin) + estimate
  revisions; pure `inflection_score` / `revisions_score`.
- `technical.rs` — multi-timeframe momentum + relative strength → timing label
  (Early/Building/Extended/Late) and pure `technical_score`.
- `filings.rs` — one `submissions` fetch reused for insider Form 4 purchases + 8-K/10-Q keyword
  signals; pure scorers.
- `convexity.rs` — room-to-run / convexity: a log-space band-pass over market cap (small/mid-cap
  sweet spot) gated by a liquidity floor; pure `convexity_score`, neutral when size is unknown.
- `changes.rs` — pure `diff_runs` (new entrants / drops / rank & score moves / timing shifts).
- `ollama.rs` — minimal local Ollama client: `ensure_ready` and `generate_json`.
- `feeds.rs` — `fetch_price`, `resolve_symbol`, `fetch_news`.
- `fundamentals.rs` — real growth research: SEC EDGAR backbone + opportunistic Yahoo enrichment;
  pure `growth_score`.
- `db.rs` — schema + CRUD for settings and watchlists (with cached last result).
- `model.rs` — shared serializable types (`Bottleneck`, `Candidate`, `GrowthData`, `RunChanges`,
  `ResearchResult`, `ProgressEvent`).
- `settings.rs` — user settings with defaults (`scoring_mode`, `data_provider`, …).
- `error.rs` — `AppError` enum, serialized to the frontend as `{ kind, message }`.
- `backtest.rs` — `#[cfg(test)]` point-in-time fixtures (Micron @ 2023 trough, SanDisk @ 2025
  relisting vs. laggards) asserting Early detection ranks the eventual winners first.

### Scoring
Composite score in `0..100`, computed by a configurable weighted blend with two presets that both
sum to 100. Share price is never a factor.

**Legacy** (default-off; restores the original behavior byte-for-byte):

```
score = 25 * (severity / 5)            // how acute the bottleneck is
      + 25 * (moat / 5)                // how dominant / monopoly-like the position is
      + 35 * growth_score              // data-derived growth (EDGAR + Yahoo), 0..1
      + 10 * ((sentiment + 1) / 2)     // news sentiment, neutral when unknown
      +  5 * momentum                  // recent price change, clamped
```

**Early detection** (default): trailing growth is reduced but retained, positioning stays strong,
and forward signals carry real weight —
`severity 20 · moat 20 · growth 10 · sentiment 4 · momentum 2 · inflection 16 · technical
8 · revisions 6 · insider 2 · filing 2 · convexity 10`. Each optional signal sits **neutral (0.5)** when its data
is unavailable, so a missing feed never penalizes a pick. A per-pick `SignalBreakdown` records each
term's contribution for "why this ranked here" explainability. Forward signals are only fetched in
Early-detection mode, so Legacy makes the exact same network calls and produces identical output.

### Commands (IPC)
- `run_research(prompt, on_event)` → streams `ProgressEvent`s, returns `ResearchResult`.
- `run_watchlist(id, on_event)` → re-runs a saved prompt, diffs against the cached result, caches
  the new one.
- `get_settings` / `save_settings`.
- `list_watchlists` / `create_watchlist` / `delete_watchlist`.
- `data_provider_status` / `data_provider_set_key` / `data_provider_clear_key` — manage the optional
  paid-provider key (keychain-backed; status is flag-backed so opening Settings never prompts).

### Persistence
Local SQLite (`trendwave.db` in the OS app-data dir):
- `settings` — single JSON row.
- `watchlists` — id, name, prompt, cached `last_result`, `last_run_at`, `created_at`.

### Frontend
A single prompt window: prompt bar, streaming progress log, identified-bottleneck cards, ranked
pick cards (ticker, price for context, positioning thesis, moat rating, a **growth-research panel**,
a **cycle-timing badge**, a **discovery badge**, an expandable **"why this score" breakdown**,
sentiment, news links), a **"what changed since last run"** panel, a saved-watchlist sidebar, and a
settings modal (ranking mode, market-data source + key management, toggles). Types in
`src/types.ts` mirror the Rust models.

## 4. Coding guidelines
1. **Typed errors, no panics:** no `.unwrap()` / `.expect()` on fallible paths in production code;
   use `?` and `AppError`. The DB mutex is locked only for short synchronous spans.
2. **Keep the model honest:** constrain LLM output to JSON and deserialize into typed structs;
   always surface sources so the user can verify a thesis.
3. **Local-first:** never send the user's prompts or results to a third party. Outbound calls are
   limited to free public endpoints (Yahoo Finance, SEC EDGAR) and the local Ollama server by
   default. A paid data source is strictly opt-in behind the `DataProvider` seam: its key lives in
   the OS keychain, it only fetches market/estimate data (never user prompts), and any failure falls
   back to the free path so a run never breaks.
4. **Idiomatic Rust:** prefer pattern matching and small, testable pure functions (e.g. scoring) so
   logic can be tested without the network.

## 5. Status
- [x] Prompt-first window with streaming research
- [x] Local Ollama reasoning (bottlenecks, sentiment)
- [x] Free price + news feeds
- [x] Bottleneck-weighted ranking
- [x] Real growth research (SEC EDGAR fundamentals + Yahoo enrichment) driving the growth score
- [x] Early-detection ranking: discovery (EDGAR full-text + Yahoo screeners), cyclical inflection &
  estimate revisions, cycle timing, insider & filing signals, room-to-run / convexity size lens,
  with a `Legacy` byte-identical mode
- [x] Optional BYO-key paid data provider (keychain-stored, free fallback)
- [x] On-demand change detection ("what changed since last run")
- [x] Backtest fixtures (Micron @ 2023 trough, SanDisk @ 2025 relisting) validating early detection
- [x] SQLite persistence + saved watchlists
- [x] Settings (model, thresholds, toggles, ranking mode, data provider)
