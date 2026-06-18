use serde::{Deserialize, Serialize};

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
    /// Require a Touch ID / Windows Hello check on launch before the saved
    /// Robinhood session (and its portfolio) is revealed. Off by default so a
    /// fresh install needs no biometric hardware.
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
            require_biometric_unlock: false,
        }
    }
}
