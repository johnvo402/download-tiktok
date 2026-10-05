# Task 10 — Production stability / 0.1.14-beta.1

## Status and scope

This is a **Beta candidate**, not a declaration of Stable readiness. Task 9 was committed as `4a63212`. Task 10 continues that architecture without changing model versions, provider behavior or the stack. The working tree contains the Task 10 implementation; a local build's base commit does not imply that these changes are committed or published.

The historical master document describes earlier tasks. Current code uses project schema 11, transcript schema 1, two isolated Python workers, protocol 2, and optional runtime ABI 7. These existing contracts take precedence over obsolete examples in the master document.

## Architecture and persistence

- `recovery_store.rs`: small JSON documents are validated, written to a sibling temporary file, flushed, and atomically replaced. Three previous JSON versions rotate through `.bak1`–`.bak3`. Only valid JSON enters backups; recovery additionally runs the caller's semantic validation. A raw future schema is rejected **before** typed deserialization or recovery. Corrupt originals are retained with UUID names and a recovery notice is shown. SRT/ASS use atomic non-JSON writes.
- `project_service.rs`: central schema migration preserves an original migration backup. Existing additive serde defaults remain compatible; the v5 legacy voice migration and legacy blur ID normalization are explicit. Managed directory creation rejects traversal and Windows reparse points, including junctions. Transcript/source assets remain relative to project root.
- `transcript_service.rs`: validates project identity, source checksum, timestamps and schema before restoring a backup. Transcript writes retain the existing rollback on project metadata failure. A 5,000-segment round trip is covered.
- `settings_service.rs`, `batch_service.rs`, job and export histories use the same recovery store. Clearing a batch removes its rotating backups to prevent resurrection. Credentials remain in the existing secure backend store, outside project/transcript JSON.
- `runtime_paths::data_dir` is the common resolved root for storage consumers. Production storage is unchanged; debug fault-injection roots now also isolate settings/models/logs. Debug QA never migrates the user's Local data into the test root.

Atomic replacement protects each file. It is **not a multi-file transaction journal**: an abrupt crash between transcript and project writes can leave metadata counters/status temporarily stale, although transcript payload remains independently recoverable. Atomic replacement does not protect against faulty disks or loss of all backups. Migration/corrupt copies intentionally remain until reviewed; they are not silently deleted.

## Session and job recovery

- `app_session.rs` holds a Windows exclusive file handle per data root. A second instance exits without opening another writer. `session.json` distinguishes orderly shutdown from an interrupted session. It does not currently focus the first window.
- Queued/running/waiting/cancelling jobs become **interrupted** after restart. There is no implicit replay of API requests or exports. Supported retries require a user action. STTN patch/model caches retain their existing independent reuse behavior; an export restarts encoding rather than resuming a partial MP4.
- Job registration must persist before expensive work starts. A failed registration rolls back the in-memory addition. Mid-job history failures emit one warning per failure streak. Terminal records ignore late events; cancelling records reject stale running progress.
- Model/runtime installations also have maintenance records. These records do not expose a cancel action during a stage that cannot be safely cancelled. Runtime installation continues to use Task 9's staging/current/previous promotion and persistent uv cache.
- `startup_recovery.rs` removes only validated, journalled interrupted export work directories and app-owned partials within that project's `exports`. Source media, completed output, external output folders and STTN/model caches are preserved. External partials deliberately require manual review.

## Export, processes and disk safety

