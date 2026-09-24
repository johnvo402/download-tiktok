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

## Tauri and frontend packages

Third-party notices for Rust crates and npm packages are available from the
package lockfiles and their upstream package metadata. Release owners should
generate and review the complete license inventory for the exact shipped
installer before distribution.
