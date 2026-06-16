//! Ses yakalama — cpal mikrofon + rubato resample (16 kHz mono) + tampon.
//!
//! Kontrat:
//! - Input: cihaz adı (opsiyonel), start()/stop()
//! - Output: `Vec<f32>` — 16 kHz, mono, normalize [-1.0, 1.0] PCM
//! - Kabul: farklı sample rate'ler 16 kHz'e; stereo→mono; izin reddinde net hata
//!
//! VAD (Faz 2): aktifken her PCM yığını konuşma tespitine tabi tutulur;
//! sürekli sessizlik `silence_ms` atomic'inde biriktirilir. Bir watcher task
//! bu sayacı periyodik okuyup eşik aşılınca kaydı durdurur. VAD kapalıysa
//! davranış Faz 1 ile birebirdir (sadece manuel durdurma).
//!
//! MVP notu: ringbuf yerine `Arc<Mutex<VecDeque<f32>>>` kullanılır (basit,
//! hatasız). Faz 2'de lock-free ringbuf'a geçilebilir.

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use parking_lot::Mutex;
use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;

use crate::settings::VadCfg;
use crate::vad::VadEngine;

/// Whisper'ın beklediği örnek hızı.
pub const TARGET_SAMPLE_RATE: usize = 16_000;
/// Tampon üst sınırı (~2 dakika 16 kHz mono).
const BUFFER_CAP: usize = TARGET_SAMPLE_RATE * 120;
/// VAD watcher'ın kontrol periyodu (ms).
const VAD_WATCHER_TICK_MS: u64 = 50;

/// Paylaşılan ses tamponu.
pub type SharedBuffer = Arc<Mutex<VecDeque<f32>>>;

/// `cpal::Stream` `!Send` işaretlenmiştir (platform katmanında `*mut ()`
/// taşır). Biz stream'i aslında thread'ler arası taşımıyoruz — sadece
/// `start`/`stop` sırasında tek thread'den erişiyoruz. `AppCtx`'in Tauri'nin
/// `Send + Sync` gereksinimini karşılaması için burada güvenli newtype.
struct SendStream(Stream);
unsafe impl Send for SendStream {}

/// Mikrofon yakalayıcı.
pub struct AudioCapture {
    buf: SharedBuffer,
    stream: Mutex<Option<SendStream>>,
    active: Mutex<bool>,
    /// VAD motoru; `None` = VAD kapalı (kayıt başında set edilir).
    /// `Arc`, audio callback'ine klonlanıp taşınabilmesi için.
    vad: Arc<Mutex<Option<VadEngine>>>,
    /// Biriken sürekli sessizlik süresi (ms). VAD tarafından yazılır,
    /// watcher tarafından okunur. Konuşma algılandığında sıfırlanır.
    silence_ms: Arc<AtomicU32>,
    /// Watcher'ı durdurmak için sinyal. `stop()` true yapar.
    watcher_cancel: Arc<AtomicBool>,
}

impl AudioCapture {
    /// Yeni yakalayıcı + paylaşılan tampon. Henüz kaydetmez.
    pub fn new() -> (Self, SharedBuffer) {
        let buf = Arc::new(Mutex::new(VecDeque::with_capacity(BUFFER_CAP)));
        let silence_ms = Arc::new(AtomicU32::new(0));
        let watcher_cancel = Arc::new(AtomicBool::new(false));
        (
            Self {
                buf: Arc::clone(&buf),
                stream: Mutex::new(None),
                active: Mutex::new(false),
                vad: Arc::new(Mutex::new(None)),
                silence_ms,
                watcher_cancel,
            },
            buf,
        )
    }

    /// Kullanılabilir giriş cihazlarını listeler (isim olarak).
    pub fn list_devices() -> Result<Vec<String>> {
        let host = cpal::default_host();
        let mut names = Vec::new();
        for dev in host.input_devices()? {
            if let Ok(name) = dev.name() {
                names.push(name);
            }
        }
        Ok(names)
    }

