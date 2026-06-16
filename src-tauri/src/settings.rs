//! Settings management — persistent settings via tauri-plugin-store (JSON).
//!
//! Contract:
//! - Input: get/set keys
//! - Output: typed [`Settings`] or error
//! - Accept: settings persist across restarts

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

/// Application settings. Serialized to JSON and written to the store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Selected microphone device name; `None` = system default.
    #[serde(default)]
    pub input_device: Option<String>,
    /// Path to the Whisper GGUF model file (relative to the app data dir).
    #[serde(default = "default_model_path")]
    pub model_path: String,
    /// Transcription language: `"tr"`, `"en"`, or `"auto"`.
    #[serde(default = "default_language")]
    pub language: String,
    /// Global shortcut (e.g. `"Ctrl+Shift+Space"`).
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    /// Shortcut mode: push-to-talk or toggle.
    #[serde(default = "default_hotkey_mode")]
    pub hotkey_mode: HotkeyMode,
    /// Injection mode: auto, always type, or always paste.
    #[serde(default = "default_injection_mode")]
    pub injection_mode: InjectionMode,
    /// Whether VAD auto-stop is enabled (stop recording when speech ends).
    #[serde(default = "default_vad_enabled")]
    pub vad_enabled: bool,
    /// Continuous silence required for auto-stop (ms).
    #[serde(default = "default_vad_silence_ms")]
    pub vad_silence_ms: u32,
    /// VAD aggressiveness: 1 = Medium, 2 = Aggressive, 3 = VeryAggressive.
    #[serde(default = "default_vad_aggressiveness")]
    pub vad_aggressiveness: u8,
    /// Transcription provider: local model or Groq API.
    #[serde(default = "default_api_provider")]
    pub api_provider: ApiProvider,
    /// Groq API key. None/empty disables API mode.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Whether the first-run onboarding has been completed.
    #[serde(default)]
    pub has_completed_onboarding: bool,
}

/// VAD configuration passed to the audio layer. Derived from `Settings`.
#[derive(Debug, Clone, Copy)]
pub struct VadCfg {
    pub enabled: bool,
    pub silence_ms: u32,
    pub aggressiveness: u8,
}

impl Settings {
    /// Extracts the VAD configuration for the audio layer.
    pub fn vad_cfg(&self) -> VadCfg {
        VadCfg {
            enabled: self.vad_enabled,
            silence_ms: self.vad_silence_ms,
            aggressiveness: self.vad_aggressiveness,
        }
    }

    /// Sanitizes critical fields; e.g. empty hotkey falls back to default.
    pub fn sanitize(&mut self) {
        if self.hotkey.trim().is_empty() {
            self.hotkey = default_hotkey();
        }
        // If API mode is selected but key is empty, fall back to offline
        if self.api_provider == ApiProvider::Groq {
            if self.api_key.as_deref().unwrap_or("").trim().is_empty() {
                self.api_provider = ApiProvider::Offline;
            }
        }
    }
}

/// Runtime-managed settings shared as Tauri state.
/// The `Arc<RwLock>` lets commands and the hotkey handler read/write safely.
#[derive(Clone)]
pub struct SharedSettings {
    pub inner: Arc<RwLock<Settings>>,
}

/// Creates a new shared settings wrapper.
pub fn shared(settings: Settings) -> SharedSettings {
    SharedSettings {
        inner: Arc::new(RwLock::new(settings)),
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyMode {
    PushToTalk,
    Toggle,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InjectionMode {
    Auto,
    Typing,
    Paste,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApiProvider {
    /// Local whisper.cpp model (ggml/bin).
    #[default]
    Offline,
    /// Groq Whisper API (cloud, requires API key).
    Groq,
}

fn default_model_path() -> String {
    "models/ggml-small.bin".to_string()
}
fn default_language() -> String {
    "auto".to_string()
}
fn default_hotkey() -> String {
    "Ctrl+Shift+Space".to_string()
}
fn default_hotkey_mode() -> HotkeyMode {
    HotkeyMode::PushToTalk
}
fn default_injection_mode() -> InjectionMode {
    InjectionMode::Auto
}
fn default_vad_enabled() -> bool {
    true
}
fn default_vad_silence_ms() -> u32 {
    1200
}
fn default_vad_aggressiveness() -> u8 {
    2
}
fn default_api_provider() -> ApiProvider {
    ApiProvider::Offline
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            input_device: None,
            model_path: default_model_path(),
            language: default_language(),
            hotkey: default_hotkey(),
            hotkey_mode: default_hotkey_mode(),
            injection_mode: default_injection_mode(),
            vad_enabled: default_vad_enabled(),
            vad_silence_ms: default_vad_silence_ms(),
            vad_aggressiveness: default_vad_aggressiveness(),
            api_provider: default_api_provider(),
            api_key: None,
            has_completed_onboarding: false,
        }
    }
}

const STORE_FILE: &str = "settings.json";
const STORE_KEY: &str = "settings";

/// Loads settings from the store; creates and saves defaults if missing.
/// Invalid/empty critical fields are corrected to defaults.
pub fn load(app: &AppHandle) -> anyhow::Result<Settings> {
    let store = app.store(STORE_FILE)?;
    let mut settings: Settings = store
        .get(STORE_KEY)
        .and_then(|v| serde_json::from_value::<Settings>(v).ok())
        .unwrap_or_default();
    settings.sanitize();
    Ok(settings)
}

/// Writes settings to the store.
pub fn save(app: &AppHandle, settings: &Settings) -> anyhow::Result<()> {
    let store = app.store(STORE_FILE)?;
    let value = serde_json::to_value(settings)?;
    store.set(STORE_KEY, value);
    store.save()?;
    Ok(())
}
