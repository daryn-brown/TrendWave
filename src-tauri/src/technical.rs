//! Phase 3 — technical / cycle-timing signals.
//!
//! Adds an explicit answer to "is it early?" on top of the fundamental picture.
//! From ~1y of daily candles (plus a market benchmark) we derive trend
//! structure (50/200-day moving averages), multi-timeframe momentum, relative
//! strength versus the benchmark, position within the 52-week range, and a
//! recent volume surge. Two outputs feed the rest of the app:
//!
//!   * `technical_score` (0..1, neutral 0.5) — a healthy-uptrend term that joins
//!     the composite blend in EarlyDetection mode, and
//!   * a `TimingLabel` (Early | Building | Extended | Late) — so a card can say
//!     "looks early here" versus "already extended."
//!
//! Everything is pure and unit-tested on plain slices; the only async piece is a
//! best-effort Yahoo chart fetch.

use serde_json::Value;

const CHART_URL: &str = "https://query1.finance.yahoo.com/v8/finance/chart/";
const BENCHMARK: &str = "SPY";

/// Approximate trading-day lookbacks for 3 / 6 month windows.
const D_3M: usize = 63;
const D_6M: usize = 126;

/// Minimum candles required before we'll emit any technical read at all.
const MIN_CANDLES: usize = 60;

/// Where a name sits in its own cycle, for the "is it early?" badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimingLabel {
    /// Turning up off a low — the early sweet spot.
    Early,
    /// Healthy established uptrend, not yet stretched.
    Building,
    /// Near highs or parabolic — late to the move.
    Extended,
    /// Established downtrend — falling, not a bottom yet.
    Late,
}

impl TimingLabel {
    pub fn as_str(self) -> &'static str {
        match self {
            TimingLabel::Early => "Early",
            TimingLabel::Building => "Building",
            TimingLabel::Extended => "Extended",
            TimingLabel::Late => "Late",
        }
    }
}

/// The technical read consumed by the pipeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Technical {
    pub score: f64,
    pub timing: TimingLabel,
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested)
// ---------------------------------------------------------------------------

/// Trailing simple moving average of the last `n` values, or `None` if the
/// series is shorter than `n`.
fn sma(xs: &[f64], n: usize) -> Option<f64> {
    if n == 0 || xs.len() < n {
        return None;
    }
    let sum: f64 = xs[xs.len() - n..].iter().sum();
    Some(sum / n as f64)
}

/// Simple return over `lookback` trading days: `last / past - 1`. `None` when
/// there isn't enough history or the past price is non-positive.
fn ret(xs: &[f64], lookback: usize) -> Option<f64> {
    if xs.len() <= lookback {
        return None;
    }
    let last = *xs.last()?;
    let past = xs[xs.len() - 1 - lookback];
    if past > 0.0 {
        Some(last / past - 1.0)
    } else {
        None
    }
}

