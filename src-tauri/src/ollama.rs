use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{AppError, AppResult};

/// Thin async client over a local Ollama server.
///
/// We deliberately keep this small: one JSON-returning chat call and a health
/// probe. `reqwest::Client` is internally reference-counted, so cloning it for
/// each request is cheap and is the documented way to share it.
#[derive(Clone)]
pub struct OllamaClient {
    http: reqwest::Client,
    endpoint: String,
    model: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ChatMessage,
}

#[derive(Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct TagsResponse {
    models: Vec<TagModel>,
}

#[derive(Deserialize)]
struct TagModel {
    name: String,
}

impl OllamaClient {
    pub fn new(http: reqwest::Client, endpoint: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            http,
            endpoint: endpoint.into(),
            model: model.into(),
        }
    }

    /// Verify the server is up and the configured model is installed. Surfaced
    /// before a run so the user gets one clear message instead of a mid-pipeline
    /// failure.
    pub async fn ensure_ready(&self) -> AppResult<()> {
        let url = format!("{}/api/tags", self.endpoint.trim_end_matches('/'));
        let resp = self.http.get(&url).send().await.map_err(|err| {
            if err.is_connect() || err.is_timeout() {
                AppError::OllamaUnavailable {
                    endpoint: self.endpoint.clone(),
                }
            } else {
                AppError::Network(err.to_string())
            }
        })?;

        let tags: TagsResponse = resp.json().await?;
        // Ollama reports models as `name:tag`; match either the exact id or the
        // bare family so `llama3.1` matches an installed `llama3.1:8b`.
        let installed = tags.models.iter().any(|m| {
            m.name == self.model
                || m.name.split(':').next() == self.model.split(':').next()
        });

        if !installed {
            return Err(AppError::ModelMissing {
                model: self.model.clone(),
            });
        }
        Ok(())
    }

    /// Run a chat completion constrained to JSON and deserialize it into `T`.
    /// Using Ollama's `format: "json"` plus a deserialize step gives us typed,
    /// validated model output instead of brittle string parsing.
    pub async fn generate_json<T: DeserializeOwned>(
        &self,
        system: &str,
        user: &str,
    ) -> AppResult<T> {
        let raw = self.chat(system, user, true).await?;
        serde_json::from_str::<T>(&raw).map_err(|err| AppError::ModelResponse {
            detail: format!("{err}: {}", truncate(&raw, 300)),
        })
    }

    async fn chat(&self, system: &str, user: &str, json_mode: bool) -> AppResult<String> {
        let url = format!("{}/api/chat", self.endpoint.trim_end_matches('/'));
        let mut body = json!({
            "model": self.model,
            "stream": false,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user }
            ],
            "options": { "temperature": 0.2 }
        });
        if json_mode {
            body["format"] = json!("json");
        }

        let resp = self.http.post(&url).json(&body).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if status.as_u16() == 404 || text.to_lowercase().contains("not found") {
                return Err(AppError::ModelMissing {
                    model: self.model.clone(),
                });
            }
            return Err(AppError::Network(format!("Ollama returned {status}: {text}")));
        }

        let parsed: ChatResponse = resp.json().await?;
        Ok(parsed.message.content)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}
