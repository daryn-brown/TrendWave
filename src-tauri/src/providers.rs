//! Pluggable market-data providers (free by default, optional BYO-key paid).
//!
//! Out of the box TrendWave is **fully functional with zero setup**: every new
//! signal is computed from free public sources (SEC EDGAR + Yahoo Finance) that
//! live elsewhere in the codebase. This module adds a thin seam so a power user
//! can *optionally* slot in a bring-your-own-key paid source for the
//! highest-ROI signals — **estimate revisions** (Phase 1) and **point-in-time
//! backtest data** (Phase 6).
//!
//! Design choices, deliberately conservative:
//! - **Enum + `match` dispatch**, not a `dyn async` trait. It fits the
//!   codebase's pattern-matching style and adds no new dependencies. New
//!   providers are added by extending [`ProviderKind`].
//! - The API key lives in the **OS keychain** (same `keyring` pattern as the
//!   broker integrations) — never the SQLite DB, the settings JSON, or logs.
//! - Every paid capability **degrades gracefully**: a missing key, a rejected
//!   key, or any network/parse failure returns `None`, and the caller falls
//!   back to the free path. A paid integration can therefore never *break* a
//!   run — at worst it is silently skipped.
//! - Paid responses are treated as **non-cacheable** (licensing): callers must
//!   not persist them into saved-watchlist result caches.

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// Keychain coordinates for the optional paid-provider API key. Same service as
/// the broker integrations but a distinct account so they never collide.
const KEYRING_SERVICE: &str = "com.trendwave.app";
const KEYRING_ACCOUNT: &str = "data-provider-key";

/// Financial Modeling Prep REST base. Chosen as the reference paid adapter
/// because it is well-documented, has a free tier (so a curious user can try a
/// key cheaply), and exposes the analyst-consensus data the free Yahoo path
/// only approximates.
const FMP_BASE: &str = "https://financialmodelingprep.com/api/v3";

/// Which market-data provider backs the optional paid capabilities.
///
/// `Free` is the default and requires no key — the app behaves exactly as it
/// always has. Additional variants are paid, bring-your-own-key adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// SEC EDGAR + Yahoo Finance only. Zero setup, nothing leaves the machine
    /// beyond public lookups. The default.
    #[default]
    Free,
    /// Financial Modeling Prep (reference paid adapter; requires an API key).
    Fmp,
}

impl ProviderKind {
    /// Whether this provider needs a user-supplied API key to do anything.
    pub fn is_paid(self) -> bool {
        !matches!(self, ProviderKind::Free)
    }

    /// Short human-readable name for progress messages and UI.
    pub fn label(self) -> &'static str {
        match self {
            ProviderKind::Free => "Free (EDGAR + Yahoo)",
            ProviderKind::Fmp => "Financial Modeling Prep",
        }
    }
}

// ---------------------------------------------------------------------------
// API-key storage (OS keychain — never the DB/config/logs)
// ---------------------------------------------------------------------------

fn keyring_entry() -> AppResult<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .map_err(|e| AppError::Other(format!("keychain unavailable: {e}")))
}

/// Read the stored paid-provider key, if any. Returns `None` (without prompting
/// for credentials that don't exist) when no key has been saved.
pub fn load_key() -> Option<String> {
    let key = keyring_entry().ok()?.get_password().ok()?;
    let key = key.trim().to_string();
    if key.is_empty() {
        None
    } else {
        Some(key)
    }
}

/// Persist the paid-provider key to the OS keychain. The key is trimmed; an
/// empty/whitespace key is rejected so a blank save can't masquerade as "set".
pub fn save_key(key: &str) -> AppResult<()> {
    let key = key.trim();
    if key.is_empty() {
        return Err(AppError::Other("API key was empty".into()));
    }
    keyring_entry()?
        .set_password(key)
        .map_err(|e| AppError::Other(format!("could not save API key: {e}")))
}

