import { invoke, isTauri } from '@tauri-apps/api/core';

export interface StorageUsage {
  modelsBytes: number;
  fasterWhisperBytes: number;
  ocrBytes: number;
  vieNeuTtsBytes: number;
  dialogueSeparationBytes: number;
  otherModelDataBytes: number;
  projectsBytes: number;
  projectCacheBytes: number;
  temporaryBytes: number;
  logsBytes: number;
  aiComponentsBytes: number;
  aiComponentsDirectory: string;
  cacheDirectory: string;
  fasterWhisperDirectory: string;
  ocrDirectory: string;
  vieNeuTtsDirectory: string;
  dialogueSeparationDirectory: string;
  warnings: string[];
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) throw new Error('Quản lý cache chỉ khả dụng trong ứng dụng desktop.');
  try { return await invoke<T>(command, args); }
  catch (error) { throw error instanceof Error ? error : new Error(typeof error === 'string' ? error : 'Thao tác cache thất bại.'); }
}

export const storageService = {
  usage: () => call<StorageUsage>('get_storage_usage'),
  clearTemporary: () => call<number>('clear_temporary_cache'),
  openCacheFolder: () => call<string>('open_cache_folder'),
  openAiComponentsFolder: () => call<string>('open_ai_components_folder'),
  openModelFolder: (kind: 'fasterWhisper' | 'vieNeuTts' | 'ocr' | 'dialogueSeparation') => call<string>('open_model_folder', { kind }),
};

export function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let size = value;
  let index = 0;
  while (size >= 1024 && index < units.length - 1) { size /= 1024; index += 1; }
  return `${size.toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
}
