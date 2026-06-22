//! Room-to-run (convexity) signal — the size lens for *meteoric* potential.
//!
//! Every other signal asks "is this a good company at a good moment?" This one
//! asks a different, complementary question: **does it still have room to
//! multiply?** A genuinely meteoric (multi-bagger) move is overwhelmingly the
//! province of small/mid caps — a $4B company can plausibly 10x; a $400B one
//! effectively cannot. The engine was otherwise size-agnostic by design ("price
//! is never a filter, large caps welcome"), which is correct for a *thesis
//! validator* but a blind spot for an *early meteoric-rise detector*.
//!
//! The score is deliberately a **band-pass**, not "smaller is always better":
//!  * a log-space Gaussian over market cap peaks in the small/mid-cap sweet spot
//!    (~$1B–$20B), so real companies with real room score highest;
//!  * it tapers toward mega-cap (no room to multiply) **and** toward micro-cap
//!    (the junk/illiquidity zone), so the engine doesn't chase $50M lottery
//!    tickets;
//!  * a liquidity gate further damps names that barely trade, screening out
//!    untradeable micro-caps even if their size factor looks tempting.
//!
//! Pure & testable like the other scorers (`fundamentals::growth_score`,
//! `inflection::inflection_score`): no network, no I/O, just arithmetic. The
//! signal is `Option<f64>` and sits **neutral** (returns `None`) whenever size
//! is unknown, so a missing market cap never penalizes a pick.

/// Center of the market-cap sweet spot in `log10(USD)` space. `10^9.6 ≈ $4.0B`
/// — the geometric middle of the ~$1B–$20B band where there is still real room
/// to compound several-fold without being a micro-cap lottery ticket.
const LOG_CENTER: f64 = 9.6;

/// Width (std-dev) of the Gaussian in `log10` decades. At `0.9`, the whole
/// ~$1B–$20B band scores high (>=~0.74) while mega-caps fall away steeply
/// (`$100B ≈ 0.30`, `$1T ≈ 0.03`) and micro-caps are damped (`$200M ≈ 0.35`).
const LOG_SIGMA: f64 = 0.9;

/// Daily dollar volume (price * average volume, USD) at or above which
/// liquidity is a non-issue and applies no penalty.
const LIQ_FULL: f64 = 2_000_000.0;

/// Multiplier floor for effectively untradeable names (near-zero dollar volume
/// or unknown). Never zero: thin tape lowers conviction but shouldn't erase an
/// otherwise compelling small-cap setup outright.
const LIQ_FLOOR: f64 = 0.4;

/// Convexity / room-to-run in `0..1`, or `None` when market cap is unknown (the
/// signal then sits neutral in scoring and never penalizes the pick).
///
/// `market_cap_usd` and `dollar_volume_usd` must be in USD; callers gate on a
/// USD quote currency so the figures are directly comparable (the broad
/// log-scale band tolerates the residual noise of split-adjusted snapshots).
pub fn convexity_score(market_cap_usd: Option<f64>, dollar_volume_usd: f64) -> Option<f64> {
    let mc = market_cap_usd?;
    if !(mc.is_finite() && mc > 0.0) {
        return None;
    }
    let score = size_factor(mc) * liquidity_multiplier(dollar_volume_usd);
    Some(score.clamp(0.0, 1.0))
}

/// Log-space Gaussian band-pass over market cap: 1.0 at the sweet-spot center,
/// tapering symmetrically (in decades) toward both micro-cap and mega-cap.
fn size_factor(market_cap_usd: f64) -> f64 {
    let z = (market_cap_usd.log10() - LOG_CENTER) / LOG_SIGMA;
    (-0.5 * z * z).exp()
}