/// Linear map of `x` from `[lo, hi]` onto `[0, 1]`, clamped outside the band.
fn lin(x: f64, lo: f64, hi: f64) -> f64 {
    if hi <= lo {
        return 0.5;
    }
    ((x - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// Recent volume surge: mean of the last ~10 sessions over the full-window mean.
/// `1.0` means in-line with the baseline; `>1` means an unusual pickup.
fn volume_surge(volumes: &[f64]) -> f64 {
    let v: Vec<f64> = volumes.iter().copied().filter(|x| *x > 0.0).collect();
    if v.len() < 10 {
        return 1.0;
    }
    let recent: f64 = v[v.len() - 10..].iter().sum::<f64>() / 10.0;
    let base: f64 = v.iter().sum::<f64>() / v.len() as f64;
    if base > 0.0 {
        recent / base
    } else {
        1.0
    }
}

/// Position within the observed 52-week range, `0.0` (at the low) .. `1.0` (at
/// the high). Defaults to the midpoint for a flat series.
fn range_position(closes: &[f64]) -> f64 {
    let low = closes.iter().copied().fold(f64::INFINITY, f64::min);
    let high = closes.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let last = *closes.last().unwrap_or(&0.0);
    if high > low {
        ((last - low) / (high - low)).clamp(0.0, 1.0)
    } else {
        0.5
    }
}

/// Compute the technical read from a price/volume series and a benchmark close
/// series. Pure; `None` when there isn't enough history to say anything.
pub fn compute(closes: &[f64], volumes: &[f64], benchmark: &[f64]) -> Option<Technical> {
    let closes: Vec<f64> = closes.iter().copied().filter(|x| x.is_finite() && *x > 0.0).collect();
    if closes.len() < MIN_CANDLES {
        return None;
    }
    let last = *closes.last()?;
    let sma50 = sma(&closes, 50);
    let sma200 = sma(&closes, 200);
    let above_50 = sma50.map(|m| last >= m).unwrap_or(false);
    let above_200 = sma200.map(|m| last >= m);

    let mom_3m = ret(&closes, D_3M);
    let mom_6m = ret(&closes, D_6M);

    // Relative strength: our 6m (or longest available) return minus the
    // benchmark's over the same window.
    let rs = match (mom_6m, ret(benchmark, D_6M)) {
        (Some(a), Some(b)) => Some(a - b),
        _ => None,
    };

    let rp = range_position(&closes);
    let from_low = {
        let low = closes.iter().copied().fold(f64::INFINITY, f64::min);
        if low > 0.0 {
            (last - low) / low
        } else {
            0.0
        }
    };
    let vol_surge = volume_surge(volumes);

    let score = technical_score(above_50, above_200, mom_6m, rs, from_low, vol_surge);
    let timing = classify_timing(rp, above_50, above_200, mom_3m, mom_6m);
    Some(Technical { score, timing })
}

/// Blend the trend/momentum/relative-strength/recovery/volume components into a
/// single 0..1 posture score (neutral ≈ 0.5). Higher = healthier technical setup.
fn technical_score(
    above_50: bool,
    above_200: Option<bool>,
    mom_6m: Option<f64>,
    rs: Option<f64>,
    from_low: f64,
    vol_surge: f64,
) -> f64 {
    // Trend structure (0.28): reward being above the long- and short-term MAs.
    let trend = {
        let long = match above_200 {
            Some(true) => 1.0,
            Some(false) => 0.0,
            None => 0.5, // unknown (short history) → neutral, no penalty
        };
        let short = if above_50 { 1.0 } else { 0.0 };
        0.6 * long + 0.4 * short
    };
    // Momentum (0.27): 6m return, -30%..+50% mapped onto 0..1.
    let momentum = mom_6m.map(|m| lin(m, -0.30, 0.50)).unwrap_or(0.5);
    // Relative strength (0.22): -20%..+20% versus benchmark mapped onto 0..1.
    let relative = rs.map(|r| lin(r, -0.20, 0.20)).unwrap_or(0.5);
    // Recovery off the low (0.10): rewards early upturns without needing new highs.
    let recovery = lin(from_low, 0.0, 0.50);
    // Volume confirmation (0.13): a pickup over the baseline (0.8x..1.6x).
    let volume = lin(vol_surge, 0.8, 1.6);

    0.28 * trend + 0.27 * momentum + 0.22 * relative + 0.10 * recovery + 0.13 * volume
}

/// Classify cycle timing from range position, trend structure and momentum.
fn classify_timing(
    rp: f64,
    above_50: bool,
    above_200: Option<bool>,
    mom_3m: Option<f64>,
    mom_6m: Option<f64>,
) -> TimingLabel {
    let m3 = mom_3m.unwrap_or(0.0);
    let m6 = mom_6m.unwrap_or(0.0);
    let below_200 = matches!(above_200, Some(false));

    // Near highs or parabolic → late to the move.
    if rp > 0.85 || m6 > 0.60 {
        return TimingLabel::Extended;
    }
    // Established downtrend: under the long MA, six-month trend down, not yet
    // recovering on the three-month view.
    if below_200 && m6 < -0.05 && m3 <= 0.0 {
        return TimingLabel::Late;
    }
    // Turning up off a low: in the lower half of the range, back above the short
    // MA, with positive recent momentum — the early sweet spot.
    if rp < 0.5 && above_50 && m3 > 0.0 {
        return TimingLabel::Early;
    }
    TimingLabel::Building
}

// ---------------------------------------------------------------------------
// Best-effort network fetches
// ---------------------------------------------------------------------------

/// Fetch ~1y of daily `(closes, volumes)` for `symbol`. `None` on any failure.
async fn fetch_series(http: &reqwest::Client, symbol: &str) -> Option<(Vec<f64>, Vec<f64>)> {
    let safe = crate::feeds::validate_ticker(symbol).ok()?;
    let url = format!("{CHART_URL}{safe}?range=1y&interval=1d");
    let json: Value = http.get(&url).send().await.ok()?.json().await.ok()?;
    let result = json["chart"]["result"].get(0)?;
    let quote = result["indicators"]["quote"].get(0)?;
    let closes: Vec<f64> = quote["close"]
        .as_array()?
        .iter()
        .filter_map(|v| v.as_f64())
        .collect();
    let volumes: Vec<f64> = quote["volume"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
        .unwrap_or_default();
    if closes.is_empty() {
        None
    } else {
        Some((closes, volumes))
    }
}

/// Fetch the benchmark (SPY) daily closes once per run, to share across every
/// candidate's relative-strength calculation.
pub async fn fetch_benchmark(http: &reqwest::Client) -> Vec<f64> {
    fetch_series(http, BENCHMARK)
        .await
        .map(|(closes, _)| closes)
        .unwrap_or_default()
}

/// Best-effort technical read for one ticker against a pre-fetched benchmark.
pub async fn fetch_technical(
    http: &reqwest::Client,
    ticker: &str,
    benchmark: &[f64],
) -> Option<Technical> {
    let (closes, volumes) = fetch_series(http, ticker).await?;
    compute(&closes, &volumes, benchmark)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(from: f64, to: f64, n: usize) -> Vec<f64> {
        (0..n)
            .map(|i| from + (to - from) * (i as f64) / ((n - 1).max(1) as f64))
            .collect()
    }

    #[test]
    fn sma_and_ret_basic() {
        let xs = vec![1.0, 2.0, 3.0, 4.0];
        assert_eq!(sma(&xs, 2), Some(3.5));
        assert_eq!(sma(&xs, 9), None);
        assert_eq!(ret(&xs, 3), Some(3.0)); // 1 -> 4
        assert_eq!(ret(&xs, 9), None);
    }

    #[test]
    fn lin_clamps_and_maps() {
        assert_eq!(lin(-1.0, 0.0, 1.0), 0.0);
        assert_eq!(lin(2.0, 0.0, 1.0), 1.0);
        assert!((lin(0.5, 0.0, 1.0) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn range_position_extremes() {
        let xs = vec![10.0, 20.0, 30.0];
        assert!((range_position(&xs) - 1.0).abs() < 1e-9); // last == high
        let ys = vec![30.0, 20.0, 10.0];
        assert!((range_position(&ys)).abs() < 1e-9); // last == low
    }

    #[test]
    fn too_little_history_is_none() {
        let short = ramp(10.0, 12.0, 20);
        assert!(compute(&short, &[], &[]).is_none());
    }

    #[test]
    fn early_recovery_is_flagged_early() {
        // Long decline then a clear recovery off the low; last sits in the lower
        // half of the range, back above the 50d MA, positive 3m momentum.
        let mut closes = ramp(100.0, 60.0, 80);
        closes.extend(ramp(60.5, 78.0, 60));
        let vols = vec![1_000.0; closes.len()];
        let t = compute(&closes, &vols, &[]).expect("enough history");
        assert_eq!(t.timing, TimingLabel::Early);
    }

    #[test]
    fn near_highs_is_extended() {
        let closes = ramp(50.0, 120.0, 140); // straight up, ending at the high
        let vols = vec![1_000.0; closes.len()];
        let t = compute(&closes, &vols, &[]).expect("enough history");
        assert_eq!(t.timing, TimingLabel::Extended);
        assert!(t.score > 0.5); // strong posture
    }

    #[test]
    fn steady_downtrend_is_late() {
        let closes = ramp(120.0, 70.0, 220); // long, persistent decline below MAs
        let vols = vec![1_000.0; closes.len()];
        let t = compute(&closes, &vols, &[]).expect("enough history");
        assert_eq!(t.timing, TimingLabel::Late);
        assert!(t.score < 0.5); // weak posture
    }

    #[test]
    fn healthy_mid_uptrend_is_building() {
        // Up over the year but mid-range now (pulled back from the high), above
        // both MAs, not parabolic.
        let mut closes = ramp(60.0, 100.0, 180);
        closes.extend(ramp(99.0, 90.0, 30)); // mild pullback off the top
        let vols = vec![1_000.0; closes.len()];
        let t = compute(&closes, &vols, &[]).expect("enough history");
        assert_eq!(t.timing, TimingLabel::Building);
    }

    #[test]
    fn relative_strength_lifts_score() {
        let closes = ramp(80.0, 100.0, 160); // +25%
        let vols = vec![1_000.0; closes.len()];
        let strong_bench = ramp(80.0, 84.0, 160); // benchmark +5%
        let weak_vs = ramp(80.0, 130.0, 160); // benchmark +62.5%
        let out = compute(&closes, &vols, &strong_bench).unwrap();
        let under = compute(&closes, &vols, &weak_vs).unwrap();
        assert!(out.score > under.score); // outperforming scores higher
    }
}
