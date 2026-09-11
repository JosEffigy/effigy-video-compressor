# Changelog

All notable changes to Effigy Video Compressor will be documented here.

This project follows the structure of [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [2.1.2] - 2026-09-11

### Fixed

- Prevent the white startup flash with a dark window background and reveal the window after loading the saved theme, with a timeout fallback.

## [2.1.1] - 2026-09-10

### Changed

- Default SVT-AV1 to preset 6 with AC bias 1.0 and Lightweight analysis only; hide analysis choices for this encoder.
- Allow 200% VBV peaks for supported fixed-size encoders, including size-correction retries.
- Add the release version to portable filenames and keep title-bar versions synchronized with package metadata.

## [2.1.0] - 2026-08-26

### Added

- Optional persistent output folder checkbox beside the output-folder picker
- Embedded SVT-AV1 version health checks with retryable FFmpeg Full updates

### Fixed

- Synchronized audio mapping and CFR timing across x264 two-pass encodes to prevent incomplete MB-tree statistics
- Kept settings, warning backdrops, and panels below the window-control strip

## [2.0.0] - 2026-08-20

### Added

- Modern and Simple interface themes with configurable appearance and accent colors
- Runtime NVENC, AMF, and QSV detection with GPU labels
- SVT-AV1, MP4/AAC, and WebM/Opus support
- Fixed-size and dynamic CRF/perceptual-quality workflows
- Queue cancellation, shutdown warnings, tray behavior, and single-instance handling
- Portable settings stored beside the executable
- Fixed-MB-only Gameplay Allocation with Lightweight motion analysis and optional constant-quality Probe analysis
- Native normalized x264/x265 bitrate zones and analysis-calibrated SVT-AV1 scene-aware quality redistribution
- Runtime-tested NVENC AQ/lookahead, AMF pre-analysis/high-motion boost, and QSV extended bitrate-control paths

### Changed

- Fixed-size x264/x265 encoding now uses constrained bitrate control and defaults to two passes
- Dynamic x264, x265, and SVT-AV1 encoding now uses CRF; NVENC, AMF, and QSV use their closest CRF-like hardware quality modes
- Fixed-size x264/x265 now combine two-pass ABR with VBV peak and buffer constraints; SVT-AV1 uses valid two-pass ABR because its encoder rejects VBV in ABR mode
- AMF detection now probes QVBR initialization and retains a compatible hardware-quality fallback when a driver advertises but rejects QVBR
- Version 1 CQP settings migrate to the version 2 CRF control
- Fixed-size NVENC no longer combines quality-driven VBR with a peak rate above its file-size budget
- All fixed-size outputs are verified against the hard limit and automatically retried at a corrected bitrate when necessary
- Encoder presets are selected with codec-appropriate controls
- Child FFmpeg processes are cleaned up when Effigy exits unexpectedly
