//! Phase 5 — on-demand change detection (calm UX, no daemon).
//!
//! Diffs a fresh run against the previous cached run for the same saved query,
//! so the UI can answer "what changed since last time?" the moment a watchlist
//! is re-run — without any always-on background monitoring. Pure and
//! unit-tested; the pipeline calls [`diff_runs`] once per run when a prior
//! result exists.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::model::Candidate;

/// Minimum rank movement (in positions) worth reporting.
const RANK_DELTA: usize = 3;
/// Minimum score movement (on the 0..100 scale) worth reporting.
const SCORE_DELTA: f64 = 5.0;

/// A ticker that entered or left the ranked set between runs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub ticker: String,
    pub company: String,
    pub score: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing: Option<String>,
}

/// A material change in a ticker's rank between runs (ranks are 1-based).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RankMove {
    pub ticker: String,
    pub company: String,
    pub from: usize,
    pub to: usize,
}

/// A material change in a ticker's composite score between runs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScoreMove {
    pub ticker: String,
    pub company: String,
    pub from: f64,
    pub to: f64,
}

/// A change in a ticker's cycle-timing label between runs (e.g. Building → Early).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimingShift {
    pub ticker: String,
    pub company: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

/// Everything that changed between the previous run and this one.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RunChanges {
    pub new_entrants: Vec<Entry>,
    pub dropped: Vec<Entry>,
    pub rank_moves: Vec<RankMove>,
    pub score_moves: Vec<ScoreMove>,
    pub timing_shifts: Vec<TimingShift>,
}

impl RunChanges {
    /// Whether nothing changed at all — used to suppress an empty event/panel.
    pub fn is_empty(&self) -> bool {
        self.new_entrants.is_empty()
            && self.dropped.is_empty()
            && self.rank_moves.is_empty()
            && self.score_moves.is_empty()
            && self.timing_shifts.is_empty()
    }
}

fn key(c: &Candidate) -> String {
    c.ticker.to_ascii_uppercase()
}

fn entry(c: &Candidate) -> Entry {
    Entry {
        ticker: c.ticker.clone(),
        company: c.company.clone(),
        score: c.score,
        timing: c.timing.clone(),
    }
}

/// Diff two ranked candidate lists (each already sorted best-first) into a
/// structured set of changes. Pure: order is deterministic (new entrants and
/// move lists follow the current run's rank order; drops follow the previous
/// run's).
pub fn diff_runs(prev: &[Candidate], curr: &[Candidate]) -> RunChanges {
    let prev_rank: HashMap<String, usize> =
        prev.iter().enumerate().map(|(i, c)| (key(c), i)).collect();
    let curr_keys: HashSet<String> = curr.iter().map(key).collect();

    let mut changes = RunChanges::default();

    // New entrants: present now, absent before.
    for c in curr {
        if !prev_rank.contains_key(&key(c)) {
            changes.new_entrants.push(entry(c));
        }
    }
    // Dropped: present before, absent now.
    for c in prev {
        if !curr_keys.contains(&key(c)) {
            changes.dropped.push(entry(c));
        }
    }
    // Moves among tickers present in both runs.
    for (i, c) in curr.iter().enumerate() {
        let Some(&p) = prev_rank.get(&key(c)) else {
            continue;
        };
        let prev_c = &prev[p];
        if i.abs_diff(p) >= RANK_DELTA {
            changes.rank_moves.push(RankMove {
                ticker: c.ticker.clone(),
                company: c.company.clone(),
                from: p + 1,
                to: i + 1,
            });
        }
        if (c.score - prev_c.score).abs() >= SCORE_DELTA {
            changes.score_moves.push(ScoreMove {
                ticker: c.ticker.clone(),
                company: c.company.clone(),
                from: prev_c.score,
                to: c.score,
            });
        }
        if c.timing != prev_c.timing {
            changes.timing_shifts.push(TimingShift {
                ticker: c.ticker.clone(),
                company: c.company.clone(),
                from: prev_c.timing.clone(),
                to: c.timing.clone(),
            });
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(ticker: &str, score: f64, timing: Option<&str>) -> Candidate {
        Candidate {
            ticker: ticker.to_string(),
            company: format!("{ticker} Inc"),
            score,
            timing: timing.map(String::from),
            ..Default::default()
        }
    }

    #[test]
    fn identical_runs_report_no_changes() {
        let prev = vec![cand("MU", 80.0, Some("Early")), cand("WDC", 60.0, None)];
        let curr = prev.clone();
        assert!(diff_runs(&prev, &curr).is_empty());
    }

    #[test]
    fn detects_new_entrants_and_drops() {
        let prev = vec![cand("MU", 80.0, None), cand("WDC", 60.0, None)];
        let curr = vec![cand("MU", 80.0, None), cand("SNDK", 70.0, None)];
        let ch = diff_runs(&prev, &curr);
        assert_eq!(ch.new_entrants.len(), 1);
        assert_eq!(ch.new_entrants[0].ticker, "SNDK");
        assert_eq!(ch.dropped.len(), 1);
        assert_eq!(ch.dropped[0].ticker, "WDC");
    }

    #[test]
    fn detects_material_rank_jump_but_ignores_small_ones() {
        // E climbs from last (rank 5) to first (rank 1) → reported.
        // A slips from rank 1 to rank 2 → below threshold, ignored.
        let prev = vec![
            cand("A", 90.0, None),
            cand("B", 80.0, None),
            cand("C", 70.0, None),
            cand("D", 60.0, None),
            cand("E", 50.0, None),
        ];
        let curr = vec![
            cand("E", 95.0, None),
            cand("A", 90.0, None),
            cand("B", 80.0, None),
            cand("C", 70.0, None),
            cand("D", 60.0, None),
        ];
        let ch = diff_runs(&prev, &curr);
        let e = ch.rank_moves.iter().find(|m| m.ticker == "E").unwrap();
        assert_eq!((e.from, e.to), (5, 1));
        assert!(ch.rank_moves.iter().all(|m| m.ticker != "A"));
    }

    #[test]
    fn detects_score_and_timing_moves() {
        let prev = vec![cand("MU", 50.0, Some("Building"))];
        let curr = vec![cand("MU", 62.0, Some("Early"))];
        let ch = diff_runs(&prev, &curr);
        assert_eq!(ch.score_moves.len(), 1);
        assert_eq!(ch.score_moves[0].from, 50.0);
        assert_eq!(ch.score_moves[0].to, 62.0);
        assert_eq!(ch.timing_shifts.len(), 1);
        assert_eq!(ch.timing_shifts[0].from.as_deref(), Some("Building"));
        assert_eq!(ch.timing_shifts[0].to.as_deref(), Some("Early"));
    }

    #[test]
    fn small_score_move_is_ignored() {
        let prev = vec![cand("MU", 60.0, None)];
        let curr = vec![cand("MU", 63.0, None)]; // +3 < 5 threshold
        assert!(diff_runs(&prev, &curr).score_moves.is_empty());
    }
}
