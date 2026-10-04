import { createSignal } from 'solid-js';
import type { Project, ProjectListResponse } from '../features/projects/types';
import type { Transcript, TranscriptionEvent, TranslationJobEvent, TtsJobEvent } from '../features/transcript/types';

export const [projects, setProjects] = createSignal<Project[]>([]);
export const [currentProject, setCurrentProject] = createSignal<Project>();
export const [currentTranscript, setCurrentTranscript] = createSignal<Transcript>();
export const [playbackTime, setPlaybackTime] = createSignal(0);
export const [seekRequest, setSeekRequest] = createSignal<number | null>(null);
export const [videoPlaying, setVideoPlaying] = createSignal(false);
export const [ocrRegionPicking, setOcrRegionPicking] = createSignal(false);
export const [blurRegionPicking, setBlurRegionPicking] = createSignal(false);
export const [subtitleRegionPicking, setSubtitleRegionPicking] = createSignal(false);
export const [ocrRegionEditing, setOcrRegionEditing] = createSignal(false);
export const [segmentVoicePreviewActive, setSegmentVoicePreviewActive] = createSignal(false);
export const [projectSaveState, setProjectSaveState] = createSignal<'saved' | 'saving' | 'error'>('saved');
export const [transcriptionEvent, setTranscriptionEvent] = createSignal<TranscriptionEvent>();
export const [translationEvent, setTranslationEvent] = createSignal<TranslationJobEvent>();
export const [ttsEvent, setTtsEvent] = createSignal<TtsJobEvent>();
export const [batchProcessingActive, setBatchProcessingActive] = createSignal(false);
export const [projectLoadWarnings, setProjectLoadWarnings] = createSignal<string[]>([]);
export const [projectsLoading, setProjectsLoading] = createSignal(true);
export const [appStartupReady, setAppStartupReady] = createSignal(false);

export interface AiRemovalPreview {
  nativeCache?: boolean;
  projectId: string;
  regionId: string;
  path: string;
  startSeconds: number;
  endSeconds: number;
  fps: number;
}

export const [aiRemovalPreview, setAiRemovalPreview] = createSignal<AiRemovalPreview>();

export type OverlaySelection =
  | { type: 'blur'; id: string }
  | { type: 'logo'; id: string }
  | { type: 'text'; id: string }
  | { type: 'subtitle' }
  | null;

export const [selectedOverlay, setSelectedOverlay] = createSignal<OverlaySelection>(null);
export type WorkspacePanelTab = 'transcript' | 'media' | 'properties';
export const [activeWorkspacePanelTab, setActiveWorkspacePanelTab] = createSignal<WorkspacePanelTab>('transcript');

export interface Notice {
  kind: 'success' | 'error' | 'info';
  message: string;
}

export const [notice, setNotice] = createSignal<Notice>();

export function applyProjectList(result: ProjectListResponse) {
  setProjects(result.projects);
  setProjectLoadWarnings(result.warnings);
}
