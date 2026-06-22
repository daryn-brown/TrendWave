//! Composite scoring: the generalized, configurable replacement for the original
//! fixed five-term `score_candidate`.
//!
//! Design goals:
//!  * **Backward compatible.** With `ScoringWeights::legacy()` and no extra
//!    signals, `composite_score` reproduces the original formula *exactly*
//!    (`research::score_candidate` delegates here), so today's ranking is
//!    unchanged bit-for-bit and a single `scoring_mode = Legacy` switch restores
//!    it forever.
//!  * **Additive.** New early-detection signals (inflection, technical timing,
//!    estimate revisions, insider, filing) are optional `0..1` inputs with their
//!    own weights. Absent signals sit **neutral** and, under legacy weights,
//!    contribute nothing — so missing data never distorts a pick.
//!  * **Transparent.** Scoring returns a per-term `SignalBreakdown` (the weighted
//!    contribution of every term) so the UI can explain *why* a pick ranked where
//!    it did.
//!  * **Pure & testable.** No network, no I/O — just arithmetic, unit-tested like
//!    `fundamentals::growth_score`.

use serde::{Deserialize, Serialize};

/// Which scoring profile to rank with. `Legacy` pins today's exact behavior;
/// `EarlyDetection` is the new default blend that weights forward/inflection
/// signals (tuned in the integration phase). Serialized in `Settings`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoringMode {
    /// The original five-term formula — severity, moat, trailing growth,
    /// sentiment, momentum — preserved verbatim for reproducibility.
    Legacy,
    /// The forward-looking blend that also weights inflection, cycle-timing,
    /// estimate revisions, insider and filing signals.
    EarlyDetection,
}

impl Default for ScoringMode {
    fn default() -> Self {
        // The approved new default now that every early-detection signal
        // (inflection, estimate revisions, technical timing, insider buying,
        // filing evidence) and the discovery screener are wired and fed by real
        // data. Fully reversible to `Legacy` (today's exact behavior) from
        // Settings, and every individual signal still degrades to neutral when a
        // feed is unavailable, so this never tanks a run.
        ScoringMode::EarlyDetection
    }
}

/// Point weights for each scoring term. The five original terms plus the new
/// optional early-detection terms. A preset's weights are intended to sum to
/// ~100 so the composite stays on the documented `0..100` scale.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScoringWeights {
    pub severity: f64,
    pub moat: f64,
    pub growth: f64,
    pub sentiment: f64,
    pub momentum: f64,
    // --- Early-detection terms (0 under legacy) ---
    pub inflection: f64,
    pub technical: f64,
    pub revisions: f64,
    pub insider: f64,
    pub filing: f64,
}

impl ScoringWeights {
    /// The original constants from `research.rs` (sum = 100). New terms are 0, so
    /// `composite_score` reduces to the historical formula exactly.
    pub const fn legacy() -> Self {
        Self {
            severity: 25.0,
            moat: 25.0,
            growth: 35.0,
            sentiment: 10.0,
            momentum: 5.0,
            inflection: 0.0,
            technical: 0.0,
            revisions: 0.0,
            insider: 0.0,
            filing: 0.0,
        }
    }

    /// The early-detection blend. Positioning (severity + moat) stays strong;
    /// trailing growth is reduced but retained and the freed weight moves to
    /// forward signals — inflection (cyclical turns), cycle-timing, estimate
    /// revisions, insider buying and filing evidence. Sums to 100 so scores stay
    /// comparable to legacy.
    pub const fn early_detection() -> Self {
        Self {
            severity: 20.0,
            moat: 20.0,
            growth: 18.0,
            sentiment: 6.0,
            momentum: 2.0,
            inflection: 16.0,
            technical: 8.0,
            revisions: 6.0,
            insider: 2.0,
            filing: 2.0,
        }
    }

    pub fn for_mode(mode: ScoringMode) -> Self {
        match mode {
            ScoringMode::Legacy => Self::legacy(),
            ScoringMode::EarlyDetection => Self::early_detection(),
        }
    }

    /// Sum of all weights — the maximum attainable composite (every normalized
    /// signal at 1.0). Used to keep presets on the `0..100` scale and to test it.
    pub fn total(&self) -> f64 {
        self.severity
            + self.moat
            + self.growth
            + self.sentiment
            + self.momentum
            + self.inflection
            + self.technical
            + self.revisions
            + self.insider
            + self.filing
    }
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self::legacy()
    }
}

