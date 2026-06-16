//! Ayar yönetimi — tauri-plugin-store (JSON) üzerinden kalıcı ayarlar.
//!
//! Kontrat:
//! - Input: get/set anahtarları
//! - Output: tiplendirilmiş [`Settings`] veya hata
//! - Kabul: uygulama yeniden başlansınca ayarlar korunuyor

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

/// Uygulama ayarları. JSON'a serileştirilir ve store'a yazılır.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Seçili mikrofon cihaz adı; `None` = sistem varsayılanı.
    #[serde(default)]
    pub input_device: Option<String>,
    /// Whisper GGUF model dosyasının yolu (uygulama veri dizinine göre).
    #[serde(default = "default_model_path")]
    pub model_path: String,
    /// Transkripsiyon dili: `"tr"`, `"en"` veya `"auto"`.
    #[serde(default = "default_language")]
    pub language: String,
    /// Global kısayol (ör. `"Ctrl+Shift+Space"`).
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    /// Kısayol modu: push-to-talk veya toggle.
    #[serde(default = "default_hotkey_mode")]
    pub hotkey_mode: HotkeyMode,
    /// Enjeksiyon modu: otomatik, her zaman yazma veya her zaman yapıştırma.
    #[serde(default = "default_injection_mode")]
    pub injection_mode: InjectionMode,
    /// VAD ile otomatik durdurma açık mı (konuşma bitince kayıt dursun).
    #[serde(default = "default_vad_enabled")]
    pub vad_enabled: bool,
    /// Otomatik durdurma için gereken sürekli sessizlik süresi (ms).
    #[serde(default = "default_vad_silence_ms")]
    pub vad_silence_ms: u32,
    /// VAD agresifliği: 1 = Medium, 2 = Aggressive, 3 = VeryAggressive.
    #[serde(default = "default_vad_aggressiveness")]
    pub vad_aggressiveness: u8,
    /// Transkripsiyon sağlayıcısı: yerel model veya Groq API.
    #[serde(default = "default_api_provider")]
    pub api_provider: ApiProvider,
    /// Groq API anahtarı. None/empty ise API modu çalışmaz.
    #[serde(default)]
    pub api_key: Option<String>,
}

/// Audio katmanına aktarılacak VAD yapılandırması. `Settings`'ten türetilir.
#[derive(Debug, Clone, Copy)]
pub struct VadCfg {
    pub enabled: bool,
    pub silence_ms: u32,
    pub aggressiveness: u8,
}

impl Settings {
    /// VAD yapılandırmasını audio katmanı için ayıklar.
    pub fn vad_cfg(&self) -> VadCfg {
        VadCfg {
            enabled: self.vad_enabled,
            silence_ms: self.vad_silence_ms,
            aggressiveness: self.vad_aggressiveness,
        }
    }

    /// Kritik alanları düzeltir; örn. boş kısayol yerine varsayılan atar.
    pub fn sanitize(&mut self) {
        if self.hotkey.trim().is_empty() {
            self.hotkey = default_hotkey();
        }
        // API modunda key boşsa offline'a düşür
        if self.api_provider == ApiProvider::Groq {
            if self.api_key.as_deref().unwrap_or("").trim().is_empty() {
                self.api_provider = ApiProvider::Offline;
            }
        }
    }
}

/// Tauri state olarak yönetilen, çalışma zamanında güncellenebilen ayarlar.
/// `Arc<RwLock>` sayesinde komutlar ve hotkey aynı anda güvenle okur/yazar.
pub type SharedSettings = Arc<RwLock<Settings>>;

/// Yeni paylaşılan ayar sarmalayıcı oluşturur.
pub fn shared(settings: Settings) -> SharedSettings {
    Arc::new(RwLock::new(settings))
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
    /// Yerel whisper.cpp modeli (ggml/bin).
    #[default]
    Offline,
    /// Groq Whisper API (cloud, API key gerekir).
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
        }
    }
}

const STORE_FILE: &str = "settings.json";
const STORE_KEY: &str = "settings";

/// Ayarları store'dan yükler; yoksa varsayılan değerle oluşturup kaydeder.
/// Geçersiz/boş kritik alanları varsayılan değerlerle düzeltir.
pub fn load(app: &AppHandle) -> anyhow::Result<Settings> {
    let store = app.store(STORE_FILE)?;
    let mut settings: Settings = store
        .get(STORE_KEY)
        .and_then(|v| serde_json::from_value::<Settings>(v).ok())
        .unwrap_or_default();
    settings.sanitize();
    Ok(settings)
}

/// Ayarları store'a yazar.
pub fn save(app: &AppHandle, settings: &Settings) -> anyhow::Result<()> {
    let store = app.store(STORE_FILE)?;
    let value = serde_json::to_value(settings)?;
    store.set(STORE_KEY, value);
    store.save()?;
    Ok(())
}
