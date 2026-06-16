//! Voice Activity Detection — webrtc-vad (libfvad) sarmalayıcı.
//!
//! Kontrat:
//! - Input: 16 kHz mono f32 PCM (audio.rs resampler çıkışı)
//! - Output: her frame için `is_speech: bool`
//! - Kabul: 30ms frame'ler (480 sample @16kHz); kısmi frame'ler bir sonraki
//!   çağrıda kullanılmak üzere tamponlanır; sessizlik tamamen f32→i16 dönüşümü
//!   ardından libfvad tarafından karar verilir
//!
//! Tasarım: `VadEngine`, audio callback'inden her PCM yığını geldiğinde
//! çağrılır. Frame tamponlama yapısı, callback'in her seferinde tam 480
//! örnek teslim etmesini gerektirmez.

use anyhow::{anyhow, Result};
use webrtc_vad::{SampleRate, Vad, VadMode};

use crate::audio::TARGET_SAMPLE_RATE;

/// 30ms'lik bir VAD frame'i için gerekli örnek sayısı (16 kHz).
pub const FRAME_SAMPLES: usize = TARGET_SAMPLE_RATE * 30 / 1000; // 480
/// Bir frame'in temsil ettiği süre (ms).
pub const FRAME_MS: u32 = 30;

/// Birim agresiflik değerini (1-3) libfvad moduna map'ler.
///
/// - `1` → `Quality` (en az agresif; konuşmayı kaçırmamak için)
/// - `2` → `Aggressive` (önerilen denge)
/// - `3` → `VeryAggressive` (arka plan gürültüsünde bile temiz stop)
fn aggressiveness_to_mode(aggressiveness: u8) -> Result<VadMode> {
    Ok(match aggressiveness {
        1 => VadMode::Quality,
        2 => VadMode::Aggressive,
        3 => VadMode::VeryAggressive,
        other => {
            return Err(anyhow!(
                "Geçersiz VAD agresifliği: {other} (1-3 arası olmalı)"
            ))
        }
    })
}

/// WebRTC VAD motoru. Frame tamponlu.
///
/// # Send güvenliği
/// `webrtc_vad::Vad` bir `*mut Fvad` raw pointer'ı taşır ve bu yüzden
/// `Send` değildir. Biz `VadEngine`'i yalnızca audio callback thread'i
/// içinde, bir `Mutex` koruması altında kullanırız — asla thread'ler arası
/// taşımayız. Bu yüzden `Send` işaretlemesi burada güvenlidir.
pub struct VadEngine {
    vad: Vad,
    /// Önceki çağrıdan arta kalan (tam frame olmayan) örnekler.
    carry: Vec<i16>,
}

// Güvenli: VadEngine'e yalnızca Mutex koruması altında tek thread erişir.
// Fvad pointer'ı thread-safe olmasa da biz asla cross-thread paylaşmıyoruz.
unsafe impl Send for VadEngine {}

impl VadEngine {
    /// Yeni motor. `aggressiveness` 1-3 arası.
    pub fn new(aggressiveness: u8) -> Result<Self> {
        let mode = aggressiveness_to_mode(aggressiveness)?;
        // webrtc-vad 0.4: ctor panik yerine yapılandırıcı döner.
        let vad = Vad::new_with_rate_and_mode(SampleRate::Rate16kHz, mode);
        Ok(Self { vad, carry: Vec::new() })
    }

    /// Mono f32 PCM'i işler. Tam frame'lere böler, her biri için konuşma
    /// tespiti yapar. Dönüş değeri: işlenen frame'lerden **herhangi biri**
    /// konuşma içeriyorsa `true` (sesi duymuş). Kısmi örnekler bir sonraki
    /// çağrıda kullanılmak üzere saklanır.
    ///
    /// `out_silence_ms`, bu çağrıda işlenen sessiz frame sayısı × FRAME_MS'tir;
    /// çağıran taraf bunu sessizlik sayacını güncellemek için kullanır.
    pub fn process(&mut self, mono_f32: &[f32]) -> VadFrameResult {
        // f32 [-1.0, 1.0] → i16 tam ölçek. Doyma korumalı.
        self.carry.reserve(mono_f32.len());
        for &s in mono_f32 {
            let clamped = s.clamp(-1.0, 1.0);
            self.carry.push((clamped * i16::MAX as f32) as i16);
        }

        let mut any_speech = false;
        let mut silent_frames: u32 = 0;
        let mut speech_frames: u32 = 0;

        let n_full = self.carry.len() / FRAME_SAMPLES;
        for i in 0..n_full {
            let start = i * FRAME_SAMPLES;
            let frame = &self.carry[start..start + FRAME_SAMPLES];
            let is_voice = self.vad.is_voice_segment(frame).unwrap_or(false);
            if is_voice {
                speech_frames += 1;
                any_speech = true;
            } else {
                silent_frames += 1;
            }
        }

        // İşlenen tam frame'leri carry'den çıkar, kalan kısmi örnekleri sakla.
        let consumed = n_full * FRAME_SAMPLES;
        self.carry.drain(..consumed);

        VadFrameResult {
            any_speech,
            silent_frames,
            speech_frames,
        }
    }
}