/// The raw, un-weighted inputs for one candidate. The five original signals are
/// always present; the early-detection signals are `Option` and sit **neutral**
/// (`0.5`) when unknown so a missing feed never tanks a pick. `composite_score`
/// owns all normalization (1..5 ratings → `0..1`, sentiment `-1..1` → `0..1`,
/// recent % change → momentum), so callers just pass the data they have.
#[derive(Debug, Clone, Copy, Default)]
pub struct Signals {
    /// Bottleneck severity, 1..5.
    pub severity: u8,
    /// Competitive moat, 1..5.
    pub moat: u8,
    /// Data-derived trailing growth, already `0..1` (see `fundamentals::growth_score`).
    pub growth: f64,
    /// News sentiment in `-1..1`; `None` sits neutral.
    pub sentiment: Option<f64>,
    /// Recent price change in percent, mapped to a `0..1` momentum factor.
    pub change_pct: f64,
    /// Cyclical inflection / acceleration, `0..1`; `None` neutral.
    pub inflection: Option<f64>,
    /// Technical cycle-timing ("earliness"), `0..1`; `None` neutral.
    pub technical: Option<f64>,
    /// Analyst estimate-revision momentum, `0..1`; `None` neutral.
    pub revisions: Option<f64>,
    /// Insider open-market buying, `0..1`; `None` neutral.
    pub insider: Option<f64>,
    /// Primary-source filing evidence of the bottleneck, `0..1`; `None` neutral.
    pub filing: Option<f64>,
}

/// The weighted contribution of every term plus the composite `total`. Returned
/// by `composite_score` and surfaced to the UI so a pick's ranking is explainable.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SignalBreakdown {
    pub severity: f64,
    pub moat: f64,
    pub growth: f64,
    pub sentiment: f64,
    pub momentum: f64,
    pub inflection: f64,
    pub technical: f64,
    pub revisions: f64,
    pub insider: f64,
    pub filing: f64,
    /// The composite score (sum of the contributions above), on the `0..100` scale.
    pub total: f64,
}

/// A signal with no data sits neutral so it neither rewards nor penalizes.
const NEUTRAL: f64 = 0.5;

fn neutral(value: Option<f64>) -> f64 {
    value.unwrap_or(NEUTRAL).clamp(0.0, 1.0)
}

