# System requirements

## Installed application

- Windows 10 or Windows 11, 64-bit
- Microsoft WebView2 Runtime (the installer can download its official
  bootstrapper when needed)
- 4 GB RAM for the application and small ASR model; 8 GB or more recommended
- Free disk space for the application, media, exports, and explicitly installed
  AI models (large Whisper and VieNeu models can each use multiple GB)
- Internet only for provider APIs, WebView2 bootstrapstrapping, or a user-started
  model download

Rust, Cargo, Node.js, Python, FFmpeg, and FFprobe are bundled/only needed by the
development release build. The app's installer includes the runtime binaries.
AI model weights are not part of the installer.

The first installer build is several hundred MB because it includes static
FFmpeg/FFprobe and the CPU Python worker. Downloaded Whisper/TTS models can use
multiple GB each; Settings shows cache use and supports removing the managed
model files.

## Development

Install Rust stable with MSVC build tools, Node.js 22.12+, Python 3.12 x64, and
Windows SDK. FFmpeg on `PATH` is used for local development; release builds use
the checksum-verified pinned copy in `src-tauri/resources/ffmpeg`.

See [BUILD_WINDOWS.md](BUILD_WINDOWS.md) for release packaging instructions.
