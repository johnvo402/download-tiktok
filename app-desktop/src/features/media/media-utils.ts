import type { MediaAsset, TimelineClip } from '../timeline/types';

export const MEDIA_EXTENSIONS = ['mp4', 'mov', 'mkv', 'webm', 'avi', 'm4v', 'png', 'jpg', 'jpeg', 'webp', 'mp3', 'wav', 'm4a', 'aac', 'flac', 'ogg', 'opus'];
export function mediaName(path: string): string { return path.split(/[\\/]/).pop() || path; }
export function mediaUsage(assetId: string, clips: TimelineClip[]): number {
  return clips.filter(clip => 'assetId' in clip.content && clip.content.assetId === assetId).length;
}
export function filterMedia(assets: MediaAsset[], kind: string, query: string): MediaAsset[] {
  const search = query.trim().toLocaleLowerCase();
  return assets.filter(asset => (kind === 'all' || kind === asset.kind)
    && (asset.metadata.originalName || mediaName(asset.relativePath)).toLocaleLowerCase().includes(search));
}
export function mediaDuration(timeUs: number | null): string {
  if (timeUs === null) return 'Ảnh tĩnh';
  const seconds = Math.floor(timeUs / 1_000_000);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
}
