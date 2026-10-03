# Task 5 — Windows native GPU export

Task 4 was committed as `ce21f63` (`feat: add wgpu visual renderer to project exports`).
Task 5 reuses that renderer, RenderSpecification, subtitle layout, project jobs,
audio graph, output validation and history. No schema, frontend command or AI
dependency changes are needed.

## Production path: Tier B

```text
H.264 / HEVC 8-bit SDR source
 → Media Foundation SourceReader + same-adapter DXGI manager
 → actual hardware NV12 D3D11 decoder surface
 → D3D11 VideoProcessor: NV12 → BGRA + GPU scaling
 → shared NT texture on the actual wgpu DX12 device
 → existing Renderer: blur → subtitle/text → logo
 → shared BGRA output
 → D3D11 VideoProcessor: BGRA → encoder allocator NV12 surface
 → actual hardware MF encoder → temporary video-only MP4
 → existing FFmpeg audio graph / AAC + video stream-copy mux
 → FFprobe validation → atomic non-overwriting publish / history
```

No rawvideo pipes, full-frame CPU upload/readback, CPU NV12 conversion or CPU
scaling occur in this native visual path. Text/logo assets still use cached CPU
preparation/upload. Compressed packets, disk I/O and audio processing use CPU.
A CPU-readable full-frame buffer is created only for the staging renderer API.

This is **Tier B GPU-to-GPU transport, not Tier A**. Tier A is represented in the
capability resolver but is not a production implementation. Tier C remains the
Task 4 staging bridge; legacy FFmpeg visuals remain the last fallback. STTN patch
sequences currently use Tier C. Merge Video and desktop preview are unchanged.

## Research and API choice

FFmpeg CLI hardware decode followed by rawvideo still crosses RAM. Shipping
libavcodec interop adds ABI/distribution dependencies to the current CLI bundle.
Media Foundation and DXGI already ship with Windows and accept D3D11 surfaces
without vendor SDKs. Driver/OS codec support is probed, not assumed.

Interop uses public wgpu-hal DX12 `raw_device`, `raw_queue`, `texture_from_raw` and
wgpu `create_texture_from_hal`. No wgpu internals or memory-layout hacks are used.
The direct Windows crate is pinned to **0.58.0**, matching wgpu-hal 27 COM types.
No CUDA, Intel or AMD SDK is required for this path.

Official references:

