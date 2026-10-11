import type { Transcript } from '../transcript/types';
import type { ClipAudio, TimelineClip, TimelineDocument } from './types';
import { clipDurationUs, clipEndUs, mapSourceInterval, MAX_TIME_US, timelineToSourceUs } from './time';
import { trackAudible } from './navigation';

export interface VoiceMedia { segmentId: string; path: string; startUs: number; endUs: number; speechStartUs: number; speechEndUs: number }
export interface SourceMedia { videoContentId: string; contentRevision: number; transcript: Transcript | null; voices: VoiceMedia[]; warnings: Array<{ segmentId: string; message: string }> }
/** Draft edits immediately stop outdated speech and move unchanged WAVs with source timestamps. */
export function sourceWithDraft(source: SourceMedia, draft?: Transcript): SourceMedia {
  if (!draft || draft.videoContentId !== source.videoContentId) return source;
  const segments = new Map(draft.segments.map(s => [s.id, s]));
  const saved = new Map(source.transcript?.segments.map(s => [s.id, s]) ?? []);
  const voices = source.voices.flatMap(voice => {
    const s = segments.get(voice.segmentId), old = saved.get(voice.segmentId);
    if (!s || !old || !s.translated.trim() || !['ready', 'durationOverflow'].includes(s.tts?.status ?? '')
      || s.translated !== old.translated || s.tts?.audioPath !== old.tts?.audioPath) return [];
    const lead = Math.round(old.start * 1e6) - voice.startUs;
    const startUs = Math.max(0, Math.round(s.start * 1e6) - lead), delta = startUs - voice.startUs;
    return [{ ...voice, startUs, endUs: voice.endUs + delta, speechStartUs: voice.speechStartUs + delta, speechEndUs: voice.speechEndUs + delta }];
  });
  const warnings = source.warnings.filter(w => segments.get(w.segmentId)?.translated.trim());
  for (const s of draft.segments) if (s.translated.trim() && !voices.some(v => v.segmentId === s.id) && !warnings.some(w => w.segmentId === s.id))
    warnings.push({ segmentId: s.id, message: s.tts?.warning || 'Chưa có file giọng phù hợp với nội dung hiện tại.' });
  return { ...source, transcript: draft, voices, warnings };
}
export const contentId = (c: TimelineClip) => 'videoContentId' in c.content ? c.content.videoContentId : undefined;
export const hasAudio = (c?: TimelineClip) => !!c && ['video', 'audio', 'sourceAudio', 'voice'].includes(c.content.kind);
export function usesSourceVoice(c: TimelineClip, doc?: TimelineDocument): boolean {
  return c.content.kind === 'voice' || (c.content.kind === 'video' && !!doc?.videoContents.find(v => v.id === contentId(c))?.settings.dubbing.enabled
    && clipAudio(c, doc).voiceEnabled && !doc.clips.some(v => v.content.kind === 'voice' && v.linkedClipId === c.id));
}
export function clipAudio(c: TimelineClip, doc?: TimelineDocument): ClipAudio {
  if (c.audio) return c.audio;
  const settings = doc?.videoContents.find(v => v.id === contentId(c))?.settings.dubbing;
  const original = c.content.kind === 'video' || c.content.kind === 'sourceAudio';
  return { volume: original ? settings?.originalVolume ?? 1 : c.content.kind === 'voice' ? settings?.dubbedVolume ?? 1 : 1, muted: original ? settings?.muteOriginal ?? false : false,
    fadeInUs: 0, fadeOutUs: 0, voiceEnabled: true, voiceSpeed: 1, duckingEnabled: original ? settings?.duckingEnabled ?? false : false, duckingAmount: settings?.duckingAmount ?? .75,
    duckAttackUs: (settings?.audioProcessing?.duckAttackMs ?? 60) * 1000, duckReleaseUs: (settings?.audioProcessing?.duckReleaseMs ?? 180) * 1000 };
}
export function validateAudio(a: ClipAudio) {
  if (!Number.isFinite(a.volume) || a.volume < 0 || a.volume > 2 || !Number.isFinite(a.voiceSpeed) || a.voiceSpeed < .5 || a.voiceSpeed > 2
    || !Number.isFinite(a.duckingAmount) || a.duckingAmount < 0 || a.duckingAmount > 1
    || ![a.fadeInUs, a.fadeOutUs, a.duckAttackUs, a.duckReleaseUs].every(v => Number.isSafeInteger(v) && v >= 0 && v <= MAX_TIME_US)
    || a.duckAttackUs > 500000 || a.duckReleaseUs > 2000000) throw new Error('Âm lượng, fade, ducking hoặc tốc độ giọng không hợp lệ.');
}
export interface AudioRegion {
  id: string; clip: TimelineClip; kind: 'source' | 'voice'; assetId?: string; path?: string;
  startUs: number; endUs: number; voice?: VoiceMedia; speed: number;
  speechStartUs: number; speechEndUs: number; audio: ClipAudio;
}
export function audioRegions(doc: TimelineDocument, sources: ReadonlyMap<string, SourceMedia>): AudioRegion[] {
  const regions: AudioRegion[] = [];
  for (const c of doc.clips) {
    if (!hasAudio(c)) continue;
    const audio = clipAudio(c, doc), id = contentId(c), d = doc.videoContents.find(v => v.id === id)?.settings.dubbing;
    if (['video', 'audio', 'sourceAudio'].includes(c.content.kind) && 'assetId' in c.content) {
      const assetId = c.content.assetId, asset = doc.assets.find(a => a.id === assetId);
      if (c.content.kind !== 'video' || asset?.metadata.audioStreams?.length) regions.push({ id: `${c.id}:source`, clip: c, kind: 'source', assetId: c.content.assetId,
        startUs: c.startUs, endUs: clipEndUs(c), speed: 1, speechStartUs: c.startUs, speechEndUs: c.startUs, audio });
    }
    const implicit = c.content.kind === 'video' && usesSourceVoice(c, doc);
    if (c.content.kind !== 'voice' && !implicit) continue;
    const voiceAudio = implicit ? { ...audio, muted: false, volume: d?.dubbedVolume ?? 1 } : audio;
    for (const voice of sources.get(id ?? '')?.voices ?? []) {
      const speed = audio.voiceSpeed, end = Math.round(voice.startUs + (voice.endUs - voice.startUs) / speed);
      const window = mapSourceInterval(c, voice.startUs, end); if (!window) continue;
      const speech = mapSourceInterval(c, Math.round(voice.startUs + (voice.speechStartUs - voice.startUs) / speed), Math.round(voice.startUs + (voice.speechEndUs - voice.startUs) / speed));
      regions.push({ id: `${c.id}:voice:${voice.segmentId}`, clip: c, kind: 'voice', path: voice.path, voice, speed, ...window,
        speechStartUs: speech?.startUs ?? window.startUs, speechEndUs: speech?.endUs ?? window.startUs, audio: voiceAudio });
    }
  }
  return regions;
}
export function regionAudible(r: AudioRegion, doc: TimelineDocument) {
  const track = doc.tracks.find(t => t.id === r.clip.trackId);
  return !r.audio.muted && r.audio.volume > 0 && !!track && trackAudible(track, doc.tracks);
}
export function audioOffsetUs(r: AudioRegion, timeUs: number) {
  if (timeUs < r.startUs || timeUs >= r.endUs) return null;
  const source = timelineToSourceUs(r.clip, timeUs); if (source === null) return null;
  return r.voice ? Math.max(0, Math.round((source - r.voice.startUs) * r.speed)) : source;
}
export function regionGain(r: AudioRegion, timeUs: number, regions: AudioRegion[], doc: TimelineDocument): number {
  if (timeUs < r.startUs || timeUs >= r.endUs || !regionAudible(r, doc)) return 0;
  const duration = clipDurationUs(r.clip), elapsed = timeUs - r.clip.startUs, remaining = clipEndUs(r.clip) - timeUs;
  const fade = Math.min(1, r.audio.fadeInUs ? elapsed / Math.min(duration, r.audio.fadeInUs) : 1, r.audio.fadeOutUs ? remaining / Math.min(duration, r.audio.fadeOutUs) : 1);
  let duck = 0;
  if (r.kind === 'source' && r.audio.duckingEnabled) for (const voice of regions) {
    if (voice.kind !== 'voice' || !regionAudible(voice, doc) || voice.speechEndUs <= voice.speechStartUs) continue;
    const from = voice.speechStartUs, to = voice.speechEndUs;
    const amount = timeUs < from ? (r.audio.duckAttackUs ? 1 - (from - timeUs) / r.audio.duckAttackUs : 0) : timeUs < to ? 1 : r.audio.duckReleaseUs ? 1 - (timeUs - to) / r.audio.duckReleaseUs : 0;
    duck = Math.max(duck, Math.min(1, Math.max(0, amount)));
  }
  return r.audio.volume * Math.max(0, fade) * (1 - duck * r.audio.duckingAmount);
}
export interface ClipWarning { segmentId: string; message: string; startUs: number; endUs: number; requiredSpeed?: number }
export function clipWarnings(c: TimelineClip, source?: SourceMedia, includeVoice = true): ClipWarning[] {
  if (!source) return [];
  const warnings: ClipWarning[] = [], speed = c.audio?.voiceSpeed ?? 1;
  for (const s of source.transcript?.segments ?? []) {
    const start = Math.round(s.start * 1e6), end = Math.round(s.end * 1e6);
    if (!mapSourceInterval(c, start, end)) continue;
    if (start + 25000 < c.sourceInUs || end - 25000 > c.sourceOutUs) warnings.push({ segmentId: s.id, message: 'Điểm trim cắt giữa lời. Phụ đề và audio chỉ phát phần nằm trong clip.', startUs: start, endUs: end });
    if (includeVoice) for (const w of source.warnings.filter(w => w.segmentId === s.id)) warnings.push({ segmentId: s.id, message: w.message, startUs: start, endUs: end });
  }
  for (const voice of includeVoice ? source.voices : []) {
    const end = Math.round(voice.startUs + (voice.speechEndUs - voice.startUs) / speed);
    if (end <= c.sourceInUs || voice.startUs >= c.sourceOutUs) continue;
    const speechStart = Math.round(voice.startUs + (voice.speechStartUs - voice.startUs) / speed);
    if (speechStart < c.sourceInUs - 25000 || end > c.sourceOutUs + 25000) warnings.push({ segmentId: voice.segmentId, message: 'Giọng đọc vượt mép clip và đang được giới hạn tại mép. File giọng vẫn được giữ.', startUs: voice.startUs, endUs: voice.speechEndUs,
      requiredSpeed: voice.startUs >= c.sourceInUs && c.sourceOutUs > voice.startUs ? (voice.speechEndUs - voice.startUs) / (c.sourceOutUs - voice.startUs) : undefined });
  }
  return warnings;
}
export function captionAt(c: TimelineClip, transcript: Transcript | null | undefined, timeUs: number): string {
  const source = timelineToSourceUs(c, timeUs); if (source === null) return '';
  return (transcript?.segments ?? []).filter(s => s.start * 1e6 <= source && s.end * 1e6 > source).map(s => s.translated || s.source).join('\n');
}
