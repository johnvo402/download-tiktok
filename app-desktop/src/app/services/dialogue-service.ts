import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

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
  environment: () => invoke<DialogueEnvironment>('get_dialogue_environment'),
  installModel: () => invoke<void>('install_dialogue_model'),
  removeModel: () => invoke<void>('remove_dialogue_model'),
  onModelProgress: (handler: (progress: DialogueModelProgress) => void) =>
    listen<DialogueModelProgress>('dialogue-model-progress', ({ payload }) => handler(payload)),
};
