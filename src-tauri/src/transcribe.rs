//! Transkripsiyon — whisper-rs (whisper.cpp) FFI sarmalayıcı + Groq Whisper API.
//!
//! Kontrat:
//! - Input: `&[f32]` 16 kHz mono PCM, dil kodu, model yolu / API key
//! - Output: `String` transkript
//! - Kabul: model bir kez yüklenir/bellekte tutulur; yüklenemezse net hata;
//!   10 sn ses < 3 sn (modern CPU, CPU modu); Groq modunda internet gerekir.

use anyhow::{anyhow, Context, Result};
use parking_lot::Mutex;
use serde::Deserialize;
use std::path::Path;
use std::sync::OnceLock;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// Global whisper context — bir kez yüklenir, bellekte kalır.
static CONTEXT: OnceLock<Mutex<Option<WhisperContext>>> = OnceLock::new();

/// Transcriber. Model yolu ilk çağrıda load() ile set edilir.
pub struct Transcriber;

impl Transcriber {
    /// (Yeniden) model yükler. Uygulama açılışında veya model yolu değişince çağrılır.
    pub fn load(model_path: &Path) -> Result<()> {
        if !model_path.exists() {
            return Err(anyhow!(
                "Model dosyası bulunamadı: {}. ggml-small.bin gibi bir GGUF modeli indirin.",
                model_path.display()
            ));
        }
        tracing::info!("Whisper modeli yükleniyor: {}", model_path.display());
        let t0 = std::time::Instant::now();
        let ctx = WhisperContext::new_with_params(
            model_path.to_str().context("Model yolu UTF-8 değil")?,
            WhisperContextParameters::default(),
        )
        .map_err(|e| anyhow!("Whisper context yüklenemedi: {e}"))?;

        let mutex = CONTEXT.get_or_init(|| Mutex::new(None));
        let mut guard = mutex.lock();
        *guard = Some(ctx);
        tracing::info!(
            "Model yüklendi ({:.2}s)",
            t0.elapsed().as_secs_f64()
        );
        Ok(())
    }

    /// PCM → metin. `lang` "auto" ise otomatik dil tespiti.
    pub fn transcribe(pcm: &[f32], lang: &str) -> Result<String> {
        let mutex = CONTEXT
            .get()
            .ok_or_else(|| anyhow!("Model yüklenmemiş; önce load() çağrılmalı"))?;
        let guard = mutex.lock();
        let ctx = guard
            .as_ref()
            .ok_or_else(|| anyhow!("Model yüklenmemiş; önce load() çağrılmalı"))?;

        // En az 16 örnek (1 sn) gerekli; kısa kayıtlarda padding
        let pcm_owned: Vec<f32> = if pcm.len() < 16 {
            return Ok(String::new());
        } else {
            pcm.to_vec()
        };

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(num_cpu_threads());
        if lang == "auto" {
            params.set_language(None);
        } else {
            params.set_language(Some(lang));
        }
        // Sadece metin üretimi (cookie/translate kapalı)
        params.set_translate(false);
        params.set_print_progress(false);
        params.set_print_special(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        let mut state = ctx
            .create_state()
            .map_err(|e| anyhow!("Whisper state oluşturulamadı: {e}"))?;
        state
            .full(params, &pcm_owned)
            .map_err(|e| anyhow!("Whisper işleme hatası: {e}"))?;

        // whisper-rs 0.16 API: full_n_segments doğrudan c_int döner,
        // segment verisi get_segment(i)? ile alınır.
        //
        // Not: whisper.cpp bazen segment metnini geçersiz UTF-8 byte'larıyla
        // döndürebilir (otomatik dil tespiti, yarım çok baytlı karakterler).
        // to_str() katı doğrulama yapıp hata fırlatır; bunun yerine
        // to_string_lossy() ile bozuk baytlar U+FFFD'ye düşürülür ve metin korunur.
        let n = state.full_n_segments();
        let mut text = String::new();
        for i in 0..n {
            let seg = state
                .get_segment(i)
                .ok_or_else(|| anyhow!("Segment {i} bulunamadı"))?;
            // to_str_lossy(): geçersiz UTF-8 byte'larını U+FFFD'ye düşürür,
            // böylece yarım çok baytlı karakterler hata fırlatmak yerine atlanır.
            let seg_text = seg
                .to_str_lossy()
                .map_err(|e| anyhow!("Segment {i} metni alınamadı: {e}"))?;
            text.push_str(&seg_text);
        }
        Ok(text.trim().to_string())
    }
}

fn num_cpu_threads() -> std::os::raw::c_int {
    let n = std::thread::available_parallelism()
        .map(|v| v.get())
        .unwrap_or(4);
    n.min(8) as std::os::raw::c_int
}

// ---------------------------------------------------------------------------
// Groq Whisper API
// ---------------------------------------------------------------------------

const GROQ_API_URL: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
const GROQ_MODEL: &str = "whisper-large-v3";
const GROQ_SAMPLE_RATE: u32 = 16_000;

/// Groq API JSON yanıtı.
#[derive(Debug, Deserialize)]
struct GroqTranscriptionResponse {
    text: String,
}

/// PCM → Groq Whisper API → metin (async).
/// `api_key` Groq hesabından alınan secret key'dir.
pub async fn transcribe_groq(pcm: &[f32], lang: &str, api_key: &str) -> Result<String> {
    if pcm.len() < 16 {
        return Ok(String::new());
    }

    let wav = pcm_f32_to_wav(pcm, GROQ_SAMPLE_RATE);
    transcribe_groq_async(wav, lang, api_key).await
}

async fn transcribe_groq_async(wav: Vec<u8>, lang: &str, api_key: &str) -> Result<String> {
    let client = reqwest::Client::new();
    let mut form = reqwest::multipart::Form::new()
        .text("model", GROQ_MODEL.to_string())
        .part(
            "file",
            reqwest::multipart::Part::bytes(wav)
                .file_name("recording.wav")
                .mime_str("audio/wav")
                .context("WAV part oluşturulamadı")?,
        );

    if lang != "auto" {
        form = form.text("language", lang.to_string());
    }

    let resp = client
        .post(GROQ_API_URL)
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .await
        .context("Groq API isteği gönderilemedi")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("Groq API hatası {status}: {body}"));
    }

    let data: GroqTranscriptionResponse = resp
        .json()
        .await
        .context("Groq yanıtı JSON olarak ayrıştırılamadı")?;

    Ok(data.text.trim().to_string())
}

/// f32 16 kHz mono PCM'i standart WAV (PCM 16-bit) byte dizisine çevirir.
fn pcm_f32_to_wav(pcm: &[f32], sample_rate: u32) -> Vec<u8> {
    let channels: u16 = 1;
    let bits_per_sample: u16 = 16;
    let bytes_per_sample = bits_per_sample / 8;
    let block_align = channels * bytes_per_sample;
    let byte_rate = sample_rate * block_align as u32;
    let data_len = (pcm.len() * bytes_per_sample as usize) as u32;
    let total_len = 36 + data_len;

    let mut out = Vec::with_capacity(44 + data_len as usize);

    // RIFF chunk
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&total_len.to_le_bytes());
    out.extend_from_slice(b"WAVE");

    // fmt chunk
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());

    // data chunk
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());

    for &sample in pcm {
        let s = (sample * i16::MAX as f32).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        out.extend_from_slice(&s.to_le_bytes());
    }

    out
}
