import { describe, expect, it } from 'vitest';
import type { Transcript, TranscriptSegment } from './types';
import { compareSegmentTime, estimatedModelSize, findPlayableDubbingSegmentAtTime, findTranslatedSegmentAtTime, formatTimestamp, validateTranscript } from './transcript-utils';

function segment(overrides: Partial<TranscriptSegment> = {}): TranscriptSegment {
  return {
    id: 'segment-1', start: 1, end: 2, source: 'Xin chào 🌿', translated: '', sourceRevision: 0,
    translatedRevision: null, translationMeta: null, speakerId: null,
    sourceType: 'subtitle', translationStatus: 'pending', tts: null, ...overrides,
  };
}

function transcript(segments: TranscriptSegment[]): Transcript {
  return { schemaVersion: 1, projectId: 'project', sourceType: 'subtitle', sourceTrack: null, language: 'vi', videoChecksum: null, segments };
}

describe('transcript validation helpers', () => {
  it('accepts Unicode segments and allows overlaps', () => {
    expect(validateTranscript(transcript([segment(), segment({ id: 'segment-2', start: 1.5, end: 2.5 })]), 3)).toBeNull();
  });

  it('rejects invalid and out-of-range timestamps and duplicate IDs', () => {
    expect(validateTranscript(transcript([segment({ start: -1 })]), 3)).toContain('không âm');
    expect(validateTranscript(transcript([segment({ end: 1 })]), 3)).toContain('lớn hơn');
    expect(validateTranscript(transcript([segment({ end: 4 })]), 3)).toContain('vượt quá');
    expect(validateTranscript(transcript([segment(), segment()]), 3)).toContain('trùng');
  });

  it('formats time and estimates model disk usage', () => {
    expect(formatTimestamp(61.25)).toBe('01:01.25');
    expect(estimatedModelSize('small')).toBe('485 MB');
    expect(compareSegmentTime(segment({ start: 1 }), segment({ start: 2 }))).toBeLessThan(0);
  });

  it('shows only translated subtitle segments inside their time range', () => {
    const translated = segment({ translated: 'Xin chào', translationStatus: 'translated', translatedRevision: 0 });
    const empty = segment({ id: 'empty', start: 3, end: 4 });
    expect(findTranslatedSegmentAtTime([translated, empty], 1.5, true)?.id).toBe('segment-1');
    expect(findTranslatedSegmentAtTime([translated, empty], 2.2, true)).toBeUndefined();
    expect(findTranslatedSegmentAtTime([translated], 1.5, false)).toBeUndefined();
  });

  it('chooses the most recently started translated event when subtitle times overlap', () => {
    const long = segment({ id: 'long', start: 1, end: 5, translated: 'Câu dài' });
    const next = segment({ id: 'next', start: 2, end: 3, translated: 'Câu tiếp theo' });
    expect(findTranslatedSegmentAtTime([long, next], 2.4, true)?.id).toBe('next');
    expect(findTranslatedSegmentAtTime([next, long], 3.2, true)?.id).toBe('long');
    expect(findTranslatedSegmentAtTime([long, next], 5, true)).toBeUndefined();
  });

  it('schedules only valid ready dubbing and excludes ended segments and overflow', () => {
    const ready = segment({ id: 'ready', start: 1, end: 3, tts: { audioPath: 'audio/tts/ready.wav', duration: 1.5, status: 'ready', fingerprint: 'f', voice: 'Ngọc Lan', speed: 1, textHash: 'x', generatedAt: null, modelId: 'v3-turbo', modelRevision: 'revision', warning: null } });
    const overflow = segment({ id: 'long', start: 2, end: 4, tts: { ...ready.tts!, status: 'durationOverflow', audioPath: 'audio/tts/long.wav' } });
    expect(findPlayableDubbingSegmentAtTime([ready], 1.5, true)?.id).toBe('ready');
    expect(findPlayableDubbingSegmentAtTime([ready], 1.5, true, 'ready')).toBeUndefined();
    expect(findPlayableDubbingSegmentAtTime([overflow], 2.5, true)).toBeUndefined();
    expect(findPlayableDubbingSegmentAtTime([ready], 1.5, false)).toBeUndefined();
  });
});
