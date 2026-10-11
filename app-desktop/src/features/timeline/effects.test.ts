import { describe, expect, it } from 'vitest';
import shared from '../../../test-fixtures/timeline-effects.json';
import { applyEdit, editLayout, validateEdit } from './edit-commands';
import { defaultTransform } from './visual-layout';
import { motionAt, motionKeys, transitionAudioGain, validateEffects, visualAt } from './effects';
import { clipDurationUs, timelineToSourceUs } from './time';
import { timelineScene, nativeSceneClip } from './scene-plan';
import type { TimelineClip, TimelineDocument, TimelineTrack } from './types';

function fixture(): TimelineDocument {
  const track = (id: string, kind: TimelineTrack['kind'], order: number): TimelineTrack => ({ id, kind, order, name: id, enabled: true, locked: false, muted: false, solo: false });
  const clip = (id: string, startUs: number): TimelineClip => ({ id, startUs, sourceInUs: 0, sourceOutUs: 4e6, rate: { numerator: 1, denominator: 1 }, trackId: 'visual', transform: defaultTransform(), content: { kind: 'video', assetId: `asset-${id}`, videoContentId: `content-${id}` }, linkedClipId: null, groupId: null });
  const clips = [clip('a', 0), clip('b', 4e6), clip('c', 8e6)];
  return { schemaVersion: 1, settings: { width: 320, height: 180, backgroundColor: '#000000', frameRate: { numerator: 25, denominator: 1 } },
    tracks: [track('visual', 'visual', 0), track('caption', 'subtitle', 1), track('voice', 'audio', 2)],
    assets: clips.map(c => ({ id: `asset-${c.id}`, kind: 'video', relativePath: `media/${c.id}`, durationUs: 8e6, frameRate: null, variableFrameRate: null, metadata: { videoPath: `media/${c.id}`, width: 320, height: 180 } })),
    videoContents: clips.map(c => ({ id: `content-${c.id}`, assetId: `asset-${c.id}`, revision: 1, settings: { dubbing: { enabled: true, originalVolume: 1, dubbedVolume: 1 } } as unknown as TimelineDocument['videoContents'][number]['settings'] })),
    clips: [...clips, ...clips.flatMap(c => [
      { ...c, id: `${c.id}-caption`, trackId: 'caption', content: { kind: 'subtitles' as const, videoContentId: `content-${c.id}` }, linkedClipId: c.id },
      { ...c, id: `${c.id}-voice`, trackId: 'voice', content: { kind: 'voice' as const, videoContentId: `content-${c.id}` }, linkedClipId: c.id }])], legacyBinding: null };
}
let count = 0;
const apply = (doc: TimelineDocument, command: Parameters<typeof applyEdit>[1]): TimelineDocument => ({ ...doc, ...applyEdit(doc, command, () => `new-${++count}`), tracks: doc.tracks });

