import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export type OptionalFeature = 'whisperx' | 'bandit' | 'acceleration' | 'sttn' | 'ocr_openvino' | 'whisper_openvino';
export type RuntimeProfile = 'cpu' | 'cuda' | 'openvino';

export interface OptionalRuntimeStatus {
  profile: RuntimeProfile | null;
  installedFeatures: OptionalFeature[];
  nvidiaDetected: boolean;
  featureProfiles: Partial<Record<OptionalFeature, RuntimeProfile>>;
  runtimeVersion: number;
  health: Partial<Record<OptionalFeature,'ready'|'missing'|'broken'|'update_required'>>;
}

export interface RuntimeInstallPlan {
  feature: OptionalFeature; backend: RuntimeProfile; alreadyInstalled: boolean;
  missingComponents: { label: string; downloadBytes: number; installedBytes: number }[];
  reusedComponents: string[]; estimatedDownloadBytes: number; estimatedInstalledBytes: number; stagingBytes: number;
}
export interface FeatureHealth { state:'ready'|'missing'|'broken'|'update_required'; feature: OptionalFeature; installed: boolean; verified: boolean; backend: RuntimeProfile | null; message: string }

export const optionalRuntimeService = {
  status: () => invoke<OptionalRuntimeStatus>('get_optional_runtime_status'),
  plan: (feature: OptionalFeature, profile: RuntimeProfile) => invoke<RuntimeInstallPlan>('get_optional_runtime_plan', { feature, profile }),
  check: (feature: OptionalFeature) => invoke<FeatureHealth>('check_optional_runtime_feature', { feature }),
  uninstall: (feature: OptionalFeature) => invoke<OptionalRuntimeStatus>('uninstall_optional_runtime', { feature }),
  install: (feature: OptionalFeature, profile: RuntimeProfile) =>
    invoke<OptionalRuntimeStatus>('install_optional_runtime', { feature, profile }),
  onProgress: (handler: (message: string) => void) =>
    listen<{ message: string }>('optional-runtime-progress', ({ payload }) => handler(payload.message)),
};
