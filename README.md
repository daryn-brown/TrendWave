# TrendWave 📈🌊

**Ask where an industry's bottlenecks are. Get the stocks best positioned to solve or monopolize them.**

TrendWave is a **local-first, prompt-first** desktop research tool. You type a question like
*"Where are the bottlenecks in the AI data-center buildout?"* and it quietly does the legwork —
identifies the real supply-chain chokepoints, finds the public companies best positioned to solve or
monopolize them, prices them for context, scans recent news for sentiment, and hands back a ranked
shortlist with the full thesis.

All the reasoning runs **on your machine** through [Ollama](https://ollama.com). No API keys, no
per-token costs, no data leaving your laptop except public price/news lookups.

> ⚠️ **Not financial advice.** TrendWave is a research aid. Its signals are heuristic and can be
> wrong. Verify every thesis against the linked sources before making any decision.

## How it works 🧠

```mermaid
flowchart LR
    A[Your prompt] --> B[Identify bottlenecks<br/>Ollama]
    B --> C[Resolve tickers]
    C --> D[Price feeds<br/>Yahoo / free]
    D --> E[News + sentiment<br/>RSS + Ollama]
    E --> F[Positioning + upside<br/>ranking]
    F --> G[Ranked stock picks]
```

1. **Identify bottlenecks** — the local model reasons about current chokepoints (scarce components,
   limited capacity, single-source suppliers, logistics constraints) and which public companies are
   best positioned to solve or monopolize them.
2. **Validate & price** — proposed tickers are checked against free price feeds and priced for
   context. Price is never a filter — large caps are welcome if they're the dominant beneficiary.
3. **News & sentiment** — recent headlines are pulled per ticker and scored locally by the model.
4. **Rank** — a transparent score weights **competitive positioning (bottleneck severity + moat) and
   upside highest**, with sentiment and momentum as tie-breakers.

Progress streams live to the UI, and you can **save any search as a watchlist** to re-run with one
click later.

## Tech stack 🛠️

- **Shell:** Tauri v2 (Rust core + web frontend)
- **Backend:** Rust — `tokio` (async pipeline), `reqwest` (HTTP), `rusqlite` (local SQLite),
  `feed-rs` (RSS), `thiserror` (typed errors)
- **Intelligence:** local Ollama model (default `llama3.1:8b`)
- **Frontend:** React + TypeScript + Tailwind CSS, built with Vite
- **Updates:** Tauri updater + process plugins, signed releases via GitHub Actions
- **Data:** free public endpoints (Yahoo Finance chart + RSS). No keys required.

## Getting started 🚀

### Prerequisites

- Node.js and npm
- Rust toolchain
- [Ollama](https://ollama.com) installed and running
- Tauri system prerequisites for your platform

### 1. Start Ollama and pull a model

```bash
ollama serve              # if it isn't already running
ollama pull llama3.1:8b   # or any instruction-following model
```

### 2. Run the app

```bash
npm install
npm run tauri dev
```

TrendWave opens to a single prompt window. Type a question (or click an example), and watch it work.

### Other commands

```bash
npm run build                                   # build the frontend only
cargo test --manifest-path src-tauri/Cargo.toml # run the Rust unit tests
cargo check --manifest-path src-tauri/Cargo.toml
```

## Install & automatic updates ⬇️

TrendWave ships as a **self-updating desktop app** — install it once and it keeps itself current.

### Install

Grab the latest `.dmg` from the
[Releases page](https://github.com/daryn-brown/TrendWave/releases/latest), open it, and drag
**TrendWave** into **Applications**. The app isn't signed with an Apple Developer certificate, so the
first launch needs a one-time approval: **right-click the app → Open → Open** (or run
`xattr -dr com.apple.quarantine /Applications/TrendWave.app`).

### Updating

Every merge to `main` triggers the [`release` workflow](.github/workflows/release.yml), which builds a
**signed universal macOS bundle** and publishes it as a GitHub Release. The running app checks that
release on launch — and whenever you click **Check for updates** in the sidebar. When a newer build
exists you get an **Update available** banner: click **Download &amp; install**, then **Restart now**.
Each update is verified against an embedded public key before it is applied.

### Maintainer setup (one-time)

Updates are signed with a [minisign](https://jedisct1.github.io/minisign/) keypair created by
`npm run tauri signer generate`. The **public** key lives in `src-tauri/tauri.conf.json`; the
**private** key and its password are stored as the repo secrets `TAURI_SIGNING_PRIVATE_KEY` and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (used by the workflow). Keep the private key safe and out of
version control — without it you can't publish updates that installed apps will accept.

## Settings ⚙️

Open **Settings** from the sidebar to tune:

| Setting | Default | Meaning |
|---|---|---|
| Ollama model | `llama3.1:8b` | Any locally installed model |
| Ollama endpoint | `http://localhost:11434` | Where the local server listens |
| Max results | `8` | Cap on returned picks |
| Scan news & sentiment | on | Pull headlines and score sentiment (slower) |

Settings and watchlists persist in a local SQLite file (`trendwave.db`) in your OS app-data
directory.

## Project structure 🗂️

```text
TrendWave/
├── project_spec.md      # Architecture and design notes
├── src/                 # React frontend (prompt UI)
│   ├── App.tsx          # Orchestration + layout
│   ├── components.tsx   # Presentational components
│   ├── api.ts           # Typed Tauri command bridge
│   ├── updater.ts       # In-app auto-update helpers
│   └── types.ts         # Shared types (mirror of the Rust models)
├── src-tauri/src/       # Rust backend
│   ├── lib.rs           # App bootstrap, state, command registration
│   ├── commands.rs      # Tauri IPC commands
│   ├── research.rs      # The bottleneck → ranking pipeline
│   ├── ollama.rs        # Local Ollama client
│   ├── feeds.rs         # Free price + news feeds
│   ├── db.rs            # SQLite persistence
│   ├── model.rs         # Shared serializable types
│   ├── settings.rs      # User settings
│   └── error.rs         # Typed, serializable errors
├── .github/workflows/
│   └── release.yml      # Build + publish signed auto-update releases
└── README.md
```

## Privacy 🔒

TrendWave is local-first by design. Your prompts and results never leave your machine. The only
outbound network calls are to free, public price and news endpoints and to your local Ollama server.
