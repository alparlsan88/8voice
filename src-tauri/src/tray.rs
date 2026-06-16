//! Dinamik tray ikonu üretimi — duruma göre renk değişen programatik ikon.
//!
//! Her `AppState` için 32×32 RGBA ikon üretilir. Tasarım genel uygulama
//! ikonuyla uyumludur: beyaz yuvarlak köşeli kare + ortada koyu nokta +
//! durum renginde ince çerçeve.
//! Renkler UI ile tutarlı:
//! - Idle: zümrüt (hazır)
//! - Recording: kırmızı (kayıt)
//! - Transcribing: amber (işleniyor)
//! - Injecting: camgöbeği (yazılıyor)
//! - Error: gül (hata)
//!
//! Üretim hızlıdır (<1ms) ve sonuç Tauri'nin `Image::new_owned` ile ikona yüklenir.

use tauri::{image::Image, AppHandle};
use tauri::tray::TrayIcon;

use crate::state::AppState;

/// Üretilecek ikonun boyutu (piksel).
const SIZE: u32 = 32;
/// Yuvarlak köşeli karenin genişliği.
const BOX_W: f32 = 24.0;
/// Yuvarlak köşeli karenin yüksekliği.
const BOX_H: f32 = 24.0;
/// Köşe yuvarlaklık yarıçapı.
const BOX_RADIUS: f32 = 7.0;
/// Durum çerçevesi kalınlığı.
const BORDER_WIDTH: f32 = 2.5;
/// Orta noktanın yarıçapı.
const DOT_RADIUS: f32 = 5.0;

/// Verilen durum için tray ikonunu RGBA olarak üretir.
///
/// PNG encode'a gerek yok — Tauri'nin `Image::new_owned` doğrudan RGBA kabul eder.
pub fn render_icon(state: AppState) -> Image<'static> {
    let state_color = state_color(state);
    let mut rgba: Vec<u8> = Vec::with_capacity((SIZE * SIZE * 4) as usize);

    let cx = SIZE as f32 / 2.0;
    let cy = SIZE as f32 / 2.0;

    for y in 0..SIZE {
        for x in 0..SIZE {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let dist = rounded_rect_sdf(px, py, cx, cy, BOX_W, BOX_H, BOX_RADIUS);

            let (r, g, b, a) = if dist > 0.0 {
                // Dışarıda — şeffaf
                (0, 0, 0, 0)
            } else if dist > -BORDER_WIDTH {
                // Çerçeve — durum rengi
                (state_color[0], state_color[1], state_color[2], 255)
            } else {
                // İç kısım
                let dx = px - cx;
                let dy = py - cy;
                let d = (dx * dx + dy * dy).sqrt();
                if d <= DOT_RADIUS {
                    // Orta koyu nokta (uygulama ikonuyla uyumlu)
                    (23, 23, 23, 255)
                } else {
                    // Beyaz arka plan
                    (255, 255, 255, 255)
                }
            };
            rgba.extend_from_slice(&[r, g, b, a]);
        }
    }

    Image::new_owned(rgba, SIZE, SIZE)
}

/// Yuvarlak köşeli dikdörtgen için signed-distance değeri.
/// Negatif içeride, pozitif dışarıda.
fn rounded_rect_sdf(x: f32, y: f32, cx: f32, cy: f32, w: f32, h: f32, r: f32) -> f32 {
    let dx = (x - cx).abs() - (w / 2.0 - r);
    let dy = (y - cy).abs() - (h / 2.0 - r);
    let outside = (dx.max(0.0)).hypot(dy.max(0.0));
    let inside = dx.max(dy).min(0.0);
    outside + inside - r
}

/// `Idle` durumu için ikon.
pub fn idle_icon() -> Image<'static> {
    render_icon(AppState::Idle)
}

/// Duruma karşılık RGBA renk (UI renkleriyle tutarlı).
fn state_color(state: AppState) -> [u8; 3] {
    match state {
        // emerald-500  #10b981
        AppState::Idle => [16, 185, 129],
        // red-500      #ef4444
        AppState::Recording => [239, 68, 68],
        // amber-500    #f59e0b
        AppState::Transcribing => [245, 158, 11],
        // cyan-500     #06b6d4
        AppState::Injecting => [6, 182, 212],
        // rose-700     #be123c
        AppState::Error => [190, 18, 60],
    }
}

/// Kayıtlı tray ikonunu bulur ve verilen duruma göre günceller.
/// Tray henüz kurulmadıysa (erken aşama) sessizce no-op.
pub fn update_icon(app: &AppHandle, state: AppState) {
    if let Some(tray) = app.tray_by_id("main") {
        let _ = set_icon(&tray, state);
    }
}

fn set_icon(tray: &TrayIcon, state: AppState) -> tauri::Result<()> {
    let img = render_icon(state);
    tray.set_icon(Some(img))?;
    tray.set_tooltip(Some(tooltip_text(state)))?;
    Ok(())
}

fn tooltip_text(state: AppState) -> &'static str {
    match state {
        AppState::Idle => "8voice — Hazır",
        AppState::Recording => "8voice — Kayıt yapıyor…",
        AppState::Transcribing => "8voice — Metne çevriliyor…",
        AppState::Injecting => "8voice — Yazılıyor…",
        AppState::Error => "8voice — Hata",
    }
}
