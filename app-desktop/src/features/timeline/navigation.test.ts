import { describe, expect, it } from 'vitest';
import { activeClips, fitScale, limitScale, MAX_CANVAS_WIDTH, rulerTicks, timeAtPixel, trackAudible, visibleClips, zoomAt } from './navigation';
import type { TimelineClip, TimelineDocument, TimelineTrack } from './types';
const track: TimelineTrack = { id: 'v', name: 'Video', kind: 'visual', order: 0, enabled: true, locked: false, muted: false, solo: false };
const clip = (id: string, startUs: number, trackId = 'v'): TimelineClip => ({ id, startUs, trackId, sourceInUs: 2e6, sourceOutUs: 12e6, rate: { numerator: 1, denominator: 1 }, content: { kind: 'video', assetId: id, videoContentId: id }, transform: {} as TimelineClip['transform'], linkedClipId: null, groupId: null });
describe('timeline navigation', () => {
  it('preserves the pointer/playhead time through zoom and clamps at the ends', () => {
    const scroll = zoomAt(30, 60, 450, 120, 10_000);
    expect((scroll + 120) / 60).toBe((450 + 120) / 30);
    expect(zoomAt(60, 30, 0, 120, 100)).toBe(0);
    expect(zoomAt(30, 600, 450, 120, 100)).toBe(100);
    expect(timeAtPixel(-20, 30, 20e6)).toBe(0);
    expect(timeAtPixel(900, 30, 20e6)).toBe(20e6);
  });
  it('fits a long project, bounds browser canvas size and renders a bounded ruler', () => {
    for (const duration of [0, 20e6, 72e9, Number.MAX_SAFE_INTEGER]) {
      const fit = limitScale(fitScale(duration, 360), duration, 360);
      expect(fit * duration / 1e6).toBeLessThanOrEqual(360);
      const zoom = limitScale(1e9, duration, 360);
      expect(zoom * duration / 1e6).toBeLessThanOrEqual(MAX_CANVAS_WIDTH + 1);
      const ticks = rulerTicks(fit, 0, 360);
      expect(ticks.length).toBeLessThan(8);
      expect(ticks.every(t => Number.isSafeInteger(t))).toBe(true);
    }
  });
  it('uses half-open boundaries across A/B, image/audio layers and gaps without changing selection', () => {
    const doc = { tracks: [track, { ...track, id: 'image', order: 1 }, { ...track, kind: 'audio', id: 'audio', order: 2 }], clips: [clip('a', 0), clip('b', 10e6), clip('image', 4e6, 'image'), clip('music', 0, 'audio')] } as TimelineDocument;
    const original = JSON.stringify(doc);
    expect(activeClips(doc, 10e6).map(c => c.id)).toEqual(['b', 'image']);
    expect(activeClips(doc, 20e6)).toEqual([]);
    expect(visibleClips(doc.clips, 'v', 10e6, 20e6).map(c => c.id)).toEqual(['b']);
    expect(JSON.stringify(doc)).toBe(original);
    doc.tracks[1].enabled = false;
    expect(activeClips(doc, 10e6).map(c => c.id)).toEqual(['b']);
  });
  it('applies audio mute/solo to video and audio but ignores disabled and subtitle solos', () => {
    const music: TimelineTrack = { ...track, kind: 'audio', id: 'music', solo: true };
    expect(trackAudible(track, [track, music])).toBe(false);
    expect(trackAudible(music, [track, music])).toBe(true);
    expect(trackAudible({ ...music, muted: true }, [track, music])).toBe(false);
    expect(trackAudible(track, [track, { ...music, enabled: false }])).toBe(true);
    expect(trackAudible(track, [track, { ...music, kind: 'subtitle' }])).toBe(true);
  });
});
