use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::Semaphore;

use crate::error::AppResult;
use crate::feeds;
use crate::fundamentals::{self, YahooEnrich};
use crate::model::{Bottleneck, Candidate, ProgressEvent, ResearchResult, DISCLAIMER};
use crate::ollama::OllamaClient;
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

// Scoring weights — positioning to win the bottleneck (severity + moat) and
// real growth potential dominate; sentiment and momentum only refine. Growth is
// the single largest factor and comes from audited data, not the model's guess.
// Share price is never scored.
const W_BOTTLENECK: f64 = 25.0;
const W_MOAT: f64 = 25.0;
const W_GROWTH: f64 = 35.0;
const W_SENTIMENT: f64 = 10.0;
const W_MOMENTUM: f64 = 5.0;

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
    severity: Option<u8>,
    #[serde(default)]
    companies: Vec<PlanCompany>,
}

#[derive(Deserialize)]
struct PlanCompany {
    company: String,
    ticker: Option<String>,
    thesis: Option<String>,
    moat: Option<u8>,
    upside: Option<u8>,
    upside_rationale: Option<String>,
}

#[derive(Deserialize)]
struct SentimentOut {
    score: f64,
}

/// A candidate plus the data we need to score it but don't expose verbatim.
struct Working {
    candidate: Candidate,
    severity: u8,
}

/// Pure, deterministic scoring so it can be unit-tested without any network.
/// Positioning to win the bottleneck (severity + moat) and real growth dominate;
/// sentiment and momentum refine. `growth` is a 0..1 data-derived score (see
/// `fundamentals::growth_score`). Share price is deliberately NOT a factor.
pub fn score_candidate(
    severity: u8,
    moat: u8,
    growth: f64,
    sentiment: Option<f64>,
    change_pct: f64,
) -> f64 {
    let sev = (severity.clamp(1, 5) as f64) / 5.0;
    let moat_n = (moat.clamp(1, 5) as f64) / 5.0;
    let growth_n = growth.clamp(0.0, 1.0);

    // Map -1..1 sentiment onto 0..1; unknown sentiment sits neutral.
    let senti = ((sentiment.unwrap_or(0.0).clamp(-1.0, 1.0)) + 1.0) / 2.0;

    // +20% over the window → full marks, -20% → zero, clamped.
    let momentum = (0.5 + change_pct / 40.0).clamp(0.0, 1.0);

    W_BOTTLENECK * sev + W_MOAT * moat_n + W_GROWTH * growth_n + W_SENTIMENT * senti + W_MOMENTUM * momentum
}

fn normalize_ticker(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('$')
        .to_ascii_uppercase()
        .replace(' ', "")
}

/// Run the full prompt → bottlenecks → candidates → ranked results pipeline,
/// emitting progress as it goes. Kept free of any Tauri types so it stays
/// testable; the caller adapts `emit` to a channel.
pub async fn run_research<F: Fn(ProgressEvent)>(
    ollama: &OllamaClient,
    http: &reqwest::Client,
    settings: &Settings,
    prompt: &str,
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
    let plan: BottleneckPlan = ollama.generate_json(SYSTEM_BOTTLENECK, prompt).await?;

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
                candidate: Candidate {
                    ticker,
                    company: c.company.clone(),
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

    // Score, rank, trim.
    emit(ProgressEvent::Stage {
        stage: "ranking".into(),
        message: "Ranking by growth & positioning…".into(),
    });
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
        w.candidate.score = score_candidate(
            w.severity,
            w.candidate.moat,
            growth,
            w.candidate.sentiment,
            w.candidate.price.as_ref().map(|p| p.change_pct).unwrap_or(0.0),
        );
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
}
