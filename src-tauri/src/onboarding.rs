//! First-run setup support: detect the machine's specs, recommend a local model
//! that fits, and probe whether Ollama is installed/running.
//!
//! The reasoning here is deliberately small and pure so the recommendation logic
//! can be unit-tested without touching the OS or network. Only [`detect_specs`]
//! (RAM) and [`ollama_status`] (disk + HTTP) reach outside the process.

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use serde::Serialize;
use sysinfo::System;

/// Hardware summary surfaced to the setup wizard so it can recommend a model and
/// explain why. `total_ram_gb` is binary GB (GiB) rounded to one decimal.
#[derive(Debug, Clone, Serialize)]
pub struct SystemSpecs {
    pub os: String,
    pub arch: String,
    pub total_ram_gb: f64,
    pub cpu_cores: u32,
}

/// A selectable local model in the setup wizard. `can_run` reflects whether the
/// detected RAM clears the model's comfortable minimum; `recommended` marks the
/// single best fit for this machine.
#[derive(Debug, Clone, Serialize)]
pub struct ModelOption {
    pub id: String,
    pub label: String,
    pub params: String,
    pub min_ram_gb: u32,
    pub download_gb: f64,
    pub blurb: String,
    pub can_run: bool,
    pub recommended: bool,
}

/// Everything the "set up local AI" step needs in one payload.
#[derive(Debug, Clone, Serialize)]
pub struct SystemReport {
    pub specs: SystemSpecs,
    pub options: Vec<ModelOption>,
    pub recommended_id: String,
}

/// Whether Ollama is present and usable. `running` means the local server
/// answered; `installed` means the binary/app is on disk even if not started;
/// `models` lists what is already pulled (empty when the server is down).
#[derive(Debug, Clone, Serialize)]
pub struct OllamaStatus {
    pub installed: bool,
    pub running: bool,
    pub models: Vec<String>,
}

/// A curated shortlist of well-known, instruction-following local models that
/// emit JSON reliably (what the research pipeline needs). Kept small on purpose
/// so the choice stays approachable.
struct CatalogEntry {
    id: &'static str,
    label: &'static str,
    params: &'static str,
    min_ram_gb: u32,
    download_gb: f64,
    blurb: &'static str,
}

const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        id: "llama3.2:3b",
        label: "Llama 3.2",
        params: "3B",
        min_ram_gb: 4,
        download_gb: 2.0,
        blurb: "Fast and lightweight. A good fit for laptops or machines with limited memory.",
    },
    CatalogEntry {
        id: "llama3.1:8b",
        label: "Llama 3.1",
        params: "8B",
        min_ram_gb: 8,
        download_gb: 4.7,
        blurb: "The balanced default TrendWave is tuned for — the best all-round pick for most machines.",
    },
    CatalogEntry {
        id: "qwen2.5:14b",
        label: "Qwen 2.5",
        params: "14B",
        min_ram_gb: 16,
        download_gb: 9.0,
        blurb: "Stronger reasoning and detail. Needs a modern machine with plenty of RAM.",
    },
];

/// Pick the largest model that comfortably fits the detected RAM. Thresholds
/// follow Ollama's own guidance (≈8 GB for ~8B, ≈16 GB for ~14B).
fn recommended_model_id(total_ram_gb: f64) -> &'static str {
    if total_ram_gb + 0.5 >= 16.0 {
        "qwen2.5:14b"
    } else if total_ram_gb + 0.5 >= 8.0 {
        "llama3.1:8b"
    } else {
        "llama3.2:3b"
    }
}

/// Turn the static catalog into per-machine options (pure, so it is testable for
/// any RAM size without probing hardware).
fn build_options(total_ram_gb: f64, recommended_id: &str) -> Vec<ModelOption> {
    CATALOG
        .iter()
        .map(|e| ModelOption {
            id: e.id.to_string(),
            label: e.label.to_string(),
            params: e.params.to_string(),
            min_ram_gb: e.min_ram_gb,
            download_gb: e.download_gb,
            blurb: e.blurb.to_string(),
            can_run: total_ram_gb + 0.5 >= e.min_ram_gb as f64,
            recommended: e.id == recommended_id,
        })
        .collect()
}

