import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

// --- Tipler (backend ile eşleşmeli) ---

type AppState =
  | "idle"
  | "recording"
  | "transcribing"
  | "injecting"
  | "error";

interface Settings {
  input_device: string | null;
  model_path: string;
  language: string;
  hotkey: string;
  hotkey_mode: "push_to_talk" | "toggle";
  injection_mode: "auto" | "typing" | "paste";
  vad_enabled: boolean;
  vad_silence_ms: number;
  vad_aggressiveness: number;
  api_provider: "offline" | "groq";
  api_key: string | null;
}

const DEFAULT_SETTINGS: Settings = {
  input_device: null,
  model_path: "models/ggml-small.bin",
  language: "auto",
  hotkey: "Ctrl+Shift+Space",
  hotkey_mode: "push_to_talk",
  injection_mode: "auto",
  vad_enabled: true,
  vad_silence_ms: 1200,
  vad_aggressiveness: 2,
  api_provider: "offline",
  api_key: null,
};

const STATE_META: Record<
  AppState,
  {
    label: string;
    description: string;
    bars: string;
    ring: string;
    glow: string;
    animate: boolean;
  }
> = {
  idle: {
    label: "Hazır",
    description: "Kayda başlamak için widget mikrofonu veya kısayol kullan.",
    bars: "bg-emerald-500",
    ring: "ring-emerald-500/20",
    glow: "shadow-emerald-500/10",
    animate: false,
  },
  recording: {
    label: "Kayıt yapılıyor",
    description: "Konuşmanız dinleniyor…",
    bars: "bg-red-500",
    ring: "ring-red-500/20",
    glow: "shadow-red-500/10",
    animate: true,
  },
  transcribing: {
    label: "Metne çevriliyor",
    description: "Ses kaydı yazıya dönüştürülüyor…",
    bars: "bg-amber-500",
    ring: "ring-amber-500/20",
    glow: "shadow-amber-500/10",
    animate: true,
  },
  injecting: {
    label: "Yazılıyor",
    description: "Metin aktif pencereye aktarılıyor…",
    bars: "bg-cyan-500",
    ring: "ring-cyan-500/20",
    glow: "shadow-cyan-500/10",
    animate: true,
  },
  error: {
    label: "Hata",
    description: "Bir sorun oluştu.",
    bars: "bg-rose-500",
    ring: "ring-rose-500/20",
    glow: "shadow-rose-500/10",
    animate: false,
  },
};

// --- Bileşen ---