/// Tek bir `process()` çağrısının sonucu.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VadFrameResult {
    /// İşlenen frame'lerden en az biri konuşma içeriyorsa `true`.
    pub any_speech: bool,
    /// Bu çağrıda işlenen sessiz frame sayısı.
    pub silent_frames: u32,
    /// Bu çağrıda işlenen konuşma içeren frame sayısı.
    pub speech_frames: u32,
}

impl VadFrameResult {
    /// İşlenen toplam sessizlik süresi (ms).
    pub fn silence_ms(&self) -> u32 {
        self.silent_frames * FRAME_MS
    }
}

// ---------------------------------------------------------------------------
// Birim testler
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn silence(samples: usize) -> Vec<f32> {
        vec![0.0; samples]
    }

    /// Basit bir ton (440 Hz) — WebRTC VAD bunu konuşma olarak görmeyebilir
    /// ama en azından sessizlikten farklı bir şey üretir. Test için tam
    /// sessizliğin `false` vermesine güveniyoruz; tonlu sinyali yalnızca
    /// "hata fırlatmıyor" olarak doğruluyoruz (VAD moduna bağlı true/false).
    fn tone(freq: usize, samples: usize) -> Vec<f32> {
        (0..samples)
            .map(|i| {
                let t = i as f32 / TARGET_SAMPLE_RATE as f32;
                (2.0 * std::f32::consts::PI * freq as f32 * t).sin() * 0.3
            })
            .collect()
    }

    #[test]
    fn silence_yields_no_speech() {
        let mut engine = VadEngine::new(2).unwrap();
        // 3 frame sessizlik (1440 sample)
        let r = engine.process(&silence(FRAME_SAMPLES * 3));
        assert!(!r.any_speech, "sessizlik konuşma olarak algılanmamalı");
        assert_eq!(r.silent_frames, 3);
        assert_eq!(r.speech_frames, 0);
        assert_eq!(r.silence_ms(), 90);
    }

    #[test]
    fn partial_frame_is_buffered() {
        let mut engine = VadEngine::new(2).unwrap();
        // 200 örnek (< 1 frame) → hiç frame işlenmemeli
        let r = engine.process(&silence(200));
        assert_eq!(r.silent_frames, 0);
        assert_eq!(r.speech_frames, 0);
        // carry'de 200 örnek kalmış olmalı
        assert_eq!(engine.carry.len(), 200);
    }

    #[test]
    fn partial_then_completes_frame() {
        let mut engine = VadEngine::new(2).unwrap();
        // İlk çağrı: 240 örnek (yarım frame)
        let _ = engine.process(&silence(240));
        assert_eq!(engine.carry.len(), 240);
        // İkinci çağrı: 240 örnek daha → toplam 480 = 1 frame
        let r = engine.process(&silence(240));
        assert_eq!(r.silent_frames + r.speech_frames, 1);
        assert!(engine.carry.is_empty(), "tam frame tüketilmeli");
    }

    #[test]
    fn tone_does_not_panic() {
        let mut engine = VadEngine::new(2).unwrap();
        // 440 Hz ton, 3 frame
        let _ = engine.process(&tone(440, FRAME_SAMPLES * 3));
        // Tonun konuşma olarak algılanıp algılanmadığı VAD moduna bağlı;
        // burada yalnızca hata fırlatmadığını doğruluyoruz.
    }

    #[test]
    fn clamp_prevents_overflow() {
        let mut engine = VadEngine::new(2).unwrap();
        // Doyma örneği [-2.0, 2.0] — clamp olmadan i16 taşardı
        let over: Vec<f32> = vec![2.0, -2.0, 1.5, -1.5];
        let _ = engine.process(&over);
        // Panik yok → test geçer
    }

    #[test]
    fn invalid_aggressiveness_errors() {
        assert!(VadEngine::new(0).is_err());
        assert!(VadEngine::new(4).is_err());
        assert!(VadEngine::new(255).is_err());
    }

    #[test]
    fn all_aggressiveness_levels_init() {
        for a in 1..=3 {
            assert!(VadEngine::new(a).is_ok(), "aggressiveness {a} geçerli olmalı");
        }
    }
}
