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

[Unreleased]: https://github.com/daryn-brown/TrendWave/compare/app-v1.3.0...HEAD
[1.3.0]: https://github.com/daryn-brown/TrendWave/releases/tag/app-v1.3.0
