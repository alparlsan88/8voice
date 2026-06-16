# 8voice

A privacy-first voice dictation app. Press a global shortcut, speak, and 8voice transcribes your speech and injects the text into the focused application. It runs in the system tray and supports both on-device (offline) transcription via Whisper and cloud transcription via the Groq Whisper API.

> **Status:** MVP — Phase 1 complete. Push-to-talk / toggle modes, whisper.cpp integration, text injection, and VAD-based auto-stop are implemented.

## Features

- **Global shortcut** — push-to-talk or toggle mode
- **Offline transcription** — local Whisper GGUF models via whisper.cpp
- **Cloud transcription** — Groq Whisper API support
- **Voice Activity Detection (VAD)** — automatic stop after silence
- **Text injection** — type or paste text into the active window
- **Floating widget** — minimal on-screen control
- **System tray** — lives in the tray when the window is closed

## Tech Stack

| Layer | Choice |
| --- | --- |
| App shell | Tauri 2.0 (Rust + Web UI) |
| UI | React + TypeScript + Tailwind CSS v4 |
| Audio capture | cpal + rubato (16 kHz mono resampling) |
| Offline transcription | whisper-rs (whisper.cpp, GGUF) |
| Cloud transcription | Groq Whisper API |
| Text injection | enigo + arboard (clipboard) |
| Global shortcut | tauri-plugin-global-shortcut |
| Settings | tauri-plugin-store (JSON) |

## Prerequisites

- [Rust](https://rustup.rs) (stable ≥ 1.75)
- [Node.js](https://nodejs.org) ≥ 20 LTS
- [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/)
  - Windows: WebView2 + MSVC build tools
  - macOS: Xcode Command Line Tools
  - Linux: webkit2gtk, libayatana-appindicator, etc.
- C/C++ toolchain (required for whisper.cpp native compilation)

## Installation

```bash
# Install frontend dependencies
npm install

# Download a Whisper GGUF model (~466 MB for small)
# https://huggingface.co/ggerganov/whisper.cpp
mkdir -p src-tauri/models
# Place ggml-small.bin (or tiny/base/medium) inside src-tauri/models/
```

Model alternatives:
- `ggml-tiny.bin` — fastest, lowest accuracy
- `ggml-base.bin` — good balance for fast CPUs
- `ggml-small.bin` — recommended balance (default)
- `ggml-medium.bin` — more accurate, slower

## Running

```bash
# Development (hot reload + Rust rebuild)
npm run tauri dev

# Production build (creates installers)
npm run tauri build
```

On first launch: the settings window opens, the shortcut (`Ctrl+Shift+Space`) is registered, and the local model is preloaded if available. Closing the window keeps the app running in the tray.

## Usage

1. **Hold** the shortcut (push-to-talk) or **press** it once (toggle mode) to start recording
2. Speak
3. **Release** (PTT) or **press again** (toggle) to stop — the audio is transcribed and typed into the focused window

Change microphone, model, language, shortcut, mode, and injection behavior from the settings window.

## Project Structure

```
src-tauri/src/
├── lib.rs          # Tauri entry, tray, plugins, commands, pipeline
├── audio.rs        # cpal capture + resample + buffer
├── transcribe.rs   # whisper-rs + Groq API
├── inject.rs       # enigo typing + clipboard paste
├── hotkey.rs       # global shortcut
├── state.rs        # recording state machine
├── settings.rs     # store (JSON) settings
├── tray.rs         # tray icon + menu
└── vad.rs          # voice activity detection
src/                # React + TypeScript UI (App.tsx + Widget.tsx)
```

## Platform Notes

| Platform | Note |
| --- | --- |
| Windows | Microphone is enabled by default; injection into elevated apps is limited |
| macOS | Accessibility permission required (System Settings → Privacy & Security → Accessibility) |
| Linux | X11 works best; Wayland text injection falls back to clipboard |

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md).

## Security

See [SECURITY.md](./SECURITY.md).

## License

[MIT](./LICENSE) © 2026 8voice
