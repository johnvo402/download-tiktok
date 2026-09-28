import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    if (error instanceof Error) throw error;
    throw new Error(typeof error === 'string' ? error : 'Không thể thao tác với model tách thoại.');
  }
}

export interface DialogueEnvironment {
  runtimeInstalled: boolean;
  cudaAvailable: boolean;
  modelCached: boolean;
  modelBytes: number;
  estimatedDownloadBytes: number;
}

export interface DialogueModelProgress {
  progress: number;
  downloadedBytes: number;
  totalBytes: number;
  message: string;
}

export const dialogueService = {
  environment: () => call<DialogueEnvironment>('get_dialogue_environment'),
  preparePreviewStem: (projectId: string) => call<string>('prepare_dialogue_preview_stem', { projectId }),
  installModel: () => call<void>('install_dialogue_model'),
  installModelFromFile: (sourcePath: string) => call<void>('install_dialogue_model_from_file', { sourcePath }),
  removeModel: () => call<void>('remove_dialogue_model'),
  onModelProgress: (handler: (progress: DialogueModelProgress) => void) =>
    listen<DialogueModelProgress>('dialogue-model-progress', ({ payload }) => handler(payload)),
  onPreviewProgress: (handler: (progress: { message: string }) => void) =>
    listen<{ message: string }>('dialogue-preview-progress', ({ payload }) => handler(payload)),
};
