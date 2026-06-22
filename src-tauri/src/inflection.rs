//! Forward-looking inflection signals from **quarterly** SEC EDGAR fundamentals.
//!
//! The legacy growth term is driven by *trailing annual* revenue/earnings, which
//! for cyclicals is inverted: at a cycle bottom (e.g. Micron FY2023 — revenue
//! roughly halved, a large loss) the audited annual numbers look worst exactly
//! when the upside is greatest, so the engine ranks the early opportunity *down*.
//!
//! This module reads the *quarterly* series instead and rewards **turns and
//! acceleration**, not absolute trailing growth:
//!  * **Revenue YoY acceleration** — is year-over-year growth itself rising?
//!  * **Trough recovery** — has YoY turned up off a recent low?
//!  * **Profitability turn** — net income crossing loss → profit (or losses
//!    shrinking).
//!  * **Margin inflection** — operating margin trending up.
//!
//! Everything network-free is a pure function with unit tests; the fetch layer
//! is best-effort and degrades to `None` (scored neutral) on any miss, so a
//! failed lookup never breaks a run. EDGAR access (the CIK map + raw concept
//! points) is shared with `fundamentals` so the ~800KB ticker map is fetched at
//! most once per process.

use std::collections::HashMap;

use chrono::{Datelike, NaiveDate};

use crate::fundamentals::{self, ConceptPoint};

/// Revenue is tagged differently across filers/eras; try the common us-gaap
/// concepts in order and keep the first that yields a usable quarterly series.
const REVENUE_CONCEPTS: &[&str] = &[
    "RevenueFromContractWithCustomerExcludingAssessedTax",
    "Revenues",
    "SalesRevenueNet",
    "RevenueFromContractWithCustomerIncludingAssessedTax",
];

/// A calendar quarter key, e.g. `(2024, 1)` for CY2024Q1. Calendar quarters
/// (not fiscal) are used so year-over-year matching lines up the same season.
type QKey = (i32, u8);

/// Quarterly series for one company, each ascending by `(year, quarter)`.
/// Built by [`fetch_inflection`]; constructed directly in tests.
pub struct QuarterlySet {
    pub revenue: Vec<(QKey, f64)>,
    pub net_income: Vec<(QKey, f64)>,
    pub operating_income: Vec<(QKey, f64)>,
}

// ---- Pure scoring -----------------------------------------------------------

/// Composite inflection score in `0.0..=1.0`, or `None` when there isn't enough
/// quarterly history to say anything (the caller then scores it neutral).
///
/// Each present sub-signal is centered on `0.5` (no signal), so a steady,
/// healthy grower lands near neutral while a company *turning up* off a trough
/// scores well above it and a decelerating one below. Absent sub-signals drop
/// out and the remaining weights renormalize (mirrors `fundamentals::growth_score`).
pub fn inflection_score(qs: &QuarterlySet) -> Option<f64> {
    let yoy = yoy_series(&qs.revenue);
    let mut weighted = 0.0;
    let mut total = 0.0;

    if yoy.len() >= 2 {
        // Acceleration: change in YoY growth between the two most recent quarters.
        let accel = yoy[yoy.len() - 1] - yoy[yoy.len() - 2];
        weighted += 0.40 * centered(accel, 0.30);
        total += 0.40;

        // Recovery: how far the latest YoY has climbed back above its recent low.
        let window = &yoy[yoy.len().saturating_sub(4)..];
        let trough = window.iter().copied().fold(f64::INFINITY, f64::min);
        let recovery = (yoy[yoy.len() - 1] - trough).max(0.0);
        weighted += 0.25 * upside_only(recovery, 0.50);
        total += 0.25;
    }

    if let Some(turn) = profit_turn(&qs.net_income) {
        weighted += 0.20 * turn;
        total += 0.20;
    }

    if let Some(margin) = margin_trend(&qs.operating_income, &qs.revenue) {
        weighted += 0.15 * margin;
        total += 0.15;
    }

    if total == 0.0 {
        None
    } else {
        Some((weighted / total).clamp(0.0, 1.0))
    }
}

/// Map a signed deviation onto `0..1` centered at `0.5`, saturating at `±span`.
/// `0.0` → neutral `0.5`; `+span` → `1.0`; `-span` → `0.0`.
fn centered(x: f64, span: f64) -> f64 {
    (0.5 + (x / (2.0 * span)).clamp(-0.5, 0.5)).clamp(0.0, 1.0)
}

