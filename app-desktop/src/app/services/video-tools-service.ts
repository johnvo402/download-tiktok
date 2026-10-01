import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { VideoCodec, VideoEncoder } from './video-encoding';

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

export type MergeFrameMode = 'source' | 'portrait' | 'landscape';
export type MergeVideoCodec = VideoCodec;
export type MergeEncoder = VideoEncoder;

export interface MergeLogoSettings {
  logoId: string;
  x: number;
  y: number;
  width: number;
  height: number;
  opacity: number;
  maintainAspectRatio: boolean;
}

export interface MergeTextSettings {
  text: string;
  x: number;
  y: number;
  width: number;
  height: number;
  opacity: number;
  fontFamily: string;
  fontSize: number;
  fontWeight: number;
  color: string;
  alignment: 'left' | 'center' | 'right';
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

export interface TikTokCookieInfo {
  fileName: string;
  totalCookies: number;
  tiktokCookies: number;
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
  hasTikTokBrowserSession: () => call<boolean>('has_tiktok_browser_session'),
  startTikTokBrowserLogin: (projectId: string) =>
    call<string>('start_tiktok_browser_login', { projectId }),
  inspectTikTokCookies: (cookieFile: string) =>
    call<TikTokCookieInfo>('inspect_tiktok_cookie_file', { cookieFile }),
  outputDirectory: (projectId: string, purpose: 'tiktok' | 'merge') =>
    call<string>('get_video_tool_output_directory', { projectId, purpose }),
  scanVideos: (sourceDirectory: string) => call<MergeScanResult>('scan_merge_videos', { sourceDirectory }),
  startTikTokDownload: (projectId: string, url: string, outputDirectory: string, cookieFile: string | null, useBrowserCookies: boolean) =>
    call<string>('start_tiktok_drama_download', { projectId, url, outputDirectory, cookieFile, useBrowserCookies }),
  startMerge: (projectId: string, sourceDirectory: string, filePaths: string[], outputDirectory: string, frameMode: MergeFrameMode, videoCodec: MergeVideoCodec, encoder: MergeEncoder, logos: MergeLogoSettings[] = [], texts: MergeTextSettings[] = []) =>
    call<string>('start_video_merge', { projectId, sourceDirectory, filePaths, outputDirectory, frameMode, videoCodec, encoder, logos, texts }),
  onProgress: (handler: (event: VideoToolProgress) => void) =>
    listen<VideoToolProgress>('video-tool-progress', (event) => handler(event.payload)),
};
