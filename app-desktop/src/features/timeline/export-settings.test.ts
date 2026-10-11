import { describe, expect, it } from 'vitest';
import { defaultTimelineExport, exportDurationUs, selectedExportRange } from './export-settings';
import type { TimelineDocument, TimelineClip } from './types';
const doc = { settings: { width: 641, height: 359, frameRate: { numerator: 30000, denominator: 1001 } },
  clips: [{ id: 'a', startUs: 2e6, sourceInUs: 1e6, sourceOutUs: 5e6, rate: { numerator: 2, denominator: 1 } },
    { id: 'b', startUs: 5e6, sourceInUs: 0, sourceOutUs: 3e6, rate: { numerator: 1, denominator: 1 } }] as TimelineClip[] } as TimelineDocument;
describe('timeline export options', () => {
  it('keeps rational FPS and normalizes to encoder compatible even dimensions', () => {
    expect(defaultTimelineExport(doc)).toEqual({ width: 642, height: 360, frameRate: doc.settings.frameRate, startUs: null, endUs: null });
  });
  it('uses selected clips in project time, including their rates and the intervening gap', () => {
    expect(selectedExportRange(doc, ['b', 'a'])).toEqual({ startUs: 2e6, endUs: 8e6 });
    expect(selectedExportRange(doc, ['missing'])).toBeNull();
    expect(exportDurationUs(doc)).toBe(8e6);
    expect(exportDurationUs(doc, { ...defaultTimelineExport(doc), startUs: 3e6, endUs: 6e6 })).toBe(3e6);
  });
});
