import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { ExportCapabilities, VideoCodec, VideoEncoder } from './video-encoding';
export type { ExportCapabilities } from './video-encoding';

export interface ExportOptions {
  quality: 'fast' | 'balanced' | 'high';
  videoCodec: VideoCodec;
  encoder?: VideoEncoder;
  videoBitrateKbps?: number | null;
  applyBlur: boolean;
  applyLogos: boolean;
  burnSubtitles: boolean;
  enableSubtitleBlurOverride: boolean;
  includeOriginalAudio: boolean;
  includeDubbing: boolean;
  ducking: boolean;
  outputDirectory: string | null;
  outputFileName: string;
  originalVolume: number;
  dubbedVolume: number;
  duckingAmount: number;
  dialogueReduction: boolean;
  dialogueReductionAmount: number;
}

export interface SubtitleLayoutWarning {
  segmentId: string;
  start: number;
  end: number;
  message: string;
}

export interface ExportValidation {
  resolvedEncoder: VideoEncoder;
  resolvedEncoderLabel: string;
  valid: boolean;
  width: number;
  height: number;
  duration: number;
  subtitleRegionId: string | null;
  translatedSegments: number;
  fitsNormally: number;
  fitWithSmallerFont: number;
  smallestFontSize: number;
  warnings: SubtitleLayoutWarning[];
  dubbingSegments: number;
  expectedAudio: boolean;
  audioWarnings: string[];
}

export interface ExportHistoryEntry {
  id: string;
  projectId: string;
  file: string | null;
  createdAt: string;
  status: 'completed' | 'failed' | 'cancelled';
  sizeBytes: number | null;
  duration: number | null;
  settings: ExportOptions;
  message: string | null;
}

export interface ExportJobEvent {
  projectId: string;
  jobId: string;
  status: 'queued' | 'running' | 'completed' | 'failed' | 'cancelled';
  progress: number;
  elapsedSeconds: number;
  totalSeconds: number;
  speed: number | null;
  output: string | null;
  message: string;
  layoutWarnings: SubtitleLayoutWarning[];
}

export const renderService = {
  capabilities: (refresh = false) => invoke<ExportCapabilities>('get_export_capabilities', { refresh }),
  validate: (projectId: string, options: ExportOptions) => invoke<ExportValidation>('validate_export', { projectId, options }),
  start: (projectId: string, options: ExportOptions) => invoke<string>('start_export', { projectId, options }),
  cancel: (jobId: string) => invoke<boolean>('cancel_export', { jobId }),
  history: (projectId: string) => invoke<ExportHistoryEntry[]>('get_export_history', { projectId }),
  exportDirectory: (projectId: string) => invoke<string>('get_export_directory', { projectId }),
  openExportFolder: (projectId: string, requestedDirectory: string | null) => invoke<string>('open_export_folder', { projectId, requestedDirectory }),
  onProgress: (handler: (event: ExportJobEvent) => void) => listen<ExportJobEvent>('export-progress', (event) => handler(event.payload)),
};
