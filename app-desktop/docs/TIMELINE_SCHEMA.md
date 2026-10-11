# Timeline schema 1 / Project schema 12

T01 adds the persisted timeline foundation. Importing multiple sources, timeline UI and multi-source playback/export are implemented in later tasks.

## Ownership

`Project.timeline` is required for schema 12 documents returned by the backend. Older project JSON can omit it during deserialization, then migration supplies it before returning the project. `TimelineDocument` contains:

- `assets`: media file IDs, kind, relative path, integer duration, rational FPS, VFR knowledge and original probe metadata. Missing files do not invalidate the document; the operation using that source checks presence.
- `videoContents`: independent content IDs bound to video assets. Settings include transcript metadata/path, translation, subtitle/dubbing preferences, OCR settings and existing visual overlays. Segment revisions and generated TTS references remain in the transcript sidecar; migration never rewrites that file.
- `tracks`: stable IDs, visual/audio/subtitle kind, name, layer order and enabled/locked/muted/solo state. Ascending order composites back to front.
- `clips`: a reference to an asset/content, track, timeline start, source interval, rational playback rate, transform, linked parent and group ID. A video source can be used repeatedly without copying its file or transcript.
- `settings`: canvas dimensions, rational output FPS and background color.
- `legacyBinding`: the asset/content/clip used by existing single-video commands during the transition to T03.

The clip content union includes video, image, audio, text, subtitles and generated voice. Video clips must reference content belonging to the same asset. Subtitles/voice can link to a video clip with the same content ID. Independent visual/audio layers use timeline positions; edit transactions in T05 handle moving linked children.

## Time contract

Times are nonnegative integer microseconds (`1,000,000` ticks per second), at most `9,007,199,254,740,991`, so persisted numbers are exact in JavaScript. Rust performs multiplication with `u128`; TypeScript uses `BigInt`. Fraction numerators/denominators are positive `u32` values.

Source and timeline intervals are half-open: `[in, out)` and `[start, end)`. A point at a clip's end belongs to the next clip, never both. Playback rate is **source time / timeline time**. Duration is `ceil((out - in) × denominator / numerator)`; arithmetic mappings round positive halves up and clamp to the last tick within the interval. Caption interval mapping intersects the cue with the source trim before converting to timeline positions.

`src/features/timeline/time.ts` and `src-tauri/src/timeline.rs` implement the same rules. Both consume `test-fixtures/timeline-time.json` to check moved/trimmed clips, fractional rates, irregular source timestamps and large safe integers. Source VFR timestamps remain authoritative; source FPS is metadata, not a substitute for decoded timestamps. Output FPS is an independent setting.

Duration is derived from the largest clip end, including independent layers. An empty timeline has duration 0. The AI video studio's minimum duration is not applied to this editor model.

## Migration and recovery

Loading project schema 1–11 first validates the old document and preserves the existing migration rules. Before saving schema 12, it writes and syncs a `project.json.pre-v{oldVersion}-{timestamp}.bak` containing the exact original bytes. The new document uses the existing atomic recovery store and rotating validated backups.

A known video becomes one asset, one content and one visual clip at time 0, with the whole source interval and speed 1. Canvas/FPS derive from available source metadata. Project ID, original timestamps, transcript/audio paths, translation, OCR/blur regions, logo and text properties are retained. No media or generated speech is copied or regenerated.

A source with missing duration still becomes an asset and content, with a null bound clip until metadata is available. No artificial duration is assigned. A project without a source gets an empty timeline, preserving the AI video studio's source-free workflow. Migration failure retains the original main document; subsequent opens can retry. Unsupported future project or nested timeline schemas are never downgraded by backup recovery.

## Transitional synchronization

Until T03 moves AI commands to content IDs, the root `source`, transcript and editing preferences remain compatible with the current UI and services. `ProjectStore.update_project` reconciles changes through the legacy binding:

- Existing single-video commands update only the bound asset/content. Clip placement, trims, IDs and other source contents are preserved.
- Timeline-only changes to bound source settings update the legacy projection. Conflicting edits to both projections are rejected before saving.
- First import creates the source binding. Restoring the same checksum updates its path/metadata while preserving content and clip identity.
- Replacement by a different source creates new IDs and is rejected if other clips or content still depend on the bound source.

Existing visual overlay settings are stored within the video's settings during this foundation task. T06 will expose them as editable timeline layers with the corresponding time/transform rules.

## Validation

Reject duplicate/invalid IDs, unresolved references, cross-video content binding, unsafe paths, unsupported versions, invalid rates/canvas/transforms, empty or overflowing intervals, trims exceeding known sources, track type mismatches and overlapping clips on the same track. Layering uses separate tracks. Missing assets remain visible and recoverable. Validation runs before persistence; malformed documents cannot overwrite a valid project.
