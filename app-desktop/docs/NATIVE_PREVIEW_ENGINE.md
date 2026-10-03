# Task 6 — Native Preview Engine

## Architecture

The Translation viewport now defaults to native GPU preview on Windows. An owned
Win32 child HWND occupies the measured WebView viewport. Its DX12 wgpu swapchain
is presented directly; no frame, screenshot, image URL or PCM buffer is sent over
Tauri IPC. The DOM provides controls, accessibility and normalized drag hit areas.

```text
Project + Transcript
  -> build_render_specification -> RenderScene
  -> existing Renderer (same passes, text stack and visual order)
       Preview: MF/D3D11 decoder -> shared DX12 texture -> native swapchain
       Export:  Task 5 decoder -> shared DX12 texture -> hardware encoder
```

The new `Renderer::present_preview` changes the output target only. Blur, edge
feather, noise, subtitle font/layout, text and logo all use the Export passes.
Order remains clean source/patch -> blur -> subtitle -> text -> logo. Preview of
an existing STTN cleaned clip uses that clip as decoder input, with its timestamp
offset; subtitle/overlays still use absolute project time. No AI model code changed.

Preview allows unfinished projects without translated text or an assigned subtitle
region. Export validation remains strict. Preview never silently assigns a subtitle
region or adds global bottom-centre subtitles.

## Modules and files

| Area | Files |
| --- | --- |
| Native lifecycle, bounded mailbox and worker | `src-tauri/src/preview/{mod,manager,commands}.rs` |
| Win32 surface, pointer bridge and file drop | `src-tauri/src/preview/surface.rs` |
| Monotonic clock and audio anchoring | `src-tauri/src/preview/clock.rs` |
| Canonical scene and in-memory patches | `src-tauri/src/preview/scene.rs` |
| FFmpeg staging fallback and quality policy | `src-tauri/src/preview/{software,quality}.rs` |
| Task 5 decoder/conversion/fences reused | `src-tauri/src/media_gpu/windows/{preview,decoder}.rs` |
| Shared GPU resources/target adaptation | `src-tauri/src/render/{graph,device,renderer,preview_target}.rs`, `passes/{blur,composite}.rs` |
| Frontend lifecycle and audio-only source | `src/features/workspace/translation/{native-preview.ts,TranslationView.tsx}` |
| Interaction-only DOM, stable transcript source identity | `src/features/overlays/OverlayCanvas.tsx`, `TranslationPanel.tsx`, `src/styles.css` |
| Registration/path/lifecycle integration | `src-tauri/src/{lib,commands,overlays_service,render_spec}.rs`, `Cargo.toml`, `Cargo.lock` |
| Tests, development watch exclusions | `preview/tests.rs`, `media_gpu/windows/metrics.rs`, `scripts/native-preview-smoke.mjs`, `vite.config.ts` |

Only new direct dependency: `raw-window-handle` 0.6. Win32 bindings use the existing
`windows` dependency, with the required window/input/shell/diagnostic feature flags.

## Playback and resources

- A dedicated `native-preview` thread owns decoder, surface, scene and clock.
  Tauri commands enqueue updates and return; source loading/teardown are dispatched
  through `spawn_blocking`.
- The live display Instance/Adapter/Device/Queue are cached across project switches.
  The existing adapter resolver runs with LowPower preference for display stability;
  Export independently keeps its encoding/HighPerformance policy. No CUDA is required.
- Hardware path reuses Task 5 Media Foundation hardware decoder, NV12 conversion,
  same-LUID D3D11/DX12 shared texture and fences. No full-frame CPU readback/upload.
- Hardware initialization/decode/seek failure switches to bounded FFmpeg staging
  upload into **the same wgpu renderer**. Only native surface/device failure or HDR
  falls back to the existing WebView/CSS compatibility path in Auto mode.