/// Liquidity gate: ramps a multiplier from [`LIQ_FLOOR`] (untradeable / unknown)
/// up to `1.0` once daily dollar volume reaches [`LIQ_FULL`]. Keeps genuinely
/// illiquid micro-caps from scoring well on size alone.
fn liquidity_multiplier(dollar_volume_usd: f64) -> f64 {
    if !(dollar_volume_usd.is_finite() && dollar_volume_usd > 0.0) {
        return LIQ_FLOOR;
    }
    let frac = (dollar_volume_usd / LIQ_FULL).clamp(0.0, 1.0);
    LIQ_FLOOR + (1.0 - LIQ_FLOOR) * frac
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Liquid enough not to be gated, so size alone drives these.
    const LIQUID: f64 = 50_000_000.0;

    #[test]
    fn unknown_size_is_neutral_not_a_penalty() {
        assert_eq!(convexity_score(None, LIQUID), None);
    }

    #[test]
    fn nonsense_market_caps_are_neutral() {
        assert_eq!(convexity_score(Some(0.0), LIQUID), None);
        assert_eq!(convexity_score(Some(-5.0), LIQUID), None);
        assert_eq!(convexity_score(Some(f64::NAN), LIQUID), None);
        assert_eq!(convexity_score(Some(f64::INFINITY), LIQUID), None);
    }

    #[test]
    fn sweet_spot_scores_highest() {
        // A ~$4B name (sweet-spot center) beats both a micro-cap and a mega-cap.
        let micro = convexity_score(Some(150_000_000.0), LIQUID).unwrap();
        let sweet = convexity_score(Some(4_000_000_000.0), LIQUID).unwrap();
        let mega = convexity_score(Some(1_000_000_000_000.0), LIQUID).unwrap();
        assert!(sweet > micro, "sweet {sweet} should beat micro {micro}");
        assert!(sweet > mega, "sweet {sweet} should beat mega {mega}");
        assert!(sweet > 0.95, "sweet spot should be near the top: {sweet}");
    }

    #[test]
    fn mega_caps_have_little_room() {
        // The core meteoric insight: a $400B+ company can't realistically 10x,
        // so it must score low even though it is a fine, liquid business.
        let mega = convexity_score(Some(500_000_000_000.0), LIQUID).unwrap();
        assert!(mega < 0.15, "mega-cap room-to-run should be small: {mega}");
    }

    #[test]
    fn the_band_covers_one_to_twenty_billion() {
        // Both edges of the intended sweet-spot band stay strong.
        let low = convexity_score(Some(1_000_000_000.0), LIQUID).unwrap();
        let high = convexity_score(Some(20_000_000_000.0), LIQUID).unwrap();
        assert!(low > 0.7, "$1B should score high: {low}");
        assert!(high > 0.7, "$20B should score high: {high}");
    }

    #[test]
    fn illiquidity_damps_an_otherwise_ideal_size() {
        // Same ideal market cap, but one barely trades. The thin-tape name must
        // be damped toward the liquidity floor, not rewarded for being tiny.
        let liquid = convexity_score(Some(3_000_000_000.0), 25_000_000.0).unwrap();
        let illiquid = convexity_score(Some(3_000_000_000.0), 50_000.0).unwrap();
        assert!(illiquid < liquid, "illiquid {illiquid} should be damped vs {liquid}");
        assert!(illiquid < 0.45, "thin tape should pull near the floor: {illiquid}");
    }

    #[test]
    fn zero_or_unknown_volume_falls_to_the_floor_but_not_zero() {
        // Ideal size (~$4B) with no volume: the liquidity floor caps the score at
        // ~LIQ_FLOOR (size factor is ~1 at the center) and never zeroes it.
        let v = convexity_score(Some(4_000_000_000.0), 0.0).unwrap();
        assert!(v <= LIQ_FLOOR + 1e-9, "no volume must not exceed the floor: {v}");
        assert!(v > 0.39, "ideal size at the floor should sit ~LIQ_FLOOR: {v}");
    }

    #[test]
    fn stays_within_unit_range() {
        for mc in [1e6, 1e7, 1e8, 5e8, 1e9, 4e9, 2e10, 1e11, 5e11, 3e12] {
            for dv in [0.0, 1e5, 1e6, 1e7, 1e9] {
                let v = convexity_score(Some(mc), dv).unwrap();
                assert!((0.0..=1.0).contains(&v), "out of range for mc={mc} dv={dv}: {v}");
            }
        }
    }
}
