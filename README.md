<div align="center">
  <img src="src-tauri/icons/icon.png" alt="Effigy Video Compressor" width="96" height="96">
  <h1>Effigy Video Compressor</h1>
  <p><strong>Smaller clips. Your size limit. All on your PC.</strong></p>
  <p>A portable Windows video compressor with CPU and GPU encoding, automatic size correction, and two interface styles.</p>
  <p>
    <a href="https://github.com/JosEffigy/effigy-video-compressor/releases/latest"><strong>Download for Windows</strong></a>
    &nbsp;·&nbsp; <a href="CHANGELOG.md">What's new</a>
    &nbsp;·&nbsp; <a href="https://github.com/JosEffigy/effigy-video-compressor/issues">Report an issue</a>
  </p>
</div>

## Support development ☕

If Effigy saves you time, a donation helps support its development. Thank you!

[![Support on Ko-fi](https://img.shields.io/badge/Support_on_Ko--fi-FF5E5B?style=for-the-badge&logo=kofi&logoColor=white)](https://ko-fi.com/joseffigy)
[![Donate with PayPal](https://img.shields.io/badge/Donate_with_PayPal-003087?style=for-the-badge&logo=paypal&logoColor=white)](https://paypal.me/JosEffigy)

## Pick your workspace

| Modern | Simple |
| --- | --- |
| ![Modern interface](docs/screenshots/modern-theme.png) | ![Simple interface](docs/screenshots/simple-theme.png) |

Both styles include dark, light, and system appearance, plus custom accent colors.

## Get started

1. Download the versioned ZIP from the [latest release](https://github.com/JosEffigy/effigy-video-compressor/releases/latest).
2. Extract it to a writable folder and run the included executable, such as `effigy-video-compressor-v2.1.2.exe`.
3. If prompted, install FFmpeg through the app.
4. Add your videos, choose a target size or quality level, and compress.

**To run:** Windows 10/11, Microsoft Edge WebView2, and FFmpeg/FFprobe. The app can install FFmpeg for you. Rust and Node.js are only needed to build from source.

Videos are processed locally. Keep `finish.wav` beside the executable for the optional completion sound.

## What you can do

- **Fit a file-size limit:** two-pass software encoding, adaptive audio budgets, and final size verification with automatic correction.
- **Choose quality instead:** CRF for software encoders or the supported hardware quality mode.
- **Use your CPU or GPU:** x264, x265, SVT-AV1, NVIDIA NVENC, AMD AMF, and Intel QSV. Hardware encoders appear only after successfully initializing on your system.
- **Control resolution and frame rate:** choose manually or let Auto use the bitrate budget and sampled motion.
- **Queue multiple videos:** per-file progress and cancellation, with an accessible FFmpeg log.
- **Export MP4 or WebM:** MP4 with AAC audio; WebM with Opus for AV1 encoders.
- **Make it yours:** remembered settings, optional persistent output folder, start-on-add, and minimize-to-tray.

## Encoding details

| Encoder | Fixed size | Quality mode |
| --- | --- | --- |
| x264 / x265 | Two-pass average bitrate with VBV | CRF |
| SVT-AV1 | Native two-pass average bitrate | CRF |
| NVIDIA NVENC | VBR; tested multipass/AQ controls when available | CQ-VBR |
| AMD AMF | Peak-constrained VBR | QVBR when supported, otherwise a tested fallback |
| Intel QSV | Bitrate-constrained VBR | Global-quality / ICQ |

Supported fixed-size VBV paths allow peaks of **200% of the average bitrate**. SVT-AV1's ABR path does not use VBV. Fixed-size outputs are checked against the target and retried when necessary; an output still over the limit is reported as a failure.

**SVT-AV1 defaults:** preset **6**, 10-bit output, `tune=0`, AQ2, variance-boost strength **2**, and `ac-bias=1.0`. These are practical defaults, not a claim of best quality for every video.

### Motion and scene analysis

- **x264 / x265:** Lightweight uses motion analysis; Probe-based adds a low-resolution constant-quality encode. Both can feed normalized bitrate zones into the encoder.
- **SVT-AV1:** always uses Lightweight motion analysis for Auto resolution/FPS. Analysis choices are hidden, and its native two-pass encoder distributes the video budget. No separate scene encodes or individual scene targets.
- **Hardware encoders:** use tested lookahead, adaptive quantization, or pre-analysis features where supported, rather than custom time-range zones.

## Settings and FFmpeg

Settings live in `effigy-video-compressor.cfg` beside the executable. Appearance preferences are saved automatically; encoder choices are restored when **Remember encoder settings** is enabled. The output folder is remembered only when its persistence option is enabled.

Close the app and remove the configuration file to reset it. First-launch settings are x264, CRF 24, Slower, Auto resolution/FPS, and MP4.

FFmpeg and FFprobe are found in this order:

1. Beside the executable.
2. `%LOCALAPPDATA%\Programs\FFmpeg\bin`.
3. System `PATH`.

The in-app installer downloads the checksum-verified Full release build from [gyan.dev](https://www.gyan.dev/ffmpeg/builds/), including SVT-AV1 support.

## Build from source

Install Rust with the MSVC toolchain, Node.js 22+, and pnpm, then run:

```powershell
pnpm install
pnpm tauri dev
```

Build and check the project:

```powershell
pnpm build
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

Create a portable release with `pnpm release`. Package and Cargo versions must match; title bars follow the package version. The archive, folder, and executable include the version:

```text
release/
├─ effigy-video-compressor-v2.1.2/
│  ├─ effigy-video-compressor-v2.1.2.exe
│  └─ finish.wav
└─ effigy-video-compressor-v2.1.2.zip
```

The interface lives in `src/`; the Rust/Tauri backend lives in `src-tauri/src/lib.rs`. See [CONTRIBUTING.md](CONTRIBUTING.md) for contribution guidance.

## License

No open-source license has been selected. All rights are reserved by the project owner.
