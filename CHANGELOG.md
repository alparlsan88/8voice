# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.34] - 2026-06-20

### Fixed

- CI signing was failing with "incorrect updater private key password" — regenerated passwordless signing key pair to eliminate password mismatch issues.

### Changed

- Removed `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` from CI workflow (new key has no password).

## [0.2.31] - 2026-06-20

### Fixed

- CI signing was failing with "incorrect updater private key password" — regenerated signing key pair with known password and updated GitHub Secrets.

### Changed

- Changed CI env var name from `TAURI_PRIVATE_KEY` to `TAURI_SIGNING_PRIVATE_KEY` (Tauri v2 CLI requirement).

## [0.2.29] - 2026-06-19

### Changed

- Removed `releaseDraft: true` so releases are published automatically and `latest` redirect works.
- Moved `permissions: contents: write` to workflow top level for reliable release creation.
- Added upload of `.tar.gz`/`.zip` updater archives and `.sig` files alongside JSON manifests.

### Fixed

- CI build was failing with "A public key has been found, but no private key" — user must now set `TAURI_PRIVATE_KEY` and `TAURI_KEY_PASSWORD` GitHub secrets; CI passes when secrets are configured.

## [0.2.28] - 2026-06-19

### Changed

- Updater endpoint switched from placeholder CDN (`your-cdn.com`) to GitHub Releases. The app now fetches update manifests from `https://github.com/alparlsan88/8voice/releases/latest/download/{{target}}.json`.
- GitHub Actions release workflow now passes `TAURI_PRIVATE_KEY` and `TAURI_KEY_PASSWORD` secrets to `tauri-action` for signing updater artifacts.
- Release workflow now generates per-platform JSON update manifests (`windows-x86_64.json`, `darwin-x86_64.json`, `darwin-aarch64.json`, `linux-x86_64.json`) and uploads them as release assets.
- Releases are now published automatically (`releaseDraft: false`) so the `latest` redirect resolves correctly.
- `createUpdaterArtifacts` enabled in `tauri.conf.json` so Tauri generates `.sig` and `.tar.gz` updater artifacts during builds.

### Fixed

- Update check was failing with "error decoding response body" because the endpoint pointed to `your-cdn.com` which doesn't exist. Now points to GitHub Releases where actual .msi/.dmg/.AppImage assets are hosted.

## [0.2.27] - 2026-06-19

### Added

- **Deepgram** cloud transcription provider with API key validation.
- **AssemblyAI** cloud transcription provider with API key validation.
- 6-step onboarding wizard (Provider → Microphone → Language → Shortcut → Injection → Finish) with progress bar and step navigation, replacing the previous sidebar-tab layout.
- `HotkeyCapture` component in the Shortcut step: press any key combination for the global shortcut.
- `store.reload()` call in `settings::load()` to ensure persisted settings are always read from disk.
- Tracing logs (`tracing::info!` / `tracing::warn!`) in `settings::load()` and `settings::save()` for debugging persistence issues.
- "Settings..." item in the floating widget context menu — right-click the widget to open the settings window without going through the tray.
- Full Whisper language list (99 languages, labeled in their native script) in the settings language picker, replacing the previous Auto/Turkish/English-only selection.
- **Vosk** offline speech recognition engine: new provider in settings + model directory picker with validation and downloadable model list.
- **Sherpa-ONNX** offline speech recognition engine: new provider in settings + model directory picker with validation and downloadable model list (Whisper ONNX models).

### Changed

- Onboarding layout completely rewritten: tab-based sidebar replaced with a 6-step sequential wizard that forces users through all essential settings.
- Provider selection uses Cloud API / Local Engine tabs (50% split buttons) instead of a 2-column grid — each tab shows its cards full-width.
- Sidebar logo alignment: changed from `justify-between` to `gap-3` so the logo and text are left-aligned together.
- Default shortcut mode is now Toggle (was Push-to-talk).
- Default global shortcut is now `Ctrl+Q` on Windows/Linux and `Super+Q` (Cmd+Q) on macOS (was `Ctrl+Shift+Space`). Existing users keep their saved shortcut; only new installs / reset settings use the new default.

