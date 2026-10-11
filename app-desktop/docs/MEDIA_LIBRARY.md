# Media library (T02)

The Translation screen's Media tab holds the project source inventory. The file picker accepts multiple files; WebView and native preview drops import every supplied path. Each file is an independent transaction: an error leaves that source unimported while the remaining files continue. The UI reports progress, shows individual errors and retries only failed paths. One batch accepts at most 100 files.

Supported inputs:

- Video: MP4, MOV, MKV, WEBM, AVI, M4V.
- Images: PNG, JPG/JPEG, WEBP; decoded dimensions are limited to 16,384 pixels per side and 64 million pixels.
- Audio: MP3, WAV, M4A, AAC, FLAC, OGG, OPUS. Embedded cover art is not treated as a video stream.

FFprobe reads stream metadata, duration and video FPS. Image decoding also validates the image payload. Imported files are staged, hashed during copying and probed before persistence. New sources live at `media/assets/{assetId}/source-{fileId}.{extension}`. Existing legacy source paths remain valid. Video assets receive an independent `VideoContent` with project language/voice preferences, empty transcript/translation results and no copied overlays from another video.

SHA-256 and media kind identify duplicate sources. Reimporting an available matching source returns its existing asset ID without copying or adding a clip. Reimporting a missing or corrupt matching source repairs its file reference with the same asset/content IDs. Explicit relink requires the original checksum and kind; a different source is rejected before changing the project. Relink preserves clip placement, trims, transcript files and generated voice references. Old missing/corrupt paths are retained for recovery rather than deleted while they may be shared.

“Thêm vào timeline” creates another clip referencing the asset and video content; it does not copy media or AI results. Videos append to the first unlocked visual track, while images use a separate visual layer with an initial five-second duration. Audio/image placement uses the current playhead; overlapping audio receives another track. The first video added becomes the transitional legacy source, keeping the existing one-video preview and processing commands usable. Clip editing, per-video AI selection and multi-source preview/export remain T03–T09 work.

## Commands and persistence

- `import_media_assets(projectId, paths)`: latest project snapshot and per-file imported/duplicate/restored/failed results; emits `media-import-progress` with project ID, index, total and filename.
- `inspect_media_assets(projectId, verify)`: available/missing/invalid statuses. Normal inventory loading checks presence and size; explicit verification hashes unchanged files once using the integrity cache.
- `relink_media_asset(projectId, assetId, sourcePath)`: restore the exact original source.
- `append_media_clip(projectId, assetId, atUs)`: persist another source reference at integer microsecond placement; null placement appends on the selected track.
- `get_media_cache(projectId, assetId, kind)`: thumbnail or waveform on demand.

Library mutations read and edit the latest project under the project save mutex. A full-project save with an outdated `updatedAt` fails rather than erasing newly imported sources. The existing timeline validation, legacy reconciliation and atomic JSON/backup store still run before committing changes. Failed metadata persistence removes the newly copied source file. Media edits are blocked while the project has active transcription, translation, voice or export jobs.

## Derived caches

Caches live at `cache/assets/{assetId}/{sourceChecksum}/`. Cards request thumbnails only when visible; waveform decoding runs only when requested. Thumbnails are validated PNGs up to 320×180. Waveform schema 1 stores 512 peak buckets over the source duration, derived from streamed 8 kHz mono PCM. PCM temporary files are removed after completion/failure and peak aggregation uses bounded memory. Cache versions and source checksum isolate generated data; malformed thumbnail/waveform caches are rebuilt. These caches do not render a timeline video or change the source.

## Verification

Rust integration tests use real FFmpeg video/audio and decoded image fixtures. They cover mixed imports, partial failure, duplicate detection, stale-save rejection, repeated clips, image/audio placement, Unicode paths, missing/corrupt source detection, wrong-source relink rejection, moved-source recovery, transcript/TTS preservation, cached thumbnail/waveform reuse and corrupt-cache rebuilding. Browser checks exercise Unicode search, type filters, repeated source usage, waveform display surviving project updates, relink, partial-import retry and a narrow panel without horizontal overflow.
