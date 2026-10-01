import { createEffect, createMemo, createSignal, For, Index, on, onCleanup, onMount, Show, untrack } from 'solid-js';
import {
  AlertTriangle, AudioLines, Check, FileImage, FileText, Languages, LoaderCircle,
  Plus, RefreshCw, Search, Settings2, Sparkles, Trash2, Volume2, X, ImagePlus,
} from 'lucide-solid';
import { activeWorkspacePanelTab, batchProcessingActive, blurRegionPicking, currentProject, currentTranscript, ocrRegionEditing, ocrRegionPicking, playbackTime, projectSaveState, selectedOverlay, segmentVoicePreviewActive, setActiveWorkspacePanelTab, setAiRemovalPreview, setBlurRegionPicking, setCurrentProject, setCurrentTranscript, setNotice, setOcrRegionEditing, setOcrRegionPicking, setProjectSaveState, setSeekRequest, setSegmentVoicePreviewActive, setSelectedOverlay, setSubtitleRegionPicking, subtitleRegionPicking, setTtsEvent, transcriptionEvent, translationEvent, ttsEvent, videoPlaying } from '../../../app/project-state';
import { projectService } from '../../../app/services/project-service';
import { translationService } from '../../../app/services/translation-service';
import { ttsService } from '../../../app/services/tts-service';
import { dialogueService, type DialogueEnvironment } from '../../../app/services/dialogue-service';
import { convertFileSrc } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { SubtitleTrackMetadata } from '../../projects/types';
import type { AsrEnvironment, OcrEnvironment, TextRemovalEnvironment, Transcript, TranscriptSegment } from '../../transcript/types';
import type { AppSettings } from '../../../app/services/translation-service';
import { compareSegmentTime, formatTimestamp, ttsWindowEnd, validateTranscript } from '../../transcript/transcript-utils';
import Modal from '../../../shared/components/Modal';
import { open } from '@tauri-apps/plugin-dialog';
import { mergeVisualProject, updateVisualSettings } from '../../overlays/overlay-state';
import type { WorkspacePanelTab } from '../../../app/project-state';

type PanelTab = WorkspacePanelTab;
type SaveState = 'saved' | 'saving' | 'error';
type TranscriptGenerationOptions = AppSettings['asr'] & {
  mode: 'speechRecognition' | 'ocr';
  ocrFps: 1 | 2 | 4;
  ocrRegion: { x: number; y: number; width: number; height: number } | null;
};

const TRANSLATION_LANGUAGES = [
  { code: 'auto', label: 'Tự nhận diện' },
  { code: 'ar', label: 'العربية' },
  { code: 'de', label: 'Deutsch' },
  { code: 'en', label: 'English' },
  { code: 'es', label: 'Español' },
  { code: 'fa', label: 'فارسی' },
  { code: 'fr', label: 'Français' },
  { code: 'hi', label: 'हिन्दी' },
  { code: 'id', label: 'Bahasa Indonesia' },
  { code: 'it', label: 'Italiano' },
  { code: 'ja', label: '日本語' },
  { code: 'ko', label: '한국어' },
  { code: 'ms', label: 'Bahasa Melayu' },
  { code: 'nl', label: 'Nederlands' },
  { code: 'pl', label: 'Polski' },
  { code: 'pt', label: 'Português' },
  { code: 'ru', label: 'Русский' },
  { code: 'th', label: 'ไทย' },
  { code: 'tr', label: 'Türkçe' },
  { code: 'uk', label: 'Українська' },
  { code: 'vi', label: 'Tiếng Việt' },
  { code: 'zh', label: '中文' },
];
const TARGET_TRANSLATION_LANGUAGES = TRANSLATION_LANGUAGES.filter((language) => language.code !== 'auto');

function formatDuration(seconds?: number | null) {
  if (!seconds || !Number.isFinite(seconds)) return '—';
  const total = Math.floor(seconds);
  return `${String(Math.floor(total / 60)).padStart(2, '0')}:${String(total % 60).padStart(2, '0')}`;
}

function formatFps(value?: number | null) {
  return value ? value.toFixed(2) : '—';
}

function formatBytes(value?: number | null) {
  if (!value || !Number.isFinite(value)) return '—';
  if (value >= 1024 * 1024 * 1024) return `${(value / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MB`;
}

function trackLabel(track: SubtitleTrackMetadata) {
  const language = track.language?.toUpperCase() ?? 'Không rõ ngôn ngữ';
  const title = track.title ? ` · ${track.title}` : '';
  return `Track #${track.streamIndex} · ${language} · ${track.codec}${title}${track.imageBased ? ' · Bitmap' : ''}`;
}

function emptyTranscript(projectId: string, checksum: string | null): Transcript {
  return { schemaVersion: 1, projectId, sourceType: 'asr', sourceTrack: null, language: null, videoChecksum: checksum, segments: [] };
}