/// Map a non-negative magnitude onto `0.5..1.0` (never penalizes): `0.0` → `0.5`,
/// `span` → `1.0`. Used for "recovery off a trough," which should only add upside.
fn upside_only(x: f64, span: f64) -> f64 {
    (0.5 + (x / (2.0 * span)).clamp(0.0, 0.5)).clamp(0.0, 1.0)
}

/// Year-over-year growth per quarter (same calendar quarter, prior year), in
/// chronological order. Uses `|prev|` in the denominator so a swing off a
/// negative base still has the right sign.
fn yoy_series(series: &[(QKey, f64)]) -> Vec<f64> {
    let map: HashMap<QKey, f64> = series.iter().copied().collect();
    let mut out = Vec::new();
    for &((year, q), v) in series {
        if let Some(&prev) = map.get(&(year - 1, q)) {
            if prev.abs() > f64::EPSILON {
                out.push((v - prev) / prev.abs());
            }
        }
    }
    out
}

/// Profitability-turn score from up to the last four quarters of net income.
/// Crossing loss → profit is the strongest tell; shrinking losses and rising
/// profits are positive; deterioration is negative. `None` with < 2 quarters.
fn profit_turn(net_income: &[(QKey, f64)]) -> Option<f64> {
    if net_income.len() < 2 {
        return None;
    }
    let vals: Vec<f64> = net_income
        .iter()
        .rev()
        .take(4)
        .rev()
        .map(|(_, v)| *v)
        .collect();
    let last = *vals.last().unwrap();
    let prev = vals[vals.len() - 2];
    let prior_min = vals[..vals.len() - 1]
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);

    let score = if last > 0.0 && prior_min < 0.0 {
        1.0 // crossed from a loss to a profit within the window
    } else if vals.iter().all(|v| *v > 0.0) && last > prev {
        0.8 // profitable and still rising
    } else if last < 0.0 && last > prev {
        0.65 // still a loss, but shrinking
    } else if vals.iter().all(|v| *v > 0.0) {
        0.6 // profitable, roughly flat
    } else if last < prev {
        0.3 // deteriorating
    } else {
        0.5
    };
    Some(score)
}

/// Operating-margin inflection: latest quarter's operating margin vs the mean of
/// the prior quarters in the series. Rising margins score above neutral. `None`
/// when fewer than two quarters have both operating income and revenue.
fn margin_trend(operating_income: &[(QKey, f64)], revenue: &[(QKey, f64)]) -> Option<f64> {
    let rev_map: HashMap<QKey, f64> = revenue.iter().copied().collect();
    let mut margins = Vec::new();
    for &(key, op) in operating_income {
        if let Some(&rev) = rev_map.get(&key) {
            if rev.abs() > f64::EPSILON {
                margins.push(op / rev);
            }
        }
    }
    if margins.len() < 2 {
        return None;
    }
    let last = *margins.last().unwrap();
    let prior = &margins[..margins.len() - 1];
    let mean_prior = prior.iter().sum::<f64>() / prior.len() as f64;
    Some(centered(last - mean_prior, 0.10))
}

// ---- EDGAR fetch (best-effort) ---------------------------------------------

/// Best-effort inflection score for one ticker from quarterly EDGAR data.
/// Returns `None` (scored neutral upstream) when the company can't be resolved
/// or there isn't enough quarterly history.
pub async fn fetch_inflection(http: &reqwest::Client, ticker: &str) -> Option<f64> {
    let cik = fundamentals::cik_for(http, ticker).await?;

    let mut revenue: Vec<(QKey, f64)> = Vec::new();
    for concept in REVENUE_CONCEPTS {
        if let Some(points) = fundamentals::fetch_concept_points(http, &cik, concept).await {
            let series = quarterly_series(&points);
            if series.len() > revenue.len() {
                revenue = series;
            }
            // Enough to compute at least one YoY acceleration → stop probing tags.
            if revenue.len() >= 5 {
                break;
            }
        }
    }

    let net_income = fundamentals::fetch_concept_points(http, &cik, "NetIncomeLoss")
        .await
        .map(|p| quarterly_series(&p))
        .unwrap_or_default();
    let operating_income = fundamentals::fetch_concept_points(http, &cik, "OperatingIncomeLoss")
        .await
        .map(|p| quarterly_series(&p))
        .unwrap_or_default();

    inflection_score(&QuarterlySet {
        revenue,
        net_income,
        operating_income,
    })
}

