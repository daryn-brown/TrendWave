use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::Arc;
use tokio::sync::Semaphore;

use crate::error::AppResult;
use crate::feeds;
use crate::fundamentals::{self, YahooEnrich};
use crate::inflection;
use crate::model::{Bottleneck, Candidate, ProgressEvent, ResearchResult, DISCLAIMER};
use crate::ollama::OllamaClient;
use crate::providers::DataProvider;
use crate::scoring::{self, ScoringMode, ScoringWeights, SignalBreakdown, Signals};
use crate::screener;
use crate::settings::Settings;

const SYSTEM_BOTTLENECK: &str = "\
You are a sharp equity research analyst who specializes in SUPPLY-CHAIN and CAPACITY \
BOTTLENECKS. Given a user's question about an industry, first identify the real, current \
chokepoints (scarce components, limited production capacity, single-source suppliers, \
permitting/logistics constraints). Then, for each bottleneck, name the PUBLIC COMPANIES \
BEST POSITIONED TO SOLVE OR MONOPOLIZE it — the dominant suppliers, critical enablers, \
picks-and-shovels plays, or emerging challengers with the most upside. Rank by competitive \
positioning and upside, NOT by share price — include large caps if they are the dominant \
beneficiary. Use real, currently US-listed tickers only. Return at least 3 companies in \
total. Respond with STRICT JSON only, no prose, matching: \
{\"industry\":string,\"summary\":string,\"bottlenecks\":[{\"title\":string,\
\"description\":string,\"severity\":1-5,\"companies\":[{\"company\":string,\"ticker\":string,\
\"thesis\":string,\"moat\":1-5,\"upside\":1-5,\"upside_rationale\":string}]}]}. \
severity = how acute the bottleneck is (5=severe). moat = how dominant or monopoly-like the \
company's position is in solving it (5=near-monopoly). upside = potential share-price upside \
(5=highest). thesis = why this company is positioned to win this bottleneck. Include 2-4 \
companies per bottleneck.";

const SYSTEM_SENTIMENT: &str = "\
You judge market sentiment from news headlines about one company. Respond with STRICT JSON \
only: {\"score\":number} where score is between -1 (very bearish) and 1 (very bullish).";

/// Upper bound on candidates the Phase 2 screener may add to a single run, so a
/// broad discovery net can't balloon the downstream pricing/fundamentals/news
/// fan-out. Discovered names still compete on score and are trimmed to
/// `max_results` like everything else.
const MAX_DISCOVERED: usize = 10;

// Scoring weights and the composite formula now live in `crate::scoring`
// (`ScoringWeights` presets + `composite_score`). `score_candidate` below
// delegates to them with the legacy preset so existing callers and tests keep
// their exact behavior, while the pipeline can rank with any `ScoringMode`.

#[derive(Deserialize)]
struct BottleneckPlan {
    industry: Option<String>,
    summary: Option<String>,
    #[serde(default)]
    bottlenecks: Vec<PlanBottleneck>,
}

#[derive(Deserialize)]
struct PlanBottleneck {
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default, deserialize_with = "de_rating")]
    severity: Option<u8>,
    #[serde(default)]
    companies: Vec<PlanCompany>,
}

#[derive(Deserialize)]
struct PlanCompany {
    company: String,
    ticker: Option<String>,
    thesis: Option<String>,
    #[serde(default, deserialize_with = "de_rating")]
    moat: Option<u8>,
    #[serde(default, deserialize_with = "de_rating")]
    upside: Option<u8>,
    upside_rationale: Option<String>,
}

/// Coerce a model-provided 1–5 rating into `Option<u8>`, tolerating the shapes
/// local models actually emit: integers (`4`), floats (`3.5`), or numeric
/// strings (`"4"`). The value is rounded to the nearest integer and clamped to
/// the documented 1..=5 band. Anything non-numeric (null, bool, array, object,
/// prose) becomes `None` — so a single odd value can never abort the whole
/// `BottleneckPlan` parse the way a bare `u8` field does (it previously failed
/// with "invalid type: floating point `3.5`, expected u8").
fn de_rating<'de, D>(deserializer: D) -> Result<Option<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<Value>::deserialize(deserializer)?;
    let n = match raw {
        Some(Value::Number(num)) => num.as_f64(),
        Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    Ok(n.filter(|x| x.is_finite()).map(round_rating))
}

