export interface Project {
  schemaVersion: number;
  id: string;
  name: string;
  createdAt: string;
  updatedAt: string;
  source: {
    videoPath: string | null;
    originalName?: string | null;
    duration?: number | null;
    width?: number | null;
    height?: number | null;
    fps?: number | null;
    videoCodec?: string | null;
    videoProfile?: string | null;
    videoPixelFormat?: string | null;
    audioCodec?: string | null;
    fileSize?: number | null;
    checksumSha256?: string | null;
    audioStreams?: AudioStreamMetadata[];
    subtitleTracks?: SubtitleTrackMetadata[];
  };
  transcript?: TranscriptMetadata | null;
  translation: {
    sourceLanguage: string;
    targetLanguage: string;
    customPrompt: string;
    /** Project glossary and translation memory; multilingual aliases share one approved target name. */
    context: string;
    segments: TranslationSegment[];
  };
  subtitle: {
    enabled: boolean;
    fontSize: number;
    textColor: string;
    outline: boolean;
    bottomMargin: number;
    fontFamily: string;
    fontWeight: number;
    outlineColor: string;
    outlineWidth: number;
    backgroundEnabled: boolean;
    backgroundColor: string;
    backgroundOpacity: number;
    alignment: 'left' | 'center' | 'right';
    maxWidth: number;
    lineHeight: number;
    verticalPosition: number;
    paddingX: number;
    paddingY: number;
    autoFit: boolean;
    minFontSize: number;
    verticalAlignment: 'top' | 'middle' | 'bottom';
    /** Normalized subtitle placement used when no blur region has the subtitle role. */
    region?: { x: number; y: number; width: number; height: number } | null;
  };
  dubbing: {
    enabled: boolean;
    provider: 'vieneu';
    modelId: string;
    voice: string | null;
    voiceProfileId: string | null;
    speed: number | null;
    fitMode: 'natural' | 'fit' | 'custom' | null;
    originalVolume: number;
    dubbedVolume: number;
    muteOriginal: boolean;
    duckingEnabled: boolean;
    duckingAmount: number;
    dialogueReductionEnabled: boolean;
    dialogueReductionAmount: number;
  };
  blurRegions: BlurRegion[];
  logos: ProjectLogo[];
  textOverlays: ProjectTextOverlay[];
  transcriptSettings: TranscriptSettings;
}

export interface ProjectTextOverlay {
  id: string;
  text: string;
  x: number;
  y: number;
  width: number;
  height: number;
  enabled: boolean;
  fontFamily: string;
  fontSize: number;
  fontWeight: number;
  color: string;
  opacity: number;
  alignment: 'left' | 'center' | 'right';
}

export interface TranscriptSettings {
  mode: 'speechRecognition' | 'ocr';
  ocrFps: 1 | 2 | 4;
  ocrRegion: { x: number; y: number; width: number; height: number } | null;
}

export interface AudioStreamMetadata {
  streamIndex: number;
  codec: string;
  language: string | null;
  title: string | null;
  channels: number | null;
  sampleRate: number | null;
}

export interface SubtitleTrackMetadata {
  streamIndex: number;
  codec: string;
  language: string | null;
  title: string | null;
  textSupported: boolean;
  imageBased: boolean;
}

export interface TranscriptMetadata {
  status: 'notStarted' | 'running' | 'completed' | 'failed' | 'canceled';
  path: string;
  sourceType?: 'subtitle' | 'asr' | 'ocr' | null;
  sourceTrack?: number | null;
  language?: string | null;
  segmentCount: number;
}

export interface TranslationSegment {
  id: string;
  startMs: number;
  endMs: number;
  speakerId?: string;
  source: string;
  translated: string;
  translationStatus: 'empty' | 'machine' | 'edited';
  tts?: {
    audioRelativePath: string;
    fingerprint: string;
    durationMs: number;
    status: 'ready' | 'stale' | 'failed';
  };
}

export interface BlurRegion {
  id: string;
  x: number;
  y: number;
  width: number;
  height: number;
  startMs: number;
  endMs: number | null;
  enabled: boolean;
  strength: number;
  effect?: 'blur' | 'sttn';
  role: 'generic' | 'subtitle';
  /** Edge feather radius as a percentage of the region's shorter side. */
  edgeBlend: number;
  /** FFmpeg noise strength from 0–100, applied only inside the blurred area. */
  pixelNoise: number;
  /** Sparse RapidOCR scan rate used only by AI text removal. */
  textRemovalScanFps: 1 | 2;
  /** Defaults to OCR-guided for projects created by older app versions. */
  textRemovalMode?: 'ocrGuided' | 'fullRegion';
}

export interface ProjectLogo {
  id: string;
  relativePath: string;
  originalName: string;
  x: number;
  y: number;
  width: number;
  height: number;
  enabled: boolean;
  opacity: number;
  maintainAspectRatio: boolean;
  /** Image asset used only by the video merge tool, not project render overlays. */
  mergeOnly: boolean;
}

export interface ProjectListResponse {
  projects: Project[];
  warnings: string[];
}

export interface ProjectSummary extends Pick<Project, 'id' | 'name' | 'createdAt' | 'updatedAt'> {
  status: 'ready';
}
