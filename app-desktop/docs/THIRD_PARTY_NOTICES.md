# Third-party notices and release review

## FFmpeg

The Windows release build script pins the BtbN FFmpeg-Builds release
`autobuild-2026-09-22-13-18`, asset `ffmpeg-n8.1.3-win64-gpl-8.1.zip`, verifies
its published SHA-256 before extraction, and records the hash and version in
`ffmpeg-build.json`. The selected GPL build includes libx264 and is GPL-3.0-or-
later. FFmpeg contains other LGPL components as well. Upstream license and
corresponding source references:

- [FFmpeg legal information](https://ffmpeg.org/legal.html)
- [FFmpeg source/download information](https://ffmpeg.org/download.html)
- [Pinned BtbN build release](https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-09-22-13-18)
- [BtbN build recipes](https://github.com/BtbN/FFmpeg-Builds)

Build configuration, source revision, hash and included feature checks are
recorded in the generated resource manifest. Preserve the binary's license
notices and make corresponding source available as required by the applicable
licenses. Obtain legal review before public or commercial release.

## Python runtime

The release worker uses exact dependency pins in `python/requirements-release.txt`.
Build output includes `python-build-packages.json` with resolved distribution
versions and declared license metadata, plus package metadata/notices carried by
included upstream wheels. The frozen worker deliberately excludes the optional
Gradio UI and PyTorch/CUDA backend; it runs the CPU ONNX backend. Review the
generated inventory and upstream notices for each release.

The default worker is CPU-first. It does not include PyTorch, CUDA libraries,
model weights, or any API credential. VieNeu v3 Turbo's CPU route uses ONNX.
The installer carries only the package license metadata and files included with
the frozen runtime; `ai-worker/python-packages.json` identifies those exact
distributions. It is distinct from the release build environment lockfile.

## Models downloaded on demand

The installer contains no model weights. The current default model repositories
publish these terms: [faster-whisper-small (MIT)](https://huggingface.co/Systran/faster-whisper-small),
[VieNeu-TTS v3 Turbo (Apache-2.0)](https://huggingface.co/pnnbao-ump/VieNeu-TTS-v3-Turbo),
and [MOSS Audio Tokenizer ONNX (Apache-2.0)](https://huggingface.co/OpenMOSS-Team/MOSS-Audio-Tokenizer-Nano-ONNX).
Review the exact model repository card/license for every model before a public
release; user-downloaded weights remain subject to their upstream terms.

The optional STTN video-inpainting mode downloads its checkpoint on demand
(about 63 MiB) from the pretrained model linked by the official project. STTN's inference architecture in
`python/worker/text_removal/sttn_model.py` follows the upstream
[researchmm/STTN](https://github.com/researchmm/STTN) implementation, licensed
under MIT; the checkpoint is not bundled in the installer. The download is
verified against a pinned SHA-256 digest before installation.

## Tauri and frontend packages

Third-party notices for Rust crates and npm packages are available from the
package lockfiles and their upstream package metadata. Release owners should
generate and review the complete license inventory for the exact shipped
installer before distribution.

## Optional Intel OCR acceleration

The separately installed Intel OCR pack pins official OpenVINO 2026.4.0 and
OpenVINO Telemetry 2025.2.0 (Apache-2.0). It is excluded from the default frozen
worker and uses the same RapidOCR 3.9.2 ONNX weights/dictionary as CPU and CUDA.
The adapter reuses RapidOCR/Paddle detection, classification, recognition and
postprocessing classes (Apache-2.0); upstream packages are not modified.
Preserve upstream wheel LICENSE/NOTICE files in the optional runtime.

- [OpenVINO source and licenses](https://github.com/openvinotoolkit/openvino)
- [RapidOCR source and license](https://github.com/RapidAI/RapidOCR)
### Intel Whisper (optional)

OpenVINO GenAI 2026.4.0.0 and OpenVINO Tokenizers 2026.4.0.0 use Apache-2.0,
matching OpenVINO 2026.4.0. They are installed only on request, outside the base
worker. Official `OpenVINO/whisper-{tiny,base,small}-fp16-ov` multilingual IR
artifacts are pinned to repository commits in `worker/asr/openvino-models.json`.
Their model cards declare Apache-2.0; retain upstream notices when distributing
weights. Native inference needs no PyTorch or conversion tools.

Sources: https://github.com/openvinotoolkit/openvino.genai,
https://github.com/openvinotoolkit/openvino_tokenizers,
https://huggingface.co/OpenVINO/whisper-tiny-fp16-ov.
## Native GPU visual renderer

The desktop Rust binary links wgpu 27.0.1 (MIT/Apache-2.0), pollster 0.4.0
(MIT/Apache-2.0), bytemuck (MIT/Apache-2.0/Zlib), cosmic-text 0.18.2
(MIT/Apache-2.0) and image (MIT/Apache-2.0). Exact transitive versions are
recorded in Cargo.lock. No font files or graphics runtime/model downloads are
bundled by this renderer; text uses installed system fonts. Preserve upstream
licenses in the release dependency inventory.

- [wgpu](https://github.com/gfx-rs/wgpu)
- [pollster](https://github.com/zesterer/pollster)
- [bytemuck](https://github.com/Lokathor/bytemuck)
- [cosmic-text](https://github.com/pop-os/cosmic-text)
- [image](https://github.com/image-rs/image)
