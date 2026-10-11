import { describe, expect, it } from 'vitest';
import { IntervalIndex, nextBoundary } from './interval-index';
import { playbackIndex, residentClips, timelineScene } from './scene-plan';
import { audioRegions, type SourceMedia } from './audio-layout';
import { PreviewClock } from './preview-clock';
import { clipEndUs, frameStepUs } from './time';
import type { TimelineClip, TimelineDocument } from './types';

function longProject(minutes: number) {
  const transform: TimelineClip['transform'] = { x: .5, y: .5, scaleX: 1, scaleY: 1, rotation: 0, opacity: 1, flipX: false, flipY: false, fit: 'contain', crop: { x: 0, y: 0, width: 1, height: 1 } };
  const tracks = ['video', 'caption', 'music', 'overlay'].map((id, order) => ({ id, name: id, kind: id === 'caption' ? 'subtitle' as const : id === 'music' ? 'audio' as const : 'visual' as const, order, enabled: true, locked: false, muted: false, solo: false }));
  const clips: TimelineClip[] = [];
  for (let i = 0; i < minutes * 10; i++) {
    const video: TimelineClip = { id: `v-${i}`, trackId: 'video', startUs: i * 6e6, sourceInUs: 1e6, sourceOutUs: 7e6, rate: { numerator: 1, denominator: 1 }, content: { kind: 'video', assetId: 'asset-a', videoContentId: 'a' }, transform, linkedClipId: null, groupId: null };
    clips.push(video, { ...video, id: `caption-${i}`, trackId: 'caption', linkedClipId: video.id, content: { kind: 'subtitles', videoContentId: 'a' } },
      { ...video, id: `music-${i}`, trackId: 'music', content: { kind: 'audio', assetId: 'music' } });
  }
  clips.push({ ...clips[0]!, id: 'long-layer', trackId: 'overlay', sourceInUs: 0, sourceOutUs: minutes * 60e6, content: { kind: 'text', text: 'Logo' } });
  const doc: TimelineDocument = { schemaVersion: 1, settings: { width: 640, height: 360, frameRate: { numerator: 30000, denominator: 1001 }, backgroundColor: '#000000' }, legacyBinding: null, tracks, clips,
    assets: ['asset-a', 'music'].map(id => ({ id, kind: id === 'music' ? 'audio' : 'video', relativePath: `media/${id}.mp4`, durationUs: 8e6, frameRate: null, variableFrameRate: true, metadata: { videoPath: `media/${id}.mp4`, width: 640, height: 360, audioStreams: [{ streamIndex: 1, codec: 'aac', language: null, title: null, channels: 2, sampleRate: 48000 }] } })), videoContents: [] };
  const source: SourceMedia = { videoContentId: 'a', contentRevision: 0, transcript: null, voices: [], warnings: [] };
  return { doc, sources: new Map([['a', source]]) };
}

describe('long timeline stability', () => {
  it('interval queries preserve nested layers, ties, gaps and half-open edges', () => {
    const values = Array.from({ length: 2000 }, (_, i) => ({ id: i, start: i % 5 ? i * 40000 : 0, end: i * 40000 + 100001 }));
    const index = new IntervalIndex(values, v => v.start, v => v.end);
    for (let i = 0; i < 150; i++) {
      const from = (i * 7919) % 80e6, to = from + (i % 3 ? 1 : 120000);
      expect(index.intersect(from, to).map(v => v.id).sort((a, b) => a - b)).toEqual(values.filter(v => v.start < to && v.end > from).map(v => v.id));
    }
    expect(index.intersect(1, 1)).toEqual([]);
    expect(new IntervalIndex([], () => 0, () => 0).at(0)).toEqual([]);
    expect(nextBoundary([0, 10, 10, 20], 10, 20)).toBe(20);
  });
  it.each([30, 60, 90])('matches the scan contract on a %i minute multi-layer document', minutes => {
    const { doc, sources } = longProject(minutes), regions = audioRegions(doc, sources), index = playbackIndex(doc, regions);
    for (const t of [0, 5_999_999, 6e6, minutes * 30e6 + 123456, index.endUs - 1, index.endUs]) {
      expect(timelineScene(doc, sources, t, regions, index)).toEqual(timelineScene(doc, sources, t, regions));
      expect(residentClips(doc, t, index)).toEqual(residentClips(doc, t));
    }
    const started = performance.now();
    for (let i = 0; i < 1000; i++) {
      const t = (i * 7919_999) % index.endUs;
      const scene = timelineScene(doc, sources, t, regions, index);
      expect(scene.layers.length).toBe(3); expect(scene.audio.length).toBe(2);
      expect(scene.nextBoundaryUs).toBeGreaterThan(t);
      expect(residentClips(doc, t, index).length).toBeLessThanOrEqual(10);
    }
    // A deliberately generous CI budget; the measured time is printed for comparison, not a FPS promise.
    const elapsed = performance.now() - started;
    console.log(`T11 ${minutes}min / ${doc.clips.length} clips: 1000 indexed scenes ${elapsed.toFixed(1)}ms`);
    expect(elapsed).toBeLessThan(5000);
  });
  it('rebuilds after edits and keeps old snapshots independent', () => {
    const { doc, sources } = longProject(30), first = playbackIndex(doc);
    const next = { ...doc, clips: doc.clips.map(c => c.id === 'v-0' ? { ...c, startUs: 20e6 } : c), tracks: doc.tracks.map(t => t.id === 'caption' ? { ...t, enabled: false } : t) };
    const second = playbackIndex(next);
    expect(timelineScene(doc, sources, 0, [], first).active.map(c => c.id)).toContain('v-0');
    const ids = timelineScene(next, sources, 0, [], second).active.map(c => c.id);
    expect(ids).not.toContain('v-0'); expect(ids).not.toContain('caption-0');
    expect(first.endUs).toBe(Math.max(...doc.clips.map(clipEndUs)));
  });
  it('does not accumulate clock or fractional frame rounding over 90 minutes', () => {
    const clock = new PreviewClock(), total = 5400e6;
    let time = clock.advance(0, 0, 1, true, true, total);
    for (let i = 1; i <= 324000; i++) time = clock.advance(i * 1000 / 60, time, 1, true, true, total);
    expect(time).toBe(total);
    let frame = 0;
    for (let i = 0; i < 161838; i++) frame = frameStepUs(frame, { numerator: 30000, denominator: 1001 }, 1);
    expect(frame).toBe(Math.round(161838 * 1e6 * 1001 / 30000));
  });
});
