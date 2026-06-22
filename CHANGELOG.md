# Changelog

All notable changes to TrendWave are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Every released version has its own `## [x.y.z]` section below. On each push to `main`
the release workflow publishes that section verbatim as the GitHub release notes and
the in-app updater's "what's new" text — so keep entries user-facing and concise, and
the release fails fast if the section is missing. When you bump the version in
`package.json`, `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml`, move the
accumulated notes from `## [Unreleased]` into a new `## [x.y.z] - YYYY-MM-DD` section.

## [Unreleased]

## [3.0.0] - 2026-06-22

### Added
- **Early-detection ranking (new default).** TrendWave now tries to surface opportunities
  *before* the boom instead of only validating names you already supplied. The ranking blend
  adds five forward-looking signals on top of the classic positioning score:
  - **Cyclical inflection** — reads *quarterly* SEC filings to detect revenue troughs and
    re-acceleration, so a cyclical scores highest near the bottom of its cycle (where the upside
    is) rather than the top.
  - **Cycle timing** — labels each pick **Early / Building / Extended / Late** from
    multi-timeframe price action and relative strength, so you can see how early you are.
  - **Estimate revisions** — rewards rising analyst estimates and growing analyst coverage.
  - **Insider buying** — flags clusters of open-market insider purchases (SEC Form 4).
  - **Filing evidence** — scans recent 8-K / 10-Q language for capacity, shortage and
    pricing-power tells.
  - **Room to run (convexity)** — favors the small/mid-cap "sweet spot" (~$1B-$20B) with
    enough liquidity to actually trade, so names that still have the size headroom for a
    *meteoric* multi-bag rank above mega-caps that have already had their run. Uses market
    cap already fetched with fundamentals (no extra requests) and sits neutral when size is
    unknown, so it never penalizes a pick it can't size.
- **Candidate discovery.** A new screener surfaces names you didn't prompt for — via SEC EDGAR
  full-text search on the bottleneck terms and Yahoo screeners — so tickers and spinoffs the
  local model never heard of (e.g. a recent relisting) can still appear. Each pick is tagged with
  how it was found.
- **"What changed since last run" panel.** Re-running a watchlist now diffs against the previous
  run and highlights new entrants, drops, rank/score moves, and timing-label changes.
- **Per-pick "Why this score" breakdown.** Each pick can expand to show exactly how every signal
  contributed to its score.
- **Optional paid data source (bring your own key).** Advanced users can plug in a Financial
  Modeling Prep API key for cleaner estimate data. The key lives in your OS keychain, never the
  database, and the app stays fully functional — and free — without one.

### Changed
- The default ranking mode is now **Early detection**. Your original ranking is preserved exactly:
  switch **Settings → Ranking mode → Legacy** to restore the previous behavior byte-for-byte.

## [1.3.2] - 2026-06-21

### Added
- **First-run setup flow.** New installs now open a short guided setup before the
  app: review and agree to the terms, then get help getting a local AI model
  running. TrendWave detects whether Ollama is installed, reads your computer's
  memory and CPU, and recommends a model that fits your machine — with a one-click
  link to install Ollama and other model options to pick from — then finishes with
  a brief tour of how the app works. Existing installs are detected automatically
  and skip setup entirely.

## [1.3.1] - 2026-06-20

### Changed
- **Brand-new app icon.** A fresh TrendWave logo — a centered ripple-and-trend-arrow mark in the
  app's sky-blue palette — now ships across the macOS and Windows app icons and the in-app favicon,
  replacing the previous placeholder.

## [1.3.0] - 2026-06-19

### Added
- **Windows desktop app.** Releases now build, sign and publish a Windows (NSIS)
  installer alongside the macOS universal build. Windows apps auto-update from the
  same release as macOS, so both platforms always ship on the same version.

### Changed
- Release notes are now sourced from this changelog and shown both on the GitHub
  release and in the in-app updater, replacing the previous generic description.
- Updated backend dependencies: Tauri 2.11.3, Tokio 1.52.3, chrono 0.4.45 and
  tauri-plugin-opener 2.5.4.
- Updated frontend toolchain: React and React DOM 19.2.7, TypeScript 6.0.3, Vite 8
  and @vitejs/plugin-react 6.

[Unreleased]: https://github.com/daryn-brown/TrendWave/compare/app-v3.0.0...HEAD
[3.0.0]: https://github.com/daryn-brown/TrendWave/releases/tag/app-v3.0.0
[1.3.2]: https://github.com/daryn-brown/TrendWave/releases/tag/app-v1.3.2
[1.3.1]: https://github.com/daryn-brown/TrendWave/releases/tag/app-v1.3.1
[1.3.0]: https://github.com/daryn-brown/TrendWave/releases/tag/app-v1.3.0