/// Reduce raw XBRL points to one value per calendar quarter, ascending by
/// `(year, quarter)`. Accepts canonical `CY####Q#` *duration* frames and, as a
/// fallback, any ~quarter-length (80–100 day) reporting period. Annual frames,
/// instantaneous (`…I`) frames, and multi-quarter spans are ignored. Later
/// restatements (a later period end for the same quarter) supersede earlier ones.
fn quarterly_series(points: &[ConceptPoint]) -> Vec<(QKey, f64)> {
    let mut by_q: HashMap<QKey, (NaiveDate, f64)> = HashMap::new();

    for p in points {
        let resolved = quarter_from_frame(p.frame.as_deref())
            .map(|key| (key, parse_date(p.end.as_deref())))
            .or_else(|| quarter_from_period(p.start.as_deref(), p.end.as_deref()));

        if let Some((key, end)) = resolved {
            // A synthetic end (quarter-mid) only orders frame points that lack an
            // explicit end; real period ends always win the restatement compare.
            let end = end.unwrap_or_else(|| synthetic_end(key));
            by_q
                .entry(key)
                .and_modify(|cur| {
                    if end > cur.0 {
                        *cur = (end, p.val);
                    }
                })
                .or_insert((end, p.val));
        }
    }

    let mut series: Vec<(QKey, f64)> = by_q.into_iter().map(|(k, (_, v))| (k, v)).collect();
    series.sort_by(|a, b| a.0.cmp(&b.0));
    series
}

/// Parse a calendar-quarter *duration* frame like `CY2024Q1`. Rejects annual
/// (`CY2024`) and instantaneous (`CY2024Q1I`) frames.
fn quarter_from_frame(frame: Option<&str>) -> Option<QKey> {
    let rest = frame?.strip_prefix("CY")?;
    if rest.ends_with('I') {
        return None;
    }
    let (year, quarter) = rest.split_once('Q')?;
    if year.len() != 4 || !year.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let year: i32 = year.parse().ok()?;
    let quarter: u8 = quarter.parse().ok()?;
    (1..=4).contains(&quarter).then_some((year, quarter))
}

/// Derive a calendar quarter from a reporting period that spans roughly one
/// quarter (80–100 days). Multi-quarter and annual spans return `None`.
fn quarter_from_period(start: Option<&str>, end: Option<&str>) -> Option<(QKey, Option<NaiveDate>)> {
    let start = parse_date(start)?;
    let end = parse_date(end)?;
    let days = (end - start).num_days();
    if (80..=100).contains(&days) {
        Some((quarter_of(end), Some(end)))
    } else {
        None
    }
}

fn quarter_of(end: NaiveDate) -> QKey {
    (end.year(), ((end.month() - 1) / 3 + 1) as u8)
}

fn parse_date(s: Option<&str>) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s?, "%Y-%m-%d").ok()
}

