//! 8voice — gizlilik öncelikli, on-device sesli dikte.
//!
//! Modüller: audio, transcribe, inject, hotkey, state, settings.

mod audio;
mod hotkey;
mod inject;
mod settings;
mod state;
mod transcribe;
mod tray;
mod vad;

use settings::{ApiProvider, Settings, SharedSettings};
use state::{AppState, StateEvent, StateMachine};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, WindowEvent,
};

/// Global paylaşılan uygulama durumu.
pub struct AppCtx {
    pub audio: audio::AudioCapture,
    pub audio_buf: audio::SharedBuffer,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,eightvoice=debug")),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // --- State ---
            app.manage(StateMachine::new());

            // --- Audio ---
            let (capture, buf) = audio::AudioCapture::new();
            app.manage(AppCtx {
                audio: capture,
                audio_buf: buf,
            });

            // --- Ayarlar (önce varsayılan, bootstrap günceller) ---
            app.manage(settings::shared(Settings::default()));

            // --- Widget penceresi: köşeleri gerçekten saydam yap ---
            make_widget_transparent(app.handle());

            // --- Tray ---
            setup_tray(app.handle())?;

            // --- Ayarları yükle + kısayol kaydet + model preload ---
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = bootstrap(&handle) {
                    tracing::error!("Bootstrap hatası: {e:#}");
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            // Pencere kapatınca gizle (tray'de yaşatır) — hem main hem widget.
            if let WindowEvent::CloseRequested { api, .. } = event {
                match window.label() {
                    "main" | "widget" => {
                        let _ = window.hide();
                        api.prevent_close();
                    }
                    _ => {}
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            cmd_get_settings,
            cmd_save_settings,
            cmd_list_devices,
            cmd_get_state,
            cmd_start_recording,
            cmd_stop_recording,
            cmd_toggle_recording,
            cmd_toggle_widget,
            cmd_open_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Widget penceresinin WebView2 arka planını tam saydam yapar.
/// Bu, Windows'ta köşelerin masaüstü rengi yerine gerçekten saydam
/// görünmesini sağlar. `rounded-full` pill gövdesi pencerenin dörtgen
/// köşelerini taşır; köşelerde WebView2'nin varsayılan opak beyaz arka
/// planı görülmemesi için bunu açıkça (0,0,0,0) yapmalıyız.
#[cfg(windows)]
fn make_widget_transparent(app: &AppHandle) {
  use tauri::Manager;
  use windows_core::Interface;
  use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2Controller2, COREWEBVIEW2_COLOR,
  };

  if let Some(widget) = app.get_webview_window("widget") {
    let _ = widget.with_webview(|webview| unsafe {
      let controller = webview.controller();
      match controller.cast::<ICoreWebView2Controller2>() {
        Ok(controller2) => {
          // A=0 → tam saydam. Önceki sürüm yanlışlıkla getter
          // (DefaultBackgroundColor) çağırıyordu; setter kullanmalıyız.
          let color = COREWEBVIEW2_COLOR {
            A: 0,
            R: 0,
            G: 0,
            B: 0,
          };
          match controller2.SetDefaultBackgroundColor(color) {
            Ok(_) => tracing::debug!("Widget WebView2 arka plani saydam yapildi"),
            Err(e) => tracing::warn!("DefaultBackgroundColor ayarlanamadi: {e}"),
          }
        }
        Err(e) => tracing::warn!("ICoreWebView2Controller2 cast edilemedi: {e}"),
      }
    });
  } else {
    tracing::warn!("Widget penceresi setup sirasinda bulunamadi");
  }
}

#[cfg(not(windows))]
fn make_widget_transparent(_app: &AppHandle) {}

/// Başlangıç: ayarları yükle, kısayolu kaydet, modeli preload et.
fn bootstrap(app: &AppHandle) -> tauri::Result<()> {
    let loaded = settings::load(app).unwrap_or_else(|e| {
        tracing::warn!("Ayarlar yüklenemedi, varsayılan: {e}");
        Settings::default()
    });

    // Shared state'i güncelle
    {
        let shared = app.state::<SharedSettings>();
        *shared.write() = loaded.clone();
    }

    // Kısayol
    if let Err(e) = hotkey::register(app, &loaded.hotkey, loaded.hotkey_mode) {
        tracing::warn!("Kısayol kaydedilemedi: {e:#}");
    }

    // Model preload (yol geçerliyse) — yalnızca offline modda gerekli
    if loaded.api_provider == ApiProvider::Offline {
        let model_path = resolve_model_path(app, &loaded.model_path);
        if model_path.exists() {
            if let Err(e) = transcribe::Transcriber::load(&model_path) {
                tracing::warn!("Model preload başarısız: {e:#}");
            }
        } else {
            tracing::warn!(
                "Model bulunamadı ({}); ayarlar ekranında uyarı gösterilecek",
                model_path.display()
            );
        }
    }

    Ok(())
}

/// Model yolunu uygulama kaynak dizinine göre çözümler.
fn resolve_model_path(app: &AppHandle, stored: &str) -> std::path::PathBuf {
    let p = std::path::PathBuf::from(stored);
    if p.is_absolute() {
        return p;
    }
    // src-tauri/models/... → resource_dir altında olabilir; önce app_local_data dene
    if let Ok(dir) = app.path().app_local_data_dir() {
        let candidate = dir.join(stored);
        if candidate.exists() {
            return candidate;
        }
    }
    // Geliştirme sırasında src-tauri/models/...
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            // target/debug veya target/release → src-tauri/models
            for ancestor in parent.ancestors().take(4) {
                let candidate = ancestor.join(stored);
                if candidate.exists() {
                    return candidate;
                }
            }
        }
    }
    // Son çare: çalışma dizinine göre
    std::path::PathBuf::from(stored)
}

/// Tray ikonu + menü kurulumu.
///
/// Sadeleştirilmiş menü (Faz 3A): "Widget'ı göster/gizle", "Ayarlar...",
/// ayraç, "Çıkış". Sol tık widget'ı toggle eder.
fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let widget = MenuItem::with_id(app, "widget", "Widget'ı göster/gizle", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Ayarlar...", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Çıkış", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&widget, &settings, &sep, &quit])?;

    let _tray = TrayIconBuilder::with_id("main")
        .icon(tray::idle_icon())
        .tooltip("8voice — Hazır")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "widget" => {
                toggle_widget(app);
            }
            "settings" => {
                open_settings(app);
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // Sol tık → widget'ı göster/gizle
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_widget(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

/// Widget penceresini gösterir/gizler (toggle).
fn toggle_widget(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("widget") {
        match w.is_visible() {
            Ok(true) => {
                let _ = w.hide();
            }
            _ => {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
    }
}

/// Ayarlar penceresini açar (yoksa görünür yap, varsa odakla).
fn open_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Kayıt durunca: transcribe → inject zincirini çalıştırır.
/// hotkey.rs (push-to-talk release / toggle) ve cmd_stop_recording tarafından çağrılır.
pub fn run_pipeline(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let ctx = app.state::<AppCtx>();
        let sm = app.state::<StateMachine>();
        let shared = app.state::<SharedSettings>();

        // 1) PCM al
        let pcm = audio::AudioCapture::drain(&ctx.audio_buf);

        // 2) Transcribe (dil + sağlayıcı ayarları kopyalanır)
        let (language, injection_mode, provider, api_key) = {
            let s = shared.read();
            (
                s.language.clone(),
                s.injection_mode,
                s.api_provider,
                s.api_key.clone(),
            )
        };
        let text = match provider {
            ApiProvider::Groq => match api_key {
                Some(key) if !key.trim().is_empty() => {
                    match transcribe::transcribe_groq(&pcm, &language, &key).await {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::error!("Groq transkripsiyon hatası: {e:#}");
                            sm.transition(
                                &app,
                                StateEvent::Fail(format!("Groq transkripsiyon: {e}")),
                            );
                            return;
                        }
                    }
                }
                _ => {
                    tracing::error!("Groq sağlayıcısı seçili ama API key boş");
                    sm.transition(
                        &app,
                        StateEvent::Fail(
                            "Groq API anahtarı eksik. Ayarlar'dan ekleyin.".into(),
                        ),
                    );
                    return;
                }
            },
            ApiProvider::Offline => match transcribe::Transcriber::transcribe(&pcm, &language) {
                Ok(t) => t,
                Err(e) => {
                    tracing::error!("Transcribe hatası: {e:#}");
                    sm.transition(&app, StateEvent::Fail(format!("Transkripsiyon: {e}")));
                    return;
                }
            },
        };
        if text.is_empty() {
            tracing::info!("Boş transkript; enjeksiyon atlandı");
            sm.transition(&app, StateEvent::TranscriptionDone);
            sm.transition(&app, StateEvent::InjectionDone);
            return;
        }

        sm.transition(&app, StateEvent::TranscriptionDone);

        // 3) Inject
        if let Err(e) = inject::inject(&text, injection_mode) {
            tracing::error!("Enjeksiyon hatası: {e:#}");
            sm.transition(&app, StateEvent::Fail(format!("Enjeksiyon: {e}")));
            return;
        }
        sm.transition(&app, StateEvent::InjectionDone);

        let _ = app.emit("app://transcript", &text);
    });
}

/// Kaydı durdurur ve transcribe→inject zincirini tetikler.
///
/// Tek orchustrasyon noktası: kısayol (PTT release / toggle), manuel
/// "Durdur" butonu ve VAD watcher hep bunu çağırır. İdempotent — zaten
/// kayıt yapılmıyorsa (state Recording değilse) güvenli no-op.
pub fn stop_recording(app: &AppHandle) {
    let ctx = app.state::<AppCtx>();
    let sm = app.state::<StateMachine>();
    let already_idle = sm.current() != AppState::Recording;
    ctx.audio.stop();
    if already_idle {
        return;
    }
    if !sm.transition(app, StateEvent::StopRecording) {
        return;
    }
    run_pipeline(app);
}

/// Kaydı başlatır (kısayol + komut tarafından ortak kullanılır).
/// VAD ayarı settings'ten okunur; toggle modunda VAD aktifse otomatik
/// durdurma devreye girer. PTT'de release manuel stop olduğu için VAD
/// yine de çalışabilir ama release önceliklidir.
pub fn start_recording(app: &AppHandle) -> Result<(), String> {
    let sm = app.state::<StateMachine>();
    let ctx = app.state::<AppCtx>();
    let shared = app.state::<SharedSettings>();
    let (device, vad_cfg) = {
        let s = shared.read();
        (s.input_device.clone(), s.vad_cfg())
    };
    ctx.audio
        .start(app, device.as_deref(), vad_cfg)
        .map_err(|e| e.to_string())?;
    if !sm.transition(app, StateEvent::StartRecording) {
        ctx.audio.stop();
        return Err("Şu anda kayıt başlatılamaz (meşgul)".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tauri komutları (frontend ↔ backend)
// ---------------------------------------------------------------------------

#[tauri::command]
fn cmd_get_state(state: tauri::State<'_, StateMachine>) -> (AppState, Option<String>) {
    (state.current(), state.last_error())
}

#[tauri::command]
fn cmd_get_settings(shared: tauri::State<'_, SharedSettings>) -> Settings {
    shared.read().clone()
}

#[tauri::command]
fn cmd_save_settings(app: AppHandle, mut settings: Settings) -> Result<(), String> {
    // Geçersiz alanları düzelt
    settings.sanitize();

    // Kaydet
    settings::save(&app, &settings).map_err(|e| e.to_string())?;
    // Shared state'i güncelle
    {
        let shared = app.state::<SharedSettings>();
        *shared.write() = settings.clone();
    }
    // Kısayolu yeniden kaydet
    if let Err(e) = hotkey::register(&app, &settings.hotkey, settings.hotkey_mode) {
        tracing::warn!("Kısayol yeniden kaydedilemedi: {e:#}");
    }
    // Model yolu değiştiyse ve offline moddaysa yeniden yükle
    if settings.api_provider == ApiProvider::Offline {
        let path = resolve_model_path(&app, &settings.model_path);
        if path.exists() {
            let _ = transcribe::Transcriber::load(&path);
        }
    }
    Ok(())
}

#[tauri::command]
fn cmd_list_devices() -> Result<Vec<String>, String> {
    audio::AudioCapture::list_devices().map_err(|e| e.to_string())
}

#[tauri::command]
fn cmd_start_recording(app: AppHandle) -> Result<(), String> {
    start_recording(&app)
}

#[tauri::command]
fn cmd_stop_recording(app: AppHandle) -> Result<(), String> {
    stop_recording(&app);
    Ok(())
}

/// Kayıt durumuna göre başlat/durdur — widget'ın mikrofon butonu bunu çağırır.
/// Idle ise başlatır, Recording ise durdurur, diğer meşgul durumlarında no-op.
#[tauri::command]
fn cmd_toggle_recording(app: AppHandle) -> Result<(), String> {
    let sm = app.state::<StateMachine>();
    match sm.current() {
        AppState::Idle => start_recording(&app),
        AppState::Recording => {
            stop_recording(&app);
            Ok(())
        }
        // Transcribing/Injecting/Error → kullanıcı beklemeli
        _ => Ok(()),
    }
}

/// Widget penceresini göster/gizle (frontend'den çağrılabilir).
#[tauri::command]
fn cmd_toggle_widget(app: AppHandle) -> Result<(), String> {
    toggle_widget(&app);
    Ok(())
}

/// Ayarlar penceresini aç (frontend'den çağrılabilir).
#[tauri::command]
fn cmd_open_settings(app: AppHandle) -> Result<(), String> {
    open_settings(&app);
    Ok(())
}
