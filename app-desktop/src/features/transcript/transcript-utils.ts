import type { Transcript, TranscriptSegment } from './types';

export function validateTranscript(transcript: Transcript, duration: number | null | undefined): string | null {
  const ids = new Set<string>();
  for (const segment of transcript.segments) {
    if (!segment.id.trim() || ids.has(segment.id)) return 'Transcript có segment ID trống hoặc bị trùng.';
    ids.add(segment.id);
    if (!Number.isFinite(segment.start) || !Number.isFinite(segment.end) || segment.start < 0) {
      return 'Timestamp transcript phải là số hữu hạn và không âm.';
    }
    if (segment.end <= segment.start) return 'Thời điểm kết thúc phải lớn hơn thời điểm bắt đầu.';
    if (duration && segment.end > duration + 0.001) return 'Timestamp transcript vượt quá thời lượng video.';
  }
  return null;
}

export function formatTimestamp(seconds: number): string {
  const value = Math.max(0, Number.isFinite(seconds) ? seconds : 0);
  const minutes = Math.floor(value / 60);
  const remainder = value % 60;
  return `${String(minutes).padStart(2, '0')}:${remainder.toFixed(2).padStart(5, '0')}`;
}

export function compareSegmentTime(left: TranscriptSegment, right: TranscriptSegment): number {
  return left.start - right.start || left.end - right.end || left.id.localeCompare(right.id);
}

/**
 * Return the TTS deadline without changing subtitle timestamps. The next cue
 * bounds TTS even if transcript cues overlap; short audio is never padded just
 * to fill the available time.
 */
export function ttsWindowEnd(segments: TranscriptSegment[], segment: TranscriptSegment): number {
  let nextStart: number | undefined;
  for (const candidate of segments) {
    if (candidate.id === segment.id || candidate.start <= segment.start + 0.001) continue;
    if (nextStart === undefined || candidate.start < nextStart) nextStart = candidate.start;
  }
  return nextStart ?? segment.end;
}

export function estimatedModelSize(model: string): string {
  const estimates: Record<string, string> = {
    tiny: '75 MB',
    base: '145 MB',
    small: '485 MB',
    medium: '1.5 GB',
    'large-v3': '3.1 GB',
  };
  return estimates[model] ?? 'chưa xác định';
}

export function findTranslatedSegmentAtTime(segments: TranscriptSegment[], time: number, enabled: boolean): TranscriptSegment | undefined {
  if (!enabled || !Number.isFinite(time) || time < 0) return undefined;
  let active: TranscriptSegment | undefined;
  for (const segment of segments) {
    if (time < segment.start || time >= segment.end || !segment.translated.trim()) continue;
    // Transcript tracks may contain overlapping events. Choose the event that began
    // most recently so an earlier long segment cannot mask every later subtitle.
    if (!active || compareSegmentTime(active, segment) < 0) active = segment;
  }
  return active;
}

export function findPlayableDubbingSegmentAtTime(segments: TranscriptSegment[], time: number, enabled: boolean, finishedId?: string): TranscriptSegment | undefined {
  if (!enabled || !Number.isFinite(time) || time < 0) return undefined;
  return segments.find((segment) => segment.id !== finishedId
    && !!segment.tts?.audioPath
    && segment.tts.status === 'ready'
    && time >= segment.start && time < ttsWindowEnd(segments, segment));
}
