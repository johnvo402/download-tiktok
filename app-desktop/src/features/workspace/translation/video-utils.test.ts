import { describe, expect, it } from 'vitest';
import { needsCompatibilityPreview } from './video-utils';

describe('needsCompatibilityPreview', () => {
  it.each(['hevc', 'HEVC', 'h265', 'hvc1', 'hev1', 'x265'])('routes %s through H.264 preview', (codec) => {
    expect(needsCompatibilityPreview(codec)).toBe(true);
  });

  it.each(['h264', 'avc1', 'vp9', 'av1', null, undefined])('keeps %s on native playback', (codec) => {
    expect(needsCompatibilityPreview(codec)).toBe(false);
  });
});
