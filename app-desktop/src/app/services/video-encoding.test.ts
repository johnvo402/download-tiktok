import { describe, expect, it } from 'vitest';
import { encoderStatus, encoderSupport, type ExportCapabilities } from './video-encoding';

const caps: ExportCapabilities = {
  encoders: [
    { id: 'nvidia', label: 'NVIDIA NVENC', vendor: 'nvidia', hardware: true, h264: true, hevc: false, h264Message: null, hevcMessage: 'NVIDIA không hỗ trợ HEVC.' },
    { id: 'intel', label: 'Intel Quick Sync', vendor: 'intel', hardware: true, h264: true, hevc: true, h264Message: null, hevcMessage: null },
  ],
  recommendedEncoders: { h264: 'nvidia', hevc: 'intel' },
};
describe('video encoding capability UI', () => {
  it('displays Auto recommendation for the selected codec', () => {
    expect(encoderStatus(caps, 'h264', 'auto')).toContain('NVIDIA NVENC');
    expect(encoderStatus(caps, 'hevc', 'auto')).toContain('Intel Quick Sync');
  });
  it('does not substitute manual selection when codec support changes', () => {
    expect(encoderSupport(caps, 'hevc', 'nvidia')).toBe(false);
    expect(encoderStatus(caps, 'hevc', 'nvidia')).toBe('NVIDIA không hỗ trợ HEVC.');
  });
  it('handles checking and no available encoders', () => {
    expect(encoderStatus(undefined, 'h264', 'auto')).toContain('Đang kiểm tra');
    expect(encoderStatus({ encoders: [], recommendedEncoders: { h264: null, hevc: null } }, 'hevc', 'auto')).toContain('Chưa có');
  });
});
