import { invoke, isTauri } from '@tauri-apps/api/core';
import type { AiAction, SceneAudio, StudioDraft } from '../../features/video-studio/types';

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) throw new Error('Tạo video AI cần chạy trong ứng dụng desktop.');
  try { return await invoke<T>(command, args); }
  catch (error) { throw error instanceof Error ? error : new Error(typeof error === 'string' ? error : 'Không thể xử lý video AI.'); }
}
export const studioService = {
  exists: (projectId: string) => call<boolean>('has_studio_draft', { projectId }),
  load: (projectId: string) => call<StudioDraft | null>('load_studio_draft', { projectId }),
  save: (draft: StudioDraft) => call<void>('save_studio_draft', { draft }),
  ai: <T>(draft: StudioDraft, action: AiAction, jobId: string, chapterId: string | null = null, sceneId: string | null = null, instruction = '') => call<T>('studio_ai', { draft, action, jobId, chapterId, sceneId, instruction }),
  importImage: (projectId: string, sourcePath: string) => call<string>('studio_import_image', { projectId, sourcePath }),
  asset: (projectId: string, relative: string) => call<string>('studio_asset', { projectId, relative }),
  voice: (draft: StudioDraft, sceneId: string, jobId: string) => call<SceneAudio>('studio_voice', { draft, sceneId, jobId }),
  export: (draft: StudioDraft, jobId: string) => call<string>('studio_export', { draft, jobId }),
  cancel: (jobId: string) => call<boolean>('cancel_studio_job', { jobId }),
};
