//! Audio capture — cpal microphone + rubato resample (16 kHz mono) + buffer.
//!
//! Contract:
//! - Input: device name (optional), start()/stop()
//! - Output: `Vec<f32>` — 16 kHz, mono, normalized [-1.0, 1.0] PCM
//! - Accept: different sample rates are resampled to 16 kHz; stereo→mono;
//!   clear error on permission denial
//!
//! VAD: when enabled each PCM chunk is analyzed for speech; continuous silence
//! is accumulated in an `AtomicU32`. A watcher task reads this counter
//! periodically and stops recording when the threshold is exceeded. When VAD is
//! disabled behavior is identical to the non-VAD case (manual stop only).
//!
//! Note: instead of a ring buffer we use `Arc<Mutex<VecDeque<f32>>>` (simple,
//! reliable). A lock-free ring buffer can be introduced later if needed.

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use parking_lot::Mutex;
use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

use crate::settings::VadCfg;
use crate::vad::VadEngine;

/// Sample rate expected by Whisper.
pub const TARGET_SAMPLE_RATE: usize = 16_000;
/// Buffer upper limit (~2 minutes of 16 kHz mono).
const BUFFER_CAP: usize = TARGET_SAMPLE_RATE * 120;
/// VAD watcher poll period (ms).
const VAD_WATCHER_TICK_MS: u64 = 50;

/// Shared audio buffer.
pub type SharedBuffer = Arc<Mutex<VecDeque<f32>>>;

/// `cpal::Stream` is `!Send` (it carries a `*mut ()` at the platform layer).
/// We never move the stream between threads — we only access it from a single
/// thread during start/stop. This newtype makes it `Send` so that `AppCtx`
/// satisfies Tauri's `Send + Sync` requirement.
struct SendStream(Stream);
unsafe impl Send for SendStream {}

/// Microphone capture.
pub struct AudioCapture {
    buf: SharedBuffer,
    stream: Mutex<Option<SendStream>>,
    active: Mutex<bool>,
    /// VAD engine; `None` = VAD disabled (set at start).
    /// `Arc` so it can be cloned into the audio callback.
    vad: Arc<Mutex<Option<VadEngine>>>,
    /// Accumulated continuous silence (ms). Written by VAD, read by watcher.
    /// Reset when speech is detected.
    silence_ms: Arc<AtomicU32>,
    /// Latest input RMS level in `[0,1]` (stored as f32 bits). Updated by the
    /// audio callback and read by the amplitude publisher thread.
    amplitude: Arc<AtomicU32>,
    /// Signal to stop the watcher. `stop()` sets this to true.
    watcher_cancel: Arc<AtomicBool>,
}

impl AudioCapture {
    /// New capture instance + shared buffer. Does not start recording yet.
    pub fn new() -> (Self, SharedBuffer) {
        let buf = Arc::new(Mutex::new(VecDeque::with_capacity(BUFFER_CAP)));
        let silence_ms = Arc::new(AtomicU32::new(0));
        let amplitude = Arc::new(AtomicU32::new(0));
        let watcher_cancel = Arc::new(AtomicBool::new(false));
        (
            Self {
                buf: Arc::clone(&buf),
                stream: Mutex::new(None),
                active: Mutex::new(false),
                vad: Arc::new(Mutex::new(None)),
                silence_ms,
                amplitude,
                watcher_cancel,
            },
            buf,
        )
    }

    /// Lists available input devices by name.
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