/// Read total physical RAM (via `sysinfo`) plus CPU/OS/arch (via `std`).
pub fn detect_specs() -> SystemSpecs {
    let mut sys = System::new();
    sys.refresh_memory();
    let total_ram_gb = (sys.total_memory() as f64) / (1024.0 * 1024.0 * 1024.0);
    let total_ram_gb = (total_ram_gb * 10.0).round() / 10.0;
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);
    SystemSpecs {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        total_ram_gb,
        cpu_cores,
    }
}

/// Build the full setup report: specs, per-machine model options, and the
/// recommended id.
pub fn system_report() -> SystemReport {
    let specs = detect_specs();
    let recommended_id = recommended_model_id(specs.total_ram_gb).to_string();
    let options = build_options(specs.total_ram_gb, &recommended_id);
    SystemReport {
        specs,
        options,
        recommended_id,
    }
}

#[derive(Deserialize)]
struct TagsResponse {
    models: Vec<TagModel>,
}

#[derive(Deserialize)]
struct TagModel {
    name: String,
}

/// Probe whether the local Ollama server answers, and what it has pulled.
async fn ollama_running(http: &reqwest::Client, endpoint: &str) -> (bool, Vec<String>) {
    let url = format!("{}/api/tags", endpoint.trim_end_matches('/'));
    match http.get(&url).timeout(Duration::from_secs(3)).send().await {
        Ok(resp) if resp.status().is_success() => {
            let models = resp
                .json::<TagsResponse>()
                .await
                .map(|t| t.models.into_iter().map(|m| m.name).collect())
                .unwrap_or_default();
            (true, models)
        }
        _ => (false, Vec::new()),
    }
}

/// Detect an installed Ollama even when the server is stopped. GUI apps launched
/// from the dock/Start menu often inherit a minimal `PATH`, so we probe the usual
/// install locations directly in addition to `PATH`.
fn binary_on_disk() -> bool {
    let exe = if cfg!(target_os = "windows") {
        "ollama.exe"
    } else {
        "ollama"
    };
    if let Some(paths) = std::env::var_os("PATH") {
        if std::env::split_paths(&paths).any(|dir| dir.join(exe).is_file()) {
            return true;
        }
    }
    common_install_paths()
        .iter()
        .any(|p| Path::new(p).exists())
}

fn common_install_paths() -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        let mut v = vec![
            "/usr/local/bin/ollama".to_string(),
            "/opt/homebrew/bin/ollama".to_string(),
            "/Applications/Ollama.app".to_string(),
        ];
        if let Some(home) = std::env::var_os("HOME") {
            v.push(
                Path::new(&home)
                    .join("Applications/Ollama.app")
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        v
    }
    #[cfg(target_os = "windows")]
    {
        let mut v = vec!["C:\\Program Files\\Ollama\\ollama.exe".to_string()];
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            v.push(
                Path::new(&local)
                    .join("Programs\\Ollama\\ollama.exe")
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        v
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        vec![
            "/usr/local/bin/ollama".to_string(),
            "/usr/bin/ollama".to_string(),
            "/snap/bin/ollama".to_string(),
        ]
    }
}

/// Combined Ollama readiness for the setup wizard.
pub async fn ollama_status(http: &reqwest::Client, endpoint: &str) -> OllamaStatus {
    let (running, models) = ollama_running(http, endpoint).await;
    let installed = running || binary_on_disk();
    OllamaStatus {
        installed,
        running,
        models,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommends_larger_models_with_more_ram() {
        assert_eq!(recommended_model_id(4.0), "llama3.2:3b");
        assert_eq!(recommended_model_id(8.0), "llama3.1:8b");
        assert_eq!(recommended_model_id(12.0), "llama3.1:8b");
        assert_eq!(recommended_model_id(16.0), "qwen2.5:14b");
        assert_eq!(recommended_model_id(64.0), "qwen2.5:14b");
    }

    #[test]
    fn report_marks_exactly_one_recommended_in_catalog() {
        let report = system_report();
        assert_eq!(report.options.iter().filter(|o| o.recommended).count(), 1);
        assert!(report
            .options
            .iter()
            .any(|o| o.id == report.recommended_id && o.recommended));
    }

    #[test]
    fn can_run_tracks_minimum_ram() {
        let low = build_options(4.0, recommended_model_id(4.0));
        assert!(low.iter().find(|o| o.id == "llama3.2:3b").unwrap().can_run);
        assert!(!low.iter().find(|o| o.id == "qwen2.5:14b").unwrap().can_run);

        let high = build_options(32.0, recommended_model_id(32.0));
        assert!(high.iter().all(|o| o.can_run));
    }
}
