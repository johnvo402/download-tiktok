import { describe, expect, it } from 'vitest';
import { blurVisualParams } from './blur-visual-params';
describe('shared blur rendering contract', () => {
  it('scales source-pixel sigma once and feathers in video geometry', () => {
    const value = blurVisualParams(50, 50, { x: 0.1, y: 0.7, width: 0.8, height: 0.2 }, 1920, 1080, 540);
    expect(value.sigma).toBe(100); expect(value.previewSigma).toBe(50);
    expect(value.featherYPercent).toBeCloseTo(10); expect(value.featherXPercent).toBeCloseTo(1.40625);
  });
  it('keeps maximum blur opaque at the edge and works in portrait', () => {
    const value = blurVisualParams(100, 50, { x: 0, y: 0.8, width: 1, height: 0.2 }, 1080, 1920, 960);
    expect(value.previewSigma).toBe(100); expect(value.featherXPercent).toBe(0); expect(value.featherYPercent).toBe(0);
  });
});
