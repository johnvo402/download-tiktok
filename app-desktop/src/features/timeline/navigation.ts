import { clipEndUs, MAX_TIME_US, timelineToSourceUs } from './time';
import type { TimelineClip, TimelineDocument, TimelineTrack } from './types';

export const TRACK_HEIGHT = 72;
export const RULER_HEIGHT = 32;
export const MAX_CANVAS_WIDTH = 4_000_000;
export function formatTimelineTime(timeUs: number, precise = false): string {
  const total = Math.floor(Math.max(0, timeUs) / 1_000_000);
  const h = Math.floor(total / 3600), m = Math.floor(total / 60) % 60, s = total % 60;
  const body = `${h ? `${String(h).padStart(2, '0')}:` : ''}${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
  return precise ? `${body}.${String(Math.floor(timeUs / 1000) % 1000).padStart(3, '0')}` : body;
}
export const fitScale = (durationUs: number, width: number) => Math.max(1, width - 24) / Math.max(1, durationUs / 1_000_000);
export function limitScale(scale: number, durationUs: number, width: number): number {
  return Math.min(Math.min(1600, MAX_CANVAS_WIDTH / Math.max(1, durationUs / 1_000_000)), Math.max(fitScale(durationUs, width) / 4, scale));
}
export function zoomAt(scale: number, next: number, scroll: number, anchorX: number, maxScroll: number): number {
  return Math.min(maxScroll, Math.max(0, (scroll + anchorX) / scale * next - anchorX));
}
export function timeAtPixel(x: number, scale: number, durationUs: number): number {
  return Math.round(Math.min(durationUs, Math.max(0, x / scale * 1_000_000)));
}
export function rulerTicks(scale: number, scroll: number, width: number): number[] {
  const desired = 90 / scale;
  const magnitude = 10 ** Math.floor(Math.log10(desired));
  const step = [1, 2, 5, 10].map(v => v * magnitude).find(v => v >= desired)!;
  const from = Math.max(0, Math.floor(scroll / scale / step));
  const count = Math.ceil(width / scale / step) + 2;
  return Array.from({ length: count }, (_, i) => Math.round((from + i) * step * 1_000_000)).filter(time => Number.isSafeInteger(time) && time <= MAX_TIME_US);
}
export const orderedTracks = (doc: TimelineDocument) => [...doc.tracks].sort((a, b) => b.order - a.order || a.id.localeCompare(b.id));
export function visibleClips(clips: TimelineClip[], trackId: string, fromUs: number, toUs: number): TimelineClip[] {
  return clips.filter(c => c.trackId === trackId && c.startUs < toUs && clipEndUs(c) > fromUs);
}
/** Solo affects audible tracks only. A hidden/disabled track cannot suppress others. */
export function trackAudible(track: TimelineTrack, tracks: TimelineTrack[]): boolean {
  return track.kind !== 'subtitle' && track.enabled && !track.muted
    && (!tracks.some(t => t.kind !== 'subtitle' && t.enabled && t.solo) || track.solo);
}
export function activeClips(doc: TimelineDocument, timeUs: number): TimelineClip[] {
  const tracks = new Map(doc.tracks.map(t => [t.id, t]));
  return doc.clips.filter(c => tracks.get(c.trackId)?.enabled && timelineToSourceUs(c, timeUs) !== null)
    .sort((a, b) => (tracks.get(a.trackId)?.order ?? 0) - (tracks.get(b.trackId)?.order ?? 0) || a.startUs - b.startUs);
}
