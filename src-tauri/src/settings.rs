use serde::{Deserialize, Serialize};

use crate::providers::ProviderKind;
use crate::scoring::ScoringMode;

/// User-tunable configuration. Persisted as a single JSON row in SQLite so we
/// can add fields later without a schema migration. Every field has a default
/// so a fresh install works with zero setup beyond having Ollama running.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Ollama HTTP endpoint. Local-first default; rarely changed.
    pub ollama_endpoint: String,
    /// Model used for reasoning. Must follow instructions / emit JSON well.
    pub model: String,
    /// Maximum number of ranked picks returned per run.
    pub max_results: u32,
    /// Pull recent news + run sentiment. Disabling speeds runs up considerably.
    pub use_news: bool,
    /// Pull real fundamentals (SEC EDGAR + Yahoo) to drive the growth score.
    /// Disabling falls back to the model's own upside guess and speeds runs up.
    pub use_fundamentals: bool,
    /// Which scoring profile ranks the picks. `Legacy` reproduces the original
    /// five-term formula exactly; `EarlyDetection` weights forward/inflection
    /// signals. Always reversible from here.
    pub scoring_mode: ScoringMode,
    /// Market-data provider backing the optional paid signals. `Free` (the
    /// default) uses only SEC EDGAR + Yahoo and needs no key; a paid mode uses a
    /// bring-your-own-key source stored in the OS keychain, and silently falls
    /// back to the free path whenever a key is absent or a call fails.
    pub data_provider: ProviderKind,
    /// Require a Touch ID / Windows Hello check before a saved broker session
    /// (and its portfolio) is revealed. On by default; it transparently degrades
    /// to unlocked on devices without biometric hardware, so it can never strand
    /// a session the user cannot get back into.
    pub require_biometric_unlock: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ollama_endpoint: "http://localhost:11434".to_string(),
            model: "llama3.1:8b".to_string(),
            max_results: 8,
            use_news: true,
            use_fundamentals: true,
            scoring_mode: ScoringMode::default(),
            data_provider: ProviderKind::default(),
            require_biometric_unlock: true,
        }
    }
}