- FFmpeg and worker processes use owned Windows Job Objects. Killing the desktop terminates its owned child tree. No name-based kill of every `ffmpeg.exe` or Python process is used.
- Worker protocol records are bounded (8 MiB), most queues are bounded, and diagnostic tails are limited. FFprobe has a 30-second deadline; preview transcoding has a 2-hour deadline; BandIt/long workers have a 12-hour maximum. Render/merge encoding stops after 10 minutes without actual frame/time advancement. These conservative limits avoid treating a valid long video as a short job.
- Export validates FFprobe video/audio expectations, codec, resolution and finite duration before publication. Duration tolerance is 0.1% of source, clamped to 0.15–2 seconds. The partial file is opened with Windows write access and flushed before `persist_noclobber`. Existing final files are never overwritten by this publication step.
- Merge output also flushes and publishes without overwriting. Its existing codec/audio/duration checks remain; it does not yet share export's full resolution validator.
- A successful video whose export-history write fails is reported as **partial**, with the validated output retained and its path shown. It is not reported as a broken render or left as an endlessly running UI job.
- Import and export perform conservative Windows free-space checks. These are estimates, not disk reservations; write errors remain authoritative. Estimates on a shared volume are not summed as one reservation. Physical disk exhaustion was not induced on the user's drive.
- Missing source can be relinked using **Tìm lại video nguồn**. SHA-256 must match the previous source; a different source is refused. Transcript and dependent references are preserved for an identical source. Replacement remains a separate operation.

## Frontend safety

`save-guard.ts` serializes registered save barriers. Desktop close, explicit workspace navigation and export start flush transcript, visual and dubbing changes. Failed saves keep the app open and leave the draft intact. Autosave remains debounced rather than writing each character. Both Tauri `allow-close` and `allow-destroy` are required for the SDK's guarded close event. The desktop test caught and fixed this permission omission.

Arbitrary browser history/navigation outside these application actions is not universally intercepted. Project-settings forms with an explicit Save button retain their existing form behavior.

## Runtime, diagnostics and download integrity

- Optional runtime status exposes `ready`, `missing`, `broken`, `update_required`; broken/obsolete installations can be repaired through the existing staged installer. Quick readiness checks files/ABI. **Kiểm tra lại** performs the deeper feature/backend verification; quick startup does not import every model.
- STTN model readiness now checks its pinned SHA-256, with a size/mtime keyed verification cache, instead of accepting size alone. Existing model/runtime dependency pins are retained to avoid untested cross-feature upgrades.
- Logs include a session ID, central credential redaction and rotation (10 × 10 MiB). Frontend failures are reported by code. Panic reports retain a local backtrace but omit the panic payload; raw crash reports are excluded from shareable diagnostics.
- Diagnostics include version/date/channel, schema/protocol/ABI, whitelisted recent job error codes, feature profiles/health and encoder availability. They exclude transcript text, media, API credentials, retry payloads, project names and raw error paths. Raw logs are reduced to safe diagnostic summaries. This conservative sanitization may remove useful detail; private local logs can be reviewed separately.
- Direct TikTok download uses bounded retries, strong ETag/Range continuation, validates each HTTPS redirect before connecting, validates Content-Range and media headers, and performs a bundled FFprobe check before publication. Offline fixtures cover transport interruption, ignored/bad Range, invalid content and unsafe redirects. A killed process may retain a partial; normal failure/cancel removes owned partials. This does not prove live authenticated TikTok compatibility on every network.

## Verification and release gates

Machine: Windows x64 development environment, Intel Iris Xe. This machine does not provide NVIDIA, AMD or hybrid-GPU coverage. Tests and generated fixtures use isolated directories. Measurements are warm developer runs; they are not clean-machine cold-start claims.

Detailed measured results live in `benchmarks/task10-desktop-recovery.json`, `benchmarks/task10-packaging.json` and local `.release-cache/task10-*.log` files. The release checklist below distinguishes actual tests from outstanding gates.

### Completed local automated coverage