    /// Kaydı başlatır. `device_name` None ise varsayılan cihaz.
    /// `vad_cfg.enabled` ise VAD motoru başlatılır ve sessizlikte otomatik
    /// durdurma için bir watcher task spawn edilir.
    pub fn start(
        &self,
        app: &AppHandle,
        device_name: Option<&str>,
        vad_cfg: VadCfg,
    ) -> Result<()> {
        let mut active = self.active.lock();
        if *active {
            return Err(anyhow!("Kayıt zaten aktif"));
        }

        // --- VAD hazırlığı ---
        self.silence_ms.store(0, Ordering::Relaxed);
        self.watcher_cancel.store(false, Ordering::Relaxed);
        let engine = if vad_cfg.enabled {
            match VadEngine::new(vad_cfg.aggressiveness) {
                Ok(e) => Some(e),
                Err(e) => {
                    tracing::warn!("VAD başlatılamadı, devre dışı devam: {e:#}");
                    None
                }
            }
        } else {
            None
        };
        *self.vad.lock() = engine;

        let host = cpal::default_host();
        let device = match device_name {
            Some(name) => host
                .input_devices()?
                .find(|d| d.name().ok().as_deref() == Some(name))
                .ok_or_else(|| anyhow!("Cihaz bulunamadı: {name}"))?,
            None => host
                .default_input_device()
                .ok_or_else(|| anyhow!("Varsayılan giriş cihazı yok"))?,
        };

        let supported = device
            .default_input_config()
            .context("Cihaz yapılandırması alınamadı")?;
        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let in_rate = config.sample_rate.0 as usize;
        let channels = config.channels as usize;

        let needs_resample = in_rate != TARGET_SAMPLE_RATE;
        let resampler_state = if needs_resample {
            Some(ResamplerState::new(in_rate)?)
        } else {
            None
        };

        let buf = Arc::clone(&self.buf);
        let resampler = Arc::new(Mutex::new(resampler_state));
        let vad = Arc::clone(&self.vad);
        let silence_ms = Arc::clone(&self.silence_ms);
        let err_fn = |err: cpal::StreamError| {
            tracing::error!("Ses akışı hatası: {err}");
        };

        let stream = match sample_format {
            SampleFormat::F32 => device.build_input_stream(
                &config,
                move |data: &[f32], _: &_| {
                    handle_input(data, channels, &buf, &resampler, needs_resample, &vad, &silence_ms);
                },
                err_fn,
                None,
            )?,
            SampleFormat::I16 => device.build_input_stream(
                &config,
                move |data: &[i16], _: &_| {
                    let f: Vec<f32> = data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                    handle_input(&f, channels, &buf, &resampler, needs_resample, &vad, &silence_ms);
                },
                err_fn,
                None,
            )?,
            SampleFormat::U16 => device.build_input_stream(
                &config,
                move |data: &[u16], _: &_| {
                    let f: Vec<f32> = data
                        .iter()
                        .map(|&s| (s as f32 - 32_768.0) / 32_768.0)
                        .collect();
                    handle_input(&f, channels, &buf, &resampler, needs_resample, &vad, &silence_ms);
                },
                err_fn,
                None,
            )?,
            other => return Err(anyhow!("Desteklenmeyen örnek formatı: {other:?}")),
        };

        stream.play().context("Akış başlatılamadı (mikrofon izni?)")?;
        *self.stream.lock() = Some(SendStream(stream));
        *active = true;
        drop(active); // kilidi serbest bırak

        // VAD aktifse watcher'ı başlat
        if vad_cfg.enabled {
            spawn_vad_watcher(
                app.clone(),
                Arc::clone(&self.silence_ms),
                Arc::clone(&self.watcher_cancel),
                vad_cfg.silence_ms,
            );
        }

        Ok(())
    }

    /// Kaydı durdurur. Watcher'ı sinyaller.
    pub fn stop(&self) {
        {
            let mut active = self.active.lock();
            *active = false;
        }
        self.watcher_cancel.store(true, Ordering::Relaxed);
        let stream = self.stream.lock().take();
        drop(stream);
    }

    /// Tampondaki tüm örnekleri döner ve tamponu temizler.
    pub fn drain(buf: &SharedBuffer) -> Vec<f32> {
        let mut b = buf.lock();
        let out: Vec<f32> = b.drain(..).collect();
        out
    }

    pub fn is_active(&self) -> bool {
        *self.active.lock()
    }
}

