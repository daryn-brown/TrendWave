# TrendWave 📈🌊

TrendWave is a background-first desktop app for spotting early stock momentum in niche tech sectors before the crowd catches on.

Built with **Tauri v2 + Rust + React + TypeScript + Tailwind CSS**, this project is also a guided Rust learning journey focused on ownership, borrowing, error handling, and async design.

## Why This Exists ✨

Most market tools are noisy dashboards. TrendWave is aiming for the opposite:

- Stay invisible most of the time
- Use minimal system resources
- Watch for unusual volume and momentum signals in targeted micro-sectors
- Surface only when something genuinely interesting happens

## Current Status 🚧

The app scaffold is up and running.

- [x] Tauri + React + TypeScript project initialized
- [x] Tailwind CSS wired into Vite
- [x] Rust-to-frontend IPC verified with the starter `greet` command
- [ ] Phase 1: Headless tray-first foundation
- [ ] Phase 2: SQLite-backed state and storage
- [ ] Phase 3: Tokio background worker and alerts
- [ ] Phase 4: Dashboard UI

## Architecture Roadmap 🧭

### Phase 1: Headless Foundation
- Launch without a standard window
- Add a macOS menu bar / Windows tray icon
- Support tray actions for dashboard, pause, settings, and quit

### Phase 2: State & Storage
- Create a local `trendwave.db`
- Store tracked tickers, daily metrics, and generated alerts
- Share state safely with Tauri managed state and Rust synchronization primitives

### Phase 3: Background Worker
- Run a periodic Tokio task during market hours
- Fetch price and volume data from external APIs
- Compare current activity against moving averages
- Trigger alerts and native notifications when thresholds are crossed

### Phase 4: Dashboard
- Show a sector heatmap
- Show a recent alert feed
- Add settings for API keys and polling intervals
- Read backend data through Tauri IPC commands

## Tech Stack 🛠️

- **Desktop shell:** Tauri v2
- **Backend:** Rust
- **Frontend:** React + TypeScript
- **Styling:** Tailwind CSS
- **Build tooling:** Vite
- **Planned data layer:** SQLite via `rusqlite`
- **Planned async runtime:** `tokio`
- **Planned HTTP client:** `reqwest`

## Learning Goals 🦀

This repo is intentionally being built step by step to learn Rust the right way.

- Understand ownership and borrowing through real app code
- Learn when to pass `&T`, when to clone, and when to share state explicitly
- Practice proper error handling with `Result`, `?`, and typed errors
- Build intuition for async work with Tokio without hiding the complexity

## Getting Started 🚀

### Prerequisites

- Node.js and npm
- Rust toolchain
- Tauri system prerequisites for your platform

### Run the app

```bash
npm install
npm run tauri dev
```

### Build the frontend only

```bash
npm run build
```

### Check the Rust backend

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

## Project Structure 🗂️

```text
TrendWave/
├── project_spec.md      # Architecture and implementation plan
├── src/                 # React frontend
├── src-tauri/           # Rust backend and Tauri config
├── public/              # Static frontend assets
└── README.md            # Project overview and workflow notes
```

## Commit Style 🧱

To keep the history easy to learn from, we want small, scoped commits:

- `chore:` tooling, config, project setup
- `docs:` README, notes, architecture updates
- `feat:` one user-visible slice of a phase
- `refactor:` cleanup without changing behavior
- `fix:` bug fixes and regressions

Examples:

```bash
git commit -m "chore: initialize TrendWave project metadata"
git commit -m "docs: add TrendWave project README"
git commit -m "feat: add tray menu shell for phase 1"
```

## Guiding Rules 🤝

- Build one phase at a time
- Prefer idiomatic Rust patterns over clever shortcuts
- Avoid `.unwrap()` in production code
- Explain memory and async decisions as we go

TrendWave is early, but the foundation is now in place and ready for the first real Rust phase.