- Rust: recovery rotation, semantic/future-schema rejection, corrupt/missing JSON, registration persistence failure, cancelling/late events, session crash marker, exclusive Windows lock, junction/traversal, source relink and 5,000 segments.
- Rust integration: real Unicode H264/AAC import, embedded subtitles, blur → subtitle → logo rendering and delayed audio mixing; actual merge copy/transcode with CPU H264/HEVC and available Intel HEVC.
- Frontend: save barriers, retry after failed save, subtitle timing/layout and existing editor tests.
- Python: offline downloader Range/redirect/FFprobe fixtures and existing ASR/OCR/TTS/worker protocol tests. Optional dependency tests skip when the corresponding interpreter lacks those packages; no API charges or large model downloads are triggered by unit tests.
- Long audio benchmark: 100 voices, 8,192-byte maximum PCM block, and 2-hour stream iteration. This is a synthetic mixer throughput test, not a full 2-hour rendered movie or ASR accuracy evaluation.

### Required before Stable

| Gate | Current limitation / next check |
| --- | --- |
| NVIDIA / AMD / hybrid | Run NVENC/AMF, CUDA inference, device loss/OOM and switch/fallback on those actual machines. Intel/CPU results cannot establish these. |
| Clean Windows 10/11 / standard user | Install without Python/Node/FFmpeg, missing WebView2, offline install, Unicode install path, launch and full legal-fixture workflow. Extracted payload tests are not installation tests. |
| Upgrade/uninstall | Upgrade a real older installation preserving projects/models, verify locked files, uninstall data policy and reinstall on a separate test machine. No user installation is replaced here. |
| Code signing | Local beta is unsigned; obtain and verify Authenticode signing before public Stable distribution. |
| Providers/browser login | Live Gemini/Google/Codex and authenticated TikTok require configured accounts and explicit cost/login testing. Offline fake tests do not prove network availability. |
| Disk/permissions | Real Windows ACL denial and physical disk-full/crash/power-loss tests on a disposable volume; current tests inject filesystem write failures and use preflight checks. |
| Long sessions/media | 4K, variable-frame-rate, damaged sources, hour-long exports, sustained preview memory and audio drift on each target machine. Synthetic stream tests do not replace these. |
| Full typed errors | Central classifier and structured critical IPC/job errors are present; legacy commands retain string errors for compatibility. Distinct codes for every provider/runtime failure remain an incremental improvement. |
| Crash diagnostics | Rust panic hook cannot capture every native driver/access-violation crash. Native minidump capture and bounded crash-report retention remain work. |
| Lint/security | Compiler warnings are fixed. Existing repository-wide formatting/Clippy style debt remains; npm audit is clean. Cargo/pip audit tools were unavailable in this environment. |

## Safe update strategy (design only)

There is no automatic updater enabled in this release. Users download a versioned installer; project/model data remain outside the install directory. Future updater work must use a signed Tauri update manifest and pinned public key, HTTPS, version/channel checks, explicit consent, pending-save/job barriers, package hash/signature verification before launch, retained previous installer, and rollback without downgrading project schemas. App, AI worker, downloader, protocol, optional ABI and schema compatibility must be checked together. Models and optional runtimes are independent installations and must not be downloaded again by every application update. Never execute an unsigned or merely downloaded updater payload.

## Reproduce

```powershell
npm run build
npm test
npm run test:ci
cargo test --manifest-path src-tauri/Cargo.toml --lib
$env:PYTHONPATH = 'python'
python -m unittest discover -s python/tests
npm run dev
# In a second terminal, with debug exe built and QA psutil env available:
node scripts/production-recovery-smoke.mjs
powershell -NoProfile -File scripts/build-release.ps1 -SkipTests -Compression zlib
```

`-SkipTests` is appropriate only after independently passing and recording the tests above. The opt-in desktop harness kills **its own** isolated app to test crash recovery, never a user's active instance. Model smoke tests use already-installed models offline; they do not silently download them.

Recommended channel: **Beta**, until all applicable release gates have evidence.



## Final local results (2026-10-04)

