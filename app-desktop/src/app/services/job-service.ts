import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export type UnifiedJobStatus = 'queued' | 'waiting' | 'running' | 'completed' | 'partial' | 'failed' | 'cancelled' | 'interrupted';

export interface UnifiedJob {
  id: string;
  projectId: string | null;
  projectName: string | null;
  type: 'transcribe' | 'translate' | 'tts' | 'export' | string;
  status: UnifiedJobStatus;
  priority: 'low' | 'normal' | 'high';
  resources: string[];
  progress: number;
  message: string;
  createdAt: string;
  startedAt: string | null;
  completedAt: string | null;
  attempt: number;
  retryOf: string | null;
  error: { code: string; message: string; retryable: boolean } | null;
  retryPayload: unknown | null;
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) throw new Error('Hàng đợi job chỉ khả dụng trong ứng dụng desktop.');
  try { return await invoke<T>(command, args); }
  catch (error) {
    if (error instanceof Error) throw error;
    throw new Error(typeof error === 'string' ? error : 'Không thể thao tác với hàng đợi job.');
  }
}

export const jobService = {
  list: () => call<UnifiedJob[]>('list_jobs'),
  cancel: (jobId: string) => call<boolean>('cancel_unified_job', { jobId }),
  retry: (jobId: string) => call<string>('retry_unified_job', { jobId }),
  onUpdated: (handler: (job: UnifiedJob) => void) =>
    listen<UnifiedJob>('job-record-updated', (event) => handler(event.payload)),
};

