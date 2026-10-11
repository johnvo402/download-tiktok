import type { Rational, TimeUs, TimelineClip } from './types';

export const MAX_TIME_US = Number.MAX_SAFE_INTEGER;
export const TIMEBASE = 1_000_000;
function integer(value: number): bigint {
  if (!Number.isSafeInteger(value) || value < 0) throw new Error('Invalid timeline time');
  return BigInt(value);
}
function rational(rate: Rational): [bigint, bigint] {
  if (!Number.isInteger(rate.numerator) || !Number.isInteger(rate.denominator)
    || rate.numerator <= 0 || rate.denominator <= 0
    || rate.numerator > 0xffffffff || rate.denominator > 0xffffffff) throw new Error('Invalid rational rate');
  return [BigInt(rate.numerator), BigInt(rate.denominator)];
}
function safe(value: bigint): number {
  if (value > BigInt(MAX_TIME_US)) throw new Error('Timeline time overflow');
  return Number(value);
}
export function secondsToTimeUs(seconds: number): TimeUs {
  if (!Number.isFinite(seconds) || seconds < 0 || seconds > MAX_TIME_US / TIMEBASE) throw new Error('Invalid source time');
  const value = Math.floor(seconds * TIMEBASE + 0.5);
  integer(value);
  return value;
}
/** Positive values round half up, identically to the Rust implementation. */
export function rescaleTime(value: TimeUs, rate: Rational): TimeUs {
  const [n, d] = rational(rate);
  return safe((integer(value) * n + d / 2n) / d);
}
export function clipDurationUs(clip: TimelineClip): TimeUs {
  const start = integer(clip.sourceInUs), end = integer(clip.sourceOutUs);
  if (end <= start) throw new Error('Empty source interval');
  const [n, d] = rational(clip.rate);
  return safe(((end - start) * d + n - 1n) / n);
}
export function clipEndUs(clip: TimelineClip): TimeUs {
  return safe(integer(clip.startUs) + integer(clipDurationUs(clip)));
}
export function timelineToSourceUs(clip: TimelineClip, time: TimeUs): TimeUs | null {
  integer(time);
  const end = clipEndUs(clip);
  if (time < clip.startUs || time >= end) return null;
  const [n, d] = rational(clip.rate);
  const value = integer(clip.sourceInUs) + (integer(time - clip.startUs) * n + d / 2n) / d;
  return Math.min(safe(value), clip.sourceOutUs - 1);
}
export function sourceToTimelineUs(clip: TimelineClip, time: TimeUs): TimeUs | null {
  integer(time);
  const end = clipEndUs(clip);
  if (time < clip.sourceInUs || time >= clip.sourceOutUs) return null;
  const [n, d] = rational(clip.rate);
  const value = integer(clip.startUs) + (integer(time - clip.sourceInUs) * d + n / 2n) / n;
  return Math.min(safe(value), end - 1);
}
export function mapSourceInterval(clip: TimelineClip, start: TimeUs, end: TimeUs): { startUs: TimeUs; endUs: TimeUs } | null {
  integer(start); integer(end);
  const from = Math.max(start, clip.sourceInUs), to = Math.min(end, clip.sourceOutUs);
  if (from >= to) return null;
  const startUs = sourceToTimelineUs(clip, from)!;
  const endUs = to === clip.sourceOutUs ? clipEndUs(clip) : sourceToTimelineUs(clip, to)!;
  return { startUs, endUs: Math.min(clipEndUs(clip), Math.max(startUs + 1, endUs)) };
}
export function timelineDurationUs(clips: TimelineClip[]): TimeUs {
  return clips.reduce((duration, clip) => Math.max(duration, clipEndUs(clip)), 0);
}
/** Step to the neighbouring output-frame timestamp, including fractional FPS. */
export function frameStepUs(timeUs: TimeUs, rate: Rational, direction: -1 | 1): TimeUs {
  const [n, d] = rational(rate), period = BigInt(TIMEBASE) * d;
  const index = (integer(timeUs) * n + period / 2n) / period;
  const next = direction < 0 ? (index > 0n ? index - 1n : 0n) : index + 1n;
  return safe((next * period + n / 2n) / n);
}