/// Remove the stored paid-provider key. A missing entry is treated as
/// already-cleared.
pub fn clear_key() -> AppResult<()> {
    if let Ok(entry) = keyring_entry() {
        match entry.delete_credential() {
            Ok(()) => {}
            Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(AppError::Other(format!("could not clear API key: {e}"))),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Capability payloads
// ---------------------------------------------------------------------------

/// A normalized analyst-revision signal. Both the free (Yahoo `recommendation
/// Trend`) and paid (FMP analyst recommendations) paths reduce to this shape so
/// the Phase 1 scorer consumes one consistent type regardless of source.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EstimateRevisions {
    /// Analysts at a bullish rating (strong-buy + buy) in the latest period.
    pub up: u32,
    /// Analysts at a bearish rating (sell + strong-sell) in the latest period.
    pub down: u32,
    /// Total contributing analysts (including holds) — a confidence proxy.
    pub total: u32,
}

impl EstimateRevisions {
    /// Net upward bias in `-1.0..=1.0`: `(up - down) / total`. Positive means
    /// more bullish than bearish coverage; `0.0` when there is no coverage.
    /// Phase 1 maps this into a `revisions_score`.
    pub fn net_bias(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        (self.up as f64 - self.down as f64) / self.total as f64
    }

    /// Revisions signal on `0.0..=1.0` centered at `0.5` (no net bias), suitable
    /// as a scoring term. Bullish consensus pushes above neutral, bearish below.
    pub fn score(&self) -> f64 {
        ((self.net_bias() + 1.0) / 2.0).clamp(0.0, 1.0)
    }
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

/// A resolved provider: the selected [`ProviderKind`] plus the API key (loaded
/// from the keychain) when one is needed. Construct via [`DataProvider::resolve`]
/// so the keychain is only touched for paid modes — the default `Free` mode
/// never reads credentials and never prompts.
#[derive(Debug, Clone)]
pub struct DataProvider {
    pub kind: ProviderKind,
    key: Option<String>,
}

impl DataProvider {
    /// Resolve a provider for the given mode. For paid modes this loads the key
    /// from the OS keychain; for `Free` it does no keychain access at all.
    pub fn resolve(kind: ProviderKind) -> Self {
        let key = if kind.is_paid() { load_key() } else { None };
        Self { kind, key }
    }

    /// True only when a paid provider is selected *and* a key is present, i.e.
    /// when a paid capability can actually be attempted.
    pub fn is_active(&self) -> bool {
        self.kind.is_paid() && self.key.is_some()
    }

    /// Best-effort paid estimate-revision lookup. Returns `None` for the free
    /// provider, a missing key, or any network/parse failure — the caller then
    /// falls back to the free Yahoo path. Never returns an error: a paid
    /// integration must not be able to break a run.
    pub async fn estimate_revisions(
        &self,
        http: &reqwest::Client,
        ticker: &str,
    ) -> Option<EstimateRevisions> {
        let key = self.key.as_deref()?;
        match self.kind {
            ProviderKind::Free => None,
            ProviderKind::Fmp => {
                let url = format!("{FMP_BASE}/analyst-stock-recommendations/{ticker}");
                let resp = http
                    .get(url)
                    .query(&[("apikey", key)])
                    .send()
                    .await
                    .ok()?;
                if !resp.status().is_success() {
                    return None;
                }
                let json: serde_json::Value = resp.json().await.ok()?;
                parse_fmp_recommendations(&json)
            }
        }
    }
}

/// Parse FMP's `analyst-stock-recommendations` payload into normalized revision
/// counts, reading the most recent (first) entry. Pure and unit-tested so the
/// adapter is verifiable without a live key. Returns `None` on any shape we
/// don't recognize so the caller degrades to the free path.
fn parse_fmp_recommendations(json: &serde_json::Value) -> Option<EstimateRevisions> {
    let latest = json.as_array()?.first()?;
    let field = |name: &str| latest.get(name).and_then(|v| v.as_u64()).unwrap_or(0) as u32;

    let strong_buy = field("analystRatingsStrongBuy");
    let buy = field("analystRatingsbuy");
    let hold = field("analystRatingsHold");
    let sell = field("analystRatingsSell");
    let strong_sell = field("analystRatingsStrongSell");

    let up = strong_buy + buy;
    let down = sell + strong_sell;
    let total = up + hold + down;
    if total == 0 {
        return None;
    }
    Some(EstimateRevisions { up, down, total })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_is_the_default_and_needs_no_key() {
        assert_eq!(ProviderKind::default(), ProviderKind::Free);
        assert!(!ProviderKind::Free.is_paid());
        assert!(ProviderKind::Fmp.is_paid());
    }

    #[test]
    fn provider_kind_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_string(&ProviderKind::Free).unwrap(),
            "\"free\""
        );
        assert_eq!(
            serde_json::to_string(&ProviderKind::Fmp).unwrap(),
            "\"fmp\""
        );
        let parsed: ProviderKind = serde_json::from_str("\"fmp\"").unwrap();
        assert_eq!(parsed, ProviderKind::Fmp);
    }

    #[test]
    fn free_provider_is_never_active_and_does_no_paid_work() {
        let p = DataProvider::resolve(ProviderKind::Free);
        assert!(!p.is_active());
    }

    #[test]
    fn net_bias_spans_minus_one_to_one() {
        let bullish = EstimateRevisions {
            up: 8,
            down: 0,
            total: 8,
        };
        assert!((bullish.net_bias() - 1.0).abs() < 1e-9);

        let bearish = EstimateRevisions {
            up: 0,
            down: 5,
            total: 5,
        };
        assert!((bearish.net_bias() + 1.0).abs() < 1e-9);

        let mixed = EstimateRevisions {
            up: 6,
            down: 2,
            total: 10, // includes 2 holds
        };
        assert!((mixed.net_bias() - 0.4).abs() < 1e-9);
        assert!((mixed.score() - 0.7).abs() < 1e-9); // (0.4 + 1) / 2

        let empty = EstimateRevisions {
            up: 0,
            down: 0,
            total: 0,
        };
        assert_eq!(empty.net_bias(), 0.0);
        assert_eq!(empty.score(), 0.5); // no coverage → neutral
    }

    #[test]
    fn parses_fmp_recommendations_latest_entry() {
        // FMP returns newest-first; only the first row should be read.
        let payload = serde_json::json!([
            {
                "symbol": "MU",
                "date": "2024-03-01",
                "analystRatingsStrongBuy": 5,
                "analystRatingsbuy": 12,
                "analystRatingsHold": 3,
                "analystRatingsSell": 1,
                "analystRatingsStrongSell": 0
            },
            {
                "symbol": "MU",
                "date": "2024-02-01",
                "analystRatingsStrongBuy": 1,
                "analystRatingsbuy": 1,
                "analystRatingsHold": 9,
                "analystRatingsSell": 4,
                "analystRatingsStrongSell": 2
            }
        ]);
        let rev = parse_fmp_recommendations(&payload).expect("should parse");
        assert_eq!(rev.up, 17); // 5 + 12
        assert_eq!(rev.down, 1); // 1 + 0
        assert_eq!(rev.total, 21); // 17 up + 3 hold + 1 down
        assert!(rev.net_bias() > 0.7);
    }

    #[test]
    fn fmp_parser_rejects_unusable_shapes() {
        // Not an array.
        assert!(parse_fmp_recommendations(&serde_json::json!({"error": "x"})).is_none());
        // Empty array.
        assert!(parse_fmp_recommendations(&serde_json::json!([])).is_none());
        // All-zero coverage carries no signal.
        let zero = serde_json::json!([{ "symbol": "X", "date": "2024-01-01" }]);
        assert!(parse_fmp_recommendations(&zero).is_none());
    }
}
