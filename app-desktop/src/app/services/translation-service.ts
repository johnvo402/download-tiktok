import { invoke, isTauri } from '@tauri-apps/api/core';
import type { TranslationJobRequest, TranslationProviderId } from '../../features/transcript/types';

export interface AppSettings {
  schemaVersion: number;
  translation: {
    provider: TranslationProviderId;
    batchSize: number;
    maxRequestBytes: number;
    contextSegments: number;
    timeoutSeconds: number;
    gemini: { modelId: string; temperature: number };
    googleCloud: { model: string };
    codexCli: { executable: string; modelId: string | null; timeoutSeconds: number };
  };
  tts: {
    provider: 'vieneu';
    modelId: string;
    device: 'cpu' | 'cuda';
    defaultVoice: string;
    speed: number;
    fitToDuration: boolean;
    voiceProfiles: Array<{ id: string; name: string; audioPath: string }>;
  };
  asr: {
    engine: 'auto' | 'fasterWhisper' | 'openVinoWhisper' | 'whisperX';
    model: 'tiny' | 'base' | 'small' | 'medium' | 'large-v3';
    device: 'auto' | 'cpu' | 'cuda' | 'openvino';
    language: 'auto' | 'en' | 'vi' | 'ja' | 'ko' | 'zh' | 'fr' | 'de' | 'es';
    ocrModel: 'tiny' | 'small' | 'medium';
    ocrDevice: 'auto' | 'cpu' | 'cuda' | 'openvino';
  };
}

export interface AppSettingsResponse {
  settings: AppSettings;
  geminiCredentialConfigured: boolean;
  googleCloudCredentialConfigured: boolean;
}

export interface TranslationTestResult {
  provider: TranslationProviderId;
  success: boolean;
  message: string;
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) throw new Error('Cài đặt provider và dịch transcript cần chạy trong ứng dụng desktop Tauri.');
  try { return await invoke<T>(command, args); }
  catch (error) {
    if (error instanceof Error) throw error;
    throw new Error(typeof error === 'string' ? error : 'Thao tác translation thất bại.');
  }
}

export const translationService = {
  getSettings: () => call<AppSettingsResponse>('get_app_settings'),
  saveSettings: (settings: AppSettings) => call<AppSettingsResponse>('save_app_settings', { settings }),
  setCredential: (provider: TranslationProviderId, secret: string) => call<void>('set_translation_credential', { provider, secret }),
  deleteCredential: (provider: TranslationProviderId) => call<void>('delete_translation_credential', { provider }),
  testProvider: (provider: TranslationProviderId) => call<TranslationTestResult>('test_translation_provider', { provider }),
  start: (projectId: string, request: TranslationJobRequest) => call<string>('start_translation', { projectId, request }),
  cancel: (jobId: string) => call<boolean>('cancel_translation', { jobId }),
};
