import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export interface MergeVideoInfo {
  path: string;
  fileName: string;
  sizeBytes: number;
  duration: number;
  width: number;
  height: number;
  fps: number | null;
  videoCodec: string;
  audioCodec: string | null;
  audioChannels: number | null;
  sampleRate: number | null;
  pixelFormat: string | null;
}

export interface SkippedMergeVideo {
  fileName: string;
  reason: string;
}

export interface MergeScanResult {
  videos: MergeVideoInfo[];
  skipped: SkippedMergeVideo[];
}

export interface TikTokDramaInfo {
  title: string;
  tags: string[];
  description: string;
  playCount: string | null;
  posterFile: string | null;
  totalEpisodes: number;
  episodeNumbers: number[];
  outputDirectory: string;
}

export interface VideoToolProgress {
  projectId: string;
  jobId: string;
  status: 'queued' | 'waiting' | 'running' | 'completed' | 'failed' | 'cancelled';
  progress: number;
  message: string;
  stage: string | null;
  details: TikTokDramaInfo | null;
  output: string | null;
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) throw new Error('Tính năng Kéo video chỉ khả dụng trong ứng dụng desktop.');
  try { return await invoke<T>(command, args); }
  catch (error) {
    if (error instanceof Error) throw error;
    throw new Error(typeof error === 'string' ? error : 'Không thể thao tác với video.');
  }
}

export const videoToolsService = {
  outputDirectory: (projectId: string, purpose: 'tiktok' | 'merge') =>
    call<string>('get_video_tool_output_directory', { projectId, purpose }),
  scanVideos: (sourceDirectory: string) => call<MergeScanResult>('scan_merge_videos', { sourceDirectory }),
  startTikTokDownload: (projectId: string, url: string, outputDirectory: string, cookieFile: string | null) =>
    call<string>('start_tiktok_drama_download', { projectId, url, outputDirectory, cookieFile }),
  startMerge: (projectId: string, sourceDirectory: string, filePaths: string[], outputDirectory: string) =>
    call<string>('start_video_merge', { projectId, sourceDirectory, filePaths, outputDirectory }),
  onProgress: (handler: (event: VideoToolProgress) => void) =>
    listen<VideoToolProgress>('video-tool-progress', (event) => handler(event.payload)),
};
