# Windows release E2E and upgrade matrix

Run these scenarios with the generated NSIS installer on Windows 10/11 x64.
For a clean-install pass, use a disposable standard user account or VM without
Node.js, Rust, Python, FFmpeg, or FFprobe. Use short owned/public-domain video
fixtures. Keep a signed-off result sheet with OS build, app version, installer
SHA-256, model revision, and pass/fail evidence.

## Results from the current build host

| Scenario | Result | Evidence / limitation |
|---|---|---|
| Frontend, Rust, and Python automated tests | Pass | See release build output. |
| Frozen worker version and JSONL ASR/TTS capability inspection | Pass | Worker starts without Python on PATH; model caches are empty; CUDA reports unavailable. |
| FFmpeg/FFprobe smoke and render unit/integration tests | Pass | Pinned resource binaries and renderer tests. |
| Clean Windows VM install, first-run project UI | Not run | No clean VM is attached to this task. |
| Real Whisper inference and model download | Not run | No model cache was downloaded for this test run. |
| Real VieNeu inference and TTS model download | Not run | No multi-GB model cache was downloaded for this test run. |
| Cloud translation with owned API credentials | Not run | No credentials were supplied. |
| NVIDIA CUDA inference | Not run | This release worker is CPU-first and the host reports CUDA unavailable. |
| Installer upgrade/uninstall on a clean account | Not run | Requires a disposable Windows user/VM to verify preservation safely. |

“Not run” items remain release qualification work; do not treat them as passed
based only on a successful installer build.

## Functional matrix

1. **Install/start:** Install per-user as a standard user. On a clean PC, verify
   WebView2 bootstrapper behavior. Launch to Project Manager. Runtime Health must
   report storage, FFmpeg, FFprobe, and AI Worker available while both model
   caches are absent. The application must not require dev tools or PATH edits.
2. **Project persistence:** Create, rename, close, and reopen a project with a
   Vietnamese/Unicode name. Confirm UUID-based storage, no data loss, and a
   successful delete confirmation for only that project.
3. **Embedded text subtitle:** Import a short MP4 with an embedded SRT/ASS text
   track. Check FFprobe metadata, select a track, extract it, edit a segment,
   save, close, and reopen. No ASR model should be required for extraction.
4. **No embedded subtitle:** Import a video with audio but no subtitle stream.
   Confirm the user can install Whisper Small on demand, choose CPU, generate
   timestamped transcript, edit it, and reopen it. Confirm partial download is
   not shown as installed.
5. **Hard-subtitle replacement:** Add a blur over the baked-in subtitle, mark
   only that region Subtitle Blur, enable translated subtitles, translate/edit,
   and export. Verify the original is blurred, translation is sharp and inside
   the blur region, and no global bottom fallback appears.
6. **Subtitle disabled:** Export with blur enabled and translated subtitle
   disabled. Blur remains and no translated event is burned in.
7. **Dubbing/mix:** Test original plus dubbing, original muted, ducking enabled,
   timestamp gaps, and one missing TTS clip. Verify synchronization and that
   missing clips are reported without corrupting the project.
8. **Portrait:** Repeat subtitle-in-blur export on 9:16 content. Check rectangle,
   line wrapping, Unicode, and font fit after export with FFprobe and a frame
   review.
9. **Cancel/recovery:** Cancel model download and a longer export; force close
   during a copied/importing video, model download, and export. On restart,
   projects remain readable, incomplete outputs are not accepted, and jobs are
   marked recoverable/interrupted.
10. **Offline:** After models are installed, disconnect network. Verify local
    subtitle extraction, CPU transcription, TTS, and export remain usable;
    cloud providers show a network error and app startup still succeeds.
11. **Hardware:** On a non-NVIDIA machine, open the app and run CPU mode. On a
    supported NVIDIA test host, separately verify actual CUDA device detection
    and inference. Never infer GPU support from a device-name string.
12. **Failure paths:** Try invalid provider credentials, network timeout,
    insufficient disk, unwritable export folder, locked input/output, malformed
    project/transcript JSON, and a missing runtime resource. Errors should be
    actionable; source files and prior completed batches stay intact.
13. **Unicode/long paths:** Test paths containing `Dự án`, `视频`, `日本語`,
    spaces, and a long parent path. Verify import, transcript, TTS and export.
14. **Scale/performance:** Use a 30-minute media file, 1,000+ transcript
    segments, and hundreds of TTS clips. Check editor responsiveness, save
    debounce, filter script length, memory growth, child-process cleanup, and
    export progress. Capture measurements rather than assuming a pass.
15. **Diagnostics/secrets:** Export a diagnostics bundle and inspect it before
    sharing. It should exclude media, transcript text, user paths, API keys, and
    reference voice audio. Check log rotation at the configured size limit.
16. **Upgrade/uninstall:** Install a previous local version, create projects,
    settings, credentials, and model caches, then upgrade. Confirm migration
    backups exist and projects, settings, models, and OS-held credentials remain.
    Uninstall, reinstall, and confirm user data remains outside the install tree.

## ASS regression input

Include translated text containing `{braces}`, `\\backslash`, `%percent%`, a
manual line break, and Vietnamese diacritics. Expected output is escaped safely,
untruncated, sharp, and clipped only to the calculated subtitle text area inside
the selected blur rectangle.
