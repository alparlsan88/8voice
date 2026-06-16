//! First-run onboarding helpers: model catalog, download manager,
//! local model validation, and Groq API key validation.
//!
//! Contract:
//! - Provides Tauri commands used only by the onboarding UI.
//! - Downloads go to `app_local_data_dir()/models/`.
//! - Local model selection is validated but not copied.

use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tokio::io::AsyncWriteExt;

const HF_BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

/// Information about a downloadable Whisper model.
#[derive(Debug, Clone, Serialize)]
pub struct WhisperModel {
    pub id: String,
    pub name: String,
    pub filename: String,
    pub size_bytes: u64,
    pub size_human: String,
    pub description: String,
}

/// Result of validating a local model file.
#[derive(Debug, Clone, Serialize)]
pub struct LocalModelInfo {
    pub path: String,
    pub exists: bool,
    pub valid_extension: bool,
    pub size_bytes: u64,
}

/// Events emitted by `cmd_download_whisper_model` via a Tauri channel.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", content = "data")]
pub enum DownloadEvent {
    Started { total: Option<u64> },
    Progress { downloaded: u64, total: Option<u64>, percent: Option<f32> },
    Done { path: String },
    Error { message: String },
    Cancelled,
}

/// Shared controller used to cancel an in-progress download.
#[derive(Default)]
pub struct DownloadController {
    cancel: AtomicBool,
}

impl DownloadController {
    pub fn new() -> Self {
        Self {
            cancel: AtomicBool::new(false),
        }
    }

