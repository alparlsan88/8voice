import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

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
    // Pill body: draggable, rounded corners blend with the transparent window.
    <main
      data-tauri-drag-region
      className="flex h-screen w-screen select-none items-center gap-2 overflow-hidden rounded-full bg-neutral-800/95 px-2 shadow-[0_4px_24px_rgba(0,0,0,0.5),0_0_20px_-4px_rgba(255,255,255,0.15)] ring-1 ring-white/25"
    >
      {/* Left: logo button — no drag-region, clickable.
          Disabled while loading (recording has already finished). */}
      <button
        onClick={() =>
          !loading && invoke("cmd_toggle_recording").catch(console.error)
        }
        disabled={loading}
        title={
          loading
            ? "Processing…"
            : state === "recording"
              ? "Stop recording"
              : "Start recording"
        }
        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-white shadow-sm transition active:scale-95 focus:outline-none disabled:opacity-60"
      >
        <span className="block h-2.5 w-2.5 rounded-full bg-neutral-900" />
      </button>

      {/* Right: status indicator area — drag-region.
          - idle/error: "8voice" text
          - recording: animated wave (speaking)
          - transcribing/injecting: spinner */}
      <div
        data-tauri-drag-region
        className="flex h-7 flex-1 items-center justify-center"
      >
        {loading ? (
          <Spinner />
        ) : state === "recording" ? (
          <WaveIndicator amplitude={amplitude} />
        ) : (
          <span className="whitespace-nowrap px-2 text-xs font-semibold tracking-tight text-white">
            8voice
          </span>
        )}
      </div>
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
      setSmoothAmp((prev) => prev + (amplitude - prev) * 0.35);
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [amplitude]);

  // 9 bars that span the full available width.
  const bars = [0, 1, 2, 3, 4, 5, 6, 7, 8];
  const phases = [0, 0.7, 1.4, 2.1, 2.8, 2.1, 1.4, 0.7, 0];
  const factors = [0.55, 0.7, 0.82, 0.92, 1.0, 0.92, 0.82, 0.7, 0.55];
  const base = 0.15; // calm/idle bar height fraction

  return (
    <div className="flex h-5 w-full items-end justify-center gap-[1px] px-0.5">
      {bars.map((i) => {
        const wave = 0.5 + 0.5 * Math.sin(time * 0.15 + phases[i]);
        const height = base + smoothAmp * (1 - base) * wave * factors[i];
        return (
          <span
            key={i}
            className="flex-1 rounded-full bg-white"
            style={{ height: `${Math.max(base, height) * 100}%` }}
          />
        );
      })}
    </div>
  );
}