- Scheduling uses actual decoded PTS and `Instant`, never `frameIndex / fps`.
  The original `<audio>` element remains the audio master and reuses existing
  Web Audio original/TTS/BandIt gain graph. An unsupported audio codec can generate
  a project-scoped AAC audio-only cache file without transcoding video.
- Audio anchors correct drift above 25 ms; status/UI are throttled to 10 Hz.
  Source controls, volume, mute and fullscreen remain available.
- Seek flushes/repositions the warm MF source near a keyframe. A generation token
  discards stale work. Mailbox seek slots and per-object transforms retain only the
  newest request; scene patches merge by segment ID and ignore older versions.
- At pause the held frame is retained and rendered only after a scene update,
  guide, seek or resize. The worker sleeps on a bounded wake channel. Decode caches
  are bounded; text cache remains capped at 64 entries / 64 MiB.
- Drag sends only transform/selection geometry. Translation editing sends changed
  or deleted segments. Persistence follows existing debounce; no disk save per move.
  Stable source identity prevents visual edits from reopening video or transcript.
- Existing logo textures and uniform buffers are reused for transforms. Asset
  replacement changes the imported asset path, invalidating its texture. Blur
  uniform buffers update without shader recompilation; ROI textures change only
  when their dimensions require it. Selection guides never enter Export.
- Heavy AI/FFmpeg resource slots take priority: preview pauses its clock/audio
  while they are held. It does not automatically restart playback afterward.
- Project deletion/replacement closes its preview before changing source files.
  App exit joins the worker; surface is dropped before its HWND and context.
  Early surface initialization errors also destroy the HWND.

## UI/surface integration

Bounds use CSS logical coordinates plus devicePixelRatio; the HWND and swapchain
use physical pixels. Resize/DPI/fullscreen changes reconfigure the surface, without
reopening decoder or device. Rendering preserves contain aspect ratio with black bars.

Windows HWND airspace does not support HTML painting over a native child. Actual
selection borders/resize handles are consequently drawn by wgpu; tiny native mouse
events feed the existing DOM hit areas/normalized overlay editor. Dialogs, menus,
job panels and media error/status overlays temporarily hide the child, preventing
it from covering controls. Native file drops use WM_DROPFILES and the existing
replace confirmation/import flow. Numeric and keyboard controls remain alternatives.

## Commands and events

```text
preview_open(projectId, bounds, clean?) -> sessionId
preview_close(sessionId)
preview_control(sessionId, action, time?)  # play/pause/seek/audio
preview_resize(sessionId, bounds)
preview_update_scene(sessionId, version, visual?, translations?, guide?, transform?)
preview_status(sessionId)
preview_audio_path(projectId)
```

Events: `preview-status`, `preview-pointer`, `preview-drop`. Commands validate
session ownership/window label, source/project/cache paths, timestamps, normalized
regions and update sizes. Frontend cannot provide arbitrary native handles. Status
includes adapter, decode mode, frame PTS, frames, dropped frames, CPU-side decode,
render/present/seek wall-clock latency and render scale. Metrics are not shown in
normal product UI and are **not GPU timestamp-query timings**.

## Internal options

- `VITE_PREVIEW_ENGINE=auto|gpu|compatibility` (Vite build/development option).
  Default Auto attempts native then compatibility. GPU reports failure without
  silently choosing CSS; Compatibility explicitly retains the old path.
- `JC_PREVIEW_QUALITY=auto|full|half|quarter` (process environment).
  Auto starts Full through 1080p, choosing a smaller target for higher resolutions
  when the physical viewport is smaller. After 60 playing samples, average
  render/present cost above 1.25 times the source frame budget reduces scale.
  Adaptation changes render targets/decoder output sizing, not project/export settings.
- Full is the exact pre-encode pixel parity mode. Half/Quarter use the same scene,
  wrapped text and passes, with scaled rect/font/outline/sigma; integer rasterization
  can differ slightly at small sizes. Export always has pixel_scale 1.

## Reproducible tests

