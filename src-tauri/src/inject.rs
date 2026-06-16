//! Metin enjeksiyonu — enigo (typing) + clipboard (paste).
//!
//! Kontrat:
//! - Input: metin, mod (Typing | Paste | Auto)
//! - Output: Result<()> — başarı/başarısızlık
//! - Kabul: >200 karakterde paste; Unicode bozulmuyor; macOS'ta Accessibility kontrolü
//! - Cross-platform:
//!   - Windows: enigo SendInput; elevated app'e yazamaz
//!   - macOS: Accessibility izni şart
//!   - Linux: X11 çalışır; Wayland'da clipboard fallback

use anyhow::{anyhow, Result};
use arboard::Clipboard;
use enigo::{Enigo, Key, Keyboard, Settings};

use crate::settings::InjectionMode;

/// Paste modu eşiği (Auto modunda).
const PASTE_THRESHOLD: usize = 200;

/// Metni odaklı pencereye enjekte eder.
pub fn inject(text: &str, mode: InjectionMode) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    check_accessibility_permission()?;

    let use_paste = match mode {
        InjectionMode::Paste => true,
        InjectionMode::Typing => false,
        InjectionMode::Auto => text.chars().count() > PASTE_THRESHOLD,
    };

    if use_paste {
        inject_paste(text)
    } else {
        inject_typing(text)
    }
}

/// Yapıştırma (clipboard) modu. Pano eski içeriği saklanıp geri yüklenir.
fn inject_paste(text: &str) -> Result<()> {
    let mut clipboard =
        Clipboard::new().map_err(|e| anyhow!("Pano erişilemedi: {e}"))?;

    // Eski panoyu sakla (metin değilse yok say)
    let old = clipboard.get_text().ok();

    clipboard
        .set_text(text)
        .map_err(|e| anyhow!("Panoya yazılamadı: {e}"))?;

    // Kısa süre bekle ki clipboard sistem tarafından okunsun
    std::thread::sleep(std::time::Duration::from_millis(50));

    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| anyhow!("Enigo başlatılamadı: {e}"))?;
    // Ctrl+V (Windows/Linux) veya Cmd+V (macOS)
    #[cfg(target_os = "macos")]
    let modifier = Key::Super;
    #[cfg(not(target_os = "macos"))]
    let modifier = Key::Control;
    enigo.key(modifier, enigo::Direction::Press)?;
    enigo.key(Key::Unicode('v'), enigo::Direction::Click)?;
    enigo.key(modifier, enigo::Direction::Release)?;

    // Panoyu geri yükle (yapıştırma tamamlandıktan sonra)
    std::thread::sleep(std::time::Duration::from_millis(150));
    if let Some(prev) = old {
        let _ = clipboard.set_text(prev);
    }
    Ok(())
}

/// Tuş simülasyonu (typing) modu.
fn inject_typing(text: &str) -> Result<()> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| anyhow!("Enigo başlatılamadı: {e}"))?;
    enigo.text(text).map_err(|e| anyhow!("Yazma hatası: {e}"))?;
    Ok(())
}

/// macOS Accessibility izni kontrolü ve kullanıcı yönlendirmesi.
#[cfg(target_os = "macos")]
fn check_accessibility_permission() -> Result<()> {
    use std::process::Command;
    // Basit yaklaşım: enigo'nun kendi kontrolüne güven; izin yoksa yazma başarısız olur.
    // Daha sağlam: ApplicationServices API ile sorgu. MVP'de uyarı mesajı veriyoruz.
    let ok: bool = {
        // AXIsProcessTrustedWithOptions ile trusted mı?
        let script = "tell application \"System Events\" to UI elements enabled";
        Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    if !ok {
        return Err(anyhow!(
            "macOS Accessibility izni gerekli. System Settings → Privacy & Security → Accessibility → 8voice'u etkinleştirin."
        ));
    }
    Ok(())
}
