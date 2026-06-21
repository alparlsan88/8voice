import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** Musical beep via Rust backend. */
function playBeep(kind: "start" | "stop") {
  invoke(kind === "start" ? "cmd_play_start_beep" : "cmd_play_stop_beep").catch(() => {});
}

// --- Types (must match backend) ---

type AppState =
  | "idle"
  | "recording"
  | "transcribing"
  | "injecting"
  | "error";

// --- Component ---

export default function Widget() {
  const [state, setState] = useState<AppState>("idle");
  const [amplitude, setAmplitude] = useState(0);

  // Load initial state
  useEffect(() => {
    (async () => {
      try {
        const [st] = await invoke<[AppState, string | null]>("cmd_get_state");
        setState(st);
      } catch (e) {
        console.error("Could not load initial state:", e);
      }
    })();
  }, []);

  // Listen for state change events (same mechanism as App.tsx)
  useEffect(() => {
    let un: UnlistenFn | undefined;
    (async () => {
      un = await listen<{ state: AppState; previous: AppState }>(
        "app://state-changed",
        (e) => {
          setState(e.payload.state);
          // Musical beep on recording start/stop
          if (e.payload.state === "recording") {
            playBeep("start");
          } else if (e.payload.previous === "recording") {
            playBeep("stop");
          }
          // Reset amplitude when recording stops so the wave goes flat.
          if (e.payload.state !== "recording") {
            setAmplitude(0);
          }
        },
      );
    })();
    return () => {
      un?.();
    };
  }, []);

  // Listen for real-time audio level from the capture pipeline
  useEffect(() => {
    let un: UnlistenFn | undefined;
    (async () => {
      un = await listen<number>("app://audio-level", (e) => {
        if (state === "recording") {
          setAmplitude(e.payload);
        }
      });
    })();
    return () => {
      un?.();
    };
  }, [state]);

  const loading = state === "transcribing" || state === "injecting";

  return (
    // Rectangular body: draggable, sharp corners blend with the transparent window.
    <main
      data-tauri-drag-region
      className="flex h-screen w-[90px] select-none items-center gap-1 overflow-hidden rounded-none bg-neutral-800/95 px-0.5 border border-white/20 shadow-[0_4px_24px_rgba(0,0,0,0.5),0_0_20px_-4px_rgba(255,255,255,0.15)]"
    >
      {/* Left: logo button — no drag-region, clickable.
          Disabled while loading (recording has already finished). */}
      <button
        onClick={() => {
          if (!loading) {
            invoke("cmd_toggle_recording").catch(console.error);
          }
        }}
        disabled={loading}
        title={
          loading
            ? "Processing…"
            : state === "recording"
              ? "Stop recording"
              : "Start recording"
        }
        className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-white shadow-sm transition active:scale-95 focus:outline-none disabled:opacity-60"
      >
        <img src="/logo.svg" alt="" className="h-full w-full" />
      </button>

      {/* Vertical divider */}
      <div className="h-4 w-px bg-white/25" />

      {/* Status: spinner / wave / text.
          Wave fills remaining space; text sits inline without extra gaps. */}
      {loading ? (
        <div data-tauri-drag-region className="flex flex-1 items-center justify-center">
          <Spinner />
        </div>
      ) : state === "recording" ? (
        <div data-tauri-drag-region className="flex flex-1 items-center justify-center">
          <WaveIndicator amplitude={amplitude} />
        </div>
      ) : (
        <div data-tauri-drag-region className="flex flex-1 items-center justify-center">
          <span className="whitespace-nowrap text-xs font-semibold tracking-tight text-white">
            8voice
          </span>
        </div>
      )}
    </main>
  );
}

/** Circular spinner — spins during transcribe/inject. */
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

/** Wave indicator — reacts to live audio amplitude.
 *  - amplitude = 0: flat/calm bars
 *  - amplitude > 0: bars scale with the input level, filling the full width
 */
function WaveIndicator({ amplitude }: { amplitude: number }) {
  const [time, setTime] = useState(0);
  const [smoothAmp, setSmoothAmp] = useState(0);

  useEffect(() => {
    let raf = 0;
    const tick = () => {
      setTime((t) => t + 1);
      setSmoothAmp((prev) => prev + (amplitude - prev) * 0.5);
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [amplitude]);

  // 15 thin bars that span the full available width, pulsing from the
  // vertical center like a heartbeat rather than growing from the baseline.
  const barCount = 15;
  const bars = Array.from({ length: barCount }, (_, i) => i);
  const center = (barCount - 1) / 2;
  const phases = bars.map((i) => Math.abs(i - center) * 0.25);
  const factors = bars.map((i) => 1.0 - Math.abs(i - center) / center * 0.2);

  return (
    <div className="flex h-6 w-full items-center justify-center gap-[1px] overflow-hidden px-0.5">
      {bars.map((i) => {
        const pulse = 0.5 + 0.5 * Math.sin(time * 0.18 + phases[i]);
        // Baseline scale is 1 (centered 4px bar). At full amplitude the bar
        // scales vertically so it touches the top and bottom edges of the
        // 24px container (scale ~6x), expanding symmetrically up and down.
        const scale = 1 + smoothAmp * pulse * factors[i] * 5.5;
        return (
          <span
            key={i}
            className="flex-1 origin-center rounded-full bg-white"
            style={{ height: "4px", transform: `scaleY(${scale})` }}
          />
        );
      })}
    </div>
  );
}