    pub fn set_cancelled(&self, value: bool) {
        self.cancel.store(value, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Returns the curated list of recommended Whisper models.
#[tauri::command]
pub fn cmd_list_whisper_models() -> Vec<WhisperModel> {
    vec![
        WhisperModel {
            id: "tiny".to_string(),
            name: "Tiny".to_string(),
            filename: "ggml-tiny.bin".to_string(),
            size_bytes: 39 * 1024 * 1024,
            size_human: "~39 MB".to_string(),
            description: "Fastest, lowest accuracy. Good for testing.".to_string(),
        },
        WhisperModel {
            id: "base".to_string(),
            name: "Base".to_string(),
            filename: "ggml-base.bin".to_string(),
            size_bytes: 74 * 1024 * 1024,
            size_human: "~74 MB".to_string(),
            description: "Balanced speed and accuracy for entry-level use.".to_string(),
        },
        WhisperModel {
            id: "small".to_string(),
            name: "Small".to_string(),
            filename: "ggml-small.bin".to_string(),
            size_bytes: 466 * 1024 * 1024,
            size_human: "~466 MB".to_string(),
            description: "Recommended default. Solid accuracy on most CPUs.".to_string(),
        },
        WhisperModel {
            id: "medium".to_string(),
            name: "Medium".to_string(),
            filename: "ggml-medium.bin".to_string(),
            size_bytes: 1_500 * 1024 * 1024,
            size_human: "~1.5 GB".to_string(),
            description: "High accuracy, slower transcription.".to_string(),
        },
        WhisperModel {
            id: "large-v3".to_string(),
            name: "Large v3".to_string(),
            filename: "ggml-large-v3.bin".to_string(),
            size_bytes: 2_900 * 1024 * 1024,
            size_human: "~2.9 GB".to_string(),
            description: "Best accuracy, requires more RAM and CPU.".to_string(),
        },
        WhisperModel {
            id: "large-v3-turbo".to_string(),
            name: "Large v3 Turbo".to_string(),
            filename: "ggml-large-v3-turbo.bin".to_string(),
            size_bytes: 1_500 * 1024 * 1024,
            size_human: "~1.5 GB".to_string(),
            description: "Large-level accuracy with faster inference.".to_string(),
        },
    ]
}

/// Resolves the application-local models directory.
fn models_dir(app: &AppHandle) -> Result<PathBuf> {
    let dir = app
        .path()
        .app_local_data_dir()
        .context("Could not resolve app local data dir")?
        .join("models");
    Ok(dir)
}

/// Downloads a Whisper model from HuggingFace into the app data directory.
/// Progress, completion, and errors are emitted through the provided channel.
#[tauri::command]
pub async fn cmd_download_whisper_model(
    app: AppHandle,
    controller: State<'_, DownloadController>,
    filename: String,
    channel: Channel<DownloadEvent>,
) -> Result<(), String> {
    controller.set_cancelled(false);

    let url = format!("{HF_BASE_URL}/{filename}");
    let dir = models_dir(&app).map_err(|e| e.to_string())?;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("Could not create models dir: {e}"))?;
    let target = dir.join(&filename);
    let temp_target = dir.join(format!(".{filename}.partial"));

    let result = download_with_progress(&url, &temp_target, &target, &controller, &channel).await;

    if controller.is_cancelled() {
        let _ = tokio::fs::remove_file(&temp_target).await;
        let _ = channel.send(DownloadEvent::Cancelled);
        return Ok(());
    }

    match result {
        Ok(path) => {
            let _ = channel.send(DownloadEvent::Done {
                path: path.to_string_lossy().to_string(),
            });
            Ok(())
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&temp_target).await;
            let _ = channel.send(DownloadEvent::Error {
                message: e.to_string(),
            });
            Err(e.to_string())
        }
    }
}

async fn download_with_progress(
    url: &str,
    temp_target: &Path,
    target: &Path,
    controller: &DownloadController,
    channel: &Channel<DownloadEvent>,
) -> Result<PathBuf> {
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .send()
        .await
        .context("Model download request failed")?;

    if !resp.status().is_success() {
        return Err(anyhow!(
            "Download failed with status {} for {}",
            resp.status(),
            url
        ));
    }

    let total = resp.content_length();
    let _ = channel.send(DownloadEvent::Started { total });

    let mut file = tokio::fs::File::create(temp_target)
        .await
        .context("Could not create temporary model file")?;

    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut last_report = std::time::Instant::now();

    while let Some(chunk) = stream.next().await {
        if controller.is_cancelled() {
            return Err(anyhow!("Download cancelled by user"));
        }

        let chunk = chunk.context("Error while downloading model chunk")?;
        file.write_all(&chunk)
            .await
            .context("Could not write model chunk")?;
        downloaded += chunk.len() as u64;

        if last_report.elapsed().as_millis() >= 200 {
            let percent = total.map(|t| (downloaded as f64 / t as f64 * 100.0) as f32);
            let _ = channel.send(DownloadEvent::Progress {
                downloaded,
                total,
                percent,
            });
            last_report = std::time::Instant::now();
        }
    }

    file.flush().await.context("Could not flush model file")?;
    drop(file);

    tokio::fs::rename(temp_target, target)
        .await
        .context("Could not finalize model file")?;

    Ok(target.to_path_buf())
}

/// Cancels the currently active model download, if any.
#[tauri::command]
pub fn cmd_cancel_download(controller: State<'_, DownloadController>) {
    controller.set_cancelled(true);
}

/// Validates a local model path without copying it.
#[tauri::command]
pub fn cmd_validate_local_model(path: String) -> Result<LocalModelInfo, String> {
    let p = PathBuf::from(&path);
    let exists = p.exists();
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());
    let valid_extension = matches!(ext.as_deref(), Some("bin") | Some("gguf"));
    let size_bytes = if exists {
        std::fs::metadata(&p)
            .map(|m| m.len())
            .unwrap_or(0)
    } else {
        0
    };

    Ok(LocalModelInfo {
        path,
        exists,
        valid_extension,
        size_bytes,
    })
}

/// Validates a Groq API key by listing available models.
#[tauri::command]
pub async fn cmd_validate_groq_key(api_key: String) -> Result<bool, String> {
    if api_key.trim().is_empty() {
        return Ok(false);
    }

    let client = reqwest::Client::new();
    let resp = client
        .get("https://api.groq.com/openai/v1/models")
        .bearer_auth(api_key.trim())
        .send()
        .await
        .map_err(|e| format!("Groq validation request failed: {e}"))?;

    if resp.status().is_success() {
        Ok(true)
    } else if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        Ok(false)
    } else {
        let body = resp.text().await.unwrap_or_default();
        Err(format!("Groq validation error: {body}"))
    }
}
