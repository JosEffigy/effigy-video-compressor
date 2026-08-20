# Contributing to Effigy Video Compressor

Thanks for helping improve Effigy.

## Before making a change

- Keep the interface approachable for non-technical users.
- Preserve both the Modern and Simple themes.
- Do not expose a hardware encoder unless a real FFmpeg initialization test succeeds.
- Keep FFmpeg processes attached to the application's Windows Job Object.
- Do not terminate unrelated FFmpeg processes.
- Preserve MP4/AAC and WebM/Opus container behavior.

## Local workflow

```powershell
pnpm install
pnpm tauri dev
```

Before opening a pull request, run:

```powershell
pnpm build
cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

Test changes with at least one short video. Encoding changes should be checked in both Fixed and Dynamic modes. Hardware-specific changes should also be tested on the relevant NVIDIA, AMD, or Intel GPU.

## Pull requests

Describe:

- What changed and why
- Which codecs and containers were tested
- Whether process cancellation and application shutdown were tested
- Screenshots for visible interface changes

Keep unrelated cleanup out of feature-focused pull requests whenever possible.
