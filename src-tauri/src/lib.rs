//! 8voice — privacy-first, on-device voice dictation.
//!
//! Modules: audio, transcribe, inject, hotkey, state, settings.

mod audio;
mod hotkey;
mod inject;
mod onboarding;
mod settings;
mod state;
mod transcribe;
mod tray;
mod vad;

use parking_lot::RwLock;
use settings::{ApiProvider, Settings};
use state::{AppState, StateEvent, StateMachine};
use std::sync::Arc;
use tauri::{
    menu::{ContextMenu, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt;

/// Globally shared application state.
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

    // --- Audio (create before app so it can be managed early) ---
    let (audio, audio_buf) = audio::AudioCapture::new();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None::<Vec<&str>>,
        ))
        // --- State: register before setup so it is available during window init ---
        .manage(StateMachine::new())
        .manage(AppCtx {
            audio,
            audio_buf,
        })
        .manage(settings::shared(Settings::default()))
        .manage(onboarding::DownloadController::new())
        .setup(|app| {
            // --- Widget window: make corners truly transparent ---
            make_widget_transparent(app.handle());

            // --- Tray ---
            setup_tray(app.handle())?;

            // --- Load settings + register shortcut + preload model ---
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = bootstrap(&handle) {
                    tracing::error!("Bootstrap error: {e:#}");
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            // Hide on close (keeps alive in tray) — both main and widget.
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
            cmd_widget_context_menu,
            onboarding::cmd_list_whisper_models,
            onboarding::cmd_download_whisper_model,
            onboarding::cmd_cancel_download,
            onboarding::cmd_validate_local_model,
            onboarding::cmd_validate_groq_key,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Makes the widget window's background fully transparent and removes the
/// faint gray "half-rectangle" artifacts at the corners.
///
/// Two things are needed on Windows:
/// 1. Set WebView2's `DefaultBackgroundColor` to fully transparent (A=0) so the
///    WebView itself does not paint an opaque background outside the pill shape.
/// 2. Tell DWM **not** to round the window corners. Windows 11 rounds every
///    top-level window (~8px) even when it is transparent/undecorated; that
///    rounded corner clipping paints a faint gray sliver around the widget's
///    own `rounded-full` pill, which reads as the white/gray "half-rectangles".
///    `DWMWCP_DONOTROUND` keeps the OS window a clean sharp rectangle, so the
///    only thing visible is the pill itself.
#[cfg(windows)]
fn make_widget_transparent(app: &AppHandle) {
  use tauri::Manager;
  use windows_core::Interface;
  use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
  };
  use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2Controller2, COREWEBVIEW2_COLOR,
  };

  if let Some(widget) = app.get_webview_window("widget") {
    // (2) Disable DWM corner rounding for this window — removes the gray corners.
    if let Ok(hwnd) = widget.hwnd() {
      let _ = unsafe {
        let preference = DWMWCP_DONOTROUND.0 as i32;
        DwmSetWindowAttribute(
          hwnd,
          DWMWA_WINDOW_CORNER_PREFERENCE,
          &preference as *const _ as *const _,
          std::mem::size_of::<i32>() as u32,
        )
      };
    }

    // (1) Make the WebView2 background transparent.
    let _ = widget.with_webview(|webview| unsafe {
      let controller = webview.controller();
      match controller.cast::<ICoreWebView2Controller2>() {
        Ok(controller2) => {
          // A=0 → fully transparent. An earlier version mistakenly called
          // the getter (DefaultBackgroundColor); we must use the setter.
          let color = COREWEBVIEW2_COLOR {
            A: 0,
            R: 0,
            G: 0,
            B: 0,
          };
          match controller2.SetDefaultBackgroundColor(color) {
            Ok(_) => tracing::debug!("Widget WebView2 background made transparent"),
            Err(e) => tracing::warn!("Could not set DefaultBackgroundColor: {e}"),
          }
        }
        Err(e) => tracing::warn!("Could not cast to ICoreWebView2Controller2: {e}"),
      }
    });
  } else {
    tracing::warn!("Widget window not found during setup");
  }
}

#[cfg(not(windows))]
fn make_widget_transparent(_app: &AppHandle) {}

/// Startup: load settings, register shortcut, preload model.
fn bootstrap(app: &AppHandle) -> tauri::Result<()> {
    let loaded = settings::load(app).unwrap_or_else(|e| {
        tracing::warn!("Could not load settings, using defaults: {e}");
        Settings::default()
    });

    // Update shared state
    {
        let shared = app.state::<Arc<RwLock<Settings>>>();
        *shared.write() = loaded.clone();
    }

    // Shortcut
    if let Err(e) = hotkey::register(app, &loaded.hotkey, loaded.hotkey_mode) {
        tracing::warn!("Could not register shortcut: {e:#}");
    }

    // Apply tray icon visibility
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_visible(loaded.show_tray_icon);
    }

    // If first run, show the main window for onboarding and hide the widget
    // so the setup screen is not covered by the floating mic.
    if !loaded.has_completed_onboarding {
        if let Some(widget) = app.get_webview_window("widget") {
            let _ = widget.hide();
        }
        open_settings(app);
    } else if loaded.start_hidden {
        // User wants a hidden start: hide both windows.
        if let Some(widget) = app.get_webview_window("widget") {
            let _ = widget.hide();
        }
        if let Some(main) = app.get_webview_window("main") {
            let _ = main.hide();
        }
    }

    // Model preload (if path is valid) — only needed in offline mode
    if loaded.api_provider == ApiProvider::Offline {
        let model_path = resolve_model_path(app, &loaded.model_path);
        if model_path.exists() {
            if let Err(e) = transcribe::Transcriber::load(&model_path) {
                tracing::warn!("Model preload failed: {e:#}");
            }
        } else {
            tracing::warn!(
                "Model not found ({}); a warning will be shown in settings",
                model_path.display()
            );
        }
    }

    Ok(())
}