| Check | Result |
| --- | --- |
| Debug + optimized release | Passed; compiler emitted no warnings on final build |
| Rust normal suite | 178 passed, 22 opt-in tests ignored, 0 failed |
| Extra Rust integration/benchmark | Real merge, FFmpeg original/dialogue PCM parity, atempo attack/cache, 100 voices / 2-hour stream passed |
| Frontend | 27 passed across 7 files |
| CI script tests | 4 passed |
| Python base interpreter | 62 passed, 12 dependency skips (74 total) |
| Python QA interpreter | 82 passed, 7 dependency skips (89 total) |
| Desktop recovery | All 12 checks passed on the final debug build; real window close/reopen and kill fault injection |
| Frozen inference | Offline CPU faster-whisper medium, RapidOCR tiny and Vietnamese VieNeu PCM passed without system Python; same tests passed again from extracted Unicode-path payload |
| Payload | All four resource trees + license notices hashed against build resources; app exe matches the NSIS type patch |
| npm audit | 0 reported vulnerabilities |
| Clippy | Completes with 80 style warnings; not clean under deny-warnings |
| Formatting | Repository-wide check fails on existing and touched unformatted modules; new recovery modules formatted; no broad formatting-only rewrite |
| Cargo/pip security audit | Audit tools unavailable; no result claimed |

Local machine: Windows 11 Home Single Language build 26200, i5-13500H,
Intel Iris Xe driver 32.0.101.6790. No other OS/GPU was tested here.

Packaging:

- Installer: **368,392,253 bytes** (351.33 MiB), unsigned NSIS per-user installer.
- Task 9 comparison: 368,267,204 bytes; change 0.034%.
- Extracted installed payload excluding installer plugins: **1,008,955,332 bytes / 1905 files**. This is not total disk use after installing models/runtimes/WebView.
- AI worker: 441,930,380 bytes; downloader: 138,069,107 bytes.
- Optional installer/source: 68,262,825 bytes. Task 9's separately installed shared CPU Torch runtime remains about 3.55 GB; Task 10 did not install another runtime or download models.
- SHA-256: `414be2ea07eca84e3b25728a56af1cc370e97e479dc9d1ffebffde93ac634378`.
- App `0.1.14-beta.1`, AI worker `0.1.8`, downloader `1.0.1`, protocol `2`, optional ABI `7`.
- Final build stages total approximately 300 seconds with existing worker/toolchain caches. This is a local cached build, not a new CI timing guarantee.

Measurements:

- Project-manager readiness: 7.14s in the final debug/Vite run. Earlier isolated run was about 3.5s. Packaging/hash/test activity and disk cache were not controlled; no startup regression/improvement percentage is inferred.
- Initial native app RSS: 35.8 MiB; initial full app/WebView tree RSS: 674.7 MiB.
- 1,000 visible transcript editors on a two-hour generated source: 3.05s, tree RSS 1344.8 MiB. **The editor still renders all rows; memory is high and virtualization remains an improvement before a large-project Stable claim.** This test does not establish sustained playback FPS, ASR accuracy or a complete two-hour export.
- Three-second legal-fixture CPU H264/AAC export completed and FFprobe/source hash checks passed. It is a correctness fixture, not a representative 1080p/4K performance benchmark.
- 100-voice PCM mix: 5.12s for 100 seconds of frames, maximum block 8,192 bytes; two-hour empty stream iteration: 0.265s. Synthetic CPU throughput only.
- Closed-stdin warm worker medians: AI 1.043s, downloader 0.141s. No model imports or network in this measurement.

Preview FPS, prolonged idle/loaded memory, clean-machine upgrade/uninstall,
NVIDIA/AMD/hybrid failures and signed distribution remain release gates. There is
no claim that a passing local suite alone establishes production stability.


Extracted installer worker regression: CPU medium ASR 8.22s, OCR tiny 1.84s,
VieNeu Vietnamese TTS 5.54s producing valid 48 kHz PCM (2.96s). These small
fixtures confirm packaging/runtime execution, not full-video inference speed.
Both runtime trees remain independent; the base worker has no installed system
Python/Node/FFmpeg dependency on PATH. The harness itself is a developer test
runner and is not included as a requirement for end users.
