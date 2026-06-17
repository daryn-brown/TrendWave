use serde::Deserialize;

use crate::error::AppResult;
use crate::feeds;
use crate::model::{Bottleneck, Candidate, ProgressEvent, ResearchResult, DISCLAIMER};
use crate::ollama::OllamaClient;
use crate::settings::Settings;

const SYSTEM_BOTTLENECK: &str = "\
You are a sharp equity research analyst who specializes in SUPPLY-CHAIN and CAPACITY \
BOTTLENECKS. Given a user's question about an industry, identify the real, current \
chokepoints (scarce components, limited production capacity, single-source suppliers, \
permitting/logistics constraints) and the smaller, cheaper public companies most exposed \
to relieving or supplying those bottlenecks. Favor under-followed small/mid caps over \
mega-caps. Use real US-listed tickers. Respond with STRICT JSON only, no prose, matching: \
{\"industry\":string,\"summary\":string,\"bottlenecks\":[{\"title\":string,\
\"description\":string,\"severity\":1-5,\"companies\":[{\"company\":string,\"ticker\":string,\
\"why_cheap\":string,\"thesis\":string,\"upside\":string}]}]}. \
severity is how acute the bottleneck is (5=severe). Include 2-4 companies per bottleneck.";

const SYSTEM_SENTIMENT: &str = "\
You judge market sentiment from news headlines about one company. Respond with STRICT JSON \
only: {\"score\":number} where score is between -1 (very bearish) and 1 (very bullish).";

// Scoring weights — bottlenecks are intentionally the dominant signal.
const W_BOTTLENECK: f64 = 50.0;
const W_CHEAP: f64 = 20.0;
const W_SENTIMENT: f64 = 20.0;
const W_MOMENTUM: f64 = 10.0;

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
    why_cheap: Option<String>,
    thesis: Option<String>,
    upside: Option<String>,
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
/// Bottleneck severity dominates; cheapness, sentiment and momentum refine.
pub fn score_candidate(
    severity: u8,
    price: Option<f64>,
    max_price: f64,
    sentiment: Option<f64>,
    change_pct: f64,
) -> f64 {
    let sev = (severity.clamp(1, 5) as f64) / 5.0;

    let cheap = match price {
        Some(p) if max_price > 0.0 => (1.0 - (p / max_price)).clamp(0.0, 1.0),
        _ => 0.0,
    };

    // Map -1..1 sentiment onto 0..1; unknown sentiment sits neutral.
    let senti = ((sentiment.unwrap_or(0.0).clamp(-1.0, 1.0)) + 1.0) / 2.0;

    // +20% over the window → full marks, -20% → zero, clamped.
    let momentum = (0.5 + change_pct / 40.0).clamp(0.0, 1.0);

    W_BOTTLENECK * sev + W_CHEAP * cheap + W_SENTIMENT * senti + W_MOMENTUM * momentum
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
                    bottleneck_thesis: c.thesis.clone().unwrap_or_default(),
                    why_cheap: c.why_cheap.clone().unwrap_or_default(),
                    upside_rationale: c.upside.clone().unwrap_or_default(),
                    sentiment: None,
                    news: Vec::new(),
                    score: 0.0,
                },
            });
        }
    }

    // Price every candidate concurrently; fall back to name→symbol resolution.
    emit(ProgressEvent::Stage {
        stage: "pricing".into(),
        message: "Pricing candidates and checking they're cheap…".into(),
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

    // Keep only validated, genuinely cheap names.
    working.retain(|w| {
        w.candidate
            .price
            .as_ref()
            .map(|p| p.price > 0.0 && p.price <= settings.max_price)
            .unwrap_or(false)
    });

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
        message: "Ranking by bottleneck exposure…".into(),
    });
    for w in working.iter_mut() {
        w.candidate.score = score_candidate(
            w.severity,
            w.candidate.price.as_ref().map(|p| p.price),
            settings.max_price,
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

    let mut candidates = Vec::with_capacity(working.len());
    for w in working {
        if w.candidate.score >= settings.min_score {
            emit(ProgressEvent::Candidate {
                candidate: w.candidate.clone(),
            });
            candidates.push(w.candidate);
        }
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
        let severe = score_candidate(5, Some(10.0), 20.0, Some(0.5), 5.0);
        let mild = score_candidate(1, Some(10.0), 20.0, Some(0.5), 5.0);
        assert!(severe > mild);
    }

    #[test]
    fn cheaper_stock_scores_higher_all_else_equal() {
        let cheap = score_candidate(3, Some(2.0), 20.0, None, 0.0);
        let pricey = score_candidate(3, Some(19.0), 20.0, None, 0.0);
        assert!(cheap > pricey);
    }

    #[test]
    fn bottleneck_weight_dominates_sentiment() {
        // A severe bottleneck with bad sentiment should still beat a mild
        // bottleneck with great sentiment — bottlenecks are the point.
        let severe_bad_news = score_candidate(5, Some(10.0), 20.0, Some(-1.0), 0.0);
        let mild_great_news = score_candidate(1, Some(10.0), 20.0, Some(1.0), 0.0);
        assert!(severe_bad_news > mild_great_news);
    }

    #[test]
    fn score_stays_within_bounds() {
        let max = score_candidate(5, Some(0.01), 20.0, Some(1.0), 100.0);
        let min = score_candidate(1, Some(20.0), 20.0, Some(-1.0), -100.0);
        assert!(max <= 100.0 + f64::EPSILON);
        assert!(min >= 0.0 - f64::EPSILON);
    }

    #[test]
    fn normalize_ticker_strips_noise() {
        assert_eq!(normalize_ticker(" $aapl "), "AAPL");
        assert_eq!(normalize_ticker("brk b"), "BRKB");
    }
}