### Fixed

- Unicode escape sequences (`\u2026`, `\u2014`) no longer render as literal text — replaced with actual `…` and `—` characters.
- Settings not persisting on restart: `store.reload()` added before reading settings file on startup.
- Vosk and Sherpa-ONNX engine modules now provide stub implementations with friendly install instructions instead of compile-time crate dependency errors.

## [0.2.26] - 2026-06-18

### Added

- Per-card download & delete actions in the model picker: each model card has its own download icon (spinner while downloading) and a trash icon to delete an already-downloaded model.
- Accuracy and speed rating bars on every model card (5-step indicators).
- "Downloaded" and "Active" badges on model cards.
- Inline download progress bar rendered on the card being downloaded.
- `cmd_delete_downloaded_model` Tauri command to remove a downloaded model file from the app data directory.

### Changed

- Enlarged the main window (onboarding + settings) from 800×600 to 1024×768.
- Redesigned the onboarding model list as a card-based grid, replacing the radio-button list.
- Redesigned the settings Transcription model downloader to match the onboarding card UI; selecting a downloaded card now activates that model directly.
- Sidebar header layout unified across onboarding and settings: logo and "8voice" brand name placed side-by-side.

## [0.2.25] - 2026-06-18

### Added

- Diagnostic logging in the recording pipeline: capture stats (sample count, duration, RMS, peak), selected device, language, VAD config, and transcript summary on every run.
- Audio device info logged on stream open (name, sample rate, channels, format).

### Changed

- Brand logo is now circular (SVG + regenerated PNG/ICO/ICNS icons across the app, tray, store tiles, and favicon).
- Empty transcripts now surface a user-visible error (microphone silent / speech not detected) instead of silently returning to idle.
- macOS builds enable whisper-rs `metal` backend to avoid a crash in ggml 1.8.3's CPU/BLAS encoder path on M1 + macOS 12.3.

### Fixed

- macOS crash (`EXC_BAD_ACCESS` at `ggml_backend_sched_graph_compute_async`, PC=0) during transcription on Apple Silicon — encoder now runs on Metal.

## [0.2.24] - 2026-06-16

### Added

- Canonical SVG brand logo (`public/logo.svg`) and `scripts/generate-icons.py` generator.
- Consistent logo usage across the app header, onboarding, floating widget, system tray, favicon, and landing page.

### Changed

- All Tauri app icons and Microsoft Store tile images regenerated from the canonical logo.

### Removed

- Removed the "Start hidden" and "Show tray icon" settings toggles. The tray icon is now always shown and the app always opens its windows on startup after onboarding.
- Removed inconsistent `public/logo-light.jpeg` and `public/logo-dark.jpeg` assets.

## [0.2.19] - 2026-06-16

### Added

- Marketing site now highlights the floating widget with an interactive, click-to-try demo.
- New "Widget" section on the landing page shows the pill control, live wave indicator, and processing spinner.

### Changed

- README feature list and landing-page navigation now emphasize the floating widget as a primary interaction surface.

## [0.2.18] - 2026-06-16

### Fixed

- Made `windows`, `windows-core`, and `webview2-com` dependencies Windows-only via `[target.'cfg(windows)'.dependencies]` so that macOS and Linux release builds no longer try to compile the Windows-only `windows-future` crate.

## [0.2.17] - 2026-06-16

### Fixed

- CI: switched macOS to `KyleMayes/install-llvm-action@v2` so `bindgen` can locate `libclang`.
- CI: set an explicit `PKG_CONFIG_PATH` on Ubuntu so `alsa-sys` can find `alsa.pc`.

## [0.2.16] - 2026-06-16

### Fixed

- CI: added `libasound2-dev`, `clang`, and `pkg-config` to the Ubuntu release runner to satisfy `alsa-sys` and `bindgen`.
- CI: installed LLVM via Homebrew and exported `LIBCLANG_PATH` on macOS for `bindgen`.