/// Mid-quarter date used only to order frame points that omit an explicit end.
fn synthetic_end(key: QKey) -> NaiveDate {
    let (year, quarter) = key;
    let month = (quarter as u32) * 3;
    NaiveDate::from_ymd_opt(year, month, 15).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_point(frame: &str, end: &str, val: f64) -> ConceptPoint {
        ConceptPoint {
            start: None,
            end: Some(end.to_string()),
            val,
            form: Some("10-Q".to_string()),
            frame: Some(frame.to_string()),
        }
    }

    fn period_point(start: &str, end: &str, val: f64) -> ConceptPoint {
        ConceptPoint {
            start: Some(start.to_string()),
            end: Some(end.to_string()),
            val,
            form: Some("10-Q".to_string()),
            frame: None,
        }
    }

    fn q(year: i32, quarter: u8, val: f64) -> (QKey, f64) {
        ((year, quarter), val)
    }

    #[test]
    fn quarter_frame_accepts_duration_rejects_annual_and_instant() {
        assert_eq!(quarter_from_frame(Some("CY2024Q1")), Some((2024, 1)));
        assert_eq!(quarter_from_frame(Some("CY2024Q4")), Some((2024, 4)));
        assert_eq!(quarter_from_frame(Some("CY2024Q1I")), None); // instantaneous
        assert_eq!(quarter_from_frame(Some("CY2024")), None); // annual
        assert_eq!(quarter_from_frame(Some("CY2024Q5")), None); // invalid quarter
    }

    #[test]
    fn quarterly_series_reduces_frames_and_periods_and_ignores_annual() {
        let points = vec![
            frame_point("CY2023Q1", "2023-03-31", 10.0),
            frame_point("CY2023Q2", "2023-06-30", 12.0),
            // A ~quarter-length period without a frame is still captured.
            period_point("2023-07-01", "2023-09-30", 14.0), // CY2023Q3
            // An annual span must be ignored.
            period_point("2023-01-01", "2023-12-31", 48.0),
            // A restatement of Q1 filed for a later period end supersedes.
            ConceptPoint {
                start: None,
                end: Some("2023-04-15".into()),
                val: 11.0,
                form: Some("10-Q/A".into()),
                frame: Some("CY2023Q1".into()),
            },
        ];
        let series = quarterly_series(&points);
        assert_eq!(
            series,
            vec![q(2023, 1, 11.0), q(2023, 2, 12.0), q(2023, 3, 14.0)]
        );
    }

    #[test]
    fn yoy_series_matches_same_calendar_quarter() {
        let series = vec![
            q(2022, 1, 100.0),
            q(2022, 2, 100.0),
            q(2023, 1, 150.0), // +50% vs 2022Q1
            q(2023, 2, 80.0),  // -20% vs 2022Q2
        ];
        let yoy = yoy_series(&series);
        assert_eq!(yoy.len(), 2);
        assert!((yoy[0] - 0.5).abs() < 1e-9);
        assert!((yoy[1] + 0.2).abs() < 1e-9);
    }

    #[test]
    fn trough_reacceleration_with_profit_turn_scores_high() {
        // Micron-like: revenue YoY deeply negative, then accelerating back up,
        // while net income crosses from heavy losses to a profit.
        let revenue = vec![
            q(2022, 1, 100.0),
            q(2022, 2, 100.0),
            q(2022, 3, 100.0),
            q(2022, 4, 100.0),
            q(2023, 1, 55.0), // -45% YoY (trough)
            q(2023, 2, 60.0), // -40% YoY
            q(2023, 3, 80.0), // -20% YoY (accelerating up)
            q(2023, 4, 110.0), // +10% YoY (turned positive)
        ];
        let net_income = vec![
            q(2023, 1, -2.0),
            q(2023, 2, -1.5),
            q(2023, 3, -0.4),
            q(2023, 4, 0.6), // back to profit
        ];
        let qs = QuarterlySet {
            revenue,
            net_income,
            operating_income: Vec::new(),
        };
        let score = inflection_score(&qs).expect("enough data");
        assert!(score > 0.75, "expected strong inflection, got {score}");
    }

    #[test]
    fn steady_grower_sits_near_neutral() {
        // Constant ~20% YoY, steadily profitable: no inflection either way.
        let revenue = vec![
            q(2022, 1, 100.0),
            q(2022, 2, 100.0),
            q(2023, 1, 120.0),
            q(2023, 2, 120.0),
            q(2024, 1, 144.0),
            q(2024, 2, 144.0),
        ];
        let net_income = vec![q(2023, 2, 10.0), q(2024, 1, 10.0), q(2024, 2, 10.0)];
        let qs = QuarterlySet {
            revenue,
            net_income,
            operating_income: Vec::new(),
        };
        let score = inflection_score(&qs).expect("enough data");
        assert!(
            (0.45..=0.7).contains(&score),
            "steady grower should be neutral-ish, got {score}"
        );
    }

    #[test]
    fn decelerating_and_deteriorating_scores_low() {
        // YoY growth collapsing quarter over quarter, profits shrinking.
        let revenue = vec![
            q(2022, 1, 100.0),
            q(2022, 2, 100.0),
            q(2023, 1, 150.0), // +50% YoY
            q(2023, 2, 120.0), // +20% YoY (decelerating hard)
        ];
        let net_income = vec![q(2023, 1, 5.0), q(2023, 2, 1.0)]; // profit shrinking
        let qs = QuarterlySet {
            revenue,
            net_income,
            operating_income: Vec::new(),
        };
        let score = inflection_score(&qs).expect("enough data");
        assert!(score < 0.45, "expected weak inflection, got {score}");
    }

    #[test]
    fn margin_inflection_lifts_score() {
        let revenue = vec![
            q(2023, 1, 100.0),
            q(2023, 2, 100.0),
            q(2023, 3, 100.0),
        ];
        // Operating margin climbing 5% → 10% → 20%.
        let operating_income = vec![q(2023, 1, 5.0), q(2023, 2, 10.0), q(2023, 3, 20.0)];
        let qs = QuarterlySet {
            revenue,
            net_income: Vec::new(),
            operating_income,
        };
        let score = inflection_score(&qs).expect("margin alone is enough");
        assert!(score > 0.6, "rising margin should lift score, got {score}");
    }

    #[test]
    fn insufficient_history_returns_none() {
        let qs = QuarterlySet {
            revenue: vec![q(2024, 1, 100.0)],
            net_income: Vec::new(),
            operating_income: Vec::new(),
        };
        assert!(inflection_score(&qs).is_none());
    }
}