/// Compute the composite score and its per-term breakdown.
///
/// Normalization matches the original `score_candidate` exactly: ratings map
/// `1..5 → 0..1`, sentiment `-1..1 → 0..1` (neutral when unknown), and momentum
/// is `0.5 + change_pct/40` clamped — so `+20%` over the window is full marks and
/// `-20%` is zero. With `ScoringWeights::legacy()` and no early-detection signals
/// this reproduces the historical formula bit-for-bit.
pub fn composite_score(weights: &ScoringWeights, signals: &Signals) -> SignalBreakdown {
    let severity_n = (signals.severity.clamp(1, 5) as f64) / 5.0;
    let moat_n = (signals.moat.clamp(1, 5) as f64) / 5.0;
    let growth_n = signals.growth.clamp(0.0, 1.0);
    let sentiment_n = (signals.sentiment.unwrap_or(0.0).clamp(-1.0, 1.0) + 1.0) / 2.0;
    let momentum_n = (0.5 + signals.change_pct / 40.0).clamp(0.0, 1.0);

    let inflection_n = neutral(signals.inflection);
    let technical_n = neutral(signals.technical);
    let revisions_n = neutral(signals.revisions);
    let insider_n = neutral(signals.insider);
    let filing_n = neutral(signals.filing);

    let mut breakdown = SignalBreakdown {
        severity: weights.severity * severity_n,
        moat: weights.moat * moat_n,
        growth: weights.growth * growth_n,
        sentiment: weights.sentiment * sentiment_n,
        momentum: weights.momentum * momentum_n,
        inflection: weights.inflection * inflection_n,
        technical: weights.technical * technical_n,
        revisions: weights.revisions * revisions_n,
        insider: weights.insider * insider_n,
        filing: weights.filing * filing_n,
        total: 0.0,
    };
    breakdown.total = breakdown.severity
        + breakdown.moat
        + breakdown.growth
        + breakdown.sentiment
        + breakdown.momentum
        + breakdown.inflection
        + breakdown.technical
        + breakdown.revisions
        + breakdown.insider
        + breakdown.filing;
    // Every normalized signal is in 0..=1, so the composite can never exceed the
    // sum of the weights. Guards against a future weight/term wiring mistake.
    debug_assert!(breakdown.total <= weights.total() + 1e-9);
    breakdown
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped default is the early-detection blend (the approved new
    /// default); flipping it back is a deliberate, reviewed change.
    #[test]
    fn default_mode_is_early_detection() {
        assert_eq!(ScoringMode::default(), ScoringMode::EarlyDetection);
    }

    /// Both presets must stay on the documented 0..100 scale.
    #[test]
    fn presets_sum_to_one_hundred() {
        assert!((ScoringWeights::legacy().total() - 100.0).abs() < 1e-9);
        assert!((ScoringWeights::early_detection().total() - 100.0).abs() < 1e-9);
    }

    /// The legacy scoring formula, computed by hand, must match `composite_score`.
    #[test]
    fn legacy_reduces_to_original_formula() {
        let signals = Signals {
            severity: 4,
            moat: 3,
            growth: 0.6,
            sentiment: Some(0.5),
            change_pct: 10.0,
            ..Default::default()
        };
        let got = composite_score(&ScoringWeights::legacy(), &signals).total;
        // 25*(4/5) + 25*(3/5) + 35*0.6 + 10*((0.5+1)/2) + 5*(0.5+10/40)
        let expected = 25.0 * 0.8 + 25.0 * 0.6 + 35.0 * 0.6 + 10.0 * 0.75 + 5.0 * 0.75;
        assert!((got - expected).abs() < 1e-9, "got {got}, expected {expected}");
    }

    /// Under legacy weights the new optional signals carry weight 0, so supplying
    /// or omitting them cannot change the score — proof the refactor is additive.
    #[test]
    fn early_signals_are_inert_under_legacy_weights() {
        let base = Signals {
            severity: 3,
            moat: 3,
            growth: 0.5,
            sentiment: None,
            change_pct: 0.0,
            ..Default::default()
        };
        let enriched = Signals {
            inflection: Some(1.0),
            technical: Some(0.0),
            revisions: Some(1.0),
            insider: Some(1.0),
            filing: Some(0.0),
            ..base
        };
        let a = composite_score(&ScoringWeights::legacy(), &base).total;
        let b = composite_score(&ScoringWeights::legacy(), &enriched).total;
        assert!((a - b).abs() < 1e-9);
    }

    /// A strong cyclical inflection must lift a pick under the early-detection
    /// blend even when trailing growth is weak — the whole point of the rework.
    #[test]
    fn inflection_lifts_score_in_early_mode() {
        let weak_trailing = Signals {
            severity: 4,
            moat: 4,
            growth: 0.2,
            sentiment: None,
            change_pct: 0.0,
            ..Default::default()
        };
        let turning = Signals {
            inflection: Some(0.95),
            ..weak_trailing
        };
        let w = ScoringWeights::early_detection();
        let flat = composite_score(&w, &weak_trailing).total;
        let lifted = composite_score(&w, &turning).total;
        assert!(lifted > flat, "inflection should raise the score: {lifted} !> {flat}");
    }

    /// Absent early signals are neutral (0.5), not zero — missing data must not
    /// silently tank a pick under the early-detection blend.
    #[test]
    fn missing_early_signals_sit_neutral() {
        let s = Signals {
            severity: 3,
            moat: 3,
            growth: 0.5,
            ..Default::default()
        };
        let w = ScoringWeights::early_detection();
        let b = composite_score(&w, &s);
        // Each absent early term contributes weight * 0.5.
        assert!((b.inflection - w.inflection * NEUTRAL).abs() < 1e-9);
        assert!((b.technical - w.technical * NEUTRAL).abs() < 1e-9);
        assert!((b.revisions - w.revisions * NEUTRAL).abs() < 1e-9);
    }

    /// The composite must stay within `0..=total` for extreme inputs.
    #[test]
    fn score_stays_within_bounds() {
        let w = ScoringWeights::early_detection();
        let max = composite_score(
            &w,
            &Signals {
                severity: 5,
                moat: 5,
                growth: 1.0,
                sentiment: Some(1.0),
                change_pct: 100.0,
                inflection: Some(1.0),
                technical: Some(1.0),
                revisions: Some(1.0),
                insider: Some(1.0),
                filing: Some(1.0),
            },
        )
        .total;
        let min = composite_score(
            &w,
            &Signals {
                severity: 1,
                moat: 1,
                growth: 0.0,
                sentiment: Some(-1.0),
                change_pct: -100.0,
                inflection: Some(0.0),
                technical: Some(0.0),
                revisions: Some(0.0),
                insider: Some(0.0),
                filing: Some(0.0),
            },
        )
        .total;
        assert!(max <= 100.0 + 1e-9);
        assert!(min >= -1e-9);
    }
}
