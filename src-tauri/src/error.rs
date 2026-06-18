use serde::Serialize;

/// Application-wide error type.
///
/// Every fallible path in the backend funnels into this enum so the frontend
/// receives a small, predictable shape instead of opaque strings. It derives
/// `Serialize` (with a tagged representation) so a thrown command error arrives
/// in JavaScript as `{ kind, message }`, which the UI can branch on — most
/// importantly to detect `OllamaUnavailable` and tell the user to start Ollama.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Ollama is not reachable at {endpoint}. Is it running? Try `ollama serve`.")]
    OllamaUnavailable { endpoint: String },

    #[error("The model `{model}` is not installed. Try `ollama pull {model}`.")]
    ModelMissing { model: String },

    #[error("The language model returned a response we could not parse: {detail}")]
    ModelResponse { detail: String },

    #[error("Network request failed: {0}")]
    Network(String),

    #[error("A data feed returned no usable data: {0}")]
    EmptyFeed(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Robinhood is not connected. Connect it in Settings to enable portfolio context.")]
    RobinhoodNotConnected,

    #[error("Robinhood integration error: {0}")]
    Robinhood(String),

    #[error("{0}")]
    Other(String),
}

impl AppError {
    pub fn kind(&self) -> &'static str {
        match self {
            AppError::OllamaUnavailable { .. } => "ollama_unavailable",
            AppError::ModelMissing { .. } => "model_missing",
            AppError::ModelResponse { .. } => "model_response",
            AppError::Network(_) => "network",
            AppError::EmptyFeed(_) => "empty_feed",
            AppError::Database(_) => "database",
            AppError::RobinhoodNotConnected => "robinhood_not_connected",
            AppError::Robinhood(_) => "robinhood",
            AppError::Other(_) => "other",
        }
    }
}

/// Custom `Serialize` so the wire format is a flat, frontend-friendly object.
impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("AppError", 2)?;
        state.serialize_field("kind", self.kind())?;
        state.serialize_field("message", &self.to_string())?;
        state.end()
    }
}

/// Translate transport errors into a friendlier shape. A connection refused to
/// the Ollama port is by far the most common failure, so we special-case it.
impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_connect() {
            return AppError::OllamaUnavailable {
                endpoint: err
                    .url()
                    .map(|u| u.to_string())
                    .unwrap_or_else(|| "http://localhost:11434".to_string()),
            };
        }
        AppError::Network(err.to_string())
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        AppError::Database(err.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::ModelResponse {
            detail: err.to_string(),
        }
    }
}

pub type AppResult<T> = Result<T, AppError>;
