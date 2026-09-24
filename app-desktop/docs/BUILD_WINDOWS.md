# Windows release build

The Windows release is a per-user NSIS installer for Windows 10/11 x64. The
installer includes the Tauri application, the frozen Python AI worker, FFmpeg,
FFprobe, and third-party notices. End users do not need Node.js, Rust, Python,
or FFmpeg on `PATH`. Windows WebView2 is installed by the official bootstrapper
when it is not already available.

## Build prerequisites

- Windows 10/11 x64
- Rust stable/MSVC toolchain and Windows SDK
- Node.js 22.12+ and npm
- Python 3.12 x64
- Network access to PyPI and GitHub release assets
- At least 12 GB free disk space for Python build caches, FFmpeg archive, and
  installer output

The AI model weights are not embedded in the installer. They are installed on
demand from Settings and stored in the operating system app data directory.

## Build

From PowerShell at the repository root:

```powershell
./scripts/build-release.ps1 -Python python
```

The release script performs a clean `npm ci`, then checks that `package.json`, `tauri.conf.json`, `Cargo.toml`, and the
Python worker use the same version, fetches a pinned FFmpeg release and verifies
the publisher's SHA-256 checksum, builds the CPU-first one-folder Python worker,
generates a license inventory from the worker that is actually bundled, runs
frontend, Rust, and Python tests, then creates an NSIS installer. Artifacts,
license notices, checksums, and a release manifest are written to `release/`.

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
`src-tauri/Cargo.lock`, `python/pyproject.toml`, and
`python/worker/_version.py` in its temporary checkout before building. No
version-manifest commit is required for the release job.
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

The workflow runs the release script and its frontend, Rust, and Python tests,
then attaches the installer, SHA-256 checksum, release manifest, and third-party
notices to a GitHub Release. It uses GitHub's automatically provided
`GITHUB_TOKEN`; no secret is needed for an unsigned build. Code signing is not
configured in CI, so the published installer is unsigned until a protected
signing setup is added. The workflow file lives at the Git repository root,
while the app source is in the `app-desktop/` subdirectory.

To rebuild the pinned FFmpeg resources, use `./scripts/build-ffmpeg.ps1 -Force`.
To rebuild the worker, use `./scripts/build-worker.ps1 -Clean`.
Generated runtime binaries are intentionally ignored by Git. Do not commit an
installer or model weights.

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
