import type { ClipStyle, ClipTransform, TimelineClip, TimelineDocument, TimelineSettings } from './types';

export const defaultTransform = (): ClipTransform => ({ x: .5, y: .5, scaleX: 1, scaleY: 1, rotation: 0, opacity: 1, flipX: false, flipY: false, fit: 'contain', crop: { x: 0, y: 0, width: 1, height: 1 } });
export const defaultStyle = (): ClipStyle => ({ fontFamily: 'Arial', fontSize: 48, fontWeight: 700, color: '#FFFFFF', backgroundColor: '#000000', backgroundOpacity: .5, alignment: 'center' });
export function clipTypography(clip: TimelineClip, doc?: TimelineDocument): ClipStyle {
  if (clip.style) return clip.style;
  if (clip.content.kind !== 'subtitles') return defaultStyle();
  const id = clip.content.videoContentId, s = doc?.videoContents.find(v => v.id === id)?.settings.subtitle;
  return { ...defaultStyle(), fontFamily: s?.fontFamily ?? 'Arial', fontSize: s?.fontSize ?? 48, fontWeight: s?.fontWeight ?? 700, color: s?.textColor ?? '#FFFFFF', backgroundColor: s?.backgroundColor ?? '#000000', backgroundOpacity: s?.backgroundEnabled ? s.backgroundOpacity ?? .5 : 0, alignment: s?.alignment ?? 'center' };
}
const color = (value: string) => /^#[\da-f]{6}$/i.test(value);
export function validateCanvas(s: TimelineSettings) {
  if (![s.width, s.height].every(v => Number.isInteger(v) && v >= 1 && v <= 16384) || !color(s.backgroundColor)
    || ![s.frameRate.numerator, s.frameRate.denominator].every(v => Number.isSafeInteger(v) && v > 0)) throw new Error('Khung hình phải từ 1 đến 16.384 pixel; màu nền và FPS phải hợp lệ.');
}
export function validateTransform(t: ClipTransform) {
  const c = t.crop;
  if (![t.x, t.y, t.scaleX, t.scaleY, t.rotation, t.opacity, c.x, c.y, c.width, c.height].every(Number.isFinite)
    || t.scaleX <= 0 || t.scaleY <= 0 || t.scaleX > 100 || t.scaleY > 100 || t.opacity < 0 || t.opacity > 1
    || c.x < 0 || c.y < 0 || c.width <= 0 || c.height <= 0 || c.x + c.width > 1 + 1e-9 || c.y + c.height > 1 + 1e-9
    || !['contain', 'cover', 'stretch'].includes(t.fit)) throw new Error('Vị trí, kích thước, độ mờ hoặc vùng crop không hợp lệ.');
}
export function validateStyle(s: ClipStyle) {
  if (!s.fontFamily.trim() || s.fontFamily.length > 96 || !Number.isFinite(s.fontSize) || s.fontSize < 8 || s.fontSize > 240
    || !Number.isInteger(s.fontWeight) || s.fontWeight < 100 || s.fontWeight > 900 || !color(s.color) || !color(s.backgroundColor)
    || !Number.isFinite(s.backgroundOpacity) || s.backgroundOpacity < 0 || s.backgroundOpacity > 1 || !['left', 'center', 'right'].includes(s.alignment)) throw new Error('Kiểu chữ không hợp lệ.');
}
/** Crop remains normalized to the source; transforms operate on the project canvas. */
export function sourceGeometry(width: number, height: number, sourceWidth: number, sourceHeight: number, t: ClipTransform) {
  const c = t.crop, croppedWidth = Math.max(1, sourceWidth) * c.width, croppedHeight = Math.max(1, sourceHeight) * c.height;
  const fit = t.fit === 'cover' ? Math.max(width / croppedWidth, height / croppedHeight) : Math.min(width / croppedWidth, height / croppedHeight);
  const sx = t.fit === 'stretch' ? width / croppedWidth : fit, sy = t.fit === 'stretch' ? height / croppedHeight : fit;
  return { width: sourceWidth * sx, height: sourceHeight * sy, left: (width - croppedWidth * sx) / 2 - c.x * sourceWidth * sx, top: (height - croppedHeight * sy) / 2 - c.y * sourceHeight * sy };
}
export const layerTransform = (t: ClipTransform) => `translate(-50%, -50%) rotate(${t.rotation}deg) scale(${t.scaleX * (t.flipX ? -1 : 1)}, ${t.scaleY * (t.flipY ? -1 : 1)})`;