function App() {
  const [state, setState] = useState<AppState>("idle");
  const [error, setError] = useState<string | null>(null);
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
  const [devices, setDevices] = useState<string[]>([]);
  const [saving, setSaving] = useState(false);
  const [lastTranscript, setLastTranscript] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [capturingHotkey, setCapturingHotkey] = useState(false);

  useEffect(() => {
    (async () => {
      try {
        const [s, st, devs] = await Promise.all([
          invoke<Settings>("cmd_get_settings"),
          invoke<[AppState, string | null]>("cmd_get_state"),
          invoke<string[]>("cmd_list_devices"),
        ]);
        setSettings(s);
        setState(st[0]);
        setError(st[1]);
        setDevices(devs);
      } catch (e) {
        console.error("Başlangıç verisi alınamadı:", e);
      }
    })();
  }, []);

  useEffect(() => {
    let un: UnlistenFn | undefined;
    let unT: UnlistenFn | undefined;
    (async () => {
      un = await listen<{ state: AppState; previous: AppState }>(
        "app://state-changed",
        (e) => setState(e.payload.state),
      );
      unT = await listen<string>("app://transcript", (e) => {
        setLastTranscript(e.payload);
        copyToClipboard(e.payload);
      });
    })();
    return () => {
      un?.();
      unT?.();
    };
  }, []);

  const copyToClipboard = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch (e) {
      console.error("Panoya kopyalanamadı:", e);
    }
  };

  const update = (patch: Partial<Settings>) => {
    setSettings((prev) => {
      const next = { ...prev, ...patch };
      setSaving(true);
      invoke("cmd_save_settings", { settings: next })
        .catch((e) => {
          console.error("Ayar kaydedilemedi:", e);
          setError(String(e));
        })
        .finally(() => setSaving(false));
      return next;
    });
  };

  const meta = STATE_META[state];

  return (
    <main className="flex min-h-screen justify-center bg-neutral-950 px-4 py-6 text-neutral-100">
      <div className="w-full max-w-md">
        {/* Header */}
        <header className="mb-6 flex items-center gap-4">
          <div className="flex h-14 w-14 shrink-0 items-center justify-center rounded-2xl bg-white shadow-lg ring-1 ring-white/20">
            <span className="block h-5 w-5 rounded-full bg-neutral-900" />
          </div>
          <div>
            <h1 className="text-xl font-bold tracking-tight">8voice</h1>
            <p className="text-sm text-neutral-400">Yerel model veya Groq API ile sesli dikte</p>
          </div>
        </header>

        {/* Durum kartı */}
        <section
          className={`mb-5 flex items-center gap-4 rounded-2xl bg-neutral-900 p-5 shadow-lg ring-1 ring-white/5 ${meta.glow}`}
        >
          <div
            className={`flex h-12 w-12 shrink-0 items-center justify-center rounded-xl bg-neutral-800/80 ring-1 ${meta.ring}`}
          >
            <WaveIndicator color={meta.bars} animate={meta.animate} />
          </div>
          <div className="flex-1 min-w-0">
            <p className="text-base font-semibold">{meta.label}</p>
            <p className="text-sm text-neutral-400 truncate">
              {error ?? meta.description}
            </p>
          </div>
          <span className="rounded-lg bg-neutral-800 px-2 py-1 text-xs font-medium text-neutral-400 ring-1 ring-white/5">
            {settings.hotkey_mode === "push_to_talk" ? "PTT" : "TGL"}
          </span>
        </section>

        {/* Son transkript */}
        {lastTranscript && (
          <section className="mb-5 rounded-2xl border border-neutral-800/60 bg-neutral-900/60 p-4 backdrop-blur-sm">
            <div className="mb-2 flex items-center justify-between">
              <p className="text-xs font-semibold uppercase tracking-wider text-neutral-500">
                Son transkript
              </p>
              <button
                type="button"
                onClick={() => copyToClipboard(lastTranscript)}
                className="flex items-center gap-1 rounded-md bg-neutral-800 px-2 py-1 text-xs font-medium text-neutral-300 transition hover:bg-neutral-700 hover:text-white"
              >
                {copied ? (
                  <>
                    <CheckIcon className="h-3 w-3" />
                    Kopyalandı
                  </>
                ) : (
                  <>
                    <CopyIcon className="h-3 w-3" />
                    Kopyala
                  </>
                )}
              </button>
            </div>
            <p className="whitespace-pre-wrap break-words text-sm leading-relaxed text-neutral-200">
              {lastTranscript}
            </p>
          </section>
        )}

        {/* Ayarlar */}
        <section className="mb-5 rounded-2xl bg-neutral-900 p-5 shadow-lg ring-1 ring-white/5">
          <div className="mb-5 flex items-center gap-2">
            <SettingsIcon className="h-4 w-4 text-neutral-400" />
            <h2 className="text-sm font-semibold uppercase tracking-wider text-neutral-400">
              Ayarlar
            </h2>
          </div>

          <div className="flex flex-col gap-5">
            <Field label="Mikrofon" icon={<MicIcon className="h-3.5 w-3.5" />}>
              <select
                className="nokat-input"
                value={settings.input_device ?? ""}
                onChange={(e) =>
                  update({
                    input_device: e.target.value || null,
                  })
                }
              >
                <option value="">Sistem varsayılanı</option>
                {devices.map((d) => (
                  <option key={d} value={d}>
                    {d}
                  </option>
                ))}
              </select>
            </Field>

            <Field
              label="Transkripsiyon sağlayıcısı"
              icon={<CloudIcon className="h-3.5 w-3.5" />}
            >
              <select
                className="nokat-input"
                value={settings.api_provider}
                onChange={(e) =>
                  update({
                    api_provider: e.target.value as Settings["api_provider"],
                  })
                }
              >
                <option value="offline">Yerel model (offline)</option>
                <option value="groq">Groq Whisper API</option>
              </select>
            </Field>

            {settings.api_provider === "offline" ? (
              <Field label="Model yolu" icon={<FileIcon className="h-3.5 w-3.5" />}>
                <div className="flex gap-2">
                  <input
                    className="nokat-input font-mono text-xs"
                    value={settings.model_path}
                    onChange={(e) => update({ model_path: e.target.value })}
                    placeholder="models/ggml-small.bin"
                  />
                  <button
                    type="button"
                    onClick={async () => {
                      try {
                        const selected = await open({
                          multiple: false,
                          directory: false,
                          filters: [
                            { name: "GGML/Whisper model", extensions: ["bin"] },
                            { name: "Tüm dosyalar", extensions: ["*"] },
                          ],
                          title: "Whisper model dosyası seç",
                        });
                        if (selected && typeof selected === "string") {
                          update({ model_path: selected });
                        }
                      } catch (e) {
                        console.error("Model seçilemedi:", e);
                      }
                    }}
                    className="shrink-0 rounded-lg bg-neutral-800 px-3 text-xs font-semibold text-neutral-200 transition hover:bg-neutral-700 hover:text-white"
                  >
                    Gözat
                  </button>
                </div>
              </Field>
            ) : (
              <Field label="Groq API anahtarı" icon={<KeyIcon className="h-3.5 w-3.5" />}>
                <input
                  type="password"
                  className="nokat-input font-mono text-xs"
                  value={settings.api_key ?? ""}
                  onChange={(e) =>
                    update({ api_key: e.target.value || null })
                  }
                  placeholder="gsk_..."
                />
              </Field>
            )}

            <Field label="Dil" icon={<GlobeIcon className="h-3.5 w-3.5" />}>
              <select
                className="nokat-input"
                value={settings.language}
                onChange={(e) => update({ language: e.target.value })}
              >
                <option value="auto">Otomatik</option>
                <option value="tr">Türkçe</option>
                <option value="en">English</option>
              </select>
            </Field>

            <div className="grid grid-cols-2 gap-4">
              <Field label="Kısayol modu">
                <select
                  className="nokat-input"
                  value={settings.hotkey_mode}
                  onChange={(e) =>
                    update({
                      hotkey_mode: e.target.value as Settings["hotkey_mode"],
                    })
                  }
                >
                  <option value="push_to_talk">Basılı tut</option>
                  <option value="toggle">Aç/kapa</option>
                </select>
              </Field>

              <Field label="Kısayol">
                <HotkeyCapture
                  value={settings.hotkey}
                  capturing={capturingHotkey}
                  onStart={() => setCapturingHotkey(true)}
                  onCapture={(hotkey) => {
                    setCapturingHotkey(false);
                    update({ hotkey });
                  }}
                  onCancel={() => setCapturingHotkey(false)}
                />
              </Field>
            </div>

            <Field label="Enjeksiyon modu" icon={<TypeIcon className="h-3.5 w-3.5" />}>
              <select
                className="nokat-input"
                value={settings.injection_mode}
                onChange={(e) =>
                  update({
                    injection_mode: e.target
                      .value as Settings["injection_mode"],
                  })
                }
              >
                <option value="auto">Otomatik (uzun metinde yapıştır)</option>
                <option value="typing">Her zaman yazma</option>
                <option value="paste">Her zaman yapıştır</option>
              </select>
            </Field>
          </div>

          <p className="mt-5 text-xs text-neutral-500">
            {saving ? "Kaydediliyor…" : "Değişiklikler otomatik kaydedilir."}
          </p>
        </section>

        {/* Otomatik durdurma (VAD) */}
        <section className="rounded-2xl bg-neutral-900 p-5 shadow-lg ring-1 ring-white/5">
          <div className="mb-5 flex items-center justify-between">
            <div className="flex items-center gap-2">
              <StopwatchIcon className="h-4 w-4 text-neutral-400" />
              <h2 className="text-sm font-semibold uppercase tracking-wider text-neutral-400">
                Otomatik durdurma
              </h2>
            </div>
            <label className="relative inline-flex cursor-pointer items-center">
              <input
                type="checkbox"
                className="peer sr-only"
                checked={settings.vad_enabled}
                onChange={(e) => update({ vad_enabled: e.target.checked })}
              />
              <div className="h-6 w-11 rounded-full bg-neutral-700 transition peer-checked:bg-emerald-500 peer-focus:ring-2 peer-focus:ring-emerald-500/30 after:absolute after:left-0.5 after:top-0.5 after:h-5 after:w-5 after:rounded-full after:bg-white after:transition-all peer-checked:after:translate-x-5" />
            </label>
          </div>

          {settings.vad_enabled ? (
            <div className="flex flex-col gap-5">
              <p className="text-sm text-neutral-400">
                Konuşmayı kestikten sonra{" "}
                <span className="font-semibold text-neutral-200">
                  {settings.vad_silence_ms} ms
                </span>{" "}
                sessizlikte kayıt otomatik durur.
              </p>
              <Field label={`Sessizlik eşiği: ${settings.vad_silence_ms} ms`}>
                <input
                  type="range"
                  min={400}
                  max={3000}
                  step={100}
                  className="w-full accent-emerald-500"
                  value={settings.vad_silence_ms}
                  onChange={(e) =>
                    update({ vad_silence_ms: Number(e.target.value) })
                  }
                />
              </Field>
              <Field label="Agresiflik">
                <select
                  className="nokat-input"
                  value={settings.vad_aggressiveness}
                  onChange={(e) =>
                    update({ vad_aggressiveness: Number(e.target.value) })
                  }
                >
                  <option value={1}>1 — Az (konuşmayı kaçırma)</option>
                  <option value={2}>2 — Dengeli (önerilen)</option>
                  <option value={3}>3 — Çok (gürültüde temiz dur)</option>
                </select>
              </Field>
            </div>
          ) : (
            <p className="text-sm text-neutral-400">
              Kapalı — yalnızca kısayol ile manuel durdurma.
            </p>
          )}
        </section>

        <footer className="mt-6 text-center text-xs text-neutral-600">
          8voice · {settings.hotkey}
        </footer>
      </div>
    </main>
  );
}