/// cpal callback'i: stereo→mono + resample + VAD + tampona yaz.
///
/// VAD açıkken her tam frame için konuşma tespiti yapılır. Konuşma algılanırsa
/// sessizlik sayacı sıfırlanır; sessizlikte artırılır. VAD kapalıysa (`vad`
/// Mutex'i None) sayaç dokunulmaz.
#[allow(clippy::too_many_arguments)]
fn handle_input(
    interleaved: &[f32],
    channels: usize,
    buf: &SharedBuffer,
    resampler: &Mutex<Option<ResamplerState>>,
    needs_resample: bool,
    vad: &Arc<Mutex<Option<VadEngine>>>,
    silence_ms: &AtomicU32,
) {
    let mono: Vec<f32> = if channels > 1 {
        interleaved
            .chunks_exact(channels)
            .map(|ch| ch.iter().sum::<f32>() / channels as f32)
            .collect()
    } else {
        interleaved.to_vec()
    };

    let final_samples = if needs_resample {
        let mut guard = resampler.lock();
        if let Some(rs) = guard.as_mut() {
            match rs.process(&mono) {
                Ok(out) => out,
                Err(e) => {
                    tracing::warn!("Resample hatası: {e}");
                    return;
                }
            }
        } else {
            mono
        }
    } else {
        mono
    };

    // VAD: resample sonrası 16kHz mono örnekleri değerlendir
    let mut guard = vad.lock();
    if let Some(engine) = guard.as_mut() {
        let r = engine.process(&final_samples);
        if r.any_speech {
            silence_ms.store(0, Ordering::Relaxed);
        } else {
            // Atomik olarak sessizlik süresini artır
            silence_ms.fetch_add(r.silence_ms(), Ordering::Relaxed);
        }
    }
    drop(guard);

    let mut b = buf.lock();
    for &s in &final_samples {
        if b.len() >= BUFFER_CAP {
            b.pop_front(); // en eski örneği at
        }
        b.push_back(s);
    }
}

/// Sessizlik eşiği aşılınca kaydı otomatik durduran periyodik görev.
///
/// `silence_ms` atomiğini her `VAD_WATCHER_TICK_MS` ms'de bir okur; değer
/// `threshold_ms`'i aşarsa `stop_recording(app)` çağırır ve sonlanır.
/// `watcher_cancel` true olursa (ör. manuel stop) anında sonlanır.
///
/// Ayrı bir OS thread'de çalışır (`std::thread`, bloklayıcı sleep) — böylece
/// `tokio` crate bağımlılığı eklemeye gerek kalmaz; sayaç atomik okunur,
/// `stop_recording` kendi async task'ini spawn eder.
fn spawn_vad_watcher(
    app: AppHandle,
    silence_ms: Arc<AtomicU32>,
    watcher_cancel: Arc<AtomicBool>,
    threshold_ms: u32,
) {
    std::thread::Builder::new()
        .name("vad-watcher".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_millis(VAD_WATCHER_TICK_MS));

            if watcher_cancel.load(Ordering::Relaxed) {
                return;
            }
            if silence_ms.load(Ordering::Relaxed) >= threshold_ms {
                tracing::info!(
                    "VAD: {threshold_ms}ms sessizlik eşiği aşıldı, kayıt durduruluyor"
                );
                watcher_cancel.store(true, Ordering::Relaxed);
                crate::stop_recording(&app);
                return;
            }
        })
        .expect("VAD watcher thread başlatılamadı");
}

/// rubato Sinc resampler durumunu sarar. Mono (tek kanal) çalışır.
struct ResamplerState {
    resampler: SincFixedIn<f32>,
    input_buffer: Vec<Vec<f32>>,
}

impl ResamplerState {
    fn new(in_rate: usize) -> Result<Self> {
        let params = SincInterpolationParameters {
            sinc_len: 256,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 256,
            window: WindowFunction::BlackmanHarris2,
        };
        let chunk = (in_rate * 30 / 1000).max(1024);
        let resampler = SincFixedIn::<f32>::new(
            TARGET_SAMPLE_RATE as f64 / in_rate as f64,
            2.0,
            params,
            chunk,
            1,
        )?;
        Ok(Self {
            resampler,
            input_buffer: vec![Vec::new()],
        })
    }

    fn process(&mut self, mono: &[f32]) -> Result<Vec<f32>> {
        self.input_buffer[0].extend_from_slice(mono);
        let chunk = self.resampler.input_frames_next();
        if self.input_buffer[0].len() < chunk {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        while self.input_buffer[0].len() >= chunk {
            let frame: Vec<f32> = self.input_buffer[0].drain(..chunk).collect();
            let input = vec![frame];
            let processed = self.resampler.process(&input, None)?;
            for ch in processed {
                out.extend(ch);
            }
        }
        Ok(out)
    }
}
