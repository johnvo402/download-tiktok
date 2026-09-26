import { invoke, isTauri } from '@tauri-apps/api/core';
import type { BatchVideoQueue } from '../../features/workspace/batch/types';

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) throw new Error('Hàng đợi video cần chạy trong ứng dụng desktop Tauri.');
  try { return await invoke<T>(command, args); }
  catch (error) {
    if (error instanceof Error) throw error;
    throw new Error(typeof error === 'string' ? error : 'Không thể lưu hàng đợi video.');
  }
}

export const batchVideoService = {
  get: (projectId: string) => call<BatchVideoQueue | null>('get_batch_video_queue', { projectId }),
  save: (projectId: string, queue: BatchVideoQueue) => call<BatchVideoQueue>('save_batch_video_queue', { projectId, queue }),
  clear: (projectId: string) => call<void>('clear_batch_video_queue', { projectId }),
};