## [0.2.15] - 2026-06-16

### Added

- GitHub Actions cross-platform release workflow that builds and drafts a GitHub Release for Windows (`.msi`), macOS universal (`.dmg`), and Linux (`.AppImage`/`.deb`) on every `v*` tag push.

## [0.2.14] - 2026-06-16

### Added

- Native right-click context menu on the floating widget with a "Quit 8voice" option.

## [0.2.13] - 2026-06-16

### Changed

- Increased the voice-reactive wave amplitude so the bars visibly reach the top and bottom edges of the widget.

## [0.2.12] - 2026-06-16

### Added

- New settings toggles: "Start hidden", "Launch on startup", and "Show tray icon".
- `tauri-plugin-autostart` integration so the app can start automatically with the system.

## [0.2.11] - 2026-06-16

### Changed

- Reworked the widget wave indicator into a center-pulsing heartbeat-style visualization.

## [0.2.10] - 2026-06-16

### Changed

- Fine-tuned the widget wave indicator: 15 thinner bars, log-scale amplitude normalization, and faster smoothing for more responsive visual feedback.

## [0.2.9] - 2026-06-16

### Changed

- Made the widget wave indicator full-width, more sensitive, and smoother.

## [0.2.8] - 2026-06-16

### Changed

- Translated onboarding and loading strings from Turkish to English.

## [0.2.7] - 2026-06-16

### Added

- Voice-reactive wave indicator on the floating widget that reflects live microphone amplitude. Rust side computes an RMS level and emits `app://audio-level` events; the widget renders a dynamic bar visualization.

## [0.2.6] - 2026-06-16

### Fixed

- Disabled the browser context menu inside the floating widget.

## [0.2.5] - 2026-06-16

### Fixed

- Corrected the HuggingFace Whisper model download URL by removing a duplicate `ggml-` prefix.

## [0.2.4] - 2026-06-16

### Fixed

- Avoided direct Tauri `State` injection in `cmd_get_settings`; now uses `AppHandle::state` for better compatibility.

## [0.2.3] - 2026-06-16

### Fixed

- Managed the settings state as an explicit `Arc<RwLock<Settings>>` for reliable Tauri state lookups.

## [0.2.2] - 2026-06-16

### Fixed

- Converted `SharedSettings` to a named-field struct to resolve Tauri state lookup issues.

## [0.2.1] - 2026-06-16

### Fixed

- Stabilized the Tauri `SharedSettings` state by wrapping it in a newtype.
- Improved onboarding and settings layouts for tablet-sized screens.

## [0.2.0] - 2026-06-16

### Added

- First-run onboarding screen with three setup options:
  - Download a Whisper model from HuggingFace with progress tracking.
  - Select an existing local `.bin` / `.gguf` model file.
  - Enter and validate a Groq API key for cloud transcription.
- `has_completed_onboarding` setting to control first-run experience.

## [0.1.0] - 2026-06-16

### Added

- Privacy-first on-device voice dictation using Whisper (whisper.cpp).
- Groq Whisper API support as an alternative transcription provider.
- Global shortcut support with push-to-talk and toggle modes.
- Floating widget for quick recording control.
- System tray integration with dynamic state icons.
- Settings UI for microphone, provider, model path, API key, language, shortcut, and injection mode.
- Voice Activity Detection (VAD) for automatic stop after silence.
- Last transcript panel with copy button and automatic clipboard copy.
- MIT license and open-source documentation (README, CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, CHANGELOG).

### Changed

- Project branding unified as **8voice** across the codebase and documentation.

### Added

- Initial MVP release.

[unreleased]: https://github.com/alparlsan88/8voice/compare/v0.2.29...HEAD
[0.2.29]: https://github.com/alparlsan88/8voice/releases/tag/v0.2.29
[0.2.28]: https://github.com/alparlsan88/8voice/releases/tag/v0.2.28
[0.2.27]: https://github.com/alparlsan88/8voice/releases/tag/v0.2.27
