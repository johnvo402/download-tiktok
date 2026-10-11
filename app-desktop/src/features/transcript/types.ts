export type TranscriptSourceType = 'subtitle' | 'asr' | 'ocr';
export type TranslationStatus = 'pending' | 'translated' | 'edited' | 'failed';

export interface TranscriptSegment {
  id: string;
  start: number;
  end: number;
  source: string;
  translated: string;
  sourceRevision: number;
  translatedRevision: number | null;
  translationMeta: TranscriptTranslationMeta | null;
  speakerId: string | null;
  sourceType: TranscriptSourceType;
  translationStatus: TranslationStatus;
  tts: {
    audioPath: string | null;
    duration: number | null;
    fingerprint?: string | null;
    status?: 'ready' | 'outdated' | 'durationOverflow' | 'failed' | null;
    voice?: string | null;
    speed?: number | null;
    textHash?: string | null;
    generatedAt?: string | null;
    provider?: 'vieneu' | null;
    modelId?: string | null;
    modelRevision?: string | null;
    warning?: string | null;
  } | null;
}

export type TranslationProviderId = 'gemini' | 'googleCloud' | 'codexCli';
export type TranslationScope = 'all' | 'missing' | 'selected' | 'segment';

export interface TranslationJobRequest {
  videoContentId?: string;
  scope: TranslationScope;
  segmentIds: string[];
  overwriteEdited: boolean;
}

export interface TranslationJobEvent {
  projectId: string;
  videoContentId?: string | null;
  contentRevision?: number | null;
  jobId: string;
  status: 'queued' | 'running' | 'completed' | 'partial' | 'failed' | 'cancelled';
  provider: TranslationProviderId;
  message: string;
  currentBatch: number;
  totalBatches: number;
  completedBatches: number;
  translatedSegments: number;
  totalSegments: number;
  errorCode?: string;
}

export interface TranscriptTranslationMeta {
  provider: 'gemini' | 'googleCloud' | 'codexCli';
  model: string | null;
  targetLanguage: string;
  translatedAt: string;
}

export interface Transcript {
  schemaVersion: number;
  projectId: string;
  videoContentId?: string | null;
  contentRevision?: number | null;
  sourceType: TranscriptSourceType;
  sourceTrack: number | null;
  language: string | null;
  videoChecksum: string | null;
  segments: TranscriptSegment[];
}

export interface TranscriptionEvent {
  projectId: string;
  videoContentId?: string | null;
  contentRevision?: number | null;
  jobId: string;
  status: 'progress' | 'completed' | 'failed' | 'canceled';
  message: string;
  stage?: string;
  progress?: number;
}

export interface TtsEnvironment {
  /** Legacy field name; this indicates the VieNeu SDK package is installed. */
  workerInstalled: boolean;
  modelCached: boolean;
  cudaAvailable: boolean;
  model: string;
  modelRevision: string | null;
  voices: string[];
  estimatedDownloadBytes: number;
  pythonPackage: string;
}

export interface TtsJobEvent {
  projectId: string;
  videoContentId?: string | null;
  contentRevision?: number | null;
  jobId: string;
  status: 'queued' | 'running' | 'completed' | 'partial' | 'failed' | 'cancelled';
  message: string;
  progress: number;
  completedSegments: number;
  totalSegments: number;
  segmentId?: string | null;
}

export interface AsrEnvironment {
  wordTimestamps: boolean;
  intelRuntimeInstalled: boolean;
  intelGpuAvailable: boolean;
  intelModelSupported: boolean;
  intelModelCached: boolean;
  ctranslateModelCached: boolean;
  intelEstimatedDownloadBytes: number;
  resolvedBackend: 'cpu' | 'cuda' | 'openvino' | null;
  resolvedEngine: string | null;
  resolvedLabel: string | null;
  selectionError: string | null;
  backends: Record<string, boolean>;
  workerInstalled: boolean;
  modelCached: boolean;
  cudaAvailable: boolean;
  model: string;
  whisperXInstalled: boolean;
  alignmentModelCached: boolean;
  alignmentLanguage: string | null;
}

export interface OcrEnvironment {
  workerInstalled: boolean;
  ocrInstalled: boolean;
  runtimeInstalled: boolean;
  modelCached: boolean;
  engine: string;
  model: string;
  modelProfile: 'tiny' | 'small' | 'medium';
  modelBytes: number;
  estimatedDownloadBytes: number;
  cudaAvailable: boolean;
  activeDevice: 'cpu' | 'cuda' | 'openvino';
  backends?: Record<string, { backend: string; available: boolean; device: string | null; label: string;
    message: string | null; installed: boolean; version: string | null }>;
  recommendedBackend?: 'cpu' | 'cuda' | 'openvino';
  resolvedLabel?: string;
  onnxRuntimeVersion?: string | null;
  onnxRuntimeDevice?: string | null;
  executionProviders?: string[];
  runtimeError?: string | null;
}

export interface TextRemovalEnvironment {
  sttnModelInstalled: boolean;
  sttnModelBytes: number;
  sttnEstimatedDownloadBytes: number;
  sttnRuntimeInstalled: boolean;
  sttnCudaAvailable: boolean;
  ocrReady: boolean;
  ocrModel: 'tiny' | 'small' | 'medium';
  ocrCudaAvailable: boolean;
  cudaAvailable: boolean;
}
