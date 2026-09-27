/**
 * Only bypass the compatibility proxy for the conservative WebView-safe
 * combination we generate ourselves: 8-bit H.264 4:2:0 and AAC audio.
 * Unknown/missing metadata is treated as unsafe, which also protects projects
 * created by older app versions that do not store profile or pixel format.
 */
export function needsCompatibilityPreview(
  codec: string | null | undefined,
  profile: string | null | undefined,
  pixelFormat: string | null | undefined,
  audioCodec: string | null | undefined,
): boolean {
  const normalizedCodec = codec?.trim().toLowerCase();
  const normalizedProfile = profile?.trim().toLowerCase();
  const normalizedPixelFormat = pixelFormat?.trim().toLowerCase();
  const normalizedAudioCodec = audioCodec?.trim().toLowerCase();
  const safeH264Profile = normalizedProfile === 'baseline'
    || normalizedProfile === 'constrained baseline'
    || normalizedProfile === 'main'
    || normalizedProfile === 'high';
  const safeAudio = !normalizedAudioCodec || normalizedAudioCodec === 'aac';

  return normalizedCodec !== 'h264'
    || !safeH264Profile
    || normalizedPixelFormat !== 'yuv420p'
    || !safeAudio;
}