function Field({
  label,
  icon,
  children,
}: {
  label: string;
  icon?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <label className="flex flex-col gap-1.5">
      <span className="flex items-center gap-1.5 text-xs font-medium text-neutral-400">
        {icon && <span className="text-neutral-500">{icon}</span>}
        {label}
      </span>
      {children}
    </label>
  );
}

/** Tuş kombinasyonu yakalayan input. */
function HotkeyCapture({
  value,
  capturing,
  onStart,
  onCapture,
  onCancel,
}: {
  value: string;
  capturing: boolean;
  onStart: () => void;
  onCapture: (hotkey: string) => void;
  onCancel: () => void;
}) {
  const ref = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!capturing) return;
    ref.current?.focus();

    const handler = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();

      if (e.key === "Escape") {
        onCancel();
        return;
      }

      const modifiers: string[] = [];
      if (e.ctrlKey) modifiers.push("Ctrl");
      if (e.altKey) modifiers.push("Alt");
      if (e.shiftKey) modifiers.push("Shift");
      if (e.metaKey) modifiers.push("Super");

      const main = mainKeyFromEvent(e);
      if (!main || main.trim() === "") return; // sadece modifier'a basıldı, bekle

      const hotkey = [...modifiers, main].join("+");
      if (hotkey.trim() === "") return;
      onCapture(hotkey);
    };

    window.addEventListener("keydown", handler, true);
    return () => window.removeEventListener("keydown", handler, true);
  }, [capturing, onCapture, onCancel]);

  return (
    <button
      ref={ref}
      type="button"
      onClick={onStart}
      className={`nokat-input text-left font-mono text-xs transition ${
        capturing
          ? "border-emerald-500 text-emerald-400 ring-1 ring-emerald-500/30"
          : "text-neutral-200"
      }`}
    >
      {capturing ? "Kısayol için tuşlara basın…" : value || "Kısayol ayarla"}
    </button>
  );
}

