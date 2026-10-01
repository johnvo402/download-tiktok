export type VideoCodec = 'h264' | 'hevc';
export type VideoEncoder = 'auto' | 'cpu' | 'nvidia' | 'intel' | 'amd';
export type ConcreteVideoEncoder = Exclude<VideoEncoder, 'auto'>;

export interface EncoderCapability {
  id: ConcreteVideoEncoder;
  label: string;
  vendor: string;
  hardware: boolean;
  h264: boolean;
  hevc: boolean;
  h264Message: string | null;
  hevcMessage: string | null;
}
export interface ExportCapabilities {
  encoders: EncoderCapability[];
  recommendedEncoders: Record<VideoCodec, ConcreteVideoEncoder | null>;
}
export const encoderLabels: Record<VideoEncoder, string> = {
  auto: 'Tự động · khuyên dùng', cpu: 'CPU · Software', nvidia: 'NVIDIA · NVENC', intel: 'Intel · Quick Sync', amd: 'AMD · AMF',
};
export const encoderBackends: ConcreteVideoEncoder[] = ['cpu', 'nvidia', 'intel', 'amd'];
export function encoderSupport(caps: ExportCapabilities | undefined, codec: VideoCodec, id: ConcreteVideoEncoder) {
  return caps?.encoders.find((item) => item.id === id)?.[codec] ?? false;
}
export function encoderStatus(caps: ExportCapabilities | undefined, codec: VideoCodec, requested: VideoEncoder): string {
  if (!caps) return 'Đang kiểm tra khả năng mã hóa video…';
  const resolved = requested === 'auto' ? caps.recommendedEncoders[codec] : requested;
  if (!resolved) return `Chưa có bộ mã hóa ${codec === 'h264' ? 'H.264' : 'HEVC'} khả dụng. Hãy kiểm tra lại.`;
  const item = caps.encoders.find((item) => item.id === resolved);
  if (!item?.[codec]) return item?.[codec === 'h264' ? 'h264Message' : 'hevcMessage'] || `${encoderLabels[resolved]} không hỗ trợ codec đã chọn.`;
  return `${requested === 'auto' ? 'Tự động → ' : ''}${item.label} · ${codec === 'h264' ? 'H.264' : 'HEVC'}`;
}
