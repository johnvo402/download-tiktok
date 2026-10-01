# Intel OCR acceleration

## Usage

Open **Cài đặt → Nhận dạng lời thoại → Đọc phụ đề trong hình**.
OCR device choices are **Tự động**, **CPU**, **NVIDIA · CUDA**, and
**Intel GPU · OpenVINO**. Choosing Intel before installation opens the small
optional pack confirmation. The existing Tiny/Small/Medium model installation
buttons remain separate; installed weights are reused. Save the OCR settings.

Auto uses the centralized priority `CUDA → OpenVINO GPU → ORT CPU`.
CPU retains the existing ONNX Runtime path. Manual CUDA and manual Intel never
silently use CPU. The priority is a parameter of `resolve_ocr_backend`; an
OpenVINO CPU option is deliberately not enabled in this task.

## Implementation

- `python/worker/ocr/backends.py`: typed capabilities, real probes, resolver.
- `python/worker/ocr/openvino_engine.py`: GPU execution adapter.
- `python/worker/ocr/service.py`: compatible entry points and diagnostics.
- `src-tauri/src/optional_runtime.rs`: feature composition and staged installation.
- `src-tauri/src/runtime_paths.rs`: OCR-specific interpreter selection.
- `src-tauri/src/asr_service.rs`: generic capability response and OCR jobs.
- `src-tauri/src/settings_service.rs`: settings validation, preserve OCR Auto.
- `src-tauri/src/text_removal_service.rs`: OCR scan routing and GPU job labels;
  STTN itself still uses its existing CPU/CUDA implementation.
- `src/features/workspace/settings/IntelOcrRuntimeCard.tsx`: installation/status.
- Settings, transcript types, services and transcript confirmation accept Intel.

RapidOCR 3.9.2's supplied OpenVINO implementation compiles on CPU. JC AI instead
uses an execution adapter that supplies OpenVINO detector/classifier/recognizer
constructors and reuses RapidOCR's processing and output assembly methods. No
site-packages changes or global factory patches are used. The ONNX dictionary
is streamed from model metadata without creating an ORT CPU session. Results
retain `boxes`, `txts`, `scores`; existing transcript and text-mask consumers
do not need a different result format.

OpenVINO compiles the same installed ONNX files once per engine. A lock protects
the engine and infer requests. Frames continue to be read individually; video
frames are not retained as a whole. Model initialization progress is emitted.
The existing Rust worker-process cancellation/progress mechanism is retained.

## Readiness checks

The probe discovers GPU identifiers from `Core.available_devices`, tries them
in deterministic sorted order, reads the existing tiny ONNX Add fixture,
compiles on the explicit GPU, verifies `EXECUTION_DEVICES` contains only GPU,
and actually runs float32 `1 + 2 == 3`. Device listing alone is insufficient.
Successful probes are cached for the worker lifetime; failure remains recheckable.
Every real OCR model's execution device is also checked. If a particular model
cannot compile or infer, manual Intel fails; Auto logs the failure, tries the
remaining backends in priority order and reuses the selected replacement.
Detailed exceptions go to diagnostics, friendly messages go to the interface.

## Runtime and models

```text
<app-data>/
├── models/rapidocr/{tiny,small,medium}/   # same weights for every backend
└── runtimes/advanced/
    ├── current/                        # one composed environment
    ├── current.json                    # installed feature/profile marker
    ├── staging/                        # temporary independent copy/new venv
    ├── previous/                       # temporary rollback backup
    ├── cache/                          # retained uv download cache
    ├── python/                         # managed Python shared by features
    └── compiled-ocr/                   # OpenVINO compiled-model cache
```

Legacy Tiny weights directly under `models/rapidocr` are still recognized.
The compiled cache is separate from model weights, counted inside AI components
storage, and never stored in a project. OpenVINO's cache keys include model and
compile/device information; unsupported cache storage falls back to in-memory
compilation, not to CPU execution.

`ocr_openvino` installs pinned official `openvino==2026.4.0`, telemetry 2025.2.0,
RapidOCR 3.9.2 and the required numerical/image packages. It does not install
Torch, TorchAudio, WhisperX, BandIt, STTN, Transformers or CUDA. A fresh pack
has ORT CPU for the compatible CPU fallback. Adding Intel to an existing runtime
preserves that runtime's CPU/CUDA profile, clones current into staging using
independent file copies, freezes existing package versions as constraints, and
installs only the missing Intel feature. CUDA ORT is not replaced by CPU ORT.
Dependency checks, checks for all retained features, the real Intel probe and
worker startup must succeed before atomic promotion. Failure retains current.
The installer cache is retained. Adding other optional features keeps the
existing installation policy, and retains Intel in the composed feature set.

`requirements-optional-ocr-openvino.in`, the `ocr-openvino` development extra,
pack staging validation and third-party notices were added. Release lock files
stay unchanged because OpenVINO is optional. PyInstaller explicitly excludes
OpenVINO; the default installer gains only small Python/source/UI changes, not
the OpenVINO DLLs. The standard staging script copies the new requirement and
adapter files into the installer resources.