function mainKeyFromEvent(e: KeyboardEvent): string | null {
  // Modifier tuşları tek başına ana tuş sayılmaz
  if (
    ["Control", "Alt", "Shift", "Meta"].includes(e.key) ||
    e.code === "ControlLeft" ||
    e.code === "ControlRight" ||
    e.code === "AltLeft" ||
    e.code === "AltRight" ||
    e.code === "ShiftLeft" ||
    e.code === "ShiftRight" ||
    e.code === "MetaLeft" ||
    e.code === "MetaRight"
  ) {
    return null;
  }

  if (e.code.startsWith("Key")) return e.code.slice(3);
  if (e.code.startsWith("Digit")) return e.code.slice(5);
  if (e.code.startsWith("F") && e.code.length > 1) return e.code;

  switch (e.code) {
    case "Space":
      return "Space";
    case "ArrowUp":
      return "Up";
    case "ArrowDown":
      return "Down";
    case "ArrowLeft":
      return "Left";
    case "ArrowRight":
      return "Right";
    case "Comma":
      return "Comma";
    case "Period":
      return "Period";
    case "Slash":
      return "Slash";
    case "Semicolon":
      return "Semicolon";
    case "Quote":
      return "Quote";
    case "BracketLeft":
      return "BracketLeft";
    case "BracketRight":
      return "BracketRight";
    case "Backslash":
      return "Backslash";
    case "Minus":
      return "Minus";
    case "Equal":
      return "Equal";
    case "Backquote":
      return "Backquote";
    case "Escape":
      return "Escape";
    case "Enter":
      return "Return";
    case "Backspace":
      return "Backspace";
    case "Tab":
      return "Tab";
    default:
      return e.key.length === 1 ? e.key.toUpperCase() : e.code;
  }
}

