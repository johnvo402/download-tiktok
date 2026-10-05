# Windows release build

The Windows release is a per-user NSIS installer for Windows 10/11 x64. The
installer includes the Tauri application, the frozen Python AI worker, a separate browser downloader worker, FFmpeg,
FFprobe, and third-party notices. End users do not need Node.js, Rust, Python,
or FFmpeg on `PATH`. Windows WebView2 is installed by the official bootstrapper
when it is not already available.

## Build prerequisites

- Windows 10/11 x64
- Rust 1.98.1/MSVC (pinned in `rust-toolchain.toml`) and Windows SDK
- Node.js 22.12+ and npm
- Python 3.12 x64 (CI pins 3.12.10)
- uv 0.11.12 (used to stage the optional runtime installer)
- Network access to PyPI and GitHub release assets
- At least 12 GB free disk space for Python build caches, FFmpeg archive, and
  installer output

The AI model weights are not embedded in the installer. They are installed on
demand from Settings and stored in the data folder selected during installation.

## Application storage

The NSIS storage page defaults to `Documents\JC AI` for new installs. It stores
the absolute selection as UTF-16LE in `jc-ai-data-dir.txt` next to the app
executable, and remembers it under the per-user registry key
`Software\com.ai.video-localization\Storage`. Upgrades/reinstalls reuse this
selection; older installs default to their existing Roaming app-data folder.
Selecting another folder does not move existing projects or models. The data
folder must be writable and outside the app installation directory.

Silent/passive installations can select a folder explicitly:

```powershell
.\JC-AI-setup.exe /S /DATADIR="D:\JC AI\Data"
```

The app resolves storage before creating its window, including the WebView2
profile, and redirects subprocess temporary files to `cache/tmp` within that
root. An invalid or inaccessible configured folder fails startup instead of
silently writing elsewhere. Releases without the selection file retain the
legacy app-data fallback for compatibility with older installations.

Development uses `JC_AI_DATA_DIR` from the process environment, `.env.local`,
or `.env`, in that order, defaulting to the repository's ignored `.app-data/`.
Relative paths resolve from the repository root. The existing `JC_QA_DATA_DIR`
test override has higher priority. Release builds ignore these dev overrides.
Development/custom roots never migrate caches from a separate installation.

## Build

From PowerShell in `app-desktop/`:

```powershell
./scripts/build-release.ps1 -Python python
```

The release script checks that the app manifests have the same version and that
the two worker version files agree with each other. Worker and app versions may
differ. It verifies the pinned FFmpeg archive, builds or reuses a validated frozen
worker, performs `npm ci` once, runs the tests, generates license notices, builds
the application with `tauri build --no-bundle -- --locked`, and packages it with
`tauri bundle`. Each stage has a separate log group and elapsed time.

The default installer compression is `zlib`, favoring packaging speed. To compare
with a smaller, slower-compressing installer, use `-Compression lzma`. Compression
changes installer size only; it does not change the app, AI models or video output.
The installer, notices, checksums, `release-manifest.json` (including installer
bytes, compression, worker version/fingerprint), and `build-timings.json` are
written to `release/`. Compare the same inputs when benchmarking compressors.

For a public release, sign the setup executable with the organization's code
signing certificate and require signing:

```powershell
./scripts/build-release.ps1 -Python python -RequireSigning `
  -CertificateThumbprint "CERTIFICATE_SHA1_THUMBPRINT"
