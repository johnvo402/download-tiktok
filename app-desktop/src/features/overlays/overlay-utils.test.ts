import { describe, expect, it } from 'vitest';
import { clampRect, fitContainRect, moveRect, resizeRect } from './overlay-utils';

describe('video overlay geometry', () => {
  it('fits landscape, portrait and square media into letterboxed preview bounds', () => {
    expect(fitContainRect(1000, 600, 1920, 1080)).toEqual({ x: 0, y: 18.75, width: 1000, height: 562.5 });
    expect(fitContainRect(1000, 600, 1080, 1920)).toEqual({ x: 331.25, y: 0, width: 337.5, height: 600 });
    expect(fitContainRect(800, 600, 1000, 1000)).toEqual({ x: 100, y: 0, width: 600, height: 600 });
  });

  it('keeps dragged rectangles inside the video bounds', () => {
    expect(clampRect({ x: 0.95, y: -0.1, width: 0.2, height: 0.3 })).toEqual({ x: 0.8, y: 0, width: 0.2, height: 0.3 });
    expect(moveRect({ x: 0.1, y: 0.2, width: 0.3, height: 0.4 }, 0.9, -1)).toEqual({ x: 0.7, y: 0, width: 0.3, height: 0.4 });
  });

  it('resizes within bounds and retains an image aspect ratio when requested', () => {
    expect(resizeRect({ x: 0.2, y: 0.2, width: 0.2, height: 0.2 }, 0.1, 0.05, 1)).toEqual({ x: 0.2, y: 0.2, width: 0.3, height: 0.3 });
    expect(resizeRect({ x: 0.7, y: 0.7, width: 0.2, height: 0.2 }, 0.5, 0.5)).toEqual({ x: 0.7, y: 0.7, width: 0.3, height: 0.3 });
  });
});