/// Resolves the model path relative to the application resource directory.
fn resolve_model_path(app: &AppHandle, stored: &str) -> std::path::PathBuf {
    let p = std::path::PathBuf::from(stored);
    if p.is_absolute() {
        return p;
    }
    // src-tauri/models/... may be under resource_dir; try app_local_data first
    if let Ok(dir) = app.path().app_local_data_dir() {
        let candidate = dir.join(stored);
        if candidate.exists() {
            return candidate;
        }
    }
    // During development: src-tauri/models/...
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            // target/debug or target/release → src-tauri/models
            for ancestor in parent.ancestors().take(4) {
                let candidate = ancestor.join(stored);
                if candidate.exists() {
                    return candidate;
                }
            }
        }
    }
    // Fallback: relative to working directory
    std::path::PathBuf::from(stored)
}

/// Tray icon + menu setup.
///
/// Simplified menu: "Show/hide widget", "Settings...", separator, "Quit".
/// Left click toggles the widget.
fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let widget = MenuItem::with_id(app, "widget", "Show/hide widget", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings...", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&widget, &settings, &sep, &quit])?;

    let _tray = TrayIconBuilder::with_id("main")
        .icon(tray::idle_icon())
        .tooltip("8voice — Ready")
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
            // Left click → toggle widget
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

/// Shows/hides the widget window (toggle).
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

/// Opens the settings window (shows it if hidden, focuses if visible).
fn open_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Runs the transcribe → inject chain when recording stops.
/// Called by hotkey.rs (PTT release / toggle) and cmd_stop_recording.
pub fn run_pipeline(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let ctx = app.state::<AppCtx>();
        let sm = app.state::<StateMachine>();
        let shared = app.state::<Arc<RwLock<Settings>>>();

        // 1) Get PCM
        let pcm = audio::AudioCapture::drain(&ctx.audio_buf);

        // 2) Transcribe (copy language + provider settings)
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
                            tracing::error!("Groq transcription error: {e:#}");
                            sm.transition(
                                &app,
                                StateEvent::Fail(format!("Groq transcription: {e}")),
                            );
                            return;
                        }
                    }
                }
                _ => {
                    tracing::error!("Groq provider selected but API key is empty");
                    sm.transition(
                        &app,
                        StateEvent::Fail(
                            "Groq API key missing. Add it in Settings.".into(),
                        ),
                    );
                    return;
                }
            },
            ApiProvider::Offline => match transcribe::Transcriber::transcribe(&pcm, &language) {
                Ok(t) => t,
                Err(e) => {
                    tracing::error!("Transcription error: {e:#}");
                    sm.transition(&app, StateEvent::Fail(format!("Transcription: {e}")));
                    return;
                }
            },
        };
        if text.is_empty() {
            tracing::info!("Empty transcript; injection skipped");
            sm.transition(&app, StateEvent::TranscriptionDone);
            sm.transition(&app, StateEvent::InjectionDone);
            return;
        }

        sm.transition(&app, StateEvent::TranscriptionDone);

        // 3) Inject
        if let Err(e) = inject::inject(&text, injection_mode) {
            tracing::error!("Injection error: {e:#}");
            sm.transition(&app, StateEvent::Fail(format!("Injection: {e}")));
            return;
        }
        sm.transition(&app, StateEvent::InjectionDone);

        let _ = app.emit("app://transcript", &text);
    });
}

/// Stops recording and triggers the transcribe→inject chain.
///
/// Single orchestration point: shortcut (PTT release / toggle), manual
/// "Stop" button, and the VAD watcher all call this. Idempotent — safe no-op
/// if not currently recording (state != Recording).
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

