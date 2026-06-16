# Contributing to 8voice

Thank you for your interest in contributing! This document explains how to get started and what we expect from pull requests.

## Getting Started

1. Fork the repository and clone your fork.
2. Install prerequisites listed in [README.md](./README.md).
3. Run `npm install` in the project root.
4. Download a Whisper model (e.g. `ggml-small.bin`) into `src-tauri/models/`.
5. Run `npm run tauri dev` to start the app in development mode.

## Development Workflow

- Create a new branch for your change: `git checkout -b feature/your-feature-name`.
- Make focused, minimal changes.
- Keep the existing code style.
- Add or update comments when the change is non-trivial.
- Verify your change with:
  - `cargo check` (Rust)
  - `npm run build` (TypeScript + Vite)
  - `cargo test` (Rust tests)
- Open a pull request with a clear description.

## Code Style

- **Rust:** follow `cargo fmt` and `cargo clippy`.
- **TypeScript/React:** use the existing Prettier/Tailwind conventions.
- **Comments:** write in English so the project stays accessible to global contributors.
- **Commits:** use clear, concise messages in English.

## Reporting Bugs

When reporting a bug, please include:

- Operating system and version
- 8voice version or commit hash
- Steps to reproduce
- Expected vs actual behavior
- Relevant logs or screenshots

## Feature Requests

Feature requests are welcome. Please open an issue first to discuss the idea before investing significant effort.

## Pull Request Process

1. Ensure the project builds and tests pass.
2. Update relevant documentation if needed.
3. Link related issues in the PR description.
4. Be responsive to review feedback.

## License

By contributing, you agree that your contributions will be licensed under the [MIT License](./LICENSE).
