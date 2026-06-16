import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// --- Tipler (backend ile eşleşmeli) ---

type AppState =
  | "idle"
  | "recording"
  | "transcribing"
  | "injecting"
  | "error";

// --- Bileşen ---

export default function Widget() {
  const [state, setState] = useState<AppState>("idle");

  // Başlangıç durumunu çek
  useEffect(() => {
    (async () => {
      try {
        const [st] = await invoke<[AppState, string | null]>("cmd_get_state");
        setState(st);
      } catch (e) {
        console.error("Başlangıç durumu alınamadı:", e);
      }
    })();
  }, []);

  // State değişim olaylarını dinle (App.tsx ile aynı mekanizma)
  useEffect(() => {
    let un: UnlistenFn | undefined;
    (async () => {
      un = await listen<{ state: AppState; previous: AppState }>(
        "app://state-changed",
        (e) => setState(e.payload.state),
      );
    })();
    return () => {
      un?.();
    };
  }, []);

  const loading = state === "transcribing" || state === "injecting";

  return (
    // Pill gövde: sürüklenebilir, köşeleri saydam pencereyle yuvarlak.
    <main
      data-tauri-drag-region
      className="flex h-screen w-screen select-none items-center gap-2 overflow-hidden rounded-full bg-neutral-800/95 px-2 shadow-[0_4px_24px_rgba(0,0,0,0.5),0_0_20px_-4px_rgba(255,255,255,0.15)] ring-1 ring-white/25"
    >
      {/* Sol: logo butonu — drag-region YOK, tıklanabilir.
          Loading sırasında disabled (kayıt zaten bitti). */}
      <button
        onClick={() =>
          !loading && invoke("cmd_toggle_recording").catch(console.error)
        }
        disabled={loading}
        title={
          loading
            ? "İşleniyor…"
            : state === "recording"
              ? "Kaydı durdur"
              : "Kaydı başlat"
        }
        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-white shadow-sm transition active:scale-95 focus:outline-none disabled:opacity-60"
      >
        <span className="block h-2.5 w-2.5 rounded-full bg-neutral-900" />
      </button>

      {/* Sağ: durum göstergesi alanı — drag-region.
          - idle/error: "8voice" yazısı
          - recording: animasyonlu dalga (konuşuyor)
          - transcribing/injecting: spinner */}
      <div
        data-tauri-drag-region
        className="flex h-7 flex-1 items-center justify-center"
      >
        {loading ? (
          <Spinner />
        ) : state === "recording" ? (
          <WaveIndicator />
        ) : (
          <span className="whitespace-nowrap px-2 text-xs font-semibold tracking-tight text-white">
            8voice
          </span>
        )}
      </div>
    </main>
  );
}

/** Daire spinner — transcribe/inject sırasında döner. */
function Spinner() {
  return (
    <svg
      className="h-4 w-4 animate-spin text-neutral-300"
      viewBox="0 0 24 24"
      fill="none"
    >
      <circle
        className="opacity-20"
        cx="12"
        cy="12"
        r="10"
        stroke="currentColor"
        strokeWidth="3"
      />
      <path
        className="opacity-90"
        fill="currentColor"
        d="M12 2a10 10 0 0 1 10 10h-3a7 7 0 0 0-7-7V2z"
      />
    </svg>
  );
}

/** Dalga göstergesi — recording'de animasyonlu. */
function WaveIndicator() {
  const bars = [0, 1, 2, 3, 4];
  return (
    <div className="flex h-5 items-center justify-center gap-[2px]">
      {bars.map((i) => (
        <span
          key={i}
          className="w-[3px] rounded-full nokat-wave-sm bg-white"
          style={{ animationDelay: `${i * 0.12}s` }}
        />
      ))}
    </div>
  );
}