/// Starts recording (shared by shortcut and command).
/// VAD setting is read from settings; in toggle mode VAD can auto-stop.
/// In PTT mode release is the manual stop, but VAD still runs (release takes priority).
pub fn start_recording(app: &AppHandle) -> Result<(), String> {
    let sm = app.state::<StateMachine>();
    let ctx = app.state::<AppCtx>();
    let shared = app.state::<Arc<RwLock<Settings>>>();
    let (device, vad_cfg) = {
        let s = shared.read();
        (s.input_device.clone(), s.vad_cfg())
    };
    ctx.audio
        .start(app, device.as_deref(), vad_cfg)
        .map_err(|e| e.to_string())?;
    if !sm.transition(app, StateEvent::StartRecording) {
        ctx.audio.stop();
        return Err("Cannot start recording right now (busy)".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tauri commands (frontend ↔ backend)
// ---------------------------------------------------------------------------

#[tauri::command]
fn cmd_get_state(state: tauri::State<'_, StateMachine>) -> (AppState, Option<String>) {
    (state.current(), state.last_error())
}

#[tauri::command]
fn cmd_get_settings(shared: tauri::State<'_, Arc<RwLock<Settings>>>) -> Settings {
    shared.read().clone()
}

#[tauri::command]
fn cmd_save_settings(app: AppHandle, mut settings: Settings) -> Result<(), String> {
    // Fix invalid fields
    settings.sanitize();

    // Snapshot of previous settings so we can react to changes.
    let old_settings = {
        let shared = app.state::<Arc<RwLock<Settings>>>();
        let s = shared.read().clone();
        s
    };

    // Save
    settings::save(&app, &settings).map_err(|e| e.to_string())?;
    // Update shared state
    {
        let shared = app.state::<Arc<RwLock<Settings>>>();
        *shared.write() = settings.clone();
    }
    // Re-register shortcut
    if let Err(e) = hotkey::register(&app, &settings.hotkey, settings.hotkey_mode) {
        tracing::warn!("Could not re-register shortcut: {e:#}");
    }
    // Reload model if path changed and in offline mode
    if settings.api_provider == ApiProvider::Offline {
        let path = resolve_model_path(&app, &settings.model_path);
        if path.exists() {
            let _ = transcribe::Transcriber::load(&path);
        }
    }

    // Apply launch-on-startup change immediately
    if settings.launch_on_startup != old_settings.launch_on_startup {
        let autolaunch = app.autolaunch();
        let res = if settings.launch_on_startup {
            autolaunch.enable()
        } else {
            autolaunch.disable()
        };
        if let Err(e) = res {
            tracing::warn!("Could not change autostart setting: {e:#}");
        }
    }

    // Apply tray icon visibility change immediately
    if settings.show_tray_icon != old_settings.show_tray_icon {
        if let Some(tray) = app.tray_by_id("main") {
            if let Err(e) = tray.set_visible(settings.show_tray_icon) {
                tracing::warn!("Could not change tray visibility: {e:#}");
            }
        }
    }

    // Once onboarding is complete, make sure the recording widget is visible
    // unless the user explicitly wants a hidden start.
    if settings.has_completed_onboarding && !settings.start_hidden {
        if let Some(widget) = app.get_webview_window("widget") {
            let _ = widget.show();
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

/// Starts/stops based on recording state — called by the widget microphone button.
/// Starts if Idle, stops if Recording, no-op for other busy states.
#[tauri::command]
fn cmd_toggle_recording(app: AppHandle) -> Result<(), String> {
    let sm = app.state::<StateMachine>();
    match sm.current() {
        AppState::Idle => start_recording(&app),
        AppState::Recording => {
            stop_recording(&app);
            Ok(())
        }
        // Transcribing/Injecting/Error → user must wait
        _ => Ok(()),
    }
}

/// Show/hide widget window (callable from frontend).
#[tauri::command]
fn cmd_toggle_widget(app: AppHandle) -> Result<(), String> {
    toggle_widget(&app);
    Ok(())
}

/// Open settings window (callable from frontend).
#[tauri::command]
fn cmd_open_settings(app: AppHandle) -> Result<(), String> {
    open_settings(&app);
    Ok(())
}

/// Shows a native context menu on the widget window with a "Quit" option.
#[tauri::command]
fn cmd_widget_context_menu(app: AppHandle, x: f64, y: f64) -> Result<(), String> {
    let webview_window = app
        .get_webview_window("widget")
        .ok_or("Widget window not found")?;
    let window = webview_window.as_ref().window();

    let quit = MenuItem::with_id(&app, "quit", "Quit 8voice", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let menu = Menu::with_items(&app, &[&quit]).map_err(|e| e.to_string())?;

    let app_clone = app.clone();
    window.on_menu_event(move |_, event| {
        if event.id().as_ref() == "quit" {
            app_clone.exit(0);
        }
    });

    menu.popup_at(
        window,
        tauri::Position::Logical(tauri::LogicalPosition { x, y }),
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}
