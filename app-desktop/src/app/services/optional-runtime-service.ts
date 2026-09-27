import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export type OptionalFeature = 'whisperx' | 'bandit' | 'acceleration';
export type RuntimeProfile = 'cpu' | 'cuda';

export interface OptionalRuntimeStatus {
  profile: RuntimeProfile | null;
  installedFeatures: OptionalFeature[];
  nvidiaDetected: boolean;
}

export const optionalRuntimeService = {
  status: () => invoke<OptionalRuntimeStatus>('get_optional_runtime_status'),
  install: (feature: OptionalFeature, profile: RuntimeProfile) =>
    invoke<OptionalRuntimeStatus>('install_optional_runtime', { feature, profile }),
  onProgress: (handler: (message: string) => void) =>
    listen<{ message: string }>('optional-runtime-progress', ({ payload }) => handler(payload.message)),
};
