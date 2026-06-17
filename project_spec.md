# Project Specification: TrendWave

## 1. Overview & philosophy
TrendWave is a **local-first, prompt-first** desktop research tool for finding the public companies
best positioned to solve or monopolize **supply-chain / capacity bottlenecks** in a given industry.

- **Bottleneck-first:** the primary signal is *where the chokepoints are* — scarce components,
  limited production capacity, single-source suppliers, permitting/logistics constraints — not
  generic momentum or news volume.
- **Local & free:** all reasoning runs on the user's machine via Ollama. No API keys, no per-token
  cost. The only network calls are to free public price/news endpoints and the local Ollama server.
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
  RSS (headlines), and `quoteSummary` (forward P/E & analyst targets, via a cookie+crumb handshake,
  best-effort); **SEC EDGAR** `companyconcept` (audited multi-year revenue & earnings).

## 3. Architecture & data flow
A Rust backend and a React frontend communicate exclusively over Tauri IPC. Long-running research is
an async pipeline that streams progress to the UI over a Tauri `Channel`.

```
prompt ─▶ run_research (command)
            │  load settings, ensure Ollama ready
            ▼
        research::run_research
            ├─ identify bottlenecks + companies positioned to win  (Ollama, JSON)
            ├─ resolve + price each ticker                  (free feeds, concurrent)
            ├─ keep every named pick (price is context, not a filter)
            ├─ research real growth (SEC EDGAR + Yahoo)      (concurrent, best-effort)
            ├─ fetch news + score sentiment                 (RSS + Ollama)
            └─ growth + positioning scoring + ranking
            ▼
        ResearchResult ─▶ frontend (streamed events + final payload)
```

### Backend modules
- `commands.rs` — Tauri commands and shared `AppState` (`Mutex<Connection>` + `reqwest::Client`).
  The DB lock is only held for short synchronous queries, never across `.await`.
- `research.rs` — the pipeline and the pure, unit-tested `score_candidate` function.
- `ollama.rs` — minimal local Ollama client: `ensure_ready` (health + model check) and
  `generate_json` (typed, validated output).
- `feeds.rs` — `fetch_price`, `resolve_symbol`, `fetch_news`.
- `fundamentals.rs` — real growth research: SEC EDGAR backbone (ticker→CIK, annual revenue &
  net-income → YoY/CAGR/profitability) + opportunistic Yahoo enrichment; pure `growth_score`.
- `db.rs` — schema + CRUD for settings and watchlists (with cached last result).
- `model.rs` — shared serializable types (`Bottleneck`, `Candidate`, `GrowthData`, `ResearchResult`,
  `ProgressEvent`).
- `settings.rs` — user settings with defaults.
- `error.rs` — `AppError` enum, serialized to the frontend as `{ kind, message }`.

### Scoring
Composite score in `0..100`. **Real growth potential is the single largest factor** and is derived
from audited data, not the model; competitive positioning (severity + moat) comes next. Share price
is never a factor:

```
score = 25 * (severity / 5)            // how acute the bottleneck is
      + 25 * (moat / 5)                // how dominant / monopoly-like the position is
      + 35 * growth_score              // data-derived growth (EDGAR + Yahoo), 0..1
      + 10 * ((sentiment + 1) / 2)     // news sentiment, neutral when unknown
      +  5 * momentum                  // recent price change, clamped
```

`growth_score` (`0..1`) weights revenue growth (YoY + CAGR) highest, then earnings trend /
profitability, then Yahoo analyst-target upside; it sits neutral at `0.5` when no fundamentals are
available. With the *Research real growth* setting off, it falls back to the model's upside guess
(`upside / 5`), preserving the legacy behavior.

### Commands (IPC)
- `run_research(prompt, on_event)` → streams `ProgressEvent`s, returns `ResearchResult`.
- `run_watchlist(id, on_event)` → re-runs a saved prompt and caches the result.
- `get_settings` / `save_settings`.
- `list_watchlists` / `create_watchlist` / `delete_watchlist`.

### Persistence
Local SQLite (`trendwave.db` in the OS app-data dir):
- `settings` — single JSON row.
- `watchlists` — id, name, prompt, cached `last_result`, `last_run_at`, `created_at`.

### Frontend
A single prompt window: prompt bar, streaming progress log, identified-bottleneck cards, ranked
pick cards (ticker, price for context, positioning thesis, moat rating, a **growth-research panel**
with real revenue/earnings growth and data provenance, sentiment, news links), a saved-watchlist
sidebar, and a settings modal. Types in `src/types.ts` mirror the Rust models.

## 4. Coding guidelines
1. **Typed errors, no panics:** no `.unwrap()` / `.expect()` on fallible paths in production code;
   use `?` and `AppError`. The DB mutex is locked only for short synchronous spans.
2. **Keep the model honest:** constrain LLM output to JSON and deserialize into typed structs;
   always surface sources so the user can verify a thesis.
3. **Local-first:** never introduce a paid API or send user prompts to a third party. Outbound calls
   are limited to free public endpoints (Yahoo Finance, SEC EDGAR) and the local Ollama server.
4. **Idiomatic Rust:** prefer pattern matching and small, testable pure functions (e.g. scoring) so
   logic can be tested without the network.

## 5. Status
- [x] Prompt-first window with streaming research
- [x] Local Ollama reasoning (bottlenecks, sentiment)
- [x] Free price + news feeds
- [x] Bottleneck-weighted ranking
- [x] Real growth research (SEC EDGAR fundamentals + Yahoo enrichment) driving the growth score
- [x] SQLite persistence + saved watchlists
- [x] Settings (model, thresholds, toggles)
