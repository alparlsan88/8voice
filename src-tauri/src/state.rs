//! Kayıt state machine — Idle → Recording → Transcribing → Injecting → Idle.
//!
//! Kontrat:
//! - Input: geçiş istekleri ([`StateEvent`])
//! - Output: güncel [`AppState`]; her geçişte frontend'e `app://state-changed`
//! - Kabul: geçersiz geçişler engellenir; hata her zaman güvenli Idle'a döner

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

/// Uygulama durumları. UI'a yansır.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppState {
    Idle,
    Recording,
    Transcribing,
    Injecting,
    Error,
}

impl Default for AppState {
    fn default() -> Self {
        Self::Idle
    }
}

/// State machine'e gönderilen olaylar.
#[derive(Debug)]
pub enum StateEvent {
    StartRecording,
    StopRecording,
    TranscriptionDone,
    InjectionDone,
    Fail(String),
}

/// Global state holder; [`tauri::State`] üzerinden erişilir.
pub struct StateMachine {
    inner: Mutex<AppState>,
    /// En son hata mesajı (UI için).
    last_error: Mutex<Option<String>>,
    /// `Transcribing` durumuna geçince tetiklenecek pipeline flag'i.
    /// run_pipeline_watcher bunu izler.
    pending_transcribe: Mutex<bool>,
}

impl StateMachine {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(AppState::default()),
            last_error: Mutex::new(None),
            pending_transcribe: Mutex::new(false),
        }
    }

    pub fn current(&self) -> AppState {
        *self.inner.lock()
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().clone()
    }

    /// `StopRecording` ile Transcribing'e geçiş olduysa true döner ve flag'i temizler.
    pub fn take_pending_transcribe(&self) -> bool {
        let mut p = self.pending_transcribe.lock();
        let v = *p;
        *p = false;
        v
    }

    /// Geçiş uygular. Geçerliyse `true`, geçersizse `false` döner.
    /// Her başarılı geçişte frontend'e olay emit edilir.
    pub fn transition(&self, app: &AppHandle, event: StateEvent) -> bool {
        let mut state = self.inner.lock();
        let prev = *state;
        let (next, pipeline) = match (*state, &event) {
            (AppState::Idle, StateEvent::StartRecording) => (Some(AppState::Recording), false),
            (AppState::Recording, StateEvent::StopRecording) => (Some(AppState::Transcribing), true),
            (AppState::Transcribing, StateEvent::TranscriptionDone) => {
                (Some(AppState::Injecting), false)
            }
            (AppState::Injecting, StateEvent::InjectionDone) => (Some(AppState::Idle), false),
            (_, StateEvent::Fail(msg)) => {
                *self.last_error.lock() = Some(msg.to_string());
                (Some(AppState::Error), false)
            }
            _ => (None, false),
        };

        match next {
            Some(new_state) => {
                *state = new_state;
                if pipeline {
                    *self.pending_transcribe.lock() = true;
                }
                drop(state);
                emit_state(app, new_state, prev);
                // Error durumundan otomatik Idle'a dön
                if new_state == AppState::Error {
                    let mut s = self.inner.lock();
                    *s = AppState::Idle;
                    emit_state(app, AppState::Idle, AppState::Error);
                }
                true
            }
            None => false,
        }
    }
}

/// Frontend'e durum değişimini emit eder ve tray ikonunu/gövdeyi günceller.
fn emit_state(app: &AppHandle, state: AppState, previous: AppState) {
    let _ = app.emit(
        "app://state-changed",
        StatePayload { state, previous },
    );
    // Tray ikonunu yeni duruma göre güncelle (renk + tooltip).
    crate::tray::update_icon(app, state);
}

#[derive(Clone, Serialize)]
struct StatePayload {
    state: AppState,
    previous: AppState,
}