/** Equalizer tarzı dalga göstergesi. */
function WaveIndicator({
  color,
  animate,
}: {
  color: string;
  animate: boolean;
}) {
  const bars = [0, 1, 2, 3, 4];
  return (
    <div className="flex h-7 w-10 items-center justify-center gap-[3px]">
      {bars.map((i) => (
        <span
          key={i}
          className={`w-[4px] rounded-full ${color} ${
            animate ? "nokat-wave" : "h-2"
          }`}
          style={
            animate
              ? { animationDelay: `${i * 0.13}s` }
              : { height: `${[8, 14, 18, 14, 8][i]}px` }
          }
        />
      ))}
    </div>
  );
}

// --- İkonlar ---

function SettingsIcon({ className }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" />
    </svg>
  );
}

function MicIcon({ className }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3z" />
      <path d="M19 10v2a7 7 0 0 1-14 0v-2" />
      <line x1="12" y1="19" x2="12" y2="23" />
      <line x1="8" y1="23" x2="16" y2="23" />
    </svg>
  );
}

function FileIcon({ className }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5L14.5 2z" />
      <polyline points="14 2 14 8 20 8" />
    </svg>
  );
}

function GlobeIcon({ className }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="12" r="10" />
      <line x1="2" y1="12" x2="22" y2="12" />
      <path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z" />
    </svg>
  );
}

function TypeIcon({ className }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <polyline points="4 7 4 4 20 4 20 7" />
      <line x1="9" y1="20" x2="15" y2="20" />
      <line x1="12" y1="4" x2="12" y2="20" />
    </svg>
  );
}

function StopwatchIcon({ className }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="13" r="8" />
      <path d="M12 9v4l3 3" />
      <path d="M12 2v2" />
      <path d="M20.39 4.22l-1.64 1.64" />
      <path d="M3.61 4.22l1.64 1.64" />
    </svg>
  );
}

function CloudIcon({ className }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M17.5 19c0-1.7-1.3-3-3-3h-11a3 3 0 0 1-3-3c0-1.6 1.2-2.9 2.8-3a5 5 0 0 1 9.4-1.6 3 3 0 0 1 4.3 2.6 3.5 3.5 0 0 1 .5 6.9V19z" />
    </svg>
  );
}

function KeyIcon({ className }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="7.5" cy="15.5" r="5.5" />
      <path d="m21 2-9.6 9.6" />
      <path d="M15.5 7.5 21 2l-3-3-3.5 3.5z" />
    </svg>
  );
}

function CopyIcon({ className }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
      <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
    </svg>
  );
}

function CheckIcon({ className }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <polyline points="20 6 9 17 4 12" />
    </svg>
  );
}

export default App;