    /// Starts recording. `device_name` `None` selects the default device.
    /// If `vad_cfg.enabled`, the VAD engine is started and a watcher task is
    /// spawned for auto-stop on silence.
    pub fn start(
        &self,
        app: &AppHandle,
        device_name: Option<&str>,
        vad_cfg: VadCfg,
    ) -> Result<()> {
        let mut active = self.active.lock();
        if *active {
            return Err(anyhow!("Recording is already active"));
        }

        // --- VAD setup ---
        self.silence_ms.store(0, Ordering::Relaxed);
        self.watcher_cancel.store(false, Ordering::Relaxed);
        let engine = if vad_cfg.enabled {
            match VadEngine::new(vad_cfg.aggressiveness) {
                Ok(e) => Some(e),
                Err(e) => {
                    tracing::warn!("Could not start VAD, continuing without it: {e:#}");
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
                .ok_or_else(|| anyhow!("Device not found: {name}"))?,
            None => host
                .default_input_device()
                .ok_or_else(|| anyhow!("No default input device"))?,
        };

        let supported = device
            .default_input_config()
            .context("Could not get device configuration")?;
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
        let amplitude = Arc::clone(&self.amplitude);
        let err_fn = |err: cpal::StreamError| {
            tracing::error!("Audio stream error: {err}");
        };

        let stream = match sample_format {
            SampleFormat::F32 => device.build_input_stream(
                &config,
                move |data: &[f32], _: &_| {
                    handle_input(data, channels, &buf, &resampler, needs_resample, &vad, &silence_ms, &amplitude);
                },
                err_fn,
                None,
            )?,
            SampleFormat::I16 => device.build_input_stream(
                &config,
                move |data: &[i16], _: &_| {
                    let f: Vec<f32> = data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                    handle_input(&f, channels, &buf, &resampler, needs_resample, &vad, &silence_ms, &amplitude);
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
                    handle_input(&f, channels, &buf, &resampler, needs_resample, &vad, &silence_ms, &amplitude);
                },
                err_fn,
                None,
            )?,
            other => return Err(anyhow!("Unsupported sample format: {other:?}")),
        };

        stream.play().context("Could not start stream (microphone permission?)")?;
        *self.stream.lock() = Some(SendStream(stream));
        *active = true;
        drop(active); // release lock

        // Start VAD watcher if enabled
        if vad_cfg.enabled {
            spawn_vad_watcher(
                app.clone(),
                Arc::clone(&self.silence_ms),
                Arc::clone(&self.watcher_cancel),
                vad_cfg.silence_ms,
            );
        }

        // Publish real-time audio level to the widget
        spawn_amplitude_publisher(
            app.clone(),
            Arc::clone(&self.amplitude),
            Arc::clone(&self.watcher_cancel),
        );

        Ok(())
    }

    /// Stops recording. Signals the watcher.
    pub fn stop(&self) {
        {
            let mut active = self.active.lock();
            *active = false;
        }
        self.watcher_cancel.store(true, Ordering::Relaxed);
        self.amplitude.store(0, Ordering::Relaxed);
        let stream = self.stream.lock().take();
        drop(stream);
    }

    /// Returns all samples from the buffer and clears it.
    pub fn drain(buf: &SharedBuffer) -> Vec<f32> {
        let mut b = buf.lock();
        let out: Vec<f32> = b.drain(..).collect();
        out
    }

    pub fn is_active(&self) -> bool {
        *self.active.lock()
    }
}

/// cpal callback: stereo→mono + resample + VAD + write to buffer.
///
/// When VAD is on, speech detection runs for every full frame. If speech is
/// detected the silence counter is reset; otherwise it is incremented. When VAD
/// is off (`vad` mutex is None) the counter is left untouched.
#[allow(clippy::too_many_arguments)]
fn handle_input(
    interleaved: &[f32],
    channels: usize,
    buf: &SharedBuffer,
    resampler: &Mutex<Option<ResamplerState>>,
    needs_resample: bool,
    vad: &Arc<Mutex<Option<VadEngine>>>,
    silence_ms: &AtomicU32,
    amplitude: &AtomicU32,
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
                    tracing::warn!("Resample error: {e}");
                    return;
                }
            }
        } else {
            mono
        }
    } else {
        mono
    };

    // VAD: evaluate 16 kHz mono samples after resampling
    let mut guard = vad.lock();
    if let Some(engine) = guard.as_mut() {
        let r = engine.process(&final_samples);
        if r.any_speech {
            silence_ms.store(0, Ordering::Relaxed);
        } else {
            // Atomically increase silence duration
            silence_ms.fetch_add(r.silence_ms(), Ordering::Relaxed);
        }
    }
    drop(guard);

    // Compute input level for the widget wave visualization.
    let rms = if final_samples.is_empty() {
        0.0
    } else {
        let sum_sq: f32 = final_samples.iter().map(|&s| s * s).sum();
        (sum_sq / final_samples.len() as f32).sqrt()
    };
    // Log-scale normalization: human hearing / VU meters are logarithmic.
    // Map RMS in dBFS so quiet speech is visible and loud speech caps at 1.0.
    // -60 dBFS -> 0, -45 dBFS -> 0.25, -30 dBFS -> 0.5, -15 dBFS -> 1.0.
    let level = if rms < 1e-6 {
        0.0
    } else {
        let db = 20.0 * rms.max(1e-6).log10();
        ((db + 60.0) / 45.0).clamp(0.0, 1.0)
    };
    amplitude.store(level.to_bits(), Ordering::Relaxed);

    let mut b = buf.lock();
    for &s in &final_samples {
        if b.len() >= BUFFER_CAP {
            b.pop_front(); // drop oldest sample
        }
        b.push_back(s);
    }
}

/// Periodic task that auto-stops recording when silence exceeds the threshold.
///
/// Reads the `silence_ms` atomic every `VAD_WATCHER_TICK_MS` ms; if it reaches
/// `threshold_ms` it calls `stop_recording(app)` and exits.
/// Exits immediately if `watcher_cancel` becomes true (e.g. manual stop).
///
/// Runs on a dedicated OS thread (`std::thread`, blocking sleep) so we don't
/// need to add a `tokio` dependency; the atomic counter is read directly and
/// `stop_recording` spawns its own async task.
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
                    "VAD: silence threshold {threshold_ms}ms exceeded, stopping recording"
                );
                watcher_cancel.store(true, Ordering::Relaxed);
                crate::stop_recording(&app);
                return;
            }
        })
        .expect("Could not start VAD watcher thread");
}

/// Periodic task that publishes the latest input RMS level to the widget.
///
/// Reads `amplitude` every 50 ms and emits `app://audio-level` until
/// `watcher_cancel` becomes true. The event payload is an f32 in `[0,1]`.
fn spawn_amplitude_publisher(
    app: AppHandle,
    amplitude: Arc<AtomicU32>,
    watcher_cancel: Arc<AtomicBool>,
) {
    std::thread::Builder::new()
        .name("audio-level".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_millis(50));

            if watcher_cancel.load(Ordering::Relaxed) {
                // Notify the widget that the level is back to zero on stop.
                let _ = app.emit("app://audio-level", 0.0_f32);
                return;
            }

            let level = f32::from_bits(amplitude.load(Ordering::Relaxed)).clamp(0.0, 1.0);
            let _ = app.emit("app://audio-level", level);
        })
        .expect("Could not start audio level thread");
}

/// Wraps the rubato Sinc resampler state. Works in mono (single channel).
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
