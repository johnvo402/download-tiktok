const CODECS_REQUIRING_COMPATIBILITY_PREVIEW = new Set([
  'hevc',
  'h265',
  'hvc1',
  'hev1',
  'x265',
]);

/**
 * HEVC decoding in Windows WebView depends on the machine's licensed media
 * decoder. Use the H.264 preview up front so playback does not depend on it.
 */
export function needsCompatibilityPreview(codec: string | null | undefined): boolean {
  return CODECS_REQUIRING_COMPATIBILITY_PREVIEW.has(codec?.trim().toLowerCase() ?? '');
}
