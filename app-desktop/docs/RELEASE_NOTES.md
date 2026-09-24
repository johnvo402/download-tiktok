# Release notes

## 0.1.0

- Windows per-user NSIS installer.
- Bundled checksum-verified FFmpeg/FFprobe and frozen CPU-first Python worker.
- Persistent user project data and on-demand model cache are outside the install
  directory.
- Runtime health status and sanitized diagnostics export in Settings → Advanced.
- Corrupt global settings and legacy project schemas are backed up before
  recovery/migration.
- Automatic in-app updates are not enabled; no update endpoint or signing key is
  configured in this repository.
