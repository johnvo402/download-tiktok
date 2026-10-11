import { describe, expect, it } from 'vitest';
import { defaultStyle, defaultTransform, layerTransform, sourceGeometry, validateCanvas, validateStyle, validateTransform } from './visual-layout';

describe('source crop and canvas composition', () => {
  it('fits the cropped source, retains its original coordinate plane, and distinguishes fill/stretch', () => {
    const t = defaultTransform(); t.crop = { x: .25, y: .1, width: .5, height: .8 };
    const fit = sourceGeometry(1080, 1920, 1920, 1080, t);
    expect(fit.width).toBe(2160); expect(fit.left + fit.width * .25).toBe(0);
    expect(fit.top + fit.height * .1).toBeCloseTo((1920 - 864 * 1.125) / 2);
    const fill = sourceGeometry(1080, 1920, 1920, 1080, { ...t, fit: 'cover' });
    expect(fill.top + fill.height * .1).toBeCloseTo(0); expect(fill.left + fill.width * .25).toBeLessThan(0);
    const stretch = sourceGeometry(1080, 1920, 1920, 1080, { ...t, fit: 'stretch' });
    expect(stretch.width * .5).toBe(1080); expect(stretch.height * .8).toBeCloseTo(1920);
  });
  it('positions by center and applies rotation, scale and flips around that center', () => {
    expect(layerTransform({ ...defaultTransform(), rotation: 45, scaleX: .25, scaleY: .5, flipX: true })).toBe('translate(-50%, -50%) rotate(45deg) scale(-0.25, 0.5)');
  });
  it('rejects malformed crops, nonfinite transforms, invalid typography and oversized canvas', () => {
    expect(() => validateTransform({ ...defaultTransform(), crop: { x: .6, y: 0, width: .5, height: 1 } })).toThrow(/crop/);
    expect(() => validateTransform({ ...defaultTransform(), rotation: NaN })).toThrow();
    expect(() => validateStyle({ ...defaultStyle(), color: 'red' })).toThrow();
    expect(() => validateCanvas({ width: 20000, height: 1080, frameRate: { numerator: 30, denominator: 1 }, backgroundColor: '#000000' })).toThrow();
  });
});