```

The private key is read from the Windows certificate store by `signtool`; it is
not stored in the repository. A release without Authenticode signing is a local
test build and may show a SmartScreen warning.

## GitHub Actions release by version tag

The repository-root workflow at `.github/workflows/app-desktop-release.yml`
builds and publishes the Windows installer when a `vX.Y.Z` or
`desktop-vX.Y.Z` tag is pushed. The tag is the release version source: CI sets
`AI_VIDEO_LOCALIZATION_VERSION` and synchronizes it into `package.json`,
`package-lock.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`,
`src-tauri/Cargo.lock` in its temporary checkout before building. No
version-manifest commit is required for the release job.
Worker version is independent: update both `python/pyproject.toml` and
`python/worker/_version.py` when deliberately changing its version. Worker source
changes invalidate the packaged-worker cache even without a version bump.
For example:

```powershell
git tag v0.2.0
git push origin v0.2.0
```

For a local versioned release build, set the same environment variable before
running the release script:

```powershell
$env:AI_VIDEO_LOCALIZATION_VERSION = "0.2.0"
./scripts/build-release.ps1 -Python python
```

Without that variable, local builds keep the versions checked into the source.

On `dev` pushes and pull requests, CI runs frontend, Rust and Python tests plus
the frontend production build. A tag checks the latest default-branch push run
for **the exact tagged commit SHA**, including annotated tags. It reuses only the
successful `Test Windows x64` job. Failed tests block release. Missing/cancelled
tests, or tests still pending after a bounded wait, cause a fresh test job. An API
error blocks the gate rather than treating unknown status as success. Packaging
and publication require passing tests, either reused or run in this workflow.

The workflow then attaches the installer, SHA-256 checksum, release manifest,
timings and third-party notices to a GitHub Release. It uses the provided
`GITHUB_TOKEN`; no secret is needed for an unsigned build. Code signing is not
configured in CI, so the published installer is unsigned until a protected
signing setup is added. The workflow file lives at the Git repository root,
while the app source is in the `app-desktop/` subdirectory.

To rebuild the pinned FFmpeg resources, use `./scripts/build-ffmpeg.ps1 -Force`.
To rebuild the worker, use `./scripts/build-worker.ps1 -Clean`.
Generated runtime binaries are intentionally ignored by Git. Do not commit an
installer or model weights.

## Cache behavior and release time

- Test and release Rust dependencies have separate cache keys. The compiler is
  pinned. Keep `RUST_TOOLCHAIN` in the workflow and `rust-toolchain.toml` aligned.
- On `dev`, the preparation job first checks cache existence without downloading
  large artifacts. It builds Rust release dependencies with the same Tauri flags
  only on a cache miss. It no longer builds release code on every source change.
  Swatinem's cache excludes workspace binaries and normalizes package versions;
  each tag still compiles the current app, not an old cached executable.
- The frozen-worker key includes Python implementation/version/architecture,
  worker source/assets, packaging scripts, pinned requirements and build scripts.
  App version and frontend changes do not invalidate it. Restored workers are
  checked against a per-file SHA-256 inventory, then started for version and
  JSONL ASR/TTS/OCR capability checks using an isolated empty model directory.
  A missing/corrupt/incompatible cache is rebuilt. No model is downloaded by CI.
- Default-branch caches are readable from tags; caches saved only under one tag
  are not a reliable way to speed up another tag. Push `dev` and let preparation
  finish before tagging for the best chance of a warm release. `workflow_dispatch`
  on `dev` can also prepare caches, without publishing a release.
  When branch and tag are pushed together, the gate also gives that branch's
  cache preparation a bounded wait; a slow/failed cache job never bypasses tests.
- FFmpeg's pinned archive is cached and its publisher checksum is checked before
  use. Cache eviction never makes the release dependent on a previous build.
- Test jobs do not enable pip caching: no pip install occurs there, so a cold
  runner must not fail in a cache post-step because the pip directory is absent.

The first build after a dependency/compiler change remains slower. Use the Actions
summary and `build-timings.json` to distinguish worker build, Rust compilation and
NSIS compression. There is no fixed promised release duration; download speed,
runner load and cache eviction affect it. The installer stays self-contained;
end users do not install Python/FFmpeg themselves.

Local compression comparison on 2026-09-28 (Windows x64, JC AI 0.1.6):

| Compressor | Bundle time | Installer size |
| --- | ---: | ---: |
| LZMA | 404.23 s | 292,096,100 bytes (278.6 MiB) |
| zlib | 132.15 s | 397,329,463 bytes (378.9 MiB) |

Both used the same application binary and worker, and both passed `7z t` with
3,341 files / 932,353,145 uncompressed bytes. This was one local measurement, not
a guarantee of GitHub Actions duration. zlib reduced measured packaging time by
about 67% while increasing download size by about 36%. The cached-worker verify
and startup stage took 2.45 s locally; GitHub cache download time is additional.

Release tooling checks can be run without a full installer build:

```powershell
npm run test:ci
python -m unittest discover -s scripts/tests -p "test_*.py" -v
```

## Optional AI installation cache (on the user's machine)

This is separate from GitHub Actions build caches. Optional WhisperX, NVIDIA and
BandIt installs share `runtimes/advanced/cache` through `UV_CACHE_DIR`. Successful
installs retain downloaded packages. The app continues to build and verify a
complete staging environment before swapping it with `current`, so a failed new
feature installation cannot damage the working runtime. Unchanged packages can
come from the retained local cache; changed versions or a manually cleared cache
may require downloads. This does not eliminate staging/copying work on disk.

Settings → Advanced shows **Cache cài đặt AI** separately from the installed AI
components. **Xóa cache cài đặt** requires confirmation, uses the install mutex
and `uv cache clean` (including uv's cross-process lock), and clears only the
managed package cache. It does not remove `current`, the
managed Python installation, model weights or project data. Ordinary temporary
file cleanup does not clear this cache. No fresh multi-GB CUDA installation is
needed to run the isolated filesystem tests for these rules.
An isolated smoke check also installed `packaging==25.0` into a second environment
using `uv pip install --offline` and the retained cache, then verified that the
installed package still imported after `uv cache clean`. This validates the cache
mechanism, not a fresh end-to-end CUDA/WhisperX/VieNeu/BandIt installation.
See [uv cache behavior and locking](https://docs.astral.sh/uv/concepts/cache/).

## Installer behavior

- NSIS, per-user install; no administrator prompt is required for the normal
  install path.
- WebView2 bootstrapper mode; internet access may be required on a clean PC.
- Projects, logs, settings, API credentials, exports, and model cache are kept
  in Tauri's operating-system application data locations, outside the install
  directory. Uninstall does not remove user projects or downloaded models.
- Credentials continue to use the OS credential manager; model weights are
  downloaded only after explicit confirmation in Settings.
- A repair/reinstall restores packaged runtime files without removing project
  data.

## Runtime layout

```text
Application install\
  JC AI.exe
  resources\
    ffmpeg\ffmpeg.exe, ffprobe.exe, ffmpeg-build.json
    ai-worker\ai-worker.exe, Python runtime and package metadata
    THIRD_PARTY_LICENSES.txt

