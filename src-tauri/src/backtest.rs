//! Phase 6 — backtest & validation harness (compiled only under `cfg(test)`).
//!
//! Encodes the exact question that started this work — *"could it have caught
//! SanDisk and Micron before the boom?"* — as deterministic, point-in-time
//! fixtures and asserts that the **EarlyDetection** blend ranks those eventual
//! winners above the classic trailing-growth trap, and does so *better than* the
//! **Legacy** formula (which is structurally inverted for cyclicals: it loves a
//! company at peak trailing earnings and hates one at a cycle trough).
//!
//! These are pure regression tests over the real [`crate::scoring::composite_score`],
//! so they need no network and pin the engine's behavior against regressions.
//! The whole module is test-only and ships no runtime surface.
//!
//! Point-in-time note: each fixture is the signal vector a run *would* have
//! produced at the as-of date noted in its doc comment, reconstructed from
//! audited figures (EDGAR is itself point-in-time) and the cyclical setup — the
//! free path's approximation of a true survivorship-free backtest universe.

use crate::scoring::{composite_score, ScoringMode, ScoringWeights, Signals};

/// A labeled historical snapshot plus whether, in hindsight, it was an early
/// (pre-boom) winner.
struct Case {
    label: &'static str,
    signals: Signals,
    eventual_winner: bool,
}

fn score(mode: ScoringMode, s: &Signals) -> f64 {
    composite_score(&ScoringWeights::for_mode(mode), s).total
}

/// Micron at its FY2023 memory-cycle trough (as-of ~2023-09-30): trailing
/// fundamentals look *worst* — revenue roughly halved, a large annual loss —
/// exactly when forward signals are turning: losses narrowing, revenue
/// re-accelerating off the low, estimates revised up, and capacity/shortage
/// language returning to filings.
fn micron_2023_trough() -> Case {
    Case {
        label: "Micron (FY2023 trough)",
        signals: Signals {
            severity: 4,
            moat: 4,
            growth: 0.12, // audited trailing growth is awful at the bottom
            sentiment: Some(0.1),
            change_pct: 0.0,
            inflection: Some(0.85),
            technical: Some(0.62),
            revisions: Some(0.78),
            insider: Some(0.6),
            filing: Some(0.8),
        },
        eventual_winner: true,
    }
}

/// SanDisk at its Feb-2025 relisting (as-of ~2025-02-21): a post-cutoff name the
/// local LLM never knew, early in a fresh memory upcycle — limited trailing
/// history (neutral growth, not a penalty) but a constructive forward setup.
fn sandisk_2025_relist() -> Case {
    Case {
        label: "SanDisk (2025 relisting)",
        signals: Signals {
            severity: 4,
            moat: 3,
            growth: 0.5, // too little history → neutral
            sentiment: Some(0.15),
            change_pct: 0.0,
            inflection: Some(0.72),
            technical: Some(0.70),
            revisions: Some(0.68),
            insider: None,
            filing: Some(0.7),
        },
        eventual_winner: true,
    }
}

/// The trap the Legacy formula loves (as-of ~2023-09-30): a late-cycle commodity
/// name at *peak* trailing earnings, already rolling over — decelerating,
/// estimates cut, extended price, "oversupply/glut" language in filings.
fn late_cycle_trap() -> Case {
    Case {
        label: "Late-cycle peak (trap)",
        signals: Signals {
            severity: 2,
            moat: 2,
            growth: 0.92, // gorgeous trailing growth — and a value trap
            sentiment: Some(0.2),
            change_pct: 0.0,
            inflection: Some(0.18),
            technical: Some(0.30),
            revisions: Some(0.28),
            insider: None,
            filing: Some(0.25),
        },
        eventual_winner: false,
    }
}

/// A stable, no-catalyst name: everything neutral. Anchors the "nothing
/// interesting here" baseline.
fn stable_no_catalyst() -> Case {
    Case {
        label: "Stable, no catalyst",
        signals: Signals {
            severity: 2,
            moat: 3,
            growth: 0.5,
            sentiment: None,
            change_pct: 0.0,
            inflection: None,
            technical: None,
            revisions: None,
            insider: None,
            filing: None,
        },
        eventual_winner: false,
    }
}

fn universe() -> Vec<Case> {
    vec![
        micron_2023_trough(),
        sandisk_2025_relist(),
        late_cycle_trap(),
        stable_no_catalyst(),
    ]
}

/// Rank the universe under `mode`, best-first, returning labels.
fn ranked_labels(mode: ScoringMode) -> Vec<&'static str> {
    let mut scored: Vec<(&'static str, f64)> = universe()
        .iter()
        .map(|c| (c.label, score(mode, &c.signals)))
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    scored.into_iter().map(|(l, _)| l).collect()
}

#[test]
fn early_detection_ranks_winners_above_every_laggard() {
    let cases = universe();
    let worst_winner = cases
        .iter()
        .filter(|c| c.eventual_winner)
        .map(|c| score(ScoringMode::EarlyDetection, &c.signals))
        .fold(f64::INFINITY, f64::min);
    let best_laggard = cases
        .iter()
        .filter(|c| !c.eventual_winner)
        .map(|c| score(ScoringMode::EarlyDetection, &c.signals))
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(
        worst_winner > best_laggard,
        "early-detection should rank Micron & SanDisk above every laggard \
         (worst winner {worst_winner:.1} vs best laggard {best_laggard:.1})"
    );
}

#[test]
fn legacy_is_fooled_by_the_trailing_growth_trap() {
    // The structural flaw: Legacy ranks the late-cycle peak ABOVE Micron at the bottom.
    let micron = score(ScoringMode::Legacy, &micron_2023_trough().signals);
    let trap = score(ScoringMode::Legacy, &late_cycle_trap().signals);
    assert!(
        trap > micron,
        "legacy is expected to be inverted for cyclicals (trap {trap:.1} > micron {micron:.1})"
    );
}

#[test]
fn early_detection_fixes_the_inversion() {
    let micron = score(ScoringMode::EarlyDetection, &micron_2023_trough().signals);
    let trap = score(ScoringMode::EarlyDetection, &late_cycle_trap().signals);
    assert!(
        micron > trap,
        "early-detection should rank Micron above the trap (micron {micron:.1} > trap {trap:.1})"
    );
}

#[test]
fn early_detection_lifts_the_trough_winner_vs_legacy() {
    let legacy = score(ScoringMode::Legacy, &micron_2023_trough().signals);
    let early = score(ScoringMode::EarlyDetection, &micron_2023_trough().signals);
    assert!(
        early > legacy + 5.0,
        "forward signals should materially lift Micron at the trough \
         (early {early:.1} vs legacy {legacy:.1})"
    );
}

#[test]
fn early_detection_puts_both_winners_in_the_top_two() {
    let early = ranked_labels(ScoringMode::EarlyDetection);
    let top2: std::collections::HashSet<&str> = early.iter().take(2).copied().collect();
    assert!(
        top2.contains("Micron (FY2023 trough)") && top2.contains("SanDisk (2025 relisting)"),
        "early-detection should put both winners in the top two, got {early:?}"
    );
}