export default function TranslationPanel() {
  const [selectedTrack, setSelectedTrack] = createSignal('');
  const [transcriptLoading, setTranscriptLoading] = createSignal(false);
  const [extracting, setExtracting] = createSignal(false);
  const [asrRunning, setAsrRunning] = createSignal(false);
  const [asrJobId, setAsrJobId] = createSignal<string>();
  const [checkingAsr, setCheckingAsr] = createSignal(false);
  const [asrEnvironment, setAsrEnvironment] = createSignal<AsrEnvironment>();
  const [ocrEnvironment, setOcrEnvironment] = createSignal<OcrEnvironment>();
  const [textRemovalEnvironment, setTextRemovalEnvironment] = createSignal<TextRemovalEnvironment>();
  const [textRemovalEnvironmentLoading, setTextRemovalEnvironmentLoading] = createSignal(false);
  const [textRemovalEnvironmentError, setTextRemovalEnvironmentError] = createSignal('');
  const [textRemovalPreviewJobId, setTextRemovalPreviewJobId] = createSignal<string>();
  const [textRemovalPreviewProgress, setTextRemovalPreviewProgress] = createSignal({ progress: 0, message: '' });
  const [cancelingTextRemovalPreview, setCancelingTextRemovalPreview] = createSignal(false);
  const [asrMessage, setAsrMessage] = createSignal('');
  const [asrStage, setAsrStage] = createSignal('');
  const [asrProgress, setAsrProgress] = createSignal<number>();
  const [showAsrConfirm, setShowAsrConfirm] = createSignal(false);
  const [showExtractConfirm, setShowExtractConfirm] = createSignal(false);
  const [asrOptions, setAsrOptions] = createSignal<TranscriptGenerationOptions>({ engine: 'fasterWhisper', model: 'small', device: 'cpu', language: 'auto', ocrModel: 'tiny', ocrDevice: 'cpu', mode: 'speechRecognition', ocrFps: 2, ocrRegion: null });
  const [dirty, setDirty] = createSignal(projectSaveState() !== 'saved');
  const [saveState, setSaveState] = createSignal<SaveState>(projectSaveState());
  const [saveError, setSaveError] = createSignal('');
  const [activeSegmentId, setActiveSegmentId] = createSignal<string>();
  const [translationRunning, setTranslationRunning] = createSignal(false);
  const [ttsRunning, setTtsRunning] = createSignal(false);
  const [ttsJobId, setTtsJobId] = createSignal<string>();
  const [ttsMessage, setTtsMessage] = createSignal('');
  const [previewingTtsSegment, setPreviewingTtsSegment] = createSignal<string>();
  const [ttsSettings, setTtsSettings] = createSignal<AppSettings['tts']>();
  const [dialogueEnvironment, setDialogueEnvironment] = createSignal<DialogueEnvironment>();
  const [translationJobId, setTranslationJobId] = createSignal<string>();
  const [translationMessage, setTranslationMessage] = createSignal('');
  const [translationSearch, setTranslationSearch] = createSignal('');
  const [selectedSegmentIds, setSelectedSegmentIds] = createSignal<Set<string>>(new Set());
  const [sourceLanguage, setSourceLanguage] = createSignal('auto');
  const [targetLanguage, setTargetLanguage] = createSignal('vi');
  const [savingLanguages, setSavingLanguages] = createSignal(false);
  const [overwriteRequest, setOverwriteRequest] = createSignal<{ scope: 'all' | 'selected' | 'segment'; ids: string[] }>();
  let saveTimer: number | undefined;
  let saveRevision = 0;
  let saving = false;
  let dubbingSaveTimer: number | undefined;
  let dubbingSaveRevision = 0;
  let previewAudio: HTMLAudioElement | undefined;
  let lastTerminalEvent = '';
  let lastTranslationTerminalEvent = '';
  let lastTtsTerminalEvent = '';
  let lastTextRemovalPreviewTerminalId: string | undefined;
  let stopTextRemovalPreviewProgress: (() => void) | undefined;

  onMount(() => {
    void translationService.getSettings().then((response) => setTtsSettings(response.settings.tts)).catch(() => undefined);
    void dialogueService.environment().then(setDialogueEnvironment).catch(() => setDialogueEnvironment(undefined));
    void listen<{ projectId: string; jobId: string; status: string; progress: number; message: string; output: string | null; startSeconds: number; endSeconds: number; regionId: string; fps: number }>('text-removal-preview-progress', (event) => {
      const payload = event.payload;
      if (payload.projectId !== currentProject()?.id || (textRemovalPreviewJobId() && payload.jobId !== textRemovalPreviewJobId())) return;
      if (!textRemovalPreviewJobId()) setTextRemovalPreviewJobId(payload.jobId);
      if (payload.status === 'completed' || payload.status === 'failed' || payload.status === 'cancelled') lastTextRemovalPreviewTerminalId = payload.jobId;
      setTextRemovalPreviewProgress({ progress: Number.isFinite(payload.progress) ? payload.progress : 0, message: payload.message ?? 'Đang xử lý preview…' });
      if (payload.status === 'completed' && payload.output) {
        setAiRemovalPreview({ projectId: payload.projectId, regionId: payload.regionId, path: payload.output, startSeconds: payload.startSeconds, endSeconds: payload.endSeconds, fps: payload.fps });
        setTextRemovalPreviewJobId(undefined);
        setNotice({ kind: 'success', message: 'Preview đã xử lý xong. Đang phát đoạn đã xóa chữ bằng AI.' });
      } else if (payload.status === 'failed') {
        setTextRemovalPreviewJobId(undefined);
        setNotice({ kind: 'error', message: payload.message || 'Không thể xử lý preview xóa chữ.' });
      } else if (payload.status === 'cancelled') {
        setTextRemovalPreviewJobId(undefined);
        setNotice({ kind: 'info', message: 'Đã hủy xử lý preview xóa chữ.' });
      }
    }).then((unlisten) => { stopTextRemovalPreviewProgress = unlisten; });
  });

  const dialogueReductionReady = () => !!dialogueEnvironment()?.runtimeInstalled && !!dialogueEnvironment()?.modelCached;

  const tracks = () => currentProject()?.source.subtitleTracks ?? [];
  const textTracks = () => tracks().filter((track) => track.textSupported && !track.imageBased);
  const transcript = () => currentTranscript();
  const videoDuration = () => currentProject()?.source.duration ?? 0;
  const hasVideo = () => !!currentProject()?.source.videoPath;
  const hasAudio = () => (currentProject()?.source.audioStreams?.length ?? 0) > 0;
  const progressPercent = () => asrProgress() === undefined ? undefined : Math.round(asrProgress()! * 100);
  const translationProgress = () => translationEvent()?.projectId === currentProject()?.id ? translationEvent() : undefined;
  const dubbingProgress = () => ttsEvent()?.projectId === currentProject()?.id ? ttsEvent() : undefined;
  const translationBatchPercent = () => {
    const value = translationProgress();
    return value && value.totalBatches > 0 ? Math.round((value.completedBatches / value.totalBatches) * 100) : undefined;
  };
  const filteredSegments = createMemo(() => {
    const query = translationSearch().trim().toLocaleLowerCase();
    const segments = transcript()?.segments ?? [];
    if (!query) return segments;
    return segments.filter((segment) => `${segment.source}\n${segment.translated}`.toLocaleLowerCase().includes(query));
  });

  createEffect(() => setProjectSaveState(saveState()));

  createEffect(on(
    () => {
      const project = currentProject();
      return `${project?.id ?? ''}:${project?.source.videoPath ?? ''}:${project?.source.checksumSha256 ?? ''}`;
    },
    () => {
    const project = untrack(currentProject);
    const retained = untrack(currentTranscript);
    const retainedSaveState = untrack(projectSaveState);
    setTranscriptLoading(false);
    if (project?.id && retained?.projectId === project.id
      && retained.videoChecksum === (project.source.checksumSha256 ?? null)
      && retainedSaveState !== 'saved') {
      setDirty(true);
      setSaveState(retainedSaveState);
      return;
    }
    setCurrentTranscript(undefined);
    setDirty(false);
    setSaveState('saved');
    if (!project?.id || !project.source.videoPath) return;
    let active = true;
    setTranscriptLoading(true);
    void projectService.getTranscript(project.id)
      .then((value) => { if (active) setCurrentTranscript(value ?? undefined); })
      .catch((error: unknown) => {
        if (active) setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể đọc transcript của dự án.' });
      })
      .finally(() => active && setTranscriptLoading(false));
    onCleanup(() => { active = false; });
    },
  ));

  createEffect(() => {
    const payload = ttsEvent();
    if (!payload || payload.projectId !== currentProject()?.id) return;
    setTtsMessage(payload.message);
    if (payload.status === 'queued' || payload.status === 'running') {
      setTtsRunning(true);
      setTtsJobId(payload.jobId);
      return;
    }
    if (ttsJobId() && ttsJobId() !== payload.jobId) return;
    const eventKey = `${payload.jobId}:${payload.status}`;
    if (lastTtsTerminalEvent === eventKey) return;
    lastTtsTerminalEvent = eventKey;
    setTtsRunning(false);
    setTtsJobId(undefined);
    setTtsEvent((current) => current?.jobId === payload.jobId && current.status === payload.status ? undefined : current);
    if (!batchProcessingActive()) {
      void reloadProjectTranscript(payload.projectId);
      setNotice({ kind: payload.status === 'completed' ? 'success' : payload.status === 'partial' || payload.status === 'cancelled' ? 'info' : 'error', message: payload.message });
    }
  });

  createEffect(() => {
    const project = currentProject();
    setSourceLanguage(project?.translation.sourceLanguage ?? 'auto');
    setTargetLanguage(project?.translation.targetLanguage ?? 'vi');
  });

  createEffect(() => {
    if (!videoPlaying() || !previewAudio) return;
    previewAudio.pause();
    previewAudio = undefined;
    setPreviewingTtsSegment(undefined);
    setSegmentVoicePreviewActive(false);
  });

  createEffect(() => {
    if (segmentVoicePreviewActive() || !previewAudio) return;
    previewAudio.pause();
    previewAudio = undefined;
    setPreviewingTtsSegment(undefined);
  });

  createEffect(() => {
    const available = textTracks();
    const selected = selectedTrack();
    if (available.length === 1 && !selected) setSelectedTrack(String(available[0].streamIndex));
    else if (selected && !available.some((track) => String(track.streamIndex) === selected)) setSelectedTrack('');
  });

  createEffect(() => {
    const time = playbackTime();
    const segments = transcript()?.segments ?? [];
    const current = segments.find((segment) => time >= segment.start && time < segment.end);
    const nextId = current?.id;
    if (untrack(activeSegmentId) !== nextId) {
      setActiveSegmentId(nextId);
      if (videoPlaying() && nextId && !document.activeElement?.closest('.transcript-segment-card')) {
        document.querySelector<HTMLElement>(`[data-segment-id="${CSS.escape(nextId)}"]`)?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
      }
    }
  });

  createEffect(() => {
    const currentTranscriptValue = currentTranscript();
    const shouldSave = dirty();
    const projectId = currentProject()?.id;
    if (!currentTranscriptValue || !shouldSave || !projectId) return;
    window.clearTimeout(saveTimer);
    saveTimer = window.setTimeout(() => void persistTranscript(), 750);
  });

  createEffect(() => {
    const payload = transcriptionEvent();
    if (!payload || payload.projectId !== currentProject()?.id) return;
    setAsrMessage(payload.message);
    setAsrStage(payload.stage ?? '');
    setAsrProgress(payload.progress);
    if (payload.status === 'progress') {
      setAsrRunning(true);
      setAsrJobId(payload.jobId);
      return;
    }

    setAsrRunning(false);
    setAsrJobId(undefined);
    const eventKey = `${payload.jobId}:${payload.status}`;
    if (lastTerminalEvent === eventKey) return;
    lastTerminalEvent = eventKey;
    if (payload.status === 'completed') {
      if (currentProject()?.transcriptSettings?.mode === 'ocr') setOcrRegionEditing(false);
      if (!batchProcessingActive()) {
        void reloadProjectTranscript(payload.projectId);
        setNotice({ kind: 'success', message: currentProject()?.transcriptSettings?.mode === 'ocr' ? 'Đã tạo transcript từ subtitle trên hình bằng OCR.' : asrOptions().engine === 'whisperX' ? 'Đã tạo transcript và căn chỉnh timestamp bằng WhisperX.' : 'Đã tạo transcript bằng faster-whisper.' });
      }
    } else if (payload.status === 'failed' || payload.status === 'canceled') {
      if (!batchProcessingActive()) setNotice({ kind: payload.status === 'failed' ? 'error' : 'info', message: payload.message });
    }
  });

  onCleanup(() => {
    window.clearTimeout(saveTimer);
    window.clearTimeout(dubbingSaveTimer);
    previewAudio?.pause();
    stopTextRemovalPreviewProgress?.();
    setSegmentVoicePreviewActive(false);
  });

  const reloadProjectTranscript = async (projectId: string) => {
    try {
      const [project, value] = await Promise.all([projectService.get(projectId), projectService.getTranscript(projectId)]);
      if (currentProject()?.id === projectId) {
        setCurrentProject(project);
        setCurrentTranscript(value ?? undefined);
        setDirty(false);
        setSaveState('saved');
      }
    } catch (error) {
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể tải transcript vừa tạo.' });
    }
  };

  createEffect(() => {
    const payload = translationEvent();
    if (!payload || payload.projectId !== currentProject()?.id) return;
    if (payload.status === 'queued' || payload.status === 'running') {
      setTranslationJobId(payload.jobId);
      setTranslationRunning(true);
      setTranslationMessage(payload.message);
      return;
    }
    if (translationJobId() && translationJobId() !== payload.jobId) return;
    setTranslationRunning(false);
    setTranslationJobId(undefined);
    setTranslationMessage(payload.message);
    const eventKey = `${payload.jobId}:${payload.status}`;
    if (lastTranslationTerminalEvent === eventKey) return;
    lastTranslationTerminalEvent = eventKey;
    void projectService.get(payload.projectId)
      .then((latestProject) => {
        if (currentProject()?.id === latestProject.id) setCurrentProject(latestProject);
      })
      .catch(() => undefined);
    if (!batchProcessingActive()) {
      void reloadProjectTranscript(payload.projectId);
      setNotice({
        kind: payload.status === 'completed' ? 'success' : payload.status === 'partial' || payload.status === 'cancelled' ? 'info' : 'error',
        message: payload.message,
      });
    }
  });

  const persistTranscript = async () => {
    if (saving) return;
    const snapshot = currentTranscript();
    const project = currentProject();
    if (!snapshot || !project || !dirty()) return;
    const invalid = validateTranscript(snapshot, project.source.duration);
    if (invalid) {
      setSaveState('error');
      setSaveError(invalid);
      return;
    }
    saving = true;
    const revision = saveRevision;
    setSaveState('saving');
    setSaveError('');
    try {
      const updated = await projectService.saveTranscript(project.id, snapshot);
      if (currentProject()?.id === project.id) setCurrentProject(updated);
      if (saveRevision === revision) {
        setDirty(false);
        setSaveState('saved');
      } else {
        setSaveState('saving');
      }
    } catch (error) {
      setSaveState('error');
      setSaveError(error instanceof Error ? error.message : 'Không thể lưu transcript.');
    } finally {
      saving = false;
      if (saveRevision !== revision && dirty()) {
        window.clearTimeout(saveTimer);
        saveTimer = window.setTimeout(() => void persistTranscript(), 300);
      }
    }
  };

  const updateSegment = (id: string, changes: Partial<TranscriptSegment>) => {
    saveRevision += 1;
    setCurrentTranscript((value) => value ? {
      ...value,
      segments: value.segments.map((segment) => {
        if (segment.id !== id) return segment;
        const updated = { ...segment, ...changes };
        if (changes.source !== undefined && changes.source !== segment.source) updated.sourceRevision = segment.sourceRevision + 1;
        if (changes.translated !== undefined && changes.translated !== segment.translated && updated.tts) {
          updated.tts = { ...updated.tts, status: 'outdated', warning: 'Bản dịch đã thay đổi sau khi tạo audio.' };
        }
        if (changes.translated !== undefined && changes.translated !== segment.translated) {
          updated.translationStatus = 'edited';
          updated.translatedRevision = updated.sourceRevision;
        }
        return updated;
      }).sort(compareSegmentTime),
    } : value);
    setDirty(true);
    setSaveState('saving');
  };

  const addSegment = () => {
    const project = currentProject();
    if (!project?.source.videoPath) return;
    let value = currentTranscript();
    if (!value) {
      value = emptyTranscript(project.id, project.source.checksumSha256 ?? null);
      setCurrentTranscript(value);
    }
    const duration = videoDuration();
    const start = duration > 0 ? Math.max(0, Math.min(playbackTime(), duration - 0.1)) : playbackTime();
    const end = duration > 0 ? Math.min(duration, start + Math.min(2, duration - start)) : start + 2;
    const segment: TranscriptSegment = {
      id: crypto.randomUUID(), start, end, source: '', translated: '', speakerId: null,
      sourceRevision: 0, translatedRevision: null, translationMeta: null,
      sourceType: value.sourceType, translationStatus: 'pending', tts: null,
    };
    saveRevision += 1;
    setCurrentTranscript({ ...value, segments: [...value.segments, segment].sort(compareSegmentTime) });
    setDirty(true);
    setActiveSegmentId(segment.id);
    setSaveState('saving');
  };

  const removeSegment = (id: string) => {
    const value = currentTranscript();
    if (!value) return;
    saveRevision += 1;
    setCurrentTranscript({ ...value, segments: value.segments.filter((segment) => segment.id !== id) });
    setDirty(true);
  };

  const saveLanguages = async () => {
    const project = currentProject();
    if (!project || savingLanguages() || translationRunning()) return;
    setSavingLanguages(true);
    try {
      const updated = await projectService.updateTranslationLanguages(project.id, sourceLanguage(), targetLanguage());
      if (currentProject()?.id === project.id) setCurrentProject(updated);
      setNotice({ kind: 'success', message: 'Đã lưu ngôn ngữ nguồn và đích cho dự án.' });
    } catch (error) {
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể lưu ngôn ngữ dự án.' });
    } finally { setSavingLanguages(false); }
  };

  const toggleSegmentSelection = (id: string) => {
    setSelectedSegmentIds((value) => {
      const next = new Set(value);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  };

  const toggleVisibleSelection = () => {
    const visibleIds = filteredSegments().map((segment) => segment.id);
    const allSelected = visibleIds.length > 0 && visibleIds.every((id) => selectedSegmentIds().has(id));
    setSelectedSegmentIds((value) => {
      const next = new Set(value);
      for (const id of visibleIds) allSelected ? next.delete(id) : next.add(id);
      return next;
    });
  };

  const launchTranslation = async (scope: 'all' | 'missing' | 'selected' | 'segment', ids: string[], overwriteEdited = false) => {
    const project = currentProject();
    if (!project || translationRunning()) return;
    if (dirty()) await persistTranscript();
    if (dirty() || saveState() !== 'saved') {
      setNotice({ kind: 'error', message: saveState() === 'error' ? 'Hãy giải quyết lỗi lưu transcript trước khi dịch.' : 'Đợi autosave hoàn tất trước khi dịch.' });
      return;
    }
    setTranslationRunning(true);
    setTranslationMessage('Đang khởi tạo tác vụ dịch…');
    try {
      const latestProject = currentProject();
      if (latestProject && (latestProject.translation.sourceLanguage !== sourceLanguage() || latestProject.translation.targetLanguage !== targetLanguage())) {
        const updated = await projectService.updateTranslationLanguages(latestProject.id, sourceLanguage(), targetLanguage());
        if (currentProject()?.id === latestProject.id) setCurrentProject(updated);
      }
      const jobId = await translationService.start(project.id, { scope, segmentIds: ids, overwriteEdited });
      if (!translationJobId()) setTranslationJobId(jobId);
    } catch (error) {
      setTranslationRunning(false);
      setTranslationJobId(undefined);
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể khởi chạy dịch transcript.' });
    }
  };

  const requestTranslation = (scope: 'all' | 'missing' | 'selected' | 'segment', segmentId?: string) => {
    const segments = transcript()?.segments ?? [];
    const ids = scope === 'selected' ? [...selectedSegmentIds()] : scope === 'segment' && segmentId ? [segmentId] : [];
    if (scope === 'selected' && ids.length === 0) {
      setNotice({ kind: 'info', message: 'Chọn ít nhất một segment cần dịch.' });
      return;
    }
    const candidates = segments.filter((segment) => {
      if (!segment.source.trim()) return false;
      if (scope === 'missing' && (segment.translated.trim() || segment.translationStatus === 'edited')) return false;
      if ((scope === 'selected' || scope === 'segment') && !ids.includes(segment.id)) return false;
      return true;
    });
    if (!candidates.length) {
      setNotice({ kind: 'info', message: 'Không có segment phù hợp để dịch.' });
      return;
    }
    const edited = candidates.filter((segment) => segment.translationStatus === 'edited');
    if (edited.length) {
      setOverwriteRequest({ scope: scope === 'missing' ? 'all' : scope, ids: scope === 'all' || scope === 'missing' ? [] : ids });
      return;
    }
    void launchTranslation(scope, ids, false);
  };

  const confirmTranslationOverwrite = () => {
    const request = overwriteRequest();
    if (!request) return;
    setOverwriteRequest(undefined);
    void launchTranslation(request.scope, request.ids, true);
  };

  const cancelTranslation = async () => {
    const jobId = translationJobId();
    if (!jobId) return;
    try {
      await translationService.cancel(jobId);
      setTranslationMessage('Đang hủy tác vụ dịch…');
    } catch (error) {
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể hủy tác vụ dịch.' });
    }
  };

  const launchTts = async (scope: 'all' | 'missing' | 'selected' | 'segment', ids: string[] = []) => {
    const project = currentProject();
    if (!project || ttsRunning() || translationRunning() || asrRunning()) return;
    if (dirty()) await persistTranscript();
    if (dirty() || saveState() !== 'saved') {
      setNotice({ kind: 'error', message: 'Đợi transcript lưu thành công trước khi tạo giọng đọc.' });
      return;
    }
    setTtsRunning(true);
    setTtsMessage('Đang khởi tạo VieNeu-TTS…');
    try {
      const jobId = await ttsService.start(project.id, { scope, segmentIds: ids });
      setTtsJobId(jobId);
    } catch (error) {
      setTtsRunning(false);
      setTtsJobId(undefined);
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể khởi chạy TTS.' });
    }
  };

  const cancelTts = async () => {
    const jobId = ttsJobId();
    if (!jobId) return;
    try { await ttsService.cancel(jobId); setTtsMessage('Đang dừng TTS…'); }
    catch (error) { setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể hủy TTS job.' }); }
  };

  const previewTts = async (segment: TranscriptSegment) => {
    if (!segment.tts?.audioPath || segment.tts.status === 'outdated' || segment.tts.status === 'failed') return;
    if (previewingTtsSegment() === segment.id) {
      previewAudio?.pause();
      previewAudio = undefined;
      setPreviewingTtsSegment(undefined);
      setSegmentVoicePreviewActive(false);
      return;
    }
    previewAudio?.pause();
    setPreviewingTtsSegment(segment.id);
    setSegmentVoicePreviewActive(true);
    try {
      const path = await ttsService.getAudioPath(currentProject()!.id, segment.id);
      if (previewingTtsSegment() !== segment.id) return;
      const audio = new Audio(convertFileSrc(path));
      previewAudio = audio;
      audio.onended = () => { setPreviewingTtsSegment(undefined); previewAudio = undefined; setSegmentVoicePreviewActive(false); };
      audio.onerror = () => { setPreviewingTtsSegment(undefined); previewAudio = undefined; setSegmentVoicePreviewActive(false); setNotice({ kind: 'error', message: 'Không thể phát audio TTS segment này.' }); };
      await audio.play();
    } catch (error) { setPreviewingTtsSegment(undefined); previewAudio = undefined; setSegmentVoicePreviewActive(false); setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể mở audio TTS.' }); }
  };

  const updateSubtitleSettings = (changes: Partial<NonNullable<ReturnType<typeof currentProject>>['subtitle']>) => {
    const project = currentProject();
    if (!project) return;
    const subtitle = { ...project.subtitle, ...changes };
    if (changes.fontSize !== undefined && subtitle.minFontSize > changes.fontSize) subtitle.minFontSize = changes.fontSize;
    updateVisualSettings({ subtitle });
  };

  const hasSubtitleBlurRegion = () => currentProject()?.blurRegions.some((region) => region.role === 'subtitle') ?? false;
  const pickSubtitleRegion = () => {
    const project = currentProject();
    if (!project?.source.videoPath || hasSubtitleBlurRegion()) return;
    setOcrRegionPicking(false);
    setOcrRegionEditing(false);
    setBlurRegionPicking(false);
    setSelectedOverlay({ type: 'subtitle' });
    setActiveWorkspacePanelTab('properties');
    setSubtitleRegionPicking(true);
    setNotice({ kind: 'info', message: 'Kéo trên video để chọn vùng hiển thị phụ đề. Nhấn Esc để hủy.' });
  };

  const updateBlurSettings = (id: string, changes: Partial<NonNullable<ReturnType<typeof currentProject>>['blurRegions'][number]>) => {
    const project = currentProject();
    if (!project) return;
    if (changes.role === 'subtitle') setSubtitleRegionPicking(false);
    updateVisualSettings({ blurRegions: project.blurRegions.map((region) => {
      if (region.id !== id) return region;
      const next = { ...region, ...changes };
      next.width = Math.max(0.02, Math.min(1, Number.isFinite(next.width) ? next.width : region.width));
      next.height = Math.max(0.02, Math.min(1, Number.isFinite(next.height) ? next.height : region.height));
      next.x = Math.max(0, Math.min(1 - next.width, Number.isFinite(next.x) ? next.x : region.x));
      next.y = Math.max(0, Math.min(1 - next.height, Number.isFinite(next.y) ? next.y : region.y));
      next.strength = Math.max(1, Math.min(100, Number.isFinite(next.strength) ? next.strength : region.strength));
      next.edgeBlend = Math.max(0, Math.min(50, Number.isFinite(next.edgeBlend) ? next.edgeBlend : region.edgeBlend ?? 0));
      next.pixelNoise = Math.max(0, Math.min(100, Number.isFinite(next.pixelNoise) ? next.pixelNoise : region.pixelNoise ?? 0));
      next.textRemovalScanFps = next.textRemovalScanFps === 1 ? 1 : 2;
      return next;
    }).map((region) => changes.role === 'subtitle' ? { ...region, role: region.id === id ? 'subtitle' as const : 'generic' as const } : region) });
  };

  const updateLogoSettings = (id: string, changes: Partial<NonNullable<ReturnType<typeof currentProject>>['logos'][number]>) => {
    const project = currentProject();
    if (!project) return;
    updateVisualSettings({ logos: project.logos.map((logo) => {
      if (logo.id !== id) return logo;
      const next = { ...logo, ...changes };
      next.opacity = Math.max(0, Math.min(1, Number.isFinite(next.opacity) ? next.opacity : logo.opacity));
      if (logo.maintainAspectRatio && (changes.width !== undefined || changes.height !== undefined)) {
        const normalizedAspect = logo.width / logo.height;
        if (changes.width !== undefined && changes.height === undefined) next.height = next.width / normalizedAspect;
        else if (changes.height !== undefined && changes.width === undefined) next.width = next.height * normalizedAspect;
      }
      next.width = Math.max(0.02, Math.min(1, Number.isFinite(next.width) ? next.width : logo.width));
      next.height = Math.max(0.02, Math.min(1, Number.isFinite(next.height) ? next.height : logo.height));
      next.x = Math.max(0, Math.min(1 - next.width, Number.isFinite(next.x) ? next.x : logo.x));
      next.y = Math.max(0, Math.min(1 - next.height, Number.isFinite(next.y) ? next.y : logo.y));
      return next;
    }) });
  };

  const selectedBlur = () => {
    const selected = selectedOverlay();
    return selected?.type === 'blur' ? currentProject()?.blurRegions.find((item) => item.id === selected.id) : undefined;
  };

  const selectedTextOverlay = () => {
    const selected = selectedOverlay();
    return selected?.type === 'text' ? currentProject()?.textOverlays.find((item) => item.id === selected.id) : undefined;
  };
  const updateTextOverlay = (id: string, changes: Partial<NonNullable<ReturnType<typeof currentProject>>['textOverlays'][number]>) => {
    const project = currentProject();
    if (!project) return;
    updateVisualSettings({ textOverlays: project.textOverlays.map((overlay) => {
      if (overlay.id !== id) return overlay;
      const next = { ...overlay, ...changes };
      next.width = Math.max(0.02, Math.min(1, Number.isFinite(next.width) ? next.width : overlay.width));
      next.height = Math.max(0.02, Math.min(1, Number.isFinite(next.height) ? next.height : overlay.height));
      next.x = Math.max(0, Math.min(1 - next.width, Number.isFinite(next.x) ? next.x : overlay.x));
      next.y = Math.max(0, Math.min(1 - next.height, Number.isFinite(next.y) ? next.y : overlay.y));
      next.fontSize = Math.max(8, Math.min(240, Math.round(next.fontSize)));
      next.fontWeight = Math.max(100, Math.min(900, Math.round(next.fontWeight)));
      next.opacity = Math.max(0, Math.min(1, Number.isFinite(next.opacity) ? next.opacity : overlay.opacity));
      return next;
    }) });
  };
  const removeTextOverlay = (id: string) => {
    const project = currentProject();
    if (!project) return;
    updateVisualSettings({ textOverlays: project.textOverlays.filter((item) => item.id !== id) }, true);
    setSelectedOverlay(null);
  };
  const refreshTextRemovalEnvironment = async () => {
    setTextRemovalEnvironmentLoading(true);
    setTextRemovalEnvironmentError('');
    try {
      setTextRemovalEnvironment(await projectService.getTextRemovalEnvironment());
    } catch (error) {
      setTextRemovalEnvironmentError(error instanceof Error ? error.message : 'Không thể kiểm tra mô hình xóa chữ.');
    } finally {
      setTextRemovalEnvironmentLoading(false);
    }
  };

  const isAiRemovalEffect = (effect?: 'blur' | 'sttn') => effect === 'sttn';
  const textRemovalReady = (effect?: 'blur' | 'sttn') => {
    const environment = textRemovalEnvironment();
    if (!environment) return false;
    if (!environment.ocrReady) return false;
    if (effect === 'sttn') return environment.sttnRuntimeInstalled && environment.sttnModelInstalled;
    return false;
  };

  const startTextRemovalPreview = async (regionId: string) => {
    const project = currentProject();
    if (!project || !textRemovalReady(selectedBlur()?.effect)) return;
    setTextRemovalPreviewProgress({ progress: 0, message: 'Đang đưa vào hàng đợi…' });
    setCancelingTextRemovalPreview(false);
    try {
      const id = await projectService.startTextRemovalPreview(project.id, regionId, playbackTime());
      if (lastTextRemovalPreviewTerminalId !== id) {
        setTextRemovalPreviewJobId(id);
        setNotice({ kind: 'info', message: 'Đang xử lý đoạn preview quanh vị trí phát hiện tại.' });
      }
    } catch (error) {
      setTextRemovalPreviewJobId(undefined);
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể bắt đầu xử lý preview xóa chữ.' });
    }
  };

  const cancelTextRemovalPreview = async () => {
    const id = textRemovalPreviewJobId();
    if (!id) return;
    setCancelingTextRemovalPreview(true);
    try {
      await projectService.cancelTextRemovalPreview(id);
    } catch (error) {
      setCancelingTextRemovalPreview(false);
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể hủy xử lý preview.' });
    }
  };
  createEffect(on(
    () => isAiRemovalEffect(selectedBlur()?.effect) ? selectedBlur()?.id : undefined,
    (id) => { if (id) void refreshTextRemovalEnvironment(); },
    { defer: true },
  ));
  const selectedLogo = () => {
    const selected = selectedOverlay();
    return selected?.type === 'logo' ? currentProject()?.logos.find((item) => item.id === selected.id) : undefined;
  };

  const removeSelectedLogo = async () => {
    const project = currentProject();
    const selected = selectedOverlay();
    if (!project || selected?.type !== 'logo') return;
    try {
      const result = await projectService.removeLogo(project.id, selected.id);
      mergeVisualProject(result.project);
      setSelectedOverlay(null);
      if (result.cleanupWarning) setNotice({ kind: 'info', message: result.cleanupWarning });
    } catch (error) {
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể xóa logo.' });
    }
  };

  const replaceSelectedLogo = async () => {
    const project = currentProject();
    const selected = selectedOverlay();
    if (!project || selected?.type !== 'logo') return;
    const path = await open({ multiple: false, filters: [{ name: 'Image', extensions: ['png', 'jpg', 'jpeg', 'webp'] }] });
    if (typeof path !== 'string') return;
    try {
      const result = await projectService.replaceLogo(project.id, selected.id, path);
      mergeVisualProject(result.project);
      if (result.cleanupWarning) setNotice({ kind: 'info', message: result.cleanupWarning });
    } catch (error) {
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể thay ảnh logo.' });
    }
  };

  const updateDubbingSettings = (changes: Partial<NonNullable<ReturnType<typeof currentProject>>['dubbing']>) => {
    const project = currentProject();
    if (!project) return;
    const revision = ++dubbingSaveRevision;
    const dubbing = { ...project.dubbing, ...changes };
    setCurrentProject({ ...project, dubbing });
    window.clearTimeout(dubbingSaveTimer);
    dubbingSaveTimer = window.setTimeout(() => {
      const latest = currentProject();
      if (!latest || latest.id !== project.id) return;
      void projectService.updateDubbingSettings(project.id, latest.dubbing)
        .then((updated) => {
          if (revision !== dubbingSaveRevision) return;
          setCurrentProject((value) => value?.id === project.id ? { ...value, dubbing: updated.dubbing, updatedAt: updated.updatedAt } : value);
        })
        .catch((error: unknown) => {
          if (revision === dubbingSaveRevision) setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể lưu cấu hình lồng tiếng.' });
        });
    }, 400);
  };

  const fitAndRegenerate = async (segmentId: string) => {
    const project = currentProject();
    if (!project || ttsRunning()) return;
    window.clearTimeout(dubbingSaveTimer);
    const revision = ++dubbingSaveRevision;
    const dubbing = { ...project.dubbing, fitMode: 'fit' as const };
    setCurrentProject({ ...project, dubbing });
    try {
      const updated = await projectService.updateDubbingSettings(project.id, dubbing);
      if (revision !== dubbingSaveRevision) return;
      setCurrentProject((value) => value?.id === project.id ? { ...value, dubbing: updated.dubbing, updatedAt: updated.updatedAt } : value);
      await launchTts('segment', [segmentId]);
    } catch (error) {
      if (revision !== dubbingSaveRevision) return;
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể lưu chế độ Fit.' });
    }
  };

  const extractSelectedSubtitle = () => {
    if ((transcript()?.segments.length ?? 0) > 0) {
      setShowExtractConfirm(true);
      return;
    }
    void performSubtitleExtraction();
  };

  const performSubtitleExtraction = async () => {
    const projectId = currentProject()?.id;
    const trackIndex = Number(selectedTrack());
    if (!projectId || !Number.isInteger(trackIndex) || extracting()) return;
    setShowExtractConfirm(false);
    setExtracting(true);
    try {
      const value = await projectService.extractSubtitle(projectId, trackIndex);
      const updated = await projectService.get(projectId);
      setCurrentProject(updated);
      setCurrentTranscript(value);
      setDirty(false);
      setSaveState('saved');
      setNotice({ kind: 'success', message: `Đã trích xuất ${value.segments.length} subtitle segments.` });
    } catch (error) {
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể trích xuất subtitle.' });
    } finally {
      setExtracting(false);
    }
  };

  const clearOcrRegion = async () => {
    const project = currentProject();
    if (!project) return;
    try {
      const settings = project.transcriptSettings ?? { mode: 'speechRecognition' as const, ocrFps: 2 as const, ocrRegion: null };
      const updated = await projectService.updateTranscriptSettings(project.id, { ...settings, ocrRegion: null });
      setCurrentProject(updated);
      setOcrRegionPicking(false);
      setOcrRegionEditing(false);
      setNotice({ kind: 'success', message: 'Đã xóa vùng OCR của dự án.' });
    } catch (error) {
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể xóa vùng OCR.' });
    }
  };

  const requestAsr = async () => {
    if (!currentProject()?.id || asrRunning() || checkingAsr() || translationRunning()) return;
    if (dirty() && !saving) await persistTranscript();
    if (dirty() || saveState() !== 'saved') {
      setNotice({ kind: 'error', message: saveState() === 'error' ? 'Hãy sửa lỗi lưu hoặc thử lưu transcript trước khi tạo transcript mới.' : 'Đang lưu thay đổi transcript. Đợi lưu xong rồi thử lại.' });
      return;
    }
    setCheckingAsr(true);
    try {
      const saved = await translationService.getSettings();
      const options = saved.settings.asr;
      const project = currentProject();
      const mode = project?.transcriptSettings?.mode ?? 'speechRecognition';
      const projectTranscriptSettings = project?.transcriptSettings ?? { mode: 'speechRecognition' as const, ocrFps: 2 as const, ocrRegion: null };
      if (mode === 'ocr') {
        if (!project?.source.videoPath) {
          setNotice({ kind: 'error', message: 'Hãy thêm video trước khi tạo transcript bằng OCR.' });
          return;
        }
        if (!projectTranscriptSettings.ocrRegion) {
          setBlurRegionPicking(false);
          setOcrRegionEditing(false);
          setOcrRegionPicking(true);
          setNotice({ kind: 'info', message: 'Kéo khoanh vùng subtitle trên video. Sau khi lưu vùng, bấm Generate Transcript để chạy OCR.' });
          return;
        }
        setAsrOptions({ ...options, mode, ocrFps: projectTranscriptSettings.ocrFps, ocrRegion: projectTranscriptSettings.ocrRegion });
        const environment = await projectService.getOcrEnvironment(options.ocrModel);
        setOcrEnvironment(environment);
        if (!environment.workerInstalled) {
          setNotice({ kind: 'error', message: 'AI Worker chưa sẵn sàng. Hãy sửa chữa hoặc cài lại ứng dụng rồi thử lại.' });
          return;
        }
        if (!environment.ocrInstalled || !environment.runtimeInstalled) {
          setNotice({ kind: 'error', message: 'Bộ đọc chữ chưa sẵn sàng. Mở Cài đặt ứng dụng → Nhận dạng lời thoại để cài.' });
          return;
        }
        if (!environment.modelCached) {
          setNotice({ kind: 'error', message: 'Model OCR chưa được cài. Mở Cài đặt ứng dụng → Speech Recognition và cài model OCR trước.' });
          return;
        }
        if (options.ocrDevice === 'cuda' && !environment.cudaAvailable) {
          setNotice({ kind: 'error', message: 'Tăng tốc NVIDIA chưa sẵn sàng. Chọn CUDA trong Cài đặt → Nhận dạng lời thoại để cài, hoặc dùng CPU.' });
          return;
        }
        setShowAsrConfirm(true);
        return;
      }
      setAsrOptions({ ...options, mode, ocrFps: projectTranscriptSettings.ocrFps, ocrRegion: projectTranscriptSettings.ocrRegion });
      const environment = await projectService.getAsrEnvironment(options.model, options.language);
      if (!environment.workerInstalled) {
        setNotice({ kind: 'error', message: 'AI Worker chưa sẵn sàng. Hãy sửa chữa hoặc cài lại ứng dụng, sau đó thử lại.' });
        return;
      }
      if (!environment.modelCached) {
        setNotice({ kind: 'error', message: `Model ${options.model} chưa được cài. Mở Cài đặt → Speech Recognition và cài model trước khi tạo transcript.` });
        return;
      }
      if (options.engine === 'whisperX' && options.language === 'auto') {
        setNotice({ kind: 'error', message: 'WhisperX cần ngôn ngữ nguồn cụ thể để dùng đúng aligner. Hãy chọn ngôn ngữ trong Cài đặt → Speech Recognition.' });
        return;
      }
      if (options.engine === 'whisperX' && !environment.whisperXInstalled) {
        setNotice({ kind: 'error', message: 'WhisperX chưa được cài. Mở Cài đặt → Nhận dạng lời thoại, cài WhisperX rồi bấm Kiểm tra lại.' });
        return;
      }
      if (options.engine === 'whisperX' && !environment.alignmentModelCached) {
        setNotice({ kind: 'error', message: `WhisperX chưa cài aligner cho ${options.language}. Mở Cài đặt → Speech Recognition để cài model trước.` });
        return;
      }
      if (options.device === 'cuda' && !environment.cudaAvailable) {
        setNotice({ kind: 'error', message: 'CUDA chưa sẵn sàng. Chọn CUDA trong Cài đặt → Nhận dạng lời thoại để cài, hoặc dùng CPU.' });
        return;
      }
      setAsrEnvironment(environment);
      setShowAsrConfirm(true);
    } catch (error) {
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể kiểm tra Speech Recognition.' });
    } finally {
      setCheckingAsr(false);
    }
  };

  const startAsr = async () => {
    const projectId = currentProject()?.id;
    if (!projectId) return;
    setShowAsrConfirm(false);
    setAsrRunning(true);
    setAsrMessage('Đang chuẩn bị tác vụ…');
    setAsrProgress(undefined);
    try {
      const options = asrOptions();
      const jobId = await projectService.generateTranscript(projectId, { engine: options.engine, model: options.model, device: options.device, language: options.language, mode: options.mode, ocrFps: options.ocrFps, ocrRegion: options.ocrRegion, ocrModel: options.ocrModel, ocrDevice: options.ocrDevice });
      const latest = transcriptionEvent();
      if (!latest || latest.jobId !== jobId || latest.status === 'progress') {
        setAsrJobId(jobId);
        setAsrRunning(true);
      }
    } catch (error) {
      setAsrRunning(false);
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể khởi chạy Speech Recognition.' });
    }
  };

  const cancelAsr = async () => {
    const jobId = asrJobId();
    if (!jobId) return;
    try {
      await projectService.cancelTranscription(jobId);
      setAsrMessage('Đang dừng transcription…');
    } catch (error) {
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể hủy tác vụ.' });
    }
  };

  const selectTranscriptSegment = (segment: TranscriptSegment) => {
    setActiveSegmentId(segment.id);
    setSeekRequest(segment.start);
  };

  const panelTabs: Array<{ id: PanelTab; label: string }> = [
    { id: 'transcript', label: 'Transcript' },
    { id: 'media', label: 'Media' },
    { id: 'properties', label: 'Properties' },
  ];

  return (
    <section class="translation-panel" aria-label="Bảng nội dung dự án">
      <header class="panel-tabs-header">
        <div class="panel-tabs" role="tablist" aria-label="Nội dung dự án">
          <For each={panelTabs}>{(item) => (
            <button class="panel-tab" classList={{ selected: activeWorkspacePanelTab() === item.id }} type="button" role="tab" aria-selected={activeWorkspacePanelTab() === item.id} onClick={() => setActiveWorkspacePanelTab(item.id)}>
              {item.label}{activeWorkspacePanelTab() === item.id && <span class="tab-indicator" />}
            </button>
          )}</For>
        </div>
        <button class="icon-button panel-options" type="button" aria-label="Tùy chọn bảng" title="Tùy chọn"><Settings2 size={16} /></button>
      </header>

      <Show when={activeWorkspacePanelTab() === 'transcript'}>
        <div class="transcript-panel-content" role="tabpanel">
          <div class="transcript-toolbar">
            <div class="language-control-group">
              <select class="transcript-track-select language-select" aria-label="Ngôn ngữ nguồn" title="Chọn ngôn ngữ nguồn" value={sourceLanguage()} disabled={translationRunning()} onChange={(event) => setSourceLanguage(event.currentTarget.value)}>
                <Show when={!TRANSLATION_LANGUAGES.some((language) => language.code === sourceLanguage())}><option value={sourceLanguage()}>{sourceLanguage()}</option></Show>
                <For each={TRANSLATION_LANGUAGES}>{(language) => <option value={language.code}>{language.label}</option>}</For>
              </select>
              <span class="language-arrow">→</span>
              <select class="transcript-track-select language-select" aria-label="Ngôn ngữ đích" title="Chọn ngôn ngữ đích" value={targetLanguage()} disabled={translationRunning()} onChange={(event) => setTargetLanguage(event.currentTarget.value)}>
                <Show when={!TARGET_TRANSLATION_LANGUAGES.some((language) => language.code === targetLanguage())}><option value={targetLanguage()}>{targetLanguage()}</option></Show>
                <For each={TARGET_TRANSLATION_LANGUAGES}>{(language) => <option value={language.code}>{language.label}</option>}</For>
              </select>
              <button class="mini-tool-button" type="button" disabled={savingLanguages() || translationRunning()} onClick={() => void saveLanguages()}>{savingLanguages() ? 'Đang lưu…' : 'Lưu ngôn ngữ'}</button>
            </div>
            <div class="transcript-actions">
      <Show when={currentProject()?.transcriptSettings?.mode === 'ocr'}>
                <button class="mini-tool-button" type="button" disabled={!hasVideo() || asrRunning() || checkingAsr()} onClick={() => { setBlurRegionPicking(false); setSubtitleRegionPicking(false); setOcrRegionEditing(false); setOcrRegionPicking(true); setNotice({ kind: 'info', message: 'Kéo khoanh vùng subtitle trên preview video. Nhấn Esc để hủy.' }); }}>
                  <ImagePlus size={14} /><span>{ocrRegionPicking() ? 'Đang chọn vùng…' : currentProject()?.transcriptSettings?.ocrRegion ? 'Chọn lại vùng OCR' : 'Chọn vùng OCR'}</span>
                </button>
                <Show when={currentProject()?.transcriptSettings?.ocrRegion}>
                  <button class="mini-tool-button" type="button" disabled={ocrRegionPicking() || blurRegionPicking()} onClick={() => { const editing = !ocrRegionEditing(); setOcrRegionPicking(false); setOcrRegionEditing(editing); setNotice({ kind: 'info', message: editing ? 'Kéo khung OCR để di chuyển; kéo góc phải dưới để đổi kích thước.' : 'Đã ẩn khung OCR.' }); }}>
                    <Settings2 size={14} /><span>{ocrRegionEditing() ? 'Xong chỉnh sửa' : 'Chỉnh vùng OCR'}</span>
                  </button>
                  <button class="mini-tool-button" type="button" disabled={ocrRegionPicking() || blurRegionPicking()} title="Xóa vùng OCR" onClick={() => void clearOcrRegion()}>
                    <Trash2 size={14} /><span>Xóa vùng</span>
                  </button>
                </Show>
              </Show>
              <Show when={textTracks().length > 0}>
                <select class="transcript-track-select" aria-label="Chọn subtitle track" value={selectedTrack()} onChange={(event) => setSelectedTrack(event.currentTarget.value)}>
                  <option value="">Chọn subtitle</option>
                  <For each={textTracks()}>{(track) => <option value={track.streamIndex}>{trackLabel(track)}</option>}</For>
                </select>
                <button class="mini-tool-button mini-primary" type="button" disabled={!selectedTrack() || extracting() || asrRunning() || translationRunning()} onClick={() => void extractSelectedSubtitle()}>
                  <Show when={extracting()} fallback={<FileText size={14} />}><LoaderCircle size={14} class="spin" /></Show><span>{extracting() ? 'Đang trích xuất' : 'Extract Subtitle'}</span>
                </button>
              </Show>
              <Show when={hasAudio() || currentProject()?.transcriptSettings?.mode === 'ocr'}>
              <button class="mini-tool-button" type="button" disabled={asrRunning() || checkingAsr() || extracting() || translationRunning()} onClick={() => void requestAsr()}>
                  <Show when={asrRunning() || checkingAsr()} fallback={currentProject()?.transcriptSettings?.mode === 'ocr' ? <ImagePlus size={14} /> : <AudioLines size={14} />}><LoaderCircle size={14} class="spin" /></Show><span>{asrRunning() ? 'Đang tạo transcript' : checkingAsr() ? 'Đang kiểm tra model' : currentProject()?.transcriptSettings?.mode === 'ocr' ? transcript()?.segments.length ? 'Tạo lại bằng OCR' : 'Tạo transcript bằng OCR' : transcript()?.segments.length ? 'Regenerate' : 'Generate Transcript'}</span>
                </button>
              </Show>
            </div>
          </div>
          <div class="transcript-translation-toolbar">
            <label class="transcript-search"><Search size={14} /><input value={translationSearch()} placeholder="Tìm trong transcript…" aria-label="Tìm văn bản gốc và bản dịch" onInput={(event) => setTranslationSearch(event.currentTarget.value)} /></label>
            <div class="translation-job-actions">
              <button class="mini-tool-button mini-primary" type="button" disabled={translationRunning() || !(transcript()?.segments.length)} onClick={() => requestTranslation('all')}>Translate All</button>
              <button class="mini-tool-button" type="button" disabled={translationRunning() || !(transcript()?.segments.length)} onClick={() => requestTranslation('missing')}>Translate Missing</button>
              <button class="mini-tool-button" type="button" disabled={translationRunning() || !selectedSegmentIds().size} onClick={() => requestTranslation('selected')}>Translate Selected ({selectedSegmentIds().size})</button>
            </div>
          </div>
          <div class="tts-job-actions">
            <button class="mini-tool-button mini-primary" type="button" disabled={ttsRunning() || translationRunning() || asrRunning() || !transcript()?.segments.some((segment) => segment.translated.trim())} onClick={() => void launchTts('all')}>Tạo audio cho bản dịch</button>
            <button class="mini-tool-button" type="button" disabled={ttsRunning() || translationRunning() || asrRunning() || !transcript()?.segments.some((segment) => segment.translated.trim() && (!segment.tts?.audioPath || segment.tts.status !== 'ready'))} onClick={() => void launchTts('missing')}>Chỉ phần còn thiếu</button>
            <button class="mini-tool-button" type="button" disabled={ttsRunning() || !selectedSegmentIds().size} onClick={() => void launchTts('selected', [...selectedSegmentIds()])}>Tạo cho mục đã chọn ({selectedSegmentIds().size})</button>
          </div>
          <div class="transcript-columns"><span>VĂN BẢN GỐC</span><span>BẢN DỊCH</span></div>
          <Show when={!hasVideo()}>
            <div class="transcript-empty-state"><div class="transcript-empty-icon"><FileText size={20} /></div><span class="transcript-empty-kicker">CHƯA CÓ VIDEO</span><h3>Thêm video để bắt đầu</h3><p>Chọn video nguồn ở khu vực xem trước để kiểm tra subtitle và tạo transcript từ audio.</p></div>
          </Show>
          <Show when={hasVideo() && transcriptLoading()}>
            <div class="transcript-empty-state"><LoaderCircle class="spin" size={23} /><h3>Đang đọc transcript…</h3></div>
          </Show>
          <Show when={hasVideo() && !transcriptLoading()}>
            <Show when={(transcript()?.segments.length ?? 0) > 0} fallback={
              <div class="transcript-empty-state">
                <div class="transcript-empty-icon"><FileText size={20} /></div>
                <span class="transcript-empty-kicker">CHƯA CÓ TRANSCRIPT</span>
                <h3>{currentProject()?.transcriptSettings?.mode === 'ocr' ? 'Đọc subtitle đã cháy vào hình' : tracks().length ? 'Chọn subtitle hoặc tạo transcript' : hasAudio() ? 'Tạo transcript từ audio' : 'Video không có audio stream'}</h3>
              <p>{currentProject()?.transcriptSettings?.mode === 'ocr' ? currentProject()?.transcriptSettings?.ocrRegion ? `Vùng OCR đã chọn · lấy ${currentProject()?.transcriptSettings?.ocrFps ?? 2} frame/s. Bấm Tạo transcript bằng OCR ở phía trên.` : 'Chọn vùng có subtitle trên preview. OCR sẽ đọc chữ trong vùng đó theo tốc độ lấy mẫu đã đặt ở Cài đặt dự án.' : tracks().some((track) => track.imageBased) ? 'Video có subtitle dạng ảnh; không thể trích xuất thành text. Chuyển sang OCR trong Cài đặt dự án hoặc dùng Speech Recognition để tạo transcript.' : 'Chọn một text subtitle track hoặc tạo transcript bằng engine đã chọn trong Speech Recognition.'}</p>
                <Show when={tracks().length > 0}>
                  <div class="empty-track-list"><For each={tracks()}>{(track) => <span classList={{ 'track-image-based': track.imageBased }}>{trackLabel(track)}</span>}</For></div>
                </Show>
                <button class="button button-outline" type="button" disabled={asrRunning()} onClick={addSegment}><Plus size={14} /> Thêm segment thủ công</button>
              </div>
            }>
              <div class="transcript-segment-list">
                <div class="transcript-selection-row"><label><input type="checkbox" checked={filteredSegments().length > 0 && filteredSegments().every((segment) => selectedSegmentIds().has(segment.id))} onChange={toggleVisibleSelection} />Chọn các dòng hiển thị</label><span>{filteredSegments().length} / {transcript()?.segments.length ?? 0}</span></div>
                <Index each={filteredSegments()}>{(segment) => (
                  <article class="transcript-segment-card" data-segment-id={segment().id} classList={{ 'segment-active': activeSegmentId() === segment().id }}>
                    <header class="transcript-segment-heading">
                      <div class="segment-heading-left"><input type="checkbox" aria-label="Chọn segment" checked={selectedSegmentIds().has(segment().id)} onChange={() => toggleSegmentSelection(segment().id)} /><button type="button" class="segment-time-jump" onClick={() => selectTranscriptSegment(segment())} title="Tìm đến segment này trong video">
                        {formatTimestamp(segment().start)} <span>→</span> {formatTimestamp(segment().end)}
                      </button></div>
                      <div class="segment-heading-actions"><span class="segment-translation-status" classList={{ 'segment-status-edited': segment().translationStatus === 'edited', 'segment-status-review': segment().translatedRevision !== null && segment().translatedRevision! < segment().sourceRevision, 'segment-status-failed': segment().translationStatus === 'failed' }}>{segment().translatedRevision !== null && segment().translatedRevision! < segment().sourceRevision ? 'REVIEW' : segment().translationStatus.toUpperCase()}</span>
                        <button class="segment-action-button" type="button" aria-label="Dịch lại segment" title="Dịch lại segment" disabled={translationRunning() || !segment().source.trim()} onClick={() => requestTranslation('segment', segment().id)}><Languages size={13} /></button>
                        <Show when={segment().translated.trim()}><button class="segment-action-button tts-segment-button" type="button" aria-label={segment().tts?.audioPath ? 'Tạo lại audio đoạn dịch' : 'Tạo audio đoạn dịch'} title={segment().tts?.audioPath ? 'Tạo lại audio đoạn dịch' : 'Tạo audio đoạn dịch'} disabled={ttsRunning()} onClick={() => void launchTts('segment', [segment().id])}><Volume2 size={13} /></button></Show>
                        <button class="segment-remove-button" type="button" aria-label="Xóa segment" title="Xóa segment" disabled={asrRunning() || translationRunning()} onClick={() => removeSegment(segment().id)}><Trash2 size={13} /></button>
                      </div>
                    </header>
                    <div class="segment-edit-grid">
                      <div class="segment-original-column">
                        <div class="segment-timestamp-fields">
                          <label><span>IN</span><input type="number" min="0" max={videoDuration() || undefined} step="0.01" aria-label="Thời điểm bắt đầu" disabled={asrRunning() || translationRunning()} value={segment().start} onChange={(event) => updateSegment(segment().id, { start: Number(event.currentTarget.value) })} /></label>
                          <label><span>OUT</span><input type="number" min="0" max={videoDuration() || undefined} step="0.01" aria-label="Thời điểm kết thúc" disabled={asrRunning() || translationRunning()} value={segment().end} onChange={(event) => updateSegment(segment().id, { end: Number(event.currentTarget.value) })} /></label>
                        </div>
                        <textarea class="segment-source-input" rows="2" aria-label="Văn bản gốc" placeholder="Nhập lời thoại…" disabled={asrRunning() || translationRunning()} value={segment().source} onInput={(event) => updateSegment(segment().id, { source: event.currentTarget.value })} />
                      </div>
                      <div class="segment-translation-column"><textarea class="segment-translation-input" rows="2" aria-label="Bản dịch" placeholder="Chưa dịch" disabled={translationRunning()} value={segment().translated} onInput={(event) => updateSegment(segment().id, { translated: event.currentTarget.value })} />
                        <Show when={segment().tts}><div class="segment-tts-state">
                          <div class="segment-tts-summary">
                            <span classList={{ 'tts-state-outdated': segment().tts?.status === 'outdated' || segment().tts?.status === 'failed', 'tts-state-overflow': segment().tts?.status === 'durationOverflow' }}>{segment().tts?.status === 'outdated' ? 'CẦN TẠO LẠI' : segment().tts?.status === 'durationOverflow' ? 'DÀI HƠN KHUNG TTS' : segment().tts?.status === 'failed' ? 'TẠO LỖI' : segment().tts?.audioPath ? 'AUDIO SẴN SÀNG' : 'CHƯA CÓ AUDIO'}</span>
                            <Show when={segment().tts?.warning}><small class="segment-tts-warning">{segment().tts?.warning}</small></Show>
                            <Show when={segment().tts?.duration}><small class="segment-tts-duration">Audio {segment().tts!.duration!.toFixed(1)}s / khung TTS tối đa {(ttsWindowEnd(transcript()?.segments ?? [], segment()) - segment().start).toFixed(1)}s</small></Show>
                          </div>
                          <div class="segment-tts-actions">
                            <button class="mini-tool-button" type="button" disabled={!segment().tts?.audioPath || segment().tts?.status === 'outdated' || segment().tts?.status === 'failed'} onClick={() => void previewTts(segment())}>{previewingTtsSegment() === segment().id ? 'Đang phát…' : 'Nghe thử'}</button>
                            <Show when={segment().tts?.status === 'durationOverflow'}><button class="mini-tool-button" type="button" disabled={ttsRunning()} onClick={() => void fitAndRegenerate(segment().id)}>Fit &amp; tạo lại</button></Show>
                          </div>
                        </div></Show>
                      </div>
                    </div>
                  </article>
                )}</Index>
                <button class="add-segment-button" type="button" disabled={asrRunning() || translationRunning()} onClick={addSegment}><Plus size={14} /> Thêm segment</button>
              </div>
            </Show>
          </Show>
          <Show when={asrRunning()}>
            <div class="transcription-job-banner" role="status">
              <div class="job-banner-icon"><AudioLines size={15} /></div>
              <div class="job-banner-copy"><strong>{asrMessage() || 'Đang xử lý audio…'}</strong><small>{asrStage() || 'Speech Recognition'}{progressPercent() !== undefined ? ` · ${progressPercent()}%` : ''}</small>
                <Show when={progressPercent() !== undefined}><div class="job-progress-track"><span style={{ width: `${progressPercent()}%` }} /></div></Show>
              </div>
              <button class="icon-button" type="button" title="Hủy transcription" aria-label="Hủy transcription" disabled={!asrJobId()} onClick={() => void cancelAsr()}><X size={16} /></button>
            </div>
          </Show>
          <Show when={translationRunning()}>
            <div class="transcription-job-banner translation-job-banner" role="status">
              <div class="job-banner-icon"><Languages size={15} /></div>
              <div class="job-banner-copy"><strong>{translationMessage() || translationProgress()?.message || 'Đang dịch transcript…'}</strong>
                <small>{translationProgress() ? `${translationProgress()!.provider} · batch ${translationProgress()!.currentBatch}/${translationProgress()!.totalBatches} · ${translationProgress()!.translatedSegments}/${translationProgress()!.totalSegments} segment đã lưu` : 'Đang chuẩn bị provider'}</small>
                <Show when={translationBatchPercent() !== undefined}><div class="job-progress-track"><span style={{ width: `${translationBatchPercent()}%` }} /></div></Show>
              </div>
              <button class="icon-button" type="button" title="Hủy dịch" aria-label="Hủy dịch" disabled={!translationJobId()} onClick={() => void cancelTranslation()}><X size={16} /></button>
            </div>
          </Show>
          <Show when={ttsRunning()}>
            <div class="transcription-job-banner tts-job-banner" role="status">
              <div class="job-banner-icon"><Volume2 size={15} /></div>
              <div class="job-banner-copy"><strong>{ttsMessage() || dubbingProgress()?.message || 'Đang tạo audio lồng tiếng…'}</strong><small>{dubbingProgress() ? `${dubbingProgress()!.completedSegments}/${dubbingProgress()!.totalSegments} segment đã lưu · ${Math.round(dubbingProgress()!.progress * 100)}%` : 'Đang khởi tạo VieNeu-TTS'}</small>
                <Show when={dubbingProgress()}><div class="job-progress-track"><span style={{ width: `${Math.round((dubbingProgress()?.progress ?? 0) * 100)}%` }} /></div></Show>
              </div>
              <button class="icon-button" type="button" title="Hủy tạo giọng đọc" aria-label="Hủy tạo giọng đọc" disabled={!ttsJobId()} onClick={() => void cancelTts()}><X size={16} /></button>
            </div>
          </Show>
          <footer class="transcript-bottom-bar">
            <span><span class="small-status-dot" /> {transcript()?.segments.length ?? 0} SEGMENTS</span>
            <span class={`transcript-save-state save-state-${saveState()}`}>
              {saveState() === 'saved' ? <><Check size={12} /> Đã lưu</> : saveState() === 'saving' ? <><LoaderCircle size={12} class="spin" /> Đang lưu…</> : <><AlertTriangle size={12} /> Chưa lưu được</>}
            </span>
          </footer>
          <Show when={saveError()}><div class="transcript-save-error" role="alert">{saveError()} <button type="button" onClick={() => void persistTranscript()}>Thử lưu lại</button></div></Show>
        </div>
      </Show>

      <Show when={activeWorkspacePanelTab() === 'media'}>
        <div class="resource-panel-content" role="tabpanel">
          <div class="resource-section-heading"><div><span>PROJECT ASSETS</span><h3>Thư viện media</h3></div><FileImage size={16} /></div>
          <Show when={hasVideo()} fallback={<div class="asset-empty-card"><div class="asset-empty-icon"><FileImage size={19} /></div><strong>Chưa có video nguồn</strong><p>Chọn video tại khu vực xem trước để thêm media vào dự án.</p><button class="button button-outline" type="button" onClick={() => setNotice({ kind: 'info', message: 'Dùng nút Chọn video tại khu vực xem trước.' })}>Chọn video</button></div>}>
            <div class="resource-subsection"><span class="resource-subtitle">VIDEO NGUỒN</span>
              <div class="media-video-card">
                <div class="media-video-title"><span class="resource-type-icon"><FileImage size={15} /></span><span><strong>{currentProject()?.source.originalName ?? 'Video nguồn'}</strong><small>{formatBytes(currentProject()?.source.fileSize)}</small></span></div>
                <div class="media-spec-grid">
                  <div><small>ĐỘ PHÂN GIẢI</small><strong>{currentProject()?.source.width && currentProject()?.source.height ? `${currentProject()?.source.width} × ${currentProject()?.source.height}` : '—'}</strong></div>
                  <div><small>THỜI LƯỢNG</small><strong>{formatDuration(currentProject()?.source.duration)}</strong></div>
                  <div><small>FPS</small><strong>{formatFps(currentProject()?.source.fps)}</strong></div>
                  <div><small>VIDEO / AUDIO</small><strong>{currentProject()?.source.videoCodec?.toUpperCase() ?? '—'} / {currentProject()?.source.audioCodec?.toUpperCase() ?? '—'}</strong></div>
                </div>
              </div>
            </div>
            <div class="resource-subsection"><span class="resource-subtitle">AUDIO STREAMS</span>
              <Show when={hasAudio()} fallback={<div class="media-inline-empty">Video không có audio stream.</div>}>
                <For each={currentProject()?.source.audioStreams ?? []}>{(stream) => <div class="resource-placeholder-row"><span class="resource-type-icon"><AudioLines size={15} /></span><span><strong>{stream.language?.toUpperCase() ?? `Audio track #${stream.streamIndex}`}</strong><small>{stream.codec.toUpperCase()} · {stream.channels ?? '—'} channels · {stream.sampleRate ? `${Math.round(stream.sampleRate / 1000)} kHz` : '—'}</small></span><span class="resource-state-label">AUDIO</span></div>}</For>
              </Show>
            </div>
            <div class="resource-subsection"><span class="resource-subtitle">SUBTITLE TRACKS</span>
              <Show when={tracks().length} fallback={<div class="media-inline-empty">Không tìm thấy embedded subtitle stream. Có thể tạo transcript từ audio.</div>}>
                <For each={tracks()}>{(track) => <div class="resource-placeholder-row"><span class="resource-type-icon"><FileText size={15} /></span><span><strong>{track.language?.toUpperCase() ?? `Track #${track.streamIndex}`}{track.title ? ` · ${track.title}` : ''}</strong><small>{track.codec.toUpperCase()} · {track.textSupported ? 'Văn bản' : 'Bitmap — không thể trích xuất text'}</small></span><span class="resource-state-label">{track.textSupported ? 'TEXT' : 'IMAGE'}</span></div>}</For>
              </Show>
              <Show when={textTracks().length > 0}>
                <div class="media-extract-row">
                  <label for="media-subtitle-track">Chọn track văn bản</label>
                  <select id="media-subtitle-track" class="settings-select media-track-select" value={selectedTrack()} onChange={(event) => setSelectedTrack(event.currentTarget.value)}>
                    <option value="">Chọn subtitle track</option><For each={textTracks()}>{(track) => <option value={track.streamIndex}>{trackLabel(track)}</option>}</For>
                  </select>
                  <button class="button button-quiet" type="button" disabled={!selectedTrack() || extracting() || asrRunning()} onClick={() => void extractSelectedSubtitle()}><Show when={extracting()} fallback={<FileText size={14} />}><LoaderCircle size={14} class="spin" /></Show> Extract Subtitle</button>
                </div>
              </Show>
            </div>
            <div class="resource-subsection"><span class="resource-subtitle">TRANSCRIPT</span>
              <div class="resource-placeholder-row"><span class="resource-type-icon"><FileText size={15} /></span><span><strong>{transcript()?.segments.length ? `${transcript()!.segments.length} segments` : 'Chưa có transcript'}</strong><small>{transcript()?.sourceType === 'subtitle' ? 'Nguồn: Embedded subtitle' : transcript()?.sourceType === 'ocr' ? 'Nguồn: OCR subtitle trên hình' : transcript()?.sourceType === 'asr' ? 'Nguồn: Speech Recognition' : 'Nhập subtitle hoặc tạo transcript'}</small></span><span class="resource-state-label">{transcript()?.segments.length ? 'READY' : 'EMPTY'}</span></div>
              <Show when={hasAudio()}><button class="button button-quiet media-generate-button" type="button" disabled={asrRunning() || checkingAsr() || extracting()} onClick={() => void requestAsr()}><RefreshCw size={14} /> {transcript()?.segments.length ? 'Regenerate Transcript' : 'Generate Transcript'}</button></Show>
            </div>
            <div class="resource-subsection"><span class="resource-subtitle">BLUR REGIONS <button class="inline-add-button" type="button" onClick={() => setNotice({ kind: 'info', message: 'Dùng nút Blur trên thanh công cụ preview để thêm vùng blur.' })}><Plus size={12} /> Thêm trên preview</button></span>
              <Show when={currentProject()?.blurRegions.length} fallback={<div class="media-inline-empty">Chưa có vùng blur.</div>}>
                <For each={currentProject()?.blurRegions ?? []}>{(region) => <div class="overlay-resource-row">
                  <button class="overlay-resource-select" type="button" onClick={() => { setSelectedOverlay({ type: 'blur', id: region.id }); setActiveWorkspacePanelTab('properties'); }}><span class="overlay-resource-chip blur-chip" /> <span><strong>{isAiRemovalEffect(region.effect) ? 'Vùng xóa chữ · STTN' : region.role === 'subtitle' ? 'Subtitle Blur' : 'Vùng blur'}</strong><small>{region.enabled ? isAiRemovalEffect(region.effect) ? region.textRemovalMode === 'fullRegion' ? 'Phục hồi toàn vùng khoanh' : 'RapidOCR + STTN' : `Mạnh ${Math.round(region.strength)}%` : 'Đang tắt'}</small></span></button>
                  <label class="overlay-resource-toggle" title="Bật/tắt vùng blur"><input aria-label="Bật vùng blur" type="checkbox" checked={region.enabled} onChange={(event) => updateBlurSettings(region.id, { enabled: event.currentTarget.checked })} /></label>
                </div>}</For>
              </Show>
            </div>
            <div class="resource-subsection"><span class="resource-subtitle">LOGOS <button class="inline-add-button" type="button" onClick={() => setActiveWorkspacePanelTab('properties')}><ImagePlus size={12} /> Quản lý</button></span>
              <Show when={currentProject()?.logos.some((logo) => !logo.mergeOnly)} fallback={<div class="media-inline-empty">Chưa có logo. Thêm logo từ thanh công cụ preview.</div>}>
                <For each={currentProject()?.logos.filter((logo) => !logo.mergeOnly) ?? []}>{(logo) => <div class="overlay-resource-row">
                  <button class="overlay-resource-select" type="button" onClick={() => { setSelectedOverlay({ type: 'logo', id: logo.id }); setActiveWorkspacePanelTab('properties'); }}><span class="overlay-resource-chip logo-chip"><FileImage size={12} /></span><span><strong>{logo.originalName}</strong><small>{Math.round(logo.opacity * 100)}% opacity</small></span></button>
                  <label class="overlay-resource-toggle" title="Bật/tắt logo"><input aria-label={`Bật logo ${logo.originalName}`} type="checkbox" checked={logo.enabled} onChange={(event) => updateLogoSettings(logo.id, { enabled: event.currentTarget.checked })} /></label>
                </div>}</For>
              </Show>
            </div>
          </Show>
        </div>
      </Show>

      <Show when={activeWorkspacePanelTab() === 'properties'}>
        <div class="properties-panel-content" role="tabpanel">
          <div class="properties-header"><span class="properties-icon"><Settings2 size={16} /></span><div><span>PROJECT SETTINGS</span><h3>Thuộc tính</h3></div></div>
          <Show when={selectedBlur()}>{(region) => <div class="property-group overlay-property-group"><div class="property-group-title"><span>{isAiRemovalEffect(region().effect) ? 'AI REMOVE · STTN' : 'VÙNG BLUR'}</span><span class="property-status" classList={{ 'status-off': !region().enabled }}>{region().enabled ? 'BẬT' : 'TẮT'}</span></div>
            <label class="property-control-row"><span>Loại vùng</span><select class="settings-select" value={region().role ?? 'generic'} onChange={(event) => updateBlurSettings(region().id, { role: event.currentTarget.value as 'generic' | 'subtitle' })}><option value="generic">{isAiRemovalEffect(region().effect) ? 'Vùng xóa chữ' : 'Generic Blur'}</option><option value="subtitle">{isAiRemovalEffect(region().effect) ? 'Subtitle Region' : 'Subtitle Blur'}</option></select></label>
            <Show when={region().role !== 'subtitle' && currentProject()?.subtitle.enabled}><p class="overlay-selection-hint">Có thể đổi loại vùng này thành Subtitle Region để đặt bản dịch bên trong nó, hoặc chọn một vùng subtitle riêng trong Properties.</p></Show>
            <label class="property-control-row"><span>Phương pháp</span><select class="settings-select" value={region().effect ?? 'blur'} onChange={(event) => updateBlurSettings(region().id, { effect: event.currentTarget.value as 'blur' | 'sttn' })}><option value="blur">Blur · nhanh</option><option value="sttn">AI Remove · STTN</option></select></label>
            <label class="property-control-row"><span>Hiển thị</span><input type="checkbox" checked={region().enabled} onChange={(event) => updateBlurSettings(region().id, { enabled: event.currentTarget.checked })} /></label>
            <Show when={(region().effect ?? 'blur') === 'blur'}>
              <label class="property-control-row"><span>Độ mạnh <small>{Math.round(region().strength)}%</small></span><input type="range" min="1" max="100" value={region().strength} onInput={(event) => updateBlurSettings(region().id, { strength: Number(event.currentTarget.value) })} /></label>
              <label class="property-control-row"><span>Hòa mềm viền <small>{region().edgeBlend ?? 0}%</small></span><input type="range" min="0" max="50" value={region().edgeBlend ?? 0} onInput={(event) => updateBlurSettings(region().id, { edgeBlend: Number(event.currentTarget.value) })} /></label>
              <label class="property-control-row"><span>Nhiễu hạt <small>{region().pixelNoise ?? 0}%</small></span><input type="range" min="0" max="100" value={region().pixelNoise ?? 0} onInput={(event) => updateBlurSettings(region().id, { pixelNoise: Number(event.currentTarget.value) })} /></label>
            </Show>
            <Show when={isAiRemovalEffect(region().effect)}>
              <div class="overlay-selection-hint ai-removal-info">
                <strong>{region().textRemovalMode === 'fullRegion' ? 'Phục hồi toàn vùng đã khoanh' : 'RapidOCR mask + STTN'}</strong>
                <span>{region().textRemovalMode === 'fullRegion'
                  ? 'RapidOCR quét thưa để tìm các khoảng có phụ đề; STTN phục hồi toàn bộ hình chữ nhật đã khoanh trong các khoảng đó. Cách này vẫn có thể thay đổi cả nền phía sau vùng.'
                  : 'RapidOCR tìm thời điểm có chữ và tạo mask theo nét chữ; STTN dùng các khung lân cận để phục hồi nền. Transcript không bị thay đổi.'}</span>
                <label class="property-control-row"><span>Phạm vi phục hồi</span><select class="settings-select" value={region().textRemovalMode ?? 'ocrGuided'} onChange={(event) => updateBlurSettings(region().id, { textRemovalMode: event.currentTarget.value as 'ocrGuided' | 'fullRegion' })}><option value="ocrGuided">Theo nét chữ · RapidOCR</option><option value="fullRegion">Phục hồi toàn vùng khoanh</option></select></label>
                <label class="property-control-row"><span>Tốc độ quét RapidOCR</span><select class="settings-select" value={region().textRemovalScanFps ?? 2} onChange={(event) => updateBlurSettings(region().id, { textRemovalScanFps: Number(event.currentTarget.value) === 1 ? 1 : 2 })}><option value="1">1 khung/giây · nhanh</option><option value="2">2 khung/giây · khuyến nghị</option></select></label>
                <Show when={textRemovalEnvironmentLoading()}><span class="ai-removal-state"><LoaderCircle size={12} class="spin" /> Đang kiểm tra thành phần…</span></Show>
                <Show when={!textRemovalEnvironmentLoading() && textRemovalEnvironment()}>{(environment) => <>
                  <span class="ai-removal-state" classList={{ 'ai-removal-ready': environment().ocrReady }}>{environment().ocrReady ? `RapidOCR ${environment().ocrModel} sẵn sàng` : `RapidOCR ${environment().ocrModel} chưa sẵn sàng`}</span>
                  <span class="ai-removal-state" classList={{ 'ai-removal-ready': environment().sttnRuntimeInstalled && environment().sttnModelInstalled }}>
                    {`STTN ${environment().sttnRuntimeInstalled && environment().sttnModelInstalled ? 'đã cài' : 'chưa cài đầy đủ'}`}
                  </span>
                  <Show when={textRemovalReady(region().effect)}>
                    <div class="ai-removal-preview-actions">
                      <button class="button button-primary ai-removal-preview-button" type="button" disabled={!region().enabled || !!textRemovalPreviewJobId()} onClick={() => void startTextRemovalPreview(region().id)}>
                        <Show when={textRemovalPreviewJobId()} fallback={<Sparkles size={14} />}><LoaderCircle size={14} class="spin" /></Show>
                        {textRemovalPreviewJobId() ? 'Đang xử lý preview…' : region().textRemovalMode === 'fullRegion' ? 'Phục hồi toàn vùng khoanh' : 'Xóa sub bằng STTN'}
                      </button>
                      <Show when={textRemovalPreviewJobId()}><button class="button button-quiet" type="button" disabled={cancelingTextRemovalPreview()} onClick={() => void cancelTextRemovalPreview()}>{cancelingTextRemovalPreview() ? 'Đang hủy…' : 'Hủy'}</button></Show>
                    </div>
                    <Show when={textRemovalPreviewJobId()}>
                      <span class="ai-removal-state" role="status">{textRemovalPreviewProgress().message} · {Math.round(textRemovalPreviewProgress().progress * 100)}%</span>
                      <div class="job-progress-track ai-removal-progress"><span style={{ width: `${Math.max(2, textRemovalPreviewProgress().progress * 100)}%` }} /></div>
                    </Show>
                  </Show>
                </>}</Show>
                <Show when={textRemovalEnvironmentError()}><span class="ai-removal-error">{textRemovalEnvironmentError()}</span><button class="button button-quiet ai-removal-download" type="button" onClick={() => void refreshTextRemovalEnvironment()}>Kiểm tra lại</button></Show>
              </div>
            </Show>
            <div class="overlay-property-grid"><label>X <input type="number" min="0" max={Math.round((1 - region().width) * 100)} step="1" value={Math.round(region().x * 100)} onInput={(event) => updateBlurSettings(region().id, { x: Number(event.currentTarget.value) / 100 })} /></label><label>Y <input type="number" min="0" max={Math.round((1 - region().height) * 100)} step="1" value={Math.round(region().y * 100)} onInput={(event) => updateBlurSettings(region().id, { y: Number(event.currentTarget.value) / 100 })} /></label><label>Rộng <input type="number" min="2" max={Math.round((1 - region().x) * 100)} value={Math.round(region().width * 100)} onInput={(event) => updateBlurSettings(region().id, { width: Number(event.currentTarget.value) / 100 })} /></label><label>Cao <input type="number" min="2" max={Math.round((1 - region().y) * 100)} value={Math.round(region().height * 100)} onInput={(event) => updateBlurSettings(region().id, { height: Number(event.currentTarget.value) / 100 })} /></label></div>
            <button class="button button-danger overlay-delete-button" type="button" onClick={() => { const project = currentProject(); if (!project) return; updateVisualSettings({ blurRegions: project.blurRegions.filter((item) => item.id !== region().id) }, true); setSelectedOverlay(null); }}><Trash2 size={13} /> Xóa vùng blur</button>
          </div>}</Show>
          <Show when={selectedLogo()}>{(logo) => <div class="property-group overlay-property-group"><div class="property-group-title"><span>LOGO</span><span class="property-status" classList={{ 'status-off': !logo().enabled }}>{logo().enabled ? 'BẬT' : 'TẮT'}</span></div>
            <div class="overlay-selected-name" title={logo().originalName}><FileImage size={14} />{logo().originalName}</div>
            <label class="property-control-row"><span>Hiển thị</span><input type="checkbox" checked={logo().enabled} onChange={(event) => updateLogoSettings(logo().id, { enabled: event.currentTarget.checked })} /></label>
            <label class="property-control-row"><span>Độ mờ <small>{Math.round(logo().opacity * 100)}%</small></span><input type="range" min="0" max="1" step="0.01" value={logo().opacity} onInput={(event) => updateLogoSettings(logo().id, { opacity: Number(event.currentTarget.value) })} /></label>
            <label class="property-control-row"><span>Giữ tỷ lệ ảnh</span><input type="checkbox" checked={logo().maintainAspectRatio} onChange={(event) => updateLogoSettings(logo().id, { maintainAspectRatio: event.currentTarget.checked })} /></label>
            <div class="overlay-property-grid"><label>X <input type="number" min="0" max={Math.round((1 - logo().width) * 100)} value={Math.round(logo().x * 100)} onInput={(event) => updateLogoSettings(logo().id, { x: Number(event.currentTarget.value) / 100 })} /></label><label>Y <input type="number" min="0" max={Math.round((1 - logo().height) * 100)} value={Math.round(logo().y * 100)} onInput={(event) => updateLogoSettings(logo().id, { y: Number(event.currentTarget.value) / 100 })} /></label><label>Rộng <input type="number" min="2" max={Math.round((1 - logo().x) * 100)} value={Math.round(logo().width * 100)} onInput={(event) => updateLogoSettings(logo().id, { width: Number(event.currentTarget.value) / 100 })} /></label><label>Cao <input type="number" min="2" max={Math.round((1 - logo().y) * 100)} value={Math.round(logo().height * 100)} onInput={(event) => updateLogoSettings(logo().id, { height: Number(event.currentTarget.value) / 100 })} /></label></div>
            <div class="overlay-property-actions"><button class="button button-quiet" type="button" onClick={() => void replaceSelectedLogo()}><RefreshCw size={13} /> Thay ảnh</button><button class="button button-danger" type="button" onClick={() => void removeSelectedLogo()}><Trash2 size={13} /> Xóa</button></div>
          </div>}</Show>
          <Show when={selectedTextOverlay()}>{(overlay) => <div class="property-group overlay-property-group"><div class="property-group-title"><span>CHỮ TRÊN VIDEO</span><span class="property-status" classList={{ 'status-off': !overlay().enabled }}>{overlay().enabled ? 'BẬT' : 'TẮT'}</span></div>
            <label class="property-control-row"><span>Hiển thị</span><input type="checkbox" checked={overlay().enabled} onChange={(event) => updateTextOverlay(overlay().id, { enabled: event.currentTarget.checked })} /></label>
            <label class="property-control-row text-overlay-content-field"><span>Nội dung</span><textarea value={overlay().text} rows={4} maxLength={4000} onInput={(event) => updateTextOverlay(overlay().id, { text: event.currentTarget.value })} /></label>
            <label class="property-control-row"><span>Font family</span><input class="settings-text-input" value={overlay().fontFamily} maxLength={80} onInput={(event) => updateTextOverlay(overlay().id, { fontFamily: event.currentTarget.value })} /></label>
            <label class="property-control-row"><span>Cỡ chữ <small>{overlay().fontSize}px</small></span><input type="range" min="8" max="240" value={overlay().fontSize} onInput={(event) => updateTextOverlay(overlay().id, { fontSize: Number(event.currentTarget.value) })} /></label>
            <label class="property-control-row"><span>Độ đậm</span><select class="settings-select" value={overlay().fontWeight} onChange={(event) => updateTextOverlay(overlay().id, { fontWeight: Number(event.currentTarget.value) })}><option value="400">Regular</option><option value="500">Medium</option><option value="600">Semibold</option><option value="700">Bold</option><option value="800">Extra bold</option><option value="900">Black</option></select></label>
            <label class="property-control-row"><span>Màu chữ</span><input class="subtitle-color-picker" type="color" value={overlay().color} onInput={(event) => updateTextOverlay(overlay().id, { color: event.currentTarget.value.toUpperCase() })} /></label>
            <label class="property-control-row"><span>Độ mờ <small>{Math.round(overlay().opacity * 100)}%</small></span><input type="range" min="0" max="1" step="0.01" value={overlay().opacity} onInput={(event) => updateTextOverlay(overlay().id, { opacity: Number(event.currentTarget.value) })} /></label>
            <label class="property-control-row"><span>Căn lề</span><select class="settings-select" value={overlay().alignment} onChange={(event) => updateTextOverlay(overlay().id, { alignment: event.currentTarget.value as 'left' | 'center' | 'right' })}><option value="left">Trái</option><option value="center">Giữa</option><option value="right">Phải</option></select></label>
            <div class="overlay-property-grid"><label>X <input type="number" min="0" max={Math.round((1 - overlay().width) * 100)} value={Math.round(overlay().x * 100)} onInput={(event) => updateTextOverlay(overlay().id, { x: Number(event.currentTarget.value) / 100 })} /></label><label>Y <input type="number" min="0" max={Math.round((1 - overlay().height) * 100)} value={Math.round(overlay().y * 100)} onInput={(event) => updateTextOverlay(overlay().id, { y: Number(event.currentTarget.value) / 100 })} /></label><label>Rộng <input type="number" min="2" max={Math.round((1 - overlay().x) * 100)} value={Math.round(overlay().width * 100)} onInput={(event) => updateTextOverlay(overlay().id, { width: Number(event.currentTarget.value) / 100 })} /></label><label>Cao <input type="number" min="2" max={Math.round((1 - overlay().y) * 100)} value={Math.round(overlay().height * 100)} onInput={(event) => updateTextOverlay(overlay().id, { height: Number(event.currentTarget.value) / 100 })} /></label></div>
            <button class="button button-danger overlay-delete-button" type="button" onClick={() => removeTextOverlay(overlay().id)}><Trash2 size={13} /> Xóa lớp chữ</button>
          </div>}</Show>
          <Show when={selectedOverlay()?.type === 'subtitle' || !selectedOverlay()}>
          <div class="property-group"><div class="property-group-title"><span>PHỤ ĐỀ DỊCH</span><span class="property-status" classList={{ 'status-off': !currentProject()?.subtitle.enabled }}>{currentProject()?.subtitle.enabled ? 'BẬT' : 'TẮT'}</span></div>
            <label class="property-control-row"><span>Hiển thị trên preview</span><input type="checkbox" checked={currentProject()?.subtitle.enabled ?? true} onChange={(event) => updateSubtitleSettings({ enabled: event.currentTarget.checked })} /></label>
            <div class="subtitle-region-control">
              <button class="mini-tool-button" type="button" disabled={!currentProject()?.source.videoPath || hasSubtitleBlurRegion()} title={hasSubtitleBlurRegion() ? 'Subtitle Region của Blur/STTN đang quyết định vị trí phụ đề.' : 'Kéo trên video để chọn vùng hiển thị phụ đề.'} onClick={pickSubtitleRegion}>
                <ImagePlus size={14} /><span>{subtitleRegionPicking() ? 'Đang chọn vùng…' : currentProject()?.subtitle.region ? 'Chọn lại vùng subtitle' : 'Chọn vùng subtitle'}</span>
              </button>
              <Show when={hasSubtitleBlurRegion()}><small>Đang dùng vùng Blur/STTN có loại Subtitle Region. Công cụ chọn vùng riêng bị tắt.</small></Show>
              <Show when={!hasSubtitleBlurRegion() && currentProject()?.subtitle.region}><small>Đã lưu vùng riêng. Khi đang chọn thuộc tính phụ đề, kéo khung trên preview để di chuyển hoặc đổi kích thước.</small></Show>
            </div>
            <label class="property-control-row"><span>Font family</span><input class="settings-select overlay-font-input" value={currentProject()?.subtitle.fontFamily ?? 'Arial'} maxLength="96" onInput={(event) => updateSubtitleSettings({ fontFamily: event.currentTarget.value })} /></label>
            <label class="property-control-row"><span>Cỡ chữ video <small>{currentProject()?.subtitle.fontSize ?? 30}px</small></span><input type="range" min="12" max="180" value={currentProject()?.subtitle.fontSize ?? 30} onInput={(event) => updateSubtitleSettings({ fontSize: Number(event.currentTarget.value) })} /></label>
            <p class="overlay-selection-hint">Cỡ chữ tính theo pixel video nguồn. Auto-fit có thể thu nhỏ đến mức tối thiểu; tắt Auto-fit để giữ đúng cỡ đã đặt.</p>
            <label class="property-control-row"><span>Độ đậm</span><select class="settings-select" value={currentProject()?.subtitle.fontWeight ?? 700} onChange={(event) => updateSubtitleSettings({ fontWeight: Number(event.currentTarget.value) })}><option value="400">Regular</option><option value="500">Medium</option><option value="600">Semibold</option><option value="700">Bold</option><option value="800">Extra bold</option></select></label>
            <label class="property-control-row"><span>Màu chữ</span><input class="subtitle-color-picker" type="color" value={currentProject()?.subtitle.textColor ?? '#FFFFFF'} onInput={(event) => updateSubtitleSettings({ textColor: event.currentTarget.value.toUpperCase() })} /></label>
            <label class="property-control-row"><span>Viền chữ</span><input type="checkbox" checked={currentProject()?.subtitle.outline ?? true} onChange={(event) => updateSubtitleSettings({ outline: event.currentTarget.checked })} /></label>
            <Show when={currentProject()?.subtitle.outline}><label class="property-control-row"><span>Màu viền</span><input class="subtitle-color-picker" type="color" value={currentProject()?.subtitle.outlineColor ?? '#000000'} onInput={(event) => updateSubtitleSettings({ outlineColor: event.currentTarget.value.toUpperCase() })} /></label><label class="property-control-row"><span>Độ dày viền <small>{currentProject()?.subtitle.outlineWidth ?? 3}px</small></span><input type="range" min="0" max="16" value={currentProject()?.subtitle.outlineWidth ?? 3} onInput={(event) => updateSubtitleSettings({ outlineWidth: Number(event.currentTarget.value) })} /></label></Show>
            <label class="property-control-row"><span>Nền phụ đề</span><input type="checkbox" checked={currentProject()?.subtitle.backgroundEnabled ?? false} onChange={(event) => updateSubtitleSettings({ backgroundEnabled: event.currentTarget.checked })} /></label>
            <Show when={currentProject()?.subtitle.backgroundEnabled}><label class="property-control-row"><span>Màu nền</span><input class="subtitle-color-picker" type="color" value={currentProject()?.subtitle.backgroundColor ?? '#000000'} onInput={(event) => updateSubtitleSettings({ backgroundColor: event.currentTarget.value.toUpperCase() })} /></label><label class="property-control-row"><span>Độ mờ nền <small>{Math.round((currentProject()?.subtitle.backgroundOpacity ?? 0.48) * 100)}%</small></span><input type="range" min="0" max="1" step="0.01" value={currentProject()?.subtitle.backgroundOpacity ?? 0.48} onInput={(event) => updateSubtitleSettings({ backgroundOpacity: Number(event.currentTarget.value) })} /></label></Show>
            <label class="property-control-row"><span>Căn lề</span><select class="settings-select" value={currentProject()?.subtitle.alignment ?? 'center'} onChange={(event) => updateSubtitleSettings({ alignment: event.currentTarget.value as 'left' | 'center' | 'right' })}><option value="left">Trái</option><option value="center">Giữa</option><option value="right">Phải</option></select></label>
            <label class="property-control-row"><span>Căn dọc trong vùng</span><select class="settings-select" value={currentProject()?.subtitle.verticalAlignment ?? 'middle'} onChange={(event) => updateSubtitleSettings({ verticalAlignment: event.currentTarget.value as 'top' | 'middle' | 'bottom' })}><option value="top">Trên</option><option value="middle">Giữa</option><option value="bottom">Dưới</option></select></label>
            <label class="property-control-row"><span>Tự giảm cỡ chữ nếu cần</span><input type="checkbox" checked={currentProject()?.subtitle.autoFit ?? true} onChange={(event) => updateSubtitleSettings({ autoFit: event.currentTarget.checked })} /></label>
            <label class="property-control-row"><span>Cỡ chữ tối thiểu <small>{currentProject()?.subtitle.minFontSize ?? 20}px</small></span><input type="range" min="8" max={currentProject()?.subtitle.fontSize ?? 30} value={Math.min(currentProject()?.subtitle.minFontSize ?? 20, currentProject()?.subtitle.fontSize ?? 30)} onInput={(event) => updateSubtitleSettings({ minFontSize: Number(event.currentTarget.value) })} /></label>
            <label class="property-control-row"><span>Padding ngang <small>{currentProject()?.subtitle.paddingX ?? 4}%</small></span><input type="range" min="0" max="30" value={currentProject()?.subtitle.paddingX ?? 4} onInput={(event) => updateSubtitleSettings({ paddingX: Number(event.currentTarget.value) })} /></label>
            <label class="property-control-row"><span>Padding dọc <small>{currentProject()?.subtitle.paddingY ?? 10}%</small></span><input type="range" min="0" max="40" value={currentProject()?.subtitle.paddingY ?? 10} onInput={(event) => updateSubtitleSettings({ paddingY: Number(event.currentTarget.value) })} /></label>
            <label class="property-control-row"><span>Khoảng dòng <small>{(currentProject()?.subtitle.lineHeight ?? 1.2).toFixed(1)}×</small></span><input type="range" min="0.8" max="2.4" step="0.1" value={currentProject()?.subtitle.lineHeight ?? 1.2} onInput={(event) => updateSubtitleSettings({ lineHeight: Number(event.currentTarget.value) })} /></label>
          </div>
          </Show>
          <Show when={!selectedOverlay()}><div class="overlay-selection-hint">Chọn một vùng hoặc logo trên preview để chỉnh thuộc tính. Dùng phím mũi tên để di chuyển; giữ Shift để đi nhanh hơn.</div></Show>
          <Show when={!selectedOverlay() || selectedOverlay()?.type === 'subtitle'}><div class="property-group"><div class="property-group-title"><span>ÂM THANH LỒNG TIẾNG</span><span class="property-status" classList={{ 'status-off': !currentProject()?.dubbing.enabled }}>{currentProject()?.dubbing.enabled ? 'BẬT' : 'TẮT'}</span></div>
            <label class="property-control-row"><span>Phát audio trên preview</span><input type="checkbox" checked={currentProject()?.dubbing.enabled ?? false} onChange={(event) => updateDubbingSettings({ enabled: event.currentTarget.checked })} /></label>
            <label class="property-control-row"><span>Provider</span><select class="settings-select" value={currentProject()?.dubbing.provider ?? 'vieneu'} disabled><option value="vieneu">VieNeu-TTS</option></select></label>
            <label class="property-control-row"><span>Tốc độ <small>{(currentProject()?.dubbing.speed ?? ttsSettings()?.speed ?? 1).toFixed(2)}×</small></span><input type="range" min="0.85" max="2" step="0.01" value={currentProject()?.dubbing.speed ?? ttsSettings()?.speed ?? 1} disabled={(currentProject()?.dubbing.fitMode ?? (ttsSettings()?.fitToDuration ? 'fit' : 'custom')) !== 'custom'} onInput={(event) => updateDubbingSettings({ speed: Number(event.currentTarget.value) })} /></label>
            <label class="property-control-row"><span>Audio Fit</span><select class="settings-select" value={currentProject()?.dubbing.fitMode ?? (ttsSettings()?.fitToDuration ? 'fit' : 'custom')} onChange={(event) => updateDubbingSettings({ fitMode: event.currentTarget.value as 'natural' | 'fit' | 'custom' })}><option value="natural">Natural</option><option value="fit">Fit · tối đa 2.00×</option><option value="custom">Custom speed</option></select></label>
            <label class="property-control-row"><span>Âm thanh gốc <small>{Math.round((currentProject()?.dubbing.originalVolume ?? 1) * 100)}%</small></span><input type="range" min="0" max="1" step="0.01" value={currentProject()?.dubbing.originalVolume ?? 1} onInput={(event) => updateDubbingSettings({ originalVolume: Number(event.currentTarget.value) })} /></label>
            <label class="property-control-row"><span>Âm thanh lồng tiếng <small>{Math.round((currentProject()?.dubbing.dubbedVolume ?? 1) * 100)}%</small></span><input type="range" min="0" max="1" step="0.01" value={currentProject()?.dubbing.dubbedVolume ?? 1} onInput={(event) => updateDubbingSettings({ dubbedVolume: Number(event.currentTarget.value) })} /></label>
            <label class="property-control-row"><span>Tắt âm thanh gốc</span><input type="checkbox" checked={currentProject()?.dubbing.muteOriginal ?? false} onChange={(event) => updateDubbingSettings({ muteOriginal: event.currentTarget.checked })} /></label>
            <label class="property-control-row"><span>Giảm âm thanh gốc khi có lời đọc</span><input type="checkbox" checked={currentProject()?.dubbing.duckingEnabled ?? false} onChange={(event) => updateDubbingSettings({ duckingEnabled: event.currentTarget.checked })} /></label>
            <Show when={currentProject()?.dubbing.duckingEnabled}><label class="property-control-row"><span>Mức giảm gốc <small>{Math.round((currentProject()?.dubbing.duckingAmount ?? 0.75) * 100)}%</small></span><input type="range" min="0" max="1" step="0.01" value={currentProject()?.dubbing.duckingAmount ?? 0.75} onInput={(event) => updateDubbingSettings({ duckingAmount: Number(event.currentTarget.value) })} /></label></Show>
            <label class="property-control-row" title={dialogueReductionReady() ? 'Giảm phần thoại trong âm thanh gốc bằng BandIt v2.' : 'Cài bộ xử lý và mô hình BandIt v2 trong Cài đặt → Nhận dạng lời thoại.'} style={{ opacity: dialogueReductionReady() ? 1 : 0.48 }}><span>Giảm thoại gốc <small>{dialogueEnvironment()?.modelCached ? 'BandIt v2' : 'Chưa cài mô hình'}</small></span><input type="checkbox" disabled={!dialogueReductionReady()} checked={dialogueReductionReady() && (currentProject()?.dubbing.dialogueReductionEnabled ?? false)} onChange={(event) => updateDubbingSettings({ dialogueReductionEnabled: event.currentTarget.checked })} /></label>
            <Show when={dialogueReductionReady() && currentProject()?.dubbing.dialogueReductionEnabled}><label class="property-control-row"><span>Mức giảm thoại <small>{Math.round((currentProject()?.dubbing.dialogueReductionAmount ?? 0.65) * 100)}%</small></span><input type="range" min="0" max="1" step="0.01" value={currentProject()?.dubbing.dialogueReductionAmount ?? 0.65} onInput={(event) => updateDubbingSettings({ dialogueReductionAmount: Number(event.currentTarget.value) })} /></label></Show>
            <button class="button button-quiet media-generate-button" type="button" disabled={ttsRunning() || !transcript()?.segments.some((segment) => segment.translated.trim())} onClick={() => void launchTts('missing')}><Volume2 size={14} /> {ttsRunning() ? 'Đang tạo audio…' : 'Tạo phần audio còn thiếu'}</button>
          </div>
          <div class="property-placeholder-note"><Sparkles size={14} /><span>Audio TTS được lưu theo từng segment và căn theo timestamp khi export.</span></div>
          </Show>
        </div>
      </Show>

      <Show when={showAsrConfirm()}>
        <Modal title={asrOptions().mode === 'ocr' ? 'Tạo transcript bằng OCR?' : asrOptions().engine === 'whisperX' ? 'Tạo transcript bằng WhisperX?' : 'Tạo transcript bằng faster-whisper?'} description={asrOptions().mode === 'ocr' ? 'OCR sẽ lấy frame trong vùng đã chọn và nhận dạng subtitle đã cháy vào hình.' : 'Cấu hình Speech Recognition được quản lý trong Cài đặt. Tác vụ này chỉ sử dụng model đã cài sẵn.'} onClose={() => setShowAsrConfirm(false)}>
          <div class="model-confirm-card"><div class="job-banner-icon">{asrOptions().mode === 'ocr' ? <ImagePlus size={17} /> : <AudioLines size={17} />}</div><div><strong>{asrOptions().mode === 'ocr' ? `RapidOCR · PP-OCRv6 ${asrOptions().ocrModel} · ${asrOptions().ocrDevice.toUpperCase()} · ${asrOptions().ocrFps} frame/s` : asrOptions().engine === 'whisperX' ? `WhisperX · ${asrOptions().model} · ${asrOptions().device.toUpperCase()} · ${asrOptions().language.toUpperCase()}` : `${asrOptions().model} · ${asrOptions().device.toUpperCase()} · ${asrOptions().language === 'auto' ? 'tự nhận diện ngôn ngữ' : asrOptions().language.toUpperCase()}`}</strong><p>{asrOptions().mode === 'ocr' ? 'Chỉ xử lý vùng crop đã chọn. Model đã có trong cache; không tải model khi tạo transcript.' : asrOptions().engine === 'whisperX' ? 'Nhận dạng bằng faster-whisper và căn chỉnh timestamp bằng aligner WhisperX đã có trong cache. Không tải model trong lúc tạo transcript.' : 'Model đã có trong cache ứng dụng. Quá trình nhận dạng chạy cục bộ trên thiết bị và không tải model.'}</p></div></div>
          <Show when={(transcript()?.segments.length ?? 0) > 0}><div class="asr-overwrite-warning"><AlertTriangle size={15} /><span>Transcript hiện tại sẽ được thay thế khi tác vụ hoàn tất thành công. Nếu tác vụ thất bại, transcript hiện tại vẫn được giữ.</span></div></Show>
          <footer class="modal-actions"><button class="button button-quiet" type="button" onClick={() => setShowAsrConfirm(false)}>Hủy</button><button class="button button-primary" type="button" disabled={asrOptions().mode === 'ocr' ? !ocrEnvironment()?.workerInstalled || !ocrEnvironment()?.ocrInstalled || !ocrEnvironment()?.runtimeInstalled || !ocrEnvironment()?.modelCached || asrOptions().ocrDevice === 'cuda' && !ocrEnvironment()?.cudaAvailable : !asrEnvironment()?.workerInstalled || !asrEnvironment()?.modelCached} onClick={() => void startAsr()}>{asrOptions().mode === 'ocr' ? <ImagePlus size={15} /> : <AudioLines size={15} />} Tạo transcript</button></footer>
        </Modal>
      </Show>

      <Show when={showExtractConfirm()}>
        <Modal title="Thay transcript hiện tại?" description="Transcript được trích xuất chỉ thay thế bản hiện tại sau khi FFmpeg trích xuất thành công." onClose={() => setShowExtractConfirm(false)}>
          <div class="asr-overwrite-warning"><AlertTriangle size={15} /><span>Các chỉnh sửa chưa lưu và nội dung transcript hiện tại sẽ bị thay thế nếu trích xuất thành công.</span></div>
          <footer class="modal-actions"><button class="button button-quiet" type="button" onClick={() => setShowExtractConfirm(false)}>Hủy</button><button class="button button-primary" type="button" disabled={extracting()} onClick={() => void performSubtitleExtraction()}><FileText size={14} /> Trích xuất subtitle</button></footer>
        </Modal>
      </Show>
      <Show when={overwriteRequest()}>
        <Modal title="Ghi đè bản dịch đã chỉnh sửa?" description="Các segment edited được chọn sẽ được dịch lại bằng provider hiện tại. Nội dung gốc và timestamp vẫn được giữ nguyên." onClose={() => setOverwriteRequest(undefined)}>
          <div class="asr-overwrite-warning"><AlertTriangle size={15} /><span>Thao tác này sẽ thay thế bản dịch thủ công trong phạm vi đã chọn. Bạn có thể dùng Translate Missing để giữ nguyên các bản dịch edited.</span></div>
          <footer class="modal-actions"><button class="button button-quiet" type="button" onClick={() => setOverwriteRequest(undefined)}>Giữ nguyên</button><button class="button button-primary" type="button" onClick={confirmTranslationOverwrite}>Ghi đè và dịch</button></footer>
        </Modal>
      </Show>
    </section>
  );
}