/// Round to nearest and clamp into the documented 1..=5 rating band.
fn round_rating(x: f64) -> u8 {
    x.round().clamp(1.0, 5.0) as u8
}

#[derive(Deserialize)]
struct SentimentOut {
    score: f64,
}

/// A candidate plus the data we need to score it but don't expose verbatim.
struct Working {
    candidate: Candidate,
    severity: u8,
    /// Early-detection signals computed in `EarlyDetection` mode (best-effort,
    /// `None` when unavailable or in `Legacy` mode → scored neutral).
    inflection: Option<f64>,
    revisions: Option<f64>,
}

/// Legacy positional scorer used by the equivalence/monotonicity test-suite.
///
/// Production code scores through `scoring::composite_score` directly (to also
/// capture the per-term breakdown and honor the user's `ScoringMode`); this thin
/// wrapper reproduces the original five-term formula with the **legacy** weights
/// so the tests below can assert that behavior with a compact call. Test-only.
#[cfg(test)]
fn score_candidate(
    severity: u8,
    moat: u8,
    growth: f64,
    sentiment: Option<f64>,
    change_pct: f64,
) -> f64 {
    let signals = Signals {
        severity,
        moat,
        growth,
        sentiment,
        change_pct,
        ..Default::default()
    };
    scoring::composite_score(&ScoringWeights::legacy(), &signals).total
}

/// Scale every contribution in a breakdown by `factor`, keeping `total` equal to
/// the sum of its (now scaled) parts. Used to apply the identity-mismatch penalty
/// to the whole breakdown so the displayed contributions still add up to `score`.
fn scale_breakdown(b: &mut SignalBreakdown, factor: f64) {
    b.severity *= factor;
    b.moat *= factor;
    b.growth *= factor;
    b.sentiment *= factor;
    b.momentum *= factor;
    b.inflection *= factor;
    b.technical *= factor;
    b.revisions *= factor;
    b.insider *= factor;
    b.filing *= factor;
    b.total *= factor;
}

/// Multiplicative penalty applied when a candidate's ticker resolves to a
/// company that plainly contradicts the model's claimed business (see
/// `name_matches`). A confirmed misattribution — e.g. a gold miner pitched as a
/// solid-state battery-equipment maker — is demoted out of the top tier so it
/// can't masquerade as a legitimate high-conviction pick. It is *halved*, never
/// zeroed: the model sometimes returns the bare ticker as the company name,
/// which trips the check, so we keep the pick visible (with its warning) rather
/// than risk burying a legitimate one.
const IDENTITY_MISMATCH_PENALTY: f64 = 0.5;

fn normalize_ticker(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('$')
        .to_ascii_uppercase()
        .replace(' ', "")
}

/// Tokenize a company name for loose identity comparison: lowercase, split on
/// non-alphanumerics, and drop common corporate suffixes / filler so
/// "Agnico Eagle Mines Limited" and "Agnico Eagle Mines" compare equal.
fn name_tokens(name: &str) -> Vec<String> {
    const NOISE: &[&str] = &[
        "inc", "incorporated", "corp", "corporation", "co", "company", "ltd",
        "limited", "plc", "llc", "lp", "holdings", "holding", "group", "the",
        "sa", "ag", "nv", "se", "and", "class",
    ];
    name.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| t.len() > 1 && !NOISE.contains(t))
        .map(|t| t.to_string())
        .collect()
}

/// Whether the model's claimed company name plausibly refers to the same entity
/// as the authoritative registrant name the price feed returns for that ticker.
/// Conservative on purpose: returns `true` (a match, no warning) unless *both*
/// names carry meaningful tokens and share none — a genuine contradiction like
/// ticker "AEM" (Agnico Eagle Mines) described as a battery-equipment maker.
/// Empty or uninformative names never trigger a false alarm.
fn name_matches(claimed: &str, authoritative: &str) -> bool {
    let a = name_tokens(claimed);
    let b = name_tokens(authoritative);
    if a.is_empty() || b.is_empty() {
        return true;
    }
    a.iter().any(|t| b.contains(t))
}