App data\
  projects\{UUID}\
  cache\fw\{model}\
  models\huggingface\hub\
  logs\app.log
```

The UI's **Settings → Advanced → Runtime Health** checks app data write access,
FFmpeg/FFprobe execution, and the bundled worker. **Export diagnostics** writes a
reviewable bundle containing scrubbed runtime information and recent sanitized
logs. It excludes media, transcripts, credentials, and voice reference audio.

## Updates and release signing

The installer is signed when the signing option is supplied. An automatic
updater is intentionally not enabled until the release owner supplies an update
endpoint, signing key, and signed update artifacts. The app keeps user data out
of the install tree so a later updater can replace application resources without
touching projects or model caches. Never put updater private keys in this repo.

## License notice

The selected FFmpeg GPL build enables `libx264`, required by the current
exporter. See `src-tauri/resources/THIRD_PARTY_LICENSES.txt`,
`docs/THIRD_PARTY_NOTICES.md`, and the generated `release/ffmpeg-build.json` for
the exact pinned build and source references. Obtain legal review and meet the
GPL corresponding-source obligations before public or commercial distribution.

## Runtime packaging audit

See [Task 9 runtime report](TASK_09_RUNTIME_PACKAGING.md) for the worker split,
optional dependency graph, install/uninstall safety and measured artifact sizes.