- [SourceReader DXGI manager](https://learn.microsoft.com/en-us/windows/win32/medfound/mf-source-reader-d3d-manager)
- [D3D11 hardware decoding](https://learn.microsoft.com/en-us/windows/win32/medfound/supporting-direct3d-11-video-decoding-in-media-foundation)
- [SinkWriter creation](https://learn.microsoft.com/en-us/windows/win32/api/mfreadwrite/nf-mfreadwrite-mfcreatesinkwriterfromurl)
- [Actual encoder MFT inspection](https://learn.microsoft.com/en-us/windows/win32/api/mfreadwrite/nf-mfreadwrite-imfsinkwriterex-gettransformforstream)
- [D3D12 shared handles](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/nf-d3d12-id3d12device-createsharedhandle)
- [Fence management](https://learn.microsoft.com/en-us/windows/win32/direct3d12/fence-based-resource-management)

## Modules and ownership

```text
src-tauri/src/media_gpu/
 capability.rs            Tier resolver, numeric vendor IDs, override policy
 frame.rs                 NV12, PTS/duration, color, adapter LUID metadata
 pipeline.rs              Preference, counters, preflight, bitrate policy
 windows/
  device.rs               Thread-affine COM/MF lifecycle; actual wgpu adapter
  decoder.rs              SourceReader, DXGI decoder buffers, timestamps
  processor.rs            GPU conversion/scaling, cached processor views
  interop.rs              Owned shared handles, public hal import, fences
  encoder.rs              Hardware MFT verification, bounded NV12 allocator
  pipeline.rs             Native decode/render/encode coordinator
  metrics.rs              Host process memory/CPU diagnostics
  path.rs                 Lossless Windows canonical-path → MF URL
  tests.rs                Real hardware tests / transport benchmark
src-tauri/examples/native-probe.rs
```

COM/DirectX unsafe calls are isolated under `windows/`; callers receive safe Rust
methods. Owned COM references/handles use RAII. An Rc lifecycle guard keeps MF
active until decoder, encoder and frames are dropped on the original worker.

## Adapter, hardware proof and synchronization

Intel/NVIDIA/AMD selections use PCI vendor IDs `0x8086` / `0x10de` / `0x1002`.
The actual wgpu DX12 LUID determines the D3D11 device and the DXGI manager supplied
to both MF components. No matching by name or assumption that adapter 0 is correct.

Decode rejects non-DXGI samples, different devices, non-NV12 textures and textures
without `D3D11_BIND_DECODER`. Encode inspects the actual SinkWriter encoder MFT:
category, D3D11-aware flag, NV12 input, nonempty hardware URL and adapter LUID when
published. A successful SinkWriter alone is not treated as hardware evidence.

Three shared input/output pairs and six NV12 encoder samples are allocated once.
Driver-owned decoder surfaces are retained until GPU reads finish. One-frame
lookahead derives VFR durations; no unbounded queues or full-video preload.

Shared D3D11/DX12 fences order conversion → render → conversion/encoder. Shared
textures return to COMMON; only a reused ring slot waits, rather than GPU idle
after every frame. Textures are initialized once on GPU before import. Decoder
array slices and coded padding (1088 coded / 1080 visible lines) are respected.

PTS uses signed 100ns units, normalized to the first presented frame. Delayed EOS
samples and encoder flush preserve frame count. Changed dimensions/format/color
are rejected. Cancel checks run between frames and in pool/fence waits. Teardown
drains pending reads before releasing resources; unpublished files are removed.
Pool/fence waits have 15s limits; blocking OS/driver COM calls cannot be preempted.

## Input, color and audio scope

Native input is NV12 8-bit SDR, with BT.601/709/2020 matrix and limited/full range.
Missing matrix uses SD/HD defaults. BT.709 full/limited and BT.601 limited plus
scaling have real pixel tests. BT.2020 SDR is implemented but not verified here.
HDR/P010, rotation, interlace, non-square SAR, unsupported containers/codecs and
non-origin display apertures use fallback; no silent HDR tone mapping.
Native HEVC decode requires an available Windows codec; hardware encode support
alone does not prove native decode availability.

Original/TTS volumes, delay, ducking, dialogue stem and AAC use the existing
FFmpeg graph. The temporary video is muxed with `-c:v copy`; it is not decoded or
encoded again. Existing FFprobe validation, filesystem checks, atomic publication,
cancel and history continue to apply. Source media is untouched.

## Overrides and fallback

```powershell
$env:JC_MEDIA_PIPELINE = "auto"       # Default: Tier B → Tier C → legacy
$env:JC_MEDIA_PIPELINE = "gpu-copy"   # Require native; error if unavailable
$env:JC_MEDIA_PIPELINE = "cpu"        # CPU frame transport; wgpu can still render
$env:JC_MEDIA_PIPELINE = "zero-copy"  # Unsupported: Tier A not implemented
$env:JC_RENDER_BACKEND = "auto"       # Existing Task 4 renderer preference
npm run tauri dev
```

`JC_RENDER_BACKEND=legacy` bypasses native and conflicts with forced gpu-copy.
Auto failure discards the native attempt and retries the staging path. Manual
encoder vendor selection is preserved. Native error codes are logged internally;
the UI receives readable operation errors. Native progress uses 0–90%, followed by
the final audio/mux stage. No installer runtime is added beyond Windows/driver.

## Diagnostics and real hardware measurements

Logs identify tier, adapter/LUID, decoder, encoder, color/range, surface pools,
frames, wall time and total FPS. Full-frame `upload_bytes` / `readback_bytes` exclude
glyph/logo assets. `gpu_copies` / `gpu_transport_passes` counts four base GPU
conversion/draw passes per frame; additional ROI effects are excluded.

Host decode/read, conversion/render-submit and encode/write/flush times are
reported separately. These are not independent GPU FPS because GPU stages overlap.
Runtime CPU % and sampled peak working set describe the Rust process, exclude FFmpeg
children and can include other app threads. The benchmark additionally includes CPU
time from the actual FFmpeg decoder/encoder process handles. The fixed surface memory estimate
excludes decoder/driver buffers, fonts and ROIs. Exact VRAM/GPU utilization needs
an external profiler; process samples do not measure whole-machine utilization.

Verified on **Intel Iris Xe, Windows 11 build 26200, DX12**, 2026-10-03:

- Actual H.264 decode and H.264/HEVC hardware encode; 1080p final audio exports.
  A separate 320x240 HEVC-source probe also decoded/re-encoded all 20 frames on GPU.
- Original/dubbing mixing, delayed TTS, ducking and dubbing-only tail padding;
  decoded PCM assertions confirm timing and level changes.
- VFR, B-frames, nonzero source video PTS, exact frame count and normalized PTS.
- Cancel after four frames, release/deletion, Unicode paths, unchanged source.
- Native encoded blur/subtitle/logo pixel parity with staging: MAE **0.196/255**;
  subtitle/logo remain sharp and logo is above blur/subtitle.
- GPU scaling, BT.709 full/limited and BT.601 limited: decoded MAE **<3/255**.

Final benchmark: 10s / 300 frames / 1080p H.264, identical wgpu blur, ten Unicode
subtitle events, alpha logo, Intel hardware encoding at 4 Mbps. Includes startup
and encoding; excludes audio and fixture creation. No other build was running.

| Measurement | Native Tier B | Staging Tier C |
|---|---:|---:|
| Elapsed | 2.070s | 5.257s |
| Total throughput | 144.9 fps | 57.1 fps |
| Average video pipeline CPU (Rust + FFmpeg children, normalized to all CPU cores) | 7.50% | 20.93% |
| Sampled Rust-process peak working set, excluding children | 575.4 MiB | 514.2 MiB |
| Full-frame CPU upload / readback bytes | 0 / 0 | 2,488,320,000 / 2,488,320,000 |

Native uses more fixed GPU surfaces; lower transport bytes do not imply lower
driver/process working set. Memory was sampled at 100ms intervals in one test
process, not measured as isolated system/VRAM peaks. Earlier blur-only runs measured
1.65–2.18s native vs 4.16–4.99s staging. This is not a universal speed guarantee;
there is no CI performance threshold. Short fixtures do not establish long-video
memory stability. Exact GPU utilization was not measured.

## Build and validation

```powershell
npm run build
npm test
npm run test:ci
cargo test --manifest-path src-tauri/Cargo.toml

# Opt-in real hardware suite; native tests target Iris Xe explicitly.
$env:JC_GPU_TEST_ARTIFACTS = Join-Path $PWD ".release-cache/native-validation"
cargo test --manifest-path src-tauri/Cargo.toml --lib gpu_ -- --ignored --nocapture --test-threads=1

# Video-only probe on the selected adapter, with a real source.
cargo run --manifest-path src-tauri/Cargo.toml --example native-probe -- source.mp4 native.mp4 1920 1080 h264

Push-Location python
python -m unittest discover -s tests
Pop-Location
cargo build --release --manifest-path src-tauri/Cargo.toml
```

Tests use isolated tempdirs and generated short fixtures. Logs/artifacts are ignored
under `.release-cache/`. Default CI ignores hardware tests and runs mocked capability
and metadata validation. Forced native hardware tests deliberately fail if an Intel
DX12 adapter or hardware codec is unavailable; they do not silently pass as proof.

Manual app checklist: export an existing project using Intel hardware, inspect the
Tier B log, check final blur/subtitle/logo and audio playback, cancel another export,
then compare forced `cpu` / `legacy`. Full GUI/clean installer testing was not done.

Known unverified cases: NVIDIA/AMD/hybrid hardware, 1080p HEVC source decode and Windows
without its codec, BT.2020 SDR, long videos, device removal, relative nonzero A/V
stream offsets, exact peak VRAM and STTN/BandIt inference end-to-end. Rust release
build produces the binary; this task does not create a new installer.

## Final results and next task

- `cargo test`: **116 passed, 12 hardware tests ignored** by default; **9 GPU tests
  passed** when explicitly run (the other three legacy backend tests remain opt-in).
- Frontend: **25 tests passed**, TypeScript/Vite production build succeeded.
- CI script tests: **4 passed**. Python regression: **57 tests, 8 skipped**, no failures.
- Final Rust release build succeeded without compiler warnings (1m48s incremental;
  first native build 3m24s). Binary: `src-tauri/target/release/ai-video-localization.exe`,
  **28,787,712 bytes**, versus Task 4's 28,665,344 bytes on this machine.
- `git diff --check` passed. Task 5 implementation remains in the working tree;
  the commit requested at the start of this turn contains completed Task 4.

Modified existing files: `src-tauri/Cargo.toml`, `Cargo.lock`, `src/lib.rs`,
`src/render/{device,graph,renderer,stream}.rs`, `src/render_service.rs`,
`docs/GPU_RENDER_ENGINE.md`. Added files are the `media_gpu/` tree shown above,
`src-tauri/examples/native-probe.rs` and this document.

Task 6 can reuse `Renderer::render_surfaces` for native preview, add visual regression
coverage against the DOM preview, and validate NVIDIA/AMD/hybrid adapters on real
machines. Tier A, HDR and vendor-specific interop remain separate future work.
