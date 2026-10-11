import { describe, expect, it } from 'vitest';
import fixture from '../../../test-fixtures/timeline-time.json';
import { clipDurationUs, clipEndUs, timelineToSourceUs, sourceToTimelineUs, mapSourceInterval, secondsToTimeUs, rescaleTime, timelineDurationUs, MAX_TIME_US } from './time';
import type { TimelineClip } from './types';

const base: TimelineClip = {
  id: 'clip', trackId: 'track', startUs: 0, sourceInUs: 0, sourceOutUs: 1, rate: { numerator: 1, denominator: 1 },
  content: { kind: 'text', text: 'fixture' }, linkedClipId: null, groupId: null,
  transform: { x: .5, y: .5, scaleX: 1, scaleY: 1, rotation: 0, opacity: 1, flipX: false, flipY: false,
    fit: 'contain', crop: { x: 0, y: 0, width: 1, height: 1 } },
};
describe('shared Rust/TypeScript timeline time contract', () => {
  for (const row of fixture.cases) it(row.name, () => {
    const clip = { ...base, startUs: row.startUs, sourceInUs: row.sourceInUs, sourceOutUs: row.sourceOutUs, rate: row.rate };
    expect(clipDurationUs(clip)).toBe(row.durationUs);
    expect(clipEndUs(clip)).toBe(row.endUs);
    for (const [time, source] of row.timelinePoints) expect(timelineToSourceUs(clip, time!)).toBe(source);
    for (const [time, global] of row.sourcePoints) expect(sourceToTimelineUs(clip, time!)).toBe(global);
    for (const [start, end, result] of row.intervals) {
      const expected = result ? { startUs: (result as number[])[0], endUs: (result as number[])[1] } : null;
      expect(mapSourceInterval(clip, start as number, end as number)).toEqual(expected);
    }
  });
  it('rejects unsafe values, zero rates, empty clips and overflowing timelines', () => {
    for (const value of [NaN, Infinity, -1, .5, MAX_TIME_US + 1]) expect(() => rescaleTime(value, base.rate)).toThrow();
    expect(() => rescaleTime(1, { numerator: 0, denominator: 1 })).toThrow();
    expect(() => clipDurationUs({ ...base, sourceOutUs: 0 })).toThrow();
    expect(() => clipEndUs({ ...base, startUs: MAX_TIME_US })).toThrow();
    expect(() => clipDurationUs({ ...base, sourceOutUs: MAX_TIME_US, rate: { numerator: 1, denominator: 2 } })).toThrow();
    for (const value of [-1, NaN, Infinity, MAX_TIME_US]) expect(() => secondsToTimeUs(value)).toThrow();
  });
  it('keeps source timestamps when a clip is moved, split or duplicated', () => {
    const whole = { ...base, sourceOutUs: 20_000_000 };
    const moved = { ...whole, startUs: 60_000_000 };
    const left = { ...moved, sourceOutUs: 10_000_000 };
    const right = { ...moved, startUs: 70_000_000, sourceInUs: 10_000_000 };
    expect(sourceToTimelineUs(right, 15_000_000)).toBe(sourceToTimelineUs(moved, 15_000_000));
    expect(timelineToSourceUs(left, 70_000_000)).toBeNull();
    expect(timelineToSourceUs(right, 70_000_000)).toBe(10_000_000);
    expect(timelineDurationUs([left, right, { ...whole, startUs: 90_000_000 }])).toBe(110_000_000);
    expect(whole.sourceInUs).toBe(0);
  });
  it('rounds positive halves up and derives empty timeline duration', () => {
    expect(secondsToTimeUs(.0000005)).toBe(1);
    expect(rescaleTime(10, { numerator: 1, denominator: 4 })).toBe(3);
    expect(timelineDurationUs([])).toBe(0);
  });
});