/// Run the full prompt → bottlenecks → candidates → ranked results pipeline,
/// emitting progress as it goes. Kept free of any Tauri types so it stays
/// testable; the caller adapts `emit` to a channel.
pub async fn run_research<F: Fn(ProgressEvent)>(
    ollama: &OllamaClient,
    http: &reqwest::Client,
    settings: &Settings,
    prompt: &str,
    owned: &BTreeSet<String>,
    emit: &F,
) -> AppResult<ResearchResult> {
    emit(ProgressEvent::Stage {
        stage: "understanding".into(),
        message: "Reading your question…".into(),
    });

    emit(ProgressEvent::Stage {
        stage: "bottlenecks".into(),
        message: "Identifying current bottlenecks…".into(),
    });
    // Give the model read-only context about what the user already holds, so it
    // can acknowledge existing positions. Ranking still happens on merit below.
    let user_prompt = if owned.is_empty() {
        prompt.to_string()
    } else {
        let held = owned.iter().cloned().collect::<Vec<_>>().join(", ");
        format!(
            "{prompt}\n\nContext (read-only): the user already holds these tickers in their \
             Robinhood account: {held}. This is background only — rank purely on merit."
        )
    };
    let plan: BottleneckPlan = ollama.generate_json(SYSTEM_BOTTLENECK, &user_prompt).await?;

    let industry = plan
        .industry
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| prompt.trim().to_string());
    let summary = plan.summary.unwrap_or_default();

    let bottlenecks: Vec<Bottleneck> = plan
        .bottlenecks
        .iter()
        .map(|b| Bottleneck {
            title: b.title.clone(),
            description: b.description.clone(),
            severity: b.severity.unwrap_or(3).clamp(1, 5),
        })
        .collect();
    emit(ProgressEvent::Bottlenecks {
        items: bottlenecks.clone(),
    });

    // Flatten plan into working candidates, de-duplicating by ticker/company.
    let mut working: Vec<Working> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for b in &plan.bottlenecks {
        let severity = b.severity.unwrap_or(3).clamp(1, 5);
        for c in &b.companies {
            let ticker = normalize_ticker(c.ticker.as_deref().unwrap_or_default());
            let key = if ticker.is_empty() {
                c.company.to_ascii_lowercase()
            } else {
                ticker.clone()
            };
            if key.is_empty() || seen.contains(&key) {
                continue;
            }
            seen.push(key);
            working.push(Working {
                severity,
                inflection: None,
                revisions: None,
                candidate: Candidate {
                    ticker,
                    company: c.company.clone(),
                    verified_name: None,
                    identity_mismatch: false,
                    price: None,
                    bottleneck: b.title.clone(),
                    thesis: c.thesis.clone().unwrap_or_default(),
                    moat: c.moat.unwrap_or(3).clamp(1, 5),
                    upside: c.upside.unwrap_or(3).clamp(1, 5),
                    upside_rationale: c.upside_rationale.clone().unwrap_or_default(),
                    growth: None,
                    growth_score: 0.0,
                    sentiment: None,
                    news: Vec::new(),
                    score: 0.0,
                    breakdown: None,
                    timing: None,
                    discovery: None,
                    owned: false,
                },
            });
        }
    }

    // Candidate discovery (Phase 2): in EarlyDetection mode, augment the model's
    // picks with names surfaced from free live sources — SEC EDGAR full-text
    // search on each bottleneck's own vocabulary plus Yahoo market screeners.
    // This breaks the model's training-cutoff ceiling: an EDGAR search for "NAND"
    // surfaces Micron / Western Digital on-thesis, and a momentum screener
    // surfaces post-cutoff relistings (e.g. SanDisk) the model has never heard
    // of. Gated on EarlyDetection so Legacy stays byte-identical and skips the
    // extra network calls entirely.
    if settings.scoring_mode == ScoringMode::EarlyDetection {
        emit(ProgressEvent::Stage {
            stage: "discovery".into(),
            message: "Discovering candidates the prompt didn't name…".into(),
        });
        let inputs: Vec<(String, String)> = bottlenecks
            .iter()
            .map(|b| (b.title.clone(), b.description.clone()))
            .collect();
        let severities: std::collections::HashMap<String, u8> =
            bottlenecks.iter().map(|b| (b.title.clone(), b.severity)).collect();
        let discovered = screener::discover(http, &inputs, MAX_DISCOVERED).await;

        // Tag provenance: any model-named pick the screener also surfaced is
        // independently corroborated ("both"); the rest are "model".
        let corroborated: BTreeSet<String> = discovered
            .iter()
            .map(|d| d.ticker.to_ascii_uppercase())
            .collect();
        for w in working.iter_mut() {
            let both = corroborated.contains(&w.candidate.ticker.to_ascii_uppercase());
            w.candidate.discovery = Some(if both { "both" } else { "model" }.into());
        }

        // Admit only genuinely new tickers — the model still owns the thesis for
        // anything it named.
        for d in discovered {
            let key = d.ticker.to_ascii_lowercase();
            if key.is_empty() || seen.contains(&key) {
                continue;
            }
            seen.push(key);
            let severity = severities.get(&d.bottleneck).copied().unwrap_or(3);
            let thesis = if d.source == screener::SRC_EDGAR {
                format!(
                    "Surfaced by SEC EDGAR full-text search for “{}” — recent filings discuss this bottleneck.",
                    d.context
                )
            } else {
                format!("Surfaced by the Yahoo “{}” screener.", d.context)
            };
            working.push(Working {
                severity,
                inflection: None,
                revisions: None,
                candidate: Candidate {
                    ticker: d.ticker,
                    company: d.company,
                    verified_name: None,
                    identity_mismatch: false,
                    price: None,
                    bottleneck: d.bottleneck,
                    thesis,
                    moat: 3,
                    upside: 3,
                    upside_rationale: String::new(),
                    growth: None,
                    growth_score: 0.0,
                    sentiment: None,
                    news: Vec::new(),
                    score: 0.0,
                    breakdown: None,
                    timing: None,
                    discovery: Some(d.source.into()),
                    owned: false,
                },
            });
        }
    }

    // Price every candidate concurrently (for context/display only), falling
    // back to name→symbol resolution. Price never filters anyone out.
    emit(ProgressEvent::Stage {
        stage: "pricing".into(),
        message: "Checking live prices for context…".into(),
    });
    let mut set = tokio::task::JoinSet::new();
    for (idx, w) in working.iter().enumerate() {
        let http = http.clone();
        let ticker = w.candidate.ticker.clone();
        let company = w.candidate.company.clone();
        set.spawn(async move {
            let mut symbol = ticker.clone();
            let mut price = if symbol.is_empty() {
                Err(crate::error::AppError::EmptyFeed("no ticker".into()))
            } else {
                feeds::fetch_price(&http, &symbol).await
            };
            if price.is_err() && !company.is_empty() {
                if let Ok(resolved) = feeds::resolve_symbol(&http, &company).await {
                    if let Ok(p) = feeds::fetch_price(&http, &resolved).await {
                        symbol = normalize_ticker(&resolved);
                        price = Ok(p);
                    }
                }
            }
            (idx, symbol, price.ok())
        });
    }
    while let Some(joined) = set.join_next().await {
        if let Ok((idx, symbol, price)) = joined {
            working[idx].candidate.ticker = symbol;
            working[idx].candidate.price = price;
        }
    }

    // Identity check: the model invents company+ticker pairs and the pipeline
    // otherwise trusts them blindly, so a real ticker stamped onto the wrong
    // business (e.g. "AEM" sold as a battery-equipment maker when it is really
    // Agnico Eagle Mines, a gold miner) flows through with real, high-scoring
    // data. Compare the model's claimed company against the authoritative name
    // the price feed returns for that ticker, surface the real name, and flag a
    // plain contradiction so the card can warn instead of silently misleading.
    for w in working.iter_mut() {
        let real = w.candidate.price.as_ref().and_then(|p| p.name.clone());
        if let Some(real) = real {
            w.candidate.identity_mismatch = !name_matches(&w.candidate.company, &real);
            w.candidate.verified_name = Some(real);
        }
    }

    // Drop only the ones with no usable ticker at all; everything the model
    // surfaced otherwise gets recommended (the user always wants picks back).
    working.retain(|w| !w.candidate.ticker.is_empty());

    // Real growth research: pull audited fundamentals from SEC EDGAR (with
    // opportunistic Yahoo enrichment) concurrently but politely. This is what
    // actually drives the growth term in scoring — not the model's guess.
    if settings.use_fundamentals {
        emit(ProgressEvent::Stage {
            stage: "growth".into(),
            message: "Researching revenue & earnings growth…".into(),
        });
        let yahoo = YahooEnrich::init().await;
        let limit = Arc::new(Semaphore::new(4));
        let mut set = tokio::task::JoinSet::new();
        for (idx, w) in working.iter().enumerate() {
            let http = http.clone();
            let ticker = w.candidate.ticker.clone();
            let yahoo = yahoo.clone();
            let limit = limit.clone();
            set.spawn(async move {
                let _permit = limit.acquire_owned().await.ok();
                let growth = fundamentals::fetch_growth(&http, &ticker, yahoo.as_ref()).await;
                (idx, growth)
            });
        }
        while let Some(joined) = set.join_next().await {
            if let Ok((idx, growth)) = joined {
                working[idx].candidate.growth = growth;
            }
        }
    }

    // News + sentiment for the survivors.
    if settings.use_news {
        emit(ProgressEvent::Stage {
            stage: "news".into(),
            message: "Scanning news & sentiment…".into(),
        });
        for w in working.iter_mut() {
            let news = feeds::fetch_news(http, &w.candidate.ticker, 5)
                .await
                .unwrap_or_default();
            if !news.is_empty() {
                if let Ok(score) = sentiment_for(ollama, &w.candidate.company, &news).await {
                    w.candidate.sentiment = Some(score);
                }
            }
            w.candidate.news = news;
        }
    }

    // Early-detection enrichment: quarterly **inflection** (free SEC EDGAR) plus
    // **estimate revisions** (paid provider when active, else free Yahoo analyst
    // recommendations). Gated on `EarlyDetection` so `Legacy` stays byte-identical
    // and skips these extra network round-trips entirely.
    if settings.scoring_mode == ScoringMode::EarlyDetection {
        let provider = DataProvider::resolve(settings.data_provider);
        let message = if provider.is_active() {
            format!(
                "Detecting inflection & estimate revisions (revisions via {})…",
                provider.kind.label()
            )
        } else {
            "Detecting inflection & estimate revisions…".into()
        };
        emit(ProgressEvent::Stage {
            stage: "inflection".into(),
            message,
        });
        let yahoo = YahooEnrich::init().await;
        let limit = Arc::new(Semaphore::new(4));
        let mut set = tokio::task::JoinSet::new();
        for (idx, w) in working.iter().enumerate() {
            let http = http.clone();
            let ticker = w.candidate.ticker.clone();
            let yahoo = yahoo.clone();
            let provider = provider.clone();
            let limit = limit.clone();
            set.spawn(async move {
                let _permit = limit.acquire_owned().await.ok();
                let inflection = inflection::fetch_inflection(&http, &ticker).await;
                // Paid revisions first; degrade to the free Yahoo path on any miss.
                let revisions = match provider.estimate_revisions(&http, &ticker).await {
                    Some(rev) => Some(rev),
                    None => match yahoo.as_ref() {
                        Some(y) => y.fetch_recommendations(&ticker).await,
                        None => None,
                    },
                };
                (idx, inflection, revisions.map(|r| r.score()))
            });
        }
        while let Some(joined) = set.join_next().await {
            if let Ok((idx, inflection, revisions)) = joined {
                working[idx].inflection = inflection;
                working[idx].revisions = revisions;
            }
        }
    }

    // Score, rank, trim.
    emit(ProgressEvent::Stage {
        stage: "ranking".into(),
        message: "Ranking by growth & positioning…".into(),
    });
    let weights = ScoringWeights::for_mode(settings.scoring_mode);
    for w in working.iter_mut() {
        // Real fundamentals drive the growth term; fall back to the model's
        // upside only when the user has turned fundamentals off.
        let growth = if settings.use_fundamentals {
            w.candidate
                .growth
                .as_ref()
                .map(fundamentals::growth_score)
                .unwrap_or(0.5)
        } else {
            (w.candidate.upside.clamp(1, 5) as f64) / 5.0
        };
        w.candidate.growth_score = growth;
        // Early-detection signals (inflection, revisions) are populated above in
        // `EarlyDetection` mode and sit `None` (scored neutral) in `Legacy` mode,
        // so this still reduces to the legacy formula exactly under legacy weights.
        // Technical, insider and filing terms arrive in later phases.
        let signals = Signals {
            severity: w.severity,
            moat: w.candidate.moat,
            growth,
            sentiment: w.candidate.sentiment,
            change_pct: w.candidate.price.as_ref().map(|p| p.change_pct).unwrap_or(0.0),
            inflection: w.inflection,
            revisions: w.revisions,
            ..Default::default()
        };
        let mut breakdown = scoring::composite_score(&weights, &signals);
        if w.candidate.identity_mismatch {
            scale_breakdown(&mut breakdown, IDENTITY_MISMATCH_PENALTY);
        }
        w.candidate.score = breakdown.total;
        w.candidate.breakdown = Some(breakdown);
        w.candidate.owned = owned.contains(&w.candidate.ticker.to_ascii_uppercase());
    }
    working.sort_by(|a, b| {
        b.candidate
            .score
            .partial_cmp(&a.candidate.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    working.truncate(settings.max_results as usize);

    // Always surface the ranked picks — no score threshold to clear.
    let mut candidates = Vec::with_capacity(working.len());
    for w in working {
        emit(ProgressEvent::Candidate {
            candidate: w.candidate.clone(),
        });
        candidates.push(w.candidate);
    }

    let result = ResearchResult {
        industry,
        summary,
        bottlenecks,
        candidates,
        disclaimer: DISCLAIMER.to_string(),
    };
    emit(ProgressEvent::Done {
        result: result.clone(),
    });
    Ok(result)
}

async fn sentiment_for(
    ollama: &OllamaClient,
    company: &str,
    news: &[crate::model::NewsItem],
) -> AppResult<f64> {
    let headlines = news
        .iter()
        .map(|n| format!("- {}", n.title))
        .collect::<Vec<_>>()
        .join("\n");
    let user = format!("Company: {company}\nRecent headlines:\n{headlines}");
    let out: SentimentOut = ollama.generate_json(SYSTEM_SENTIMENT, &user).await?;
    Ok(out.score.clamp(-1.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severe_bottleneck_outscores_mild_one() {
        let severe = score_candidate(5, 3, 0.6, Some(0.5), 5.0);
        let mild = score_candidate(1, 3, 0.6, Some(0.5), 5.0);
        assert!(severe > mild);
    }

    #[test]
    fn stronger_moat_scores_higher_all_else_equal() {
        let monopoly = score_candidate(3, 5, 0.6, None, 0.0);
        let commodity = score_candidate(3, 1, 0.6, None, 0.0);
        assert!(monopoly > commodity);
    }

    #[test]
    fn higher_growth_scores_higher_all_else_equal() {
        let big = score_candidate(3, 3, 1.0, None, 0.0);
        let small = score_candidate(3, 3, 0.0, None, 0.0);
        assert!(big > small);
    }

    #[test]
    fn positioning_dominates_sentiment() {
        // A severe bottleneck with bad sentiment should still beat a mild
        // bottleneck with great sentiment — positioning is the point.
        let severe_bad_news = score_candidate(5, 3, 0.6, Some(-1.0), 0.0);
        let mild_great_news = score_candidate(1, 3, 0.6, Some(1.0), 0.0);
        assert!(severe_bad_news > mild_great_news);
    }

    #[test]
    fn score_stays_within_bounds() {
        let max = score_candidate(5, 5, 1.0, Some(1.0), 100.0);
        let min = score_candidate(1, 1, 0.0, Some(-1.0), -100.0);
        assert!(max <= 100.0 + f64::EPSILON);
        assert!(min >= 0.0 - f64::EPSILON);
    }

    #[test]
    fn normalize_ticker_strips_noise() {
        assert_eq!(normalize_ticker(" $aapl "), "AAPL");
        assert_eq!(normalize_ticker("brk b"), "BRKB");
    }

    #[test]
    fn name_matches_ignores_corporate_suffixes() {
        assert!(name_matches("Agnico Eagle Mines", "Agnico Eagle Mines Limited"));
        assert!(name_matches("NVIDIA Corporation", "NVIDIA Corporation"));
        assert!(name_matches("3M", "3M Company"));
    }

    #[test]
    fn name_matches_flags_real_contradiction() {
        // The AEM case: model claims a battery-equipment maker, but the ticker
        // resolves to a gold miner — no shared token, so it's a mismatch.
        assert!(!name_matches("AEM", "Agnico Eagle Mines Limited"));
        assert!(!name_matches(
            "Solid Power Battery Equipment",
            "Agnico Eagle Mines Limited"
        ));
    }

    #[test]
    fn name_matches_never_warns_on_empty_names() {
        assert!(name_matches("", "Agnico Eagle Mines Limited"));
        assert!(name_matches("Anything Inc", ""));
        // Pure noise tokens collapse to empty → treated as a match, not a warning.
        assert!(name_matches("The Company", "Holdings Group"));
    }

    #[test]
    fn identity_mismatch_demotes_but_never_zeroes() {
        let signals = Signals {
            severity: 5,
            moat: 5,
            growth: 0.8,
            sentiment: Some(0.5),
            change_pct: 10.0,
            ..Default::default()
        };
        let full = scoring::composite_score(&ScoringWeights::legacy(), &signals);
        let mut demoted = full.clone();
        scale_breakdown(&mut demoted, IDENTITY_MISMATCH_PENALTY);
        assert!(demoted.total < full.total, "a mismatch must lower the score");
        assert!(demoted.total > 0.0, "a mismatch must not zero a pick out");
        assert!((demoted.total - full.total * IDENTITY_MISMATCH_PENALTY).abs() < 1e-9);
        // Scaling keeps the breakdown internally consistent: parts still sum to total.
        let sum = demoted.severity
            + demoted.moat
            + demoted.growth
            + demoted.sentiment
            + demoted.momentum;
        assert!((sum - demoted.total).abs() < 1e-9);
    }

    #[test]
    fn fractional_rating_does_not_abort_parse() {
        // Reproduces the reported bug: a local model returned `severity: 3.5`
        // (and fractional moat/upside) where the schema asks for integers, which
        // used to fail the entire `from_str::<BottleneckPlan>` with "invalid
        // type: floating point `3.5`, expected u8".
        let json = r#"{
            "industry": "Energy",
            "summary": "…",
            "bottlenecks": [{
                "title": "Refining Capacity Constraints",
                "description": "…",
                "severity": 3.5,
                "companies": [{
                    "company": "Acme Refining",
                    "ticker": "ACME",
                    "thesis": "…",
                    "moat": 4.5,
                    "upside": 2.4,
                    "upside_rationale": "…"
                }]
            }]
        }"#;
        let plan: BottleneckPlan =
            serde_json::from_str(json).expect("fractional ratings must parse, not error");
        let b = &plan.bottlenecks[0];
        assert_eq!(b.severity, Some(4)); // 3.5 → 4
        assert_eq!(b.companies[0].moat, Some(5)); // 4.5 → 5
        assert_eq!(b.companies[0].upside, Some(2)); // 2.4 → 2
    }

    #[test]
    fn round_rating_rounds_and_clamps() {
        assert_eq!(round_rating(3.5), 4);
        assert_eq!(round_rating(2.4), 2);
        assert_eq!(round_rating(0.2), 1); // clamps up to the 1..=5 floor
        assert_eq!(round_rating(99.0), 5); // clamps down to the ceiling
    }

    #[test]
    fn de_rating_tolerates_strings_clamps_and_defaults() {
        let sev = |s: &str| serde_json::from_str::<PlanBottleneck>(s).unwrap().severity;
        assert_eq!(sev(r#"{"title":"t","severity":"5"}"#), Some(5)); // numeric string
        assert_eq!(sev(r#"{"title":"t","severity":9}"#), Some(5)); // out of range → ceiling
        assert_eq!(sev(r#"{"title":"t","severity":0}"#), Some(1)); // out of range → floor
        assert_eq!(sev(r#"{"title":"t","severity":"high"}"#), None); // prose → default later
        assert_eq!(sev(r#"{"title":"t","severity":null}"#), None);
        assert_eq!(sev(r#"{"title":"t"}"#), None); // missing field
    }
}
