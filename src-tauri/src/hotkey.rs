//! Global kısayol — tauri-plugin-global-shortcut.
//!
//! Kontrat:
//! - Input: kısayol kombinasyonu, mod (PushToTalk | Toggle)
//! - Output: StartRecording / StopRecording olayları → state machine
//! - Kabul: arka planda çalışıyor; ayar değişince re-register; çakışmada net hata
//!
//! Not: Push-to-talk için hem Pressed hem Released olayları gerekir.
//! Toggle modunda sadece Pressed sayılır.

use anyhow::{anyhow, Result};
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::settings::HotkeyMode;
use crate::state::{AppState, StateMachine};

/// Kısayolu kaydeder. Önce eskileri temizler.
pub fn register(app: &AppHandle, hotkey: &str, mode: HotkeyMode) -> Result<()> {
    let manager = app.global_shortcut();
    manager.unregister_all()?;

    let shortcut: Shortcut = hotkey
        .parse()
        .map_err(|e| anyhow!("Geçersiz kısayol '{hotkey}': {e}"))?;

    manager.on_shortcut(shortcut, move |app, _shortcut, event| {
        let pressed = event.state == ShortcutState::Pressed;
        let released = event.state == ShortcutState::Released;

        let (do_start, do_stop) = match mode {
            HotkeyMode::PushToTalk => {
                if pressed {
                    (true, false)
                } else if released {
                    (false, true)
                } else {
                    (false, false)
                }
            }
            HotkeyMode::Toggle => {
                if pressed {
                    // Mevcut duruma göre start/stop kararını stop_recording /
                    // start_recording helper'ları idempotent biçimde ele alır.
                    match app.state::<StateMachine>().current() {
                        AppState::Idle => (true, false),
                        AppState::Recording => (false, true),
                        _ => (false, false),
                    }
                } else {
                    (false, false)
                }
            }
        };

        if do_start {
            if let Err(e) = crate::start_recording(app) {
                tracing::warn!("Kayıt başlatılamadı: {e}");
            }
        }
        if do_stop {
            // stop_recording idempotent: VAD veya manuel kapatma fark etmez.
            crate::stop_recording(app);
        }
    })?;

    tracing::info!("Kısayol kaydedildi: {hotkey} (mod: {mode:?})");
    Ok(())
}

/// Tüm kısayolları kaldırır.
pub fn unregister_all(app: &AppHandle) -> Result<()> {
    app.global_shortcut().unregister_all()?;
    Ok(())
}
