import { invoke, isTauri } from '@tauri-apps/api/core';
import type { Project, ProjectListResponse, SubtitleTrackMetadata, TranscriptSettings } from '../../features/projects/types';
import type { AsrEnvironment, OcrEnvironment, TextRemovalEnvironment, Transcript } from '../../features/transcript/types';

function asUserMessage(error: unknown): Error {
  if (error instanceof Error) return error;
  if (typeof error === 'string') return new Error(error);
  return new Error('Đã xảy ra lỗi khi thao tác với dự án. Vui lòng thử lại.');
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) {
    throw new Error('Quản lý dự án cần chạy trong ứng dụng desktop Tauri để truy cập dữ liệu trên thiết bị.');
  }
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw asUserMessage(error);
  }
}

export const projectService = {
  list: () => call<ProjectListResponse>('list_projects'),
  create: (name: string, storageDirectory: string | null = null) => call<Project>('create_project', { name, storageDirectory }),
  get: (id: string) => call<Project>('get_project', { id }),
  rename: (id: string, name: string) => call<Project>('rename_project', { id, name }),
  delete: (id: string) => call<void>('delete_project', { id }),
  importVideo: (projectId: string, sourcePath: string, replace = false) => call<{
    project: Project;
    duplicate: boolean;
    replaced: boolean;
  }>('import_video', { projectId, sourcePath, replace }),
  getVideoAssetPath: (projectId: string) => call<string>('get_video_asset_path', { projectId }),
  createPreviewProxy: (projectId: string, forceRebuild = false) => call<string>('create_preview_proxy', { projectId, forceRebuild }),
  getVideoMetadata: (projectId: string) => call<Project['source']>('get_video_metadata', { projectId }),
  getSubtitleTracks: (projectId: string) => call<SubtitleTrackMetadata[]>('get_subtitle_tracks', { projectId }),
  extractSubtitle: (projectId: string, trackIndex: number) => call<Transcript>('extract_subtitle', { projectId, trackIndex }),
  getTranscript: (projectId: string) => call<Transcript | null>('get_transcript', { projectId }),
  saveTranscript: (projectId: string, transcript: Transcript) => call<Project>('save_transcript', { projectId, transcript }),
  updateTranslationLanguages: (projectId: string, sourceLanguage: string, targetLanguage: string) => call<Project>('update_translation_languages', { projectId, sourceLanguage, targetLanguage }),
  updateSubtitleSettings: (projectId: string, subtitle: Project['subtitle']) => call<Project>('update_subtitle_settings', { projectId, subtitle }),
  saveVisualSettings: (projectId: string, settings: Pick<Project, 'subtitle' | 'blurRegions' | 'logos' | 'textOverlays'>) => call<Project>('save_visual_settings', { projectId, settings }),
  importLogo: (projectId: string, sourcePath: string) => call<{
    project: Project;
    logo: Project['logos'][number] | null;
    cleanupWarning: string | null;
  }>('import_logo', { projectId, sourcePath }),
  replaceLogo: (projectId: string, logoId: string, sourcePath: string) => call<{
    project: Project;
    logo: Project['logos'][number] | null;
    cleanupWarning: string | null;
  }>('replace_logo', { projectId, logoId, sourcePath }),
  removeLogo: (projectId: string, logoId: string) => call<{
    project: Project;
    logo: Project['logos'][number] | null;
    cleanupWarning: string | null;
  }>('remove_logo', { projectId, logoId }),
  getLogoAssetPath: (projectId: string, logoId: string) => call<string>('get_logo_asset_path', { projectId, logoId }),
  updateDubbingSettings: (projectId: string, dubbing: Project['dubbing']) => call<Project>('update_dubbing_settings', { projectId, dubbing }),
  updateProjectPreferences: (projectId: string, preferences: { customPrompt: string; translationContext: string; voice: string | null; voiceProfileId: string | null; transcriptSettings: TranscriptSettings }) => call<Project>('update_project_preferences', { projectId, ...preferences }),
  updateTranscriptSettings: (projectId: string, settings: TranscriptSettings) => call<Project>('update_transcript_settings', { projectId, settings }),
  getAsrEnvironment: (model: string, alignmentLanguage?: string, engine?: string, device?: string) => call<AsrEnvironment>('get_asr_environment', { model, engine, device, ...(alignmentLanguage ? { alignmentLanguage } : {}) }),
  getOcrEnvironment: (model: 'tiny' | 'small' | 'medium' = 'tiny') => call<OcrEnvironment>('get_ocr_environment', { model }),
  getTextRemovalEnvironment: () => call<TextRemovalEnvironment>('get_text_removal_environment'),
  installSttnTextRemovalModel: () => call<void>('install_sttn_text_removal_model'),
  removeSttnTextRemovalModel: () => call<number>('remove_sttn_text_removal_model'),
  startTextRemovalPreview: (projectId: string, regionId: string, currentTime: number) => call<string>('start_text_removal_preview', { projectId, regionId, currentTime }),
  cancelTextRemovalPreview: (jobId: string) => call<boolean>('cancel_export', { jobId }),
  installOcrRuntime: () => call<void>('install_ocr_runtime'),
  installOcrModel: (model: 'tiny' | 'small' | 'medium' = 'tiny') => call<void>('install_ocr_model', { model }),
  removeOcrModel: (model: 'tiny' | 'small' | 'medium' = 'tiny') => call<number>('remove_ocr_model', { model }),
  installAsrModel: (model: string, backend?: string) => call<void>('install_asr_model', { model, backend }),
  installWhisperXAlignment: (language: string) => call<void>('install_whisperx_alignment', { language }),
  removeAsrModel: (model: string, backend?: string) => call<void>('remove_asr_model', { model, backend }),
  generateTranscript: (projectId: string, options: { engine: 'auto' | 'fasterWhisper' | 'openVinoWhisper' | 'whisperX'; model: string; device: string; language: string; mode: 'speechRecognition' | 'ocr'; ocrFps: 1 | 2 | 4; ocrRegion: TranscriptSettings['ocrRegion']; ocrModel: 'tiny' | 'small' | 'medium'; ocrDevice: 'auto' | 'cpu' | 'cuda' | 'openvino' }) => call<string>('generate_transcript', { projectId, options }),
  cancelTranscription: (jobId: string) => call<boolean>('cancel_transcription', { jobId }),
};
