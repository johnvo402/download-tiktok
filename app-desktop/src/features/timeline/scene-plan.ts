import { audioRegions, captionAt, contentId, regionAudible, type AudioRegion, type SourceMedia } from './audio-layout';
import { activeClips } from './navigation';
import { clipEndUs, timelineDurationUs, timelineToSourceUs } from './time';
import type { TimelineClip, TimelineDocument } from './types';
import { visualAt } from './effects';
import { IntervalIndex, nextBoundary } from './interval-index';

export function playbackIndex(doc: TimelineDocument, regions: AudioRegion[] = []) {
  const tracks = new Map(doc.tracks.map(t => [t.id, t]));
  const enabled = doc.clips.filter(c => tracks.get(c.trackId)?.enabled);
  const audible = regions.filter(r => regionAudible(r, doc));
  return {
    clips: new IntervalIndex(enabled, c => c.startUs, clipEndUs),
    audio: new IntervalIndex(audible, r => r.startUs, r => r.endUs),
    tracks, assets: new Map(doc.assets.map(a => [a.id, a])),
    endUs: timelineDurationUs(doc.clips),
    boundaries: [...new Set([...doc.clips.flatMap(c => [c.startUs, clipEndUs(c)]), ...audible.flatMap(r => [r.startUs, r.endUs])])].sort((a, b) => a - b),
  };
}
type PlaybackIndex = ReturnType<typeof playbackIndex>;
function indexedActive(index: PlaybackIndex, timeUs: number) {
  return index.clips.at(timeUs).sort((a, b) => index.tracks.get(a.trackId)!.order - index.tracks.get(b.trackId)!.order || a.startUs - b.startUs);
}

/** A scene is evaluated from the document snapshot, never from the editor selection.
 * Keep this contract for export: half-open intervals, ascending layer order,
 * integer source timestamps, and independent clip instances of the same asset. */
export function timelineScene(doc: TimelineDocument, sources: ReadonlyMap<string, SourceMedia>, timeUs: number, regions: AudioRegion[] = audioRegions(doc, sources), index?: PlaybackIndex) {
  const active = index ? indexedActive(index, timeUs) : activeClips(doc, timeUs);
  const layers = active.filter(c => ['video', 'image', 'text', 'subtitles'].includes(c.content.kind)).map(clip => {
    const assetId = 'assetId' in clip.content ? clip.content.assetId : undefined;
    return {
      clip, sourceUs: timelineToSourceUs(clip, timeUs)!,
      ...visualAt(clip, doc, timeUs),
      asset: index ? index.assets.get(assetId ?? '') : doc.assets.find(a => a.id === assetId),
      text: clip.content.kind === 'text' ? clip.content.text : clip.content.kind === 'subtitles' ? captionAt(clip, sources.get(contentId(clip) ?? '')?.transcript, timeUs) : '',
    };
  });
  const audible = index ? [] : regions.filter(r => regionAudible(r, doc));
  const audio = index ? index.audio.at(timeUs) : audible.filter(r => r.startUs <= timeUs && timeUs < r.endUs);
  const endUs = index?.endUs ?? timelineDurationUs(doc.clips);
  // Stop at every boundary before evaluating the following scene, even after a slow RAF.
  const nextBoundaryUs = index ? nextBoundary(index.boundaries, timeUs, endUs)
    : [...doc.clips.flatMap(c => [c.startUs, clipEndUs(c)]), ...audible.flatMap(r => [r.startUs, r.endUs])].reduce((end, t) => t > timeUs ? Math.min(t, end) : end, endUs);
  return { timeUs, endUs, active, layers, regions, audio, nextBoundaryUs };
}

export const PREFETCH_US = 1_500_000;
/** Active layers are never capped. Only speculative decoders are bounded. */
export function residentClips(doc: TimelineDocument, timeUs: number, index?: PlaybackIndex): TimelineClip[] {
  if (index) return [...indexedActive(index, timeUs), ...index.clips.upcoming(timeUs, timeUs + PREFETCH_US, 6).sort((a, b) => a.startUs - b.startUs || a.id.localeCompare(b.id))];
  const active = activeClips(doc, timeUs), ids = new Set(active.map(c => c.id));
  const enabled = new Set(doc.tracks.filter(t => t.enabled).map(t => t.id));
  const next = doc.clips.filter(c => enabled.has(c.trackId) && !ids.has(c.id) && c.startUs > timeUs && c.startUs <= timeUs + PREFETCH_US)
    .sort((a, b) => a.startUs - b.startUs || a.id.localeCompare(b.id)).slice(0, 6);
  return [...active, ...next];
}

/** A native child surface cannot composite DOM layers. Use it only for an exact
 * single-video scene; the browser compositor handles all other layouts. */
export function nativeSceneClip(doc: TimelineDocument, layers: ReturnType<typeof timelineScene>['layers'], guides: boolean) {
  if (guides || layers.length !== 1) return;
  const { clip, asset } = layers[0]!, t = clip.transform, crop = t.crop;
  if (clip.transitionIn || clip.effects?.keyframes.length || clip.effects?.fadeInUs || clip.effects?.fadeOutUs) return;
  if (clip.content.kind !== 'video' || !asset || t.x !== .5 || t.y !== .5 || t.scaleX !== 1 || t.scaleY !== 1 || t.rotation !== 0
    || t.opacity !== 1 || t.flipX || t.flipY || t.fit !== 'contain' || crop.x || crop.y || crop.width !== 1 || crop.height !== 1
    || doc.settings.backgroundColor.toLowerCase() !== '#000000'
    || !asset.metadata.width || !asset.metadata.height || Math.abs(asset.metadata.width / asset.metadata.height - doc.settings.width / doc.settings.height) > .001) return;
  return clip;
}