Measured clean Windows pack: **517,711,416 bytes (~494 MiB)** for the venv,
excluding shared managed Python, retained download cache and model weights.
OpenVINO's compressed Windows wheel is about **84 MB**; a first installation
also needs common Python/image packages. Existing environments reuse these.
Cloning a large CUDA runtime temporarily requires another copy of that runtime
on disk, but does not redownload Torch/CUDA. Conflicting frozen requirements
fail safely rather than changing another model's dependencies.

## Verification (2026-10-01 / 2026-10-02)

Machine: Intel Iris Xe, Windows driver `32.0.101.6790`, OpenVINO 2026.4.0,
RapidOCR 3.9.2, ORT CPU 1.30.0. Real Add inference and the three Tiny OCR models
reported `GPU.0`. CPU/GPU recognized identical test text from identical crops.

| Synthetic subtitle fixture | ORT CPU | OpenVINO GPU |
| --- | ---: | ---: |
| 6 images, warmed inference | 871 ms/image | 556 ms/image |
| 12 images, separate run | 1,192 ms/image | 750 ms/image |

The first measured GPU engine initialization took about 32 seconds; the later
cache-enabled run took 3.7 seconds plus 3.7 seconds to warm inference. Results
are workload/driver dependent and were measured while development work ran;
they do not imply a fixed speedup for full videos. Preprocessing, polygons,
dictionary decoding and image I/O still run on CPU.

Run normal tests/build:

```powershell
npm run build
npm test
$env:LOCALIZATION_FFMPEG_DIR = (Resolve-Path 'src-tauri/resources/ffmpeg').Path
$env:PATH = $env:LOCALIZATION_FFMPEG_DIR + ';' + $env:PATH
cargo test --manifest-path src-tauri/Cargo.toml
$env:PYTHONPATH = (Resolve-Path python).Path
python -m unittest discover -s python/tests -v
python -m unittest discover -s scripts/tests -v
```

The worker's test interpreter needs the core dependencies for inference tests.
GPU/model tests are opt-in and skip when hardware or cached Tiny weights are
missing; they never download weights:

```powershell
$env:JC_AI_TEST_OPENVINO = '1'
$env:JC_AI_TEST_OCR_MODELS = '<app-data>/models'
python -m unittest discover -s python/tests -p 'test_ocr*' -v
python scripts/benchmark-ocr.py --models '<app-data>/models' --frames 12
python scripts/test-openvino-runtime.py --uv '<uv.exe>'
```

The last command explicitly downloads the small runtime into a disposable
environment, checks dependencies, verifies no heavy packages were installed,
probes real GPU inference, and executes the worker's JSONL OCR inspection.
It also exercises paths with spaces and Vietnamese characters. Unit tests cover
selection priority/manual failures, false GPU readiness, malformed probe output,
device verification, output compatibility, safe runtime copying/rollback, feature
validation and settings migration. Opt-in tests exercise the real OCR transcript
loop, progress, repeated-caption merging, real Auto selection and shared weight paths.

Passed in this workspace:

- Frontend production build; 23 Vitest tests.
- 104 Rust tests; 3 encoder hardware tests remain opt-in.
- 47 Python tests collected; 4 opt-in hardware cases skipped in the normal run.
- 28 OCR tests passed with Intel inference/model tests enabled.
- 3 CI cache script tests; optional-runtime release-source staging validation.
- Disposable Intel runtime installation, dependency check and real worker inspection.

Additional changed/new files: `src/app/services/optional-runtime-service.ts`,
`project-service.ts`, `translation-service.ts`, `src/features/transcript/types.ts`,
`SettingsView.tsx`, `OptionalAiRuntimeCard.tsx`, `TranslationPanel.tsx`,
`python/pyproject.toml`, `python/requirements-optional-ocr-openvino.in`,
`python/packaging/ai_worker.spec`, `scripts/build-optional-runtime.ps1`,
`python/tests/test_ocr_backends.py`, `python/tests/test_ocr_openvino_integration.py`,
`scripts/benchmark-ocr.py`, `scripts/test-openvino-runtime.py` and
`docs/THIRD_PARTY_NOTICES.md`.

## Limits / next task

Only Intel Iris Xe + Tiny was tested with real hardware here. Intel Arc, Small/
Medium and NVIDIA coexistence need a hardware validation matrix; CUDA selection
is covered by mocks and the original CUDA implementation remains intact.
No full Windows installer or manual GUI session was produced for this task.
There is no automatic Intel driver installation. Long first-time GPU compilation
and remaining CPU preprocessing can limit short-job gains. No acceleration of
Whisper, VieNeu, BandIt or STTN was added.

Suggested Task 3: validate Arc and Tiny/Small/Medium on real subtitle crops;
measure crop/shape batching and bounded dynamic shapes before changing defaults.
Generalize AI capability reporting to other features only after those engines
have independently verified hardware backends.
