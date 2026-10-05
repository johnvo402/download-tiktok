import { invoke, isTauri } from '@tauri-apps/api/core';

export interface RuntimeHealth {
  appVersion: string;
  buildCommit: string;
  buildDate: string;
  releaseChannel: string;
  os: string;
  architecture: string;
  projectStorageAvailable: boolean;
  ffmpegAvailable: boolean;
  ffmpegVersion: string | null;
  ffprobeAvailable: boolean;
  workerAvailable: boolean;
  workerMode: 'bundled' | 'development' | 'missing' | string;
  workerVersion: string | null;
  cpuThreads: number;
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) throw new Error('Chẩn đoán chỉ khả dụng trong ứng dụng desktop.');
  try { return await invoke<T>(command, args); }
  catch (error) { throw error instanceof Error ? error : new Error(typeof error === 'string' ? error : 'Không thể kiểm tra môi trường ứng dụng.'); }
}

export const runtimeService = {
  health: (deep = true) => call<RuntimeHealth>('get_runtime_health', { deep }),
  exportDiagnostics: (destination: string) => call<string>('export_diagnostics', { destination }),
};