```powershell
npm run build
npm test
npm run test:ci
cargo test --manifest-path src-tauri/Cargo.toml --lib
$env:PYTHONPATH = (Join-Path (Get-Location) 'python')
python -m unittest discover -s python/tests -v

# Opt-in real GPU/surface tests; not required on headless CI.
$env:JC_PREVIEW_STRESS_SECONDS = '120'
cargo test --manifest-path src-tauri/Cargo.toml --lib preview:: -- --ignored --nocapture --test-threads=1
```

Pixel comparison renders the same timestamp/input/blur/logo/text/Unicode subtitles
through Export RGBA and Preview BGRA targets, normalizes channel order and asserts
exact equality. Snapshot is written only by this test to
`.release-cache/preview-validation/parity.png`, not streamed into the app.

`create_desktop_preview_fixture` creates an isolated 10-second H.264/AAC QA project.
For the opt-in developer WebView integration test, start Vite and the debug app with
`JC_QA_DATA_DIR` pointing to `.release-cache/preview-validation/data/com.ai.video-localization`,
`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9224` and a separate
`WEBVIEW2_USER_DATA_FOLDER`; then run `node scripts/native-preview-smoke.mjs`.
It refuses project lists containing non-QA names. This test exercises play/pause,
audio clock, rapid seek, in-memory text/blur edits, resize and modal coexistence.
The QA root override is ignored by release builds. CDP is never enabled by app code.

## Results and limits

Verified locally on Windows 11, Intel Iris Xe, driver 32.0.101.6790, wgpu DX12:

- Full Preview/Export pixel equality: pass, including transparent logo, blur,
  Vietnamese/Chinese text and subtitle rendering.
- Native H.264 decode/present, 300 frames; 60 seek/resize operations at DPR
  100/125/150/200%; seek into a 3605-second source: pass.
- 720p / 1080p / 4K Full / 4K Quarter, 90 frames each, actual swapchain and
  software staging fallback: pass. Measured render+present averages were
  4.52 / 5.64 / 4.55 / 4.16 ms in the recorded run. These are fixture measurements,
  not universal performance guarantees.
- 20 open/close lifecycles: handles 418 -> 408. Two-minute continuous playback:
  5724 presents; handles 611 -> 604, threads 21 -> 16, RAM 504 -> 420 MiB,
  DXGI local GPU usage 125.5 -> 131.3 MiB. No unbounded growth in this test.
- Developer app playback test: hardware decode; audio drift 0.030 s at the sampled
  UI/status instant; latest seek 27.7 ms; scene edits preserve decoder session.
- Windows computer-use checks on the isolated QA project: select and drag blur,
  logo and text with their Properties panels staying open,
  resize its handle (80%/25% -> 83%/31%), resize the preview/panel divider,
  enter/exit fullscreen, edit a translation and continue typing without clicking
  again, open/close Project Settings. GPU subtitles update/wrap immediately;
  the native surface hides for the dialog and returns afterward.
- Frontend build and optimized Rust release build: pass. Rust suite: 123 passed,
  16 hardware/opt-in tests ignored; frontend: 25 passed; release-script tests:
  4 passed; Python: 57 discovered, 8 skipped. Real GPU pixel parity was also
  explicitly run and passed, separately from the default Rust suite.

Remaining hardware coverage: NVIDIA/AMD, hybrid/multiple-monitor DPI and HDR.
HDR/native non-Windows use compatibility preview. Exact audio/pixel behaviour on
all source codecs is not implied by the H.264/AAC fixtures. Existing TTS/BandIt graph
is reused; no new real-model inference was run for this task. GPU/device-loss and
corrupted-source fallback need broader physical-machine testing. The native child
approach intentionally hides its contents for overlapping WebView popups.

Recommended Task 7: hardware/release validation matrix, hybrid adapter policy,
native popup/drag/drop/accessibility regression coverage, preview-quality controls
in Advanced Settings, and refined audio buffering/diagnostics. Keep compatibility
preview until that coverage is complete.