describe('timeline effects contract', () => {
  it('matches shared source-clock motion/fade samples and retimes without moving source keys', () => {
    const doc = fixture(), c = { ...doc.clips[0], ...shared, effects: shared.effects };
    for (const s of shared.samples) {
      const motion = motionAt(c, timelineToSourceUs(c, s.timeUs)!);
      motionKeys.forEach((key, i) => expect(motion[key]).toBeCloseTo(s.motion[i], 8));
      expect(visualAt(c, doc, s.timeUs).transform.opacity).toBeCloseTo(s.opacity, 8);
    }
    const alone = { ...doc, clips: [c] };
    const trimmed = apply(alone, { kind: 'trim', id: c.id, edge: 'start', atUs: 3e6 }).clips[0];
    expect(trimmed.effects).toEqual(c.effects); expect(motionAt(trimmed, trimmed.sourceInUs).x).toBeCloseTo(.5);
    const retimed = apply(alone, { kind: 'rate', id: c.id, rate: { numerator: 1, denominator: 1 } }).clips[0];
    expect(motionAt(retimed, timelineToSourceUs(retimed, 4e6)!).x).toBeCloseTo(.5);
    const split = apply(alone, { kind: 'split', ids: [c.id], atUs: 3e6 });
    expect(split.clips[1].effects).toEqual(c.effects); expect(motionAt(split.clips[1], split.clips[1].sourceInUs).x).toBeCloseTo(.5);
    expect(Object.keys(motionAt(doc.clips[0], 0)).sort()).toEqual([...motionKeys].sort());
  });
  it('overlaps a sequence and linked captions/voices without rewriting source data, and restores after removal', () => {
    const doc = fixture(), original = JSON.stringify(doc);
    const next = apply(doc, { kind: 'transition', id: 'b', transition: { kind: 'dissolve', durationUs: 1e6 } });
    expect(next.clips.find(c => c.id === 'b')?.startUs).toBe(3e6); expect(next.clips.find(c => c.id === 'c-voice')?.startUs).toBe(7e6);
    expect(next.clips.find(c => c.id === 'b-caption')?.startUs).toBe(3e6);
    expect(next.videoContents).toBe(doc.videoContents); expect(JSON.stringify(doc)).toBe(original);
    const changed = apply(next, { kind: 'transition', id: 'b', transition: { kind: 'wipeLeft', durationUs: .5e6 } });
    expect(changed.clips.find(c => c.id === 'b')?.startUs).toBe(3.5e6);
    expect(apply(changed, { kind: 'transition', id: 'b', transition: null })).toEqual(doc);
  });
  it('evaluates dissolve/wipe and linked caption/audio ramps in the same overlapping interval', () => {
    const doc = apply(fixture(), { kind: 'transition', id: 'b', transition: { kind: 'dissolve', durationUs: 1e6 } });
    const a = doc.clips[0], b = doc.clips[1];
    expect(visualAt(a, doc, 3.5e6).transform.opacity).toBe(1);
    expect(visualAt(b, doc, 3.5e6).transform.opacity).toBe(.5);
    expect(transitionAudioGain(a, doc, 3.5e6)).toBe(.5); expect(transitionAudioGain(b, doc, 3.5e6)).toBe(.5);
    expect(visualAt(doc.clips.find(c => c.id === 'a-caption')!, doc, 3.5e6).transform.opacity).toBe(.5);
    const scene = timelineScene(doc, new Map(), 3.5e6); expect(scene.layers.slice(0, 2).map(l => l.clip.id)).toEqual(['a', 'b']);
    expect(nativeSceneClip(doc, timelineScene(doc, new Map(), 4.5e6).layers.filter(l => l.clip.id === 'b'), false)).toBeUndefined();
    b.transitionIn!.kind = 'wipeLeft'; expect(visualAt(b, doc, 3.5e6).reveal).toBe(.5); expect(visualAt(b, doc, 3.5e6).transform.opacity).toBe(1);
  });
  it('rejects accidental/triple overlaps, invalid references, short sources and locked dependents atomically', () => {
    const doc = fixture(), before = JSON.stringify(doc);
    expect(() => apply(doc, { kind: 'transition', id: 'b', transition: { kind: 'dissolve', durationUs: 4e6 } })).toThrow(/ngắn/);
    doc.clips[1].startUs += 1; expect(() => apply(doc, { kind: 'transition', id: 'b', transition: { kind: 'dissolve', durationUs: 1e6 } })).toThrow(/liền/); doc.clips[1].startUs -= 1;
    doc.tracks[2].locked = true; expect(() => apply(doc, { kind: 'transition', id: 'b', transition: { kind: 'dissolve', durationUs: 1e6 } })).toThrow(/khóa/); doc.tracks[2].locked = false;
    expect(JSON.stringify(doc)).toBe(before);
    const next = apply(doc, { kind: 'transition', id: 'b', transition: { kind: 'dissolve', durationUs: 3e6 } });
    expect(() => apply(next, { kind: 'transition', id: 'c', transition: { kind: 'dissolve', durationUs: 3e6 } })).toThrow(/chồng/);
    expect(() => apply(next, { kind: 'rate', id: 'b', rate: { numerator: 2, denominator: 1 } })).toThrow(/Chuyển cảnh/);
    const accidental = editLayout(doc); accidental.clips[1].startUs = 3e6; expect(() => validateEdit(doc, accidental)).toThrow(/chồng/);
  });
  it('copies transition pairs with remapped IDs and permits crossed cue starts inside a deliberate overlap', () => {
    const doc = apply(fixture(), { kind: 'transition', id: 'b', transition: { kind: 'dissolve', durationUs: 1e6 } });
    const a = doc.clips.find(c => c.id === 'a-caption')!, b = doc.clips.find(c => c.id === 'b-caption')!;
    a.startUs = 3.7e6; a.sourceInUs = 3.7e6; b.startUs = 3.6e6; b.sourceInUs = .6e6;
    validateEdit(doc, editLayout(doc));
    const copy = applyEdit(doc, { kind: 'duplicate', ids: ['a', 'b'], atUs: 20e6 }, () => `new-${++count}`);
    const copiedB = copy.clips.find(c => c.id !== 'b' && c.transitionIn);
    expect(copiedB?.transitionIn?.fromClipId).not.toBe('a');
    expect(copy.clips.find(c => c.id === copiedB?.transitionIn?.fromClipId)?.startUs).toBe(20e6);
  });
  it('rejects malformed motion and preserves old clips without optional effects', () => {
    const c = fixture().clips[0]; validateEffects(c);
    c.effects = { ...shared.effects, keyframes: [...shared.effects.keyframes].reverse() }; expect(() => validateEffects(c)).toThrow(/tăng/);
    c.effects = structuredClone(shared.effects); c.effects.keyframes[0].scaleX = 0; expect(() => validateEffects(c)).toThrow();
    c.effects.keyframes[0].scaleX = .5; c.effects.fadeInUs = NaN; expect(() => validateEffects(c)).toThrow();
    expect(clipDurationUs(fixture().clips[0])).toBe(4e6);
  });
});
