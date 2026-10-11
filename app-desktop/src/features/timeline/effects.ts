import { clipDurationUs, clipEndUs, MAX_TIME_US, timelineToSourceUs } from './time';
import type { MotionValues, TimelineClip, TimelineDocument, VisualEffects } from './types';

export const motionKeys = ['x', 'y', 'scaleX', 'scaleY', 'opacity'] as const;
export const defaultEffects = (): VisualEffects => ({ fadeInUs: 0, fadeOutUs: 0, keyframes: [] });
export function motionAt(c: TimelineClip, sourceUs: number): MotionValues {
  const keys = c.effects?.keyframes ?? [];
  if (!keys.length) return Object.fromEntries(motionKeys.map(key => [key, c.transform[key]])) as unknown as MotionValues;
  const before = [...keys].reverse().find(k => k.sourceUs <= sourceUs) ?? keys[0];
  const after = keys.find(k => k.sourceUs >= sourceUs) ?? keys[keys.length - 1];
  const p = before === after ? 0 : (sourceUs - before.sourceUs) / (after.sourceUs - before.sourceUs);
  return Object.fromEntries(motionKeys.map(key => [key, before[key] + (after[key] - before[key]) * p])) as unknown as MotionValues;
}
export function validateEffects(c: TimelineClip) {
  const e = c.effects; if (!e) return;
  if (['audio', 'voice', 'sourceAudio'].includes(c.content.kind) || e.keyframes.length > 64
    || ![e.fadeInUs, e.fadeOutUs].every(v => Number.isSafeInteger(v) && v >= 0 && v <= MAX_TIME_US)) throw new Error('Fade hình/keyframe không hợp lệ (tối đa 64 keyframe).');
  for (const [i, k] of e.keyframes.entries()) {
    if (!Number.isSafeInteger(k.sourceUs) || k.sourceUs < 0 || k.sourceUs > MAX_TIME_US || i && k.sourceUs <= e.keyframes[i - 1].sourceUs
      || !motionKeys.every(key => Number.isFinite(k[key])) || k.scaleX <= 0 || k.scaleY <= 0 || k.scaleX > 100 || k.scaleY > 100 || k.opacity < 0 || k.opacity > 1) throw new Error('Keyframe phải tăng dần theo thời gian nguồn; scale và opacity phải hợp lệ.');
  }
}
type Clips = Pick<TimelineDocument, 'clips'>;
export const transitionOwner = (c: TimelineClip, doc: Clips) => c.linkedClipId ? doc.clips.find(p => p.id === c.linkedClipId) ?? c : c;
export function transitionAudioGain(c: TimelineClip, doc: Clips, timeUs: number): number {
  const owner = transitionOwner(c, doc), incoming = owner.transitionIn;
  let gain = incoming ? Math.max(0, Math.min(1, (timeUs - owner.startUs) / incoming.durationUs)) : 1;
  const outgoing = doc.clips.find(next => next.transitionIn?.fromClipId === owner.id);
  if (outgoing) gain *= Math.max(0, Math.min(1, (clipEndUs(owner) - timeUs) / outgoing.transitionIn!.durationUs));
  return gain;
}
export function visualAt(c: TimelineClip, doc: Clips, timeUs: number) {
  const source = timelineToSourceUs(c, timeUs) ?? c.sourceInUs, values = motionAt(c, source), e = c.effects;
  const duration = clipDurationUs(c);
  let opacity = values.opacity * Math.max(0, Math.min(1, e?.fadeInUs ? (timeUs - c.startUs) / Math.min(duration, e.fadeInUs) : 1,
    e?.fadeOutUs ? (clipEndUs(c) - timeUs) / Math.min(duration, e.fadeOutUs) : 1));
  const transition = c.transitionIn;
  const p = transition ? Math.max(0, Math.min(1, (timeUs - c.startUs) / transition.durationUs)) : 1;
  if (transition?.kind === 'dissolve') opacity *= p;
  if (c.linkedClipId) opacity *= transitionAudioGain(c, doc, timeUs);
  return { transform: { ...c.transform, ...values, opacity }, reveal: transition?.kind === 'wipeLeft' ? p : 1 };
}
export function validateTransitions(doc: Clips) {
  for (const c of doc.clips) {
    const t = c.transitionIn; if (!t) continue;
    const a = doc.clips.find(a => a.id === t.fromClipId);
    if (!a || a.trackId !== c.trackId || !['video', 'image'].includes(a.content.kind) || !['video', 'image'].includes(c.content.kind)
      || !['dissolve', 'wipeLeft'].includes(t.kind) || !Number.isSafeInteger(t.durationUs) || t.durationUs <= 0
      || t.durationUs >= Math.min(clipDurationUs(a), clipDurationUs(c)) || a.startUs >= c.startUs || clipEndUs(a) !== c.startUs + t.durationUs
      || doc.clips.some(b => b.trackId === c.trackId && a.startUs < b.startUs && b.startUs < c.startUs)) throw new Error('Chuyển cảnh cần hai clip hình liền kề, chồng đúng thời lượng và còn phần nguồn riêng. Gỡ chuyển cảnh trước khi đổi điểm cắt/tốc độ.');
  }
}
/** Only a deliberate transition (and its linked layers) may overlap on one track. */
export function transitionOverlap(a: TimelineClip, b: TimelineClip, doc: Clips) {
  let left = transitionOwner(a, doc), right = transitionOwner(b, doc);
  if (left.transitionIn?.fromClipId === right.id) [left, right] = [right, left];
  const t = right.transitionIn;
  return !!t && t.fromClipId === left.id && Math.max(a.startUs, b.startUs) >= right.startUs
    && Math.min(clipEndUs(a), clipEndUs(b)) <= right.startUs + t.durationUs;
}
