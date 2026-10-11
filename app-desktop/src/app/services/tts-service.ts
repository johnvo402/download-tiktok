import { invoke, isTauri } from '@tauri-apps/api/core';
import type { TtsEnvironment } from '../../features/transcript/types';

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) throw new Error('VieNeu-TTS cần chạy trong ứng dụng desktop.');
  try { return await invoke<T>(command, args); }
  catch (error) {
    if (error instanceof Error) throw error;
    throw new Error(typeof error === 'string' ? error : 'Thao tác TTS thất bại.');
  }
}

export const ttsService = {
  getEnvironment: () => call<TtsEnvironment>('get_tts_environment'),
  installModel: () => call<void>('install_tts_model'),
  removeModel: () => call<void>('remove_tts_model'),
  previewVoice: (voice: string) => call<string>('preview_tts_voice', { voice }),
  addVoiceProfile: (name: string, sourcePath: string, consent: boolean) => call<void>('add_tts_voice_profile', { name, sourcePath, consent }),
  deleteVoiceProfile: (profileId: string) => call<void>('delete_tts_voice_profile', { profileId }),
  start: (projectId: string, request: { scope: 'all' | 'missing' | 'selected' | 'segment'; segmentIds: string[]; videoContentId?: string }) =>
    call<string>('start_tts_generation', { projectId, request }),
  cancel: (jobId: string) => call<boolean>('cancel_tts_generation', { jobId }),
  getAudioPath: (projectId: string, segmentId: string, videoContentId?: string) => call<string>('get_tts_audio_path', { projectId, segmentId, videoContentId }),
};
