import { describe, expect, it } from 'vitest';
import { nativeSceneClip, residentClips, timelineScene } from './scene-plan';
import type { TimelineClip, TimelineDocument } from './types';

function fixture() {
  const transform = { x: .5, y: .5, scaleX: 1, scaleY: 1, rotation: 0, opacity: 1, flipX: false, flipY: false, fit: 'contain' as const, crop: { x: 0, y: 0, width: 1, height: 1 } };
  const clip = (id: string, startUs: number, videoContentId = id): TimelineClip => ({ id, startUs, sourceInUs: 2e6, sourceOutUs: 6e6,
    rate: { numerator: 2, denominator: 1 }, trackId: 'video', content: { kind: 'video', assetId: `asset-${videoContentId}`, videoContentId }, transform, linkedClipId: null, groupId: null });
  const doc: TimelineDocument = { schemaVersion: 1, settings: { width: 640, height: 360, frameRate: { numerator: 25, denominator: 1 }, backgroundColor: '#000000' }, legacyBinding: null,
    tracks: [{ id: 'video', kind: 'visual', name: 'Video', order: 0, locked: false, enabled: true, muted: false, solo: false }],
    assets: ['a','b'].map(id => ({ id: `asset-${id}`, kind: 'video', relativePath: `media/${id}.mp4`, durationUs: 10e6, frameRate: null, variableFrameRate: null,
      metadata: { videoPath: `media/${id}.mp4`, width: 640, height: 360, audioStreams: [] } })), videoContents: [], clips: [clip('a', 0), clip('b', 2e6), clip('a-copy', 6e6, 'a')] };
  return { doc, clip };
}
describe('preview/export scene snapshot contract', () => {
  it('chooses clip instances at half-open boundaries with independent trimmed source times', () => {
    const { doc } = fixture();
    let scene = timelineScene(doc, new Map(), 1999999);
    expect(scene.layers.map(l => [l.clip.id, l.sourceUs])).toEqual([['a', 5999998]]); expect(scene.nextBoundaryUs).toBe(2e6);
    scene = timelineScene(doc, new Map(), 2e6);
    expect(scene.layers.map(l => [l.clip.id, l.sourceUs])).toEqual([['b', 2e6]]);
    expect(timelineScene(doc, new Map(), 5e6).layers).toEqual([]);
    scene = timelineScene(doc, new Map(), 7e6);
    expect(scene.layers.map(l => [l.clip.id, l.sourceUs])).toEqual([['a-copy', 4e6]]);
    expect(scene.endUs).toBe(8e6);
  });
  it('bounds speculative prefetch on large projects without discarding active overlapping layers', () => {
    const { doc, clip } = fixture();
    doc.clips = [...Array.from({ length: 10 }, (_, i) => clip(`active-${i}`, 0, 'a')), ...Array.from({ length: 500 }, (_, i) => clip(`next-${i}`, 1e6 + i * 1000, 'b'))];
    const resident = residentClips(doc, 0);
    expect(resident.length).toBe(16); expect(resident.filter(c => c.startUs === 0).length).toBe(10);
    expect(resident.slice(10).map(c => c.id)).toEqual(['next-0','next-1','next-2','next-3','next-4','next-5']);
    doc.tracks[0]!.enabled = false; expect(residentClips(doc, 0)).toEqual([]);
  });
  it('keeps layer order and rejects native acceleration for DOM overlays or transformed video', () => {
    const { doc } = fixture();
    expect(nativeSceneClip(doc, timelineScene(doc, new Map(), 0).layers, false)?.id).toBe('a');
    expect(nativeSceneClip(doc, timelineScene(doc, new Map(), 0).layers, true)).toBeUndefined();
    doc.clips[0] = { ...doc.clips[0]!, transform: { ...doc.clips[0]!.transform, scaleX: .5 } };
    expect(nativeSceneClip(doc, timelineScene(doc, new Map(), 0).layers, false)).toBeUndefined();
    doc.tracks.push({ ...doc.tracks[0]!, id: 'overlay', order: 1 });
    doc.clips.push({ ...doc.clips[0]!, id: 'text', trackId: 'overlay', content: { kind: 'text', text: 'Title' } });
    const scene = timelineScene(doc, new Map(), 0);
    expect(scene.layers.map(l => l.clip.id)).toEqual(['a','text']); expect(scene.layers[1]!.text).toBe('Title');
    expect(nativeSceneClip(doc, scene.layers, false)).toBeUndefined();
  });
});
