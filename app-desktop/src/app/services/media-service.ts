import { invoke, isTauri } from '@tauri-apps/api/core';
import type { Project } from '../../features/projects/types';
import type { EditLayout } from '../../features/timeline/edit-commands';

export interface MediaImportItem {
  sourcePath: string; assetId: string | null;
  status: 'imported' | 'duplicate' | 'restored' | 'failed'; error: string | null;
}
export interface MediaAssetStatus { assetId: string; state: 'available' | 'missing' | 'invalid'; message: string | null }
export interface MediaCache {
  thumbnailPath: string | null;
  waveform: { schemaVersion: number; checksum: string; durationUs: number; peaks: number[] } | null;
}
async function call<T>(command: string, args: Record<string, unknown>): Promise<T> {
  if (!isTauri()) throw new Error('Kho nguồn cần chạy trong ứng dụng desktop.');
  try { return await invoke<T>(command, args); }
  catch (error) { throw error instanceof Error ? error : new Error(typeof error === 'string' ? error : 'Không thao tác được với kho nguồn.'); }
}
export const mediaService = {
  import: (projectId: string, paths: string[]) => call<{ project: Project; items: MediaImportItem[] }>('import_media_assets', { projectId, paths }),
  inspect: (projectId: string, verify = false) => call<MediaAssetStatus[]>('inspect_media_assets', { projectId, verify }),
  relink: (projectId: string, assetId: string, sourcePath: string) => call<Project>('relink_media_asset', { projectId, assetId, sourcePath }),
  append: (projectId: string, assetId: string, atUs: number | null = null) => call<Project>('append_media_clip', { projectId, assetId, atUs }),
  cache: (projectId: string, assetId: string, kind: 'thumbnail' | 'waveform') => call<MediaCache>('get_media_cache', { projectId, assetId, kind }),
  path: (projectId: string, assetId: string) => call<string>('get_media_asset_path', { projectId, assetId }),
  trackControl: (projectId: string, trackId: string, control: 'locked' | 'enabled' | 'muted' | 'solo', value: boolean) => call<Project>('set_timeline_track_control', { projectId, trackId, control, value }),
  edit: (projectId: string, expected: EditLayout, next: EditLayout) => call<Project>('apply_timeline_edit', { projectId, expected, next }),
};
