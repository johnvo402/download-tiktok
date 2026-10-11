import { Dynamic } from 'solid-js/web';
import { createNativePreview } from './native-preview';
import { createAudioPreview } from './audio-preview';
import { convertFileSrc } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { browseMedia, importMediaPaths, mediaBusy, setMediaProgress } from '../../media/media-state';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { open } from '@tauri-apps/plugin-dialog';
import { createEffect, createMemo, createSignal, on, onCleanup, onMount, Show } from 'solid-js';
import {
  AlertTriangle, Film, Maximize2, Pause, Play, Plus, Volume1, Volume2, VolumeX,
} from 'lucide-solid';
import { aiRemovalPreview, blurRegionPicking, currentProject, currentTranscript, ocrRegionEditing, ocrRegionPicking, playbackTime, seekRequest, segmentVoicePreviewActive, setActiveWorkspacePanelTab, setAiRemovalPreview, setBlurRegionPicking, setCurrentProject, setCurrentTranscript, setNotice, setOcrRegionEditing, setOcrRegionPicking, setPlaybackTime, setSeekRequest, setSelectedOverlay, setSegmentVoicePreviewActive, setSubtitleRegionPicking, setVideoPlaying, selectedOverlay, subtitleRegionPicking } from '../../../app/project-state';
import { projectService } from '../../../app/services/project-service';
import { findTranslatedSegmentAtTime } from '../../transcript/transcript-utils';
import { needsCompatibilityPreview } from './video-utils';
import Modal from '../../../shared/components/Modal';
import OverlayCanvas from '../../overlays/OverlayCanvas';
import { fitContainRect } from '../../overlays/overlay-utils';
import { updateVisualSettings, mergeVisualProject, flushVisualSettings } from '../../overlays/overlay-state';

function formatTime(seconds: number) {
  if (!Number.isFinite(seconds) || seconds < 0) return '00:00';
  const total = Math.floor(seconds);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const remainder = total % 60;
  return hours > 0
    ? `${String(hours).padStart(2, '0')}:${String(minutes).padStart(2, '0')}:${String(remainder).padStart(2, '0')}`
    : `${String(minutes).padStart(2, '0')}:${String(remainder).padStart(2, '0')}`;
}

import SourceVideoPreview from '../../timeline/SourceVideoPreview';
import { selectedVideoId } from '../../timeline/video-context-state';

export default function TranslationView() {
  const secondary = () => !!selectedVideoId() && selectedVideoId() !== currentProject()?.timeline.legacyBinding?.videoContentId;
  return <Show when={!secondary()} fallback={<SourceVideoPreview />}><LegacyTranslationView /></Show>;
}

function LegacyTranslationView() {
  let videoElement!: HTMLVideoElement;
  let previewFrame!: HTMLDivElement;
  let audioVisualSeek = false;
  const [videoUrl, setVideoUrl] = createSignal('');
  const [importing, setImporting] = createSignal(false);
  const [loadingVideo, setLoadingVideo] = createSignal(false);
  const [dragging, setDragging] = createSignal(false);
  const [replacePath, setReplacePath] = createSignal<string>();
  const [usingProxy, setUsingProxy] = createSignal(false);
  const [creatingProxy, setCreatingProxy] = createSignal(false);
  const [mediaStatus, setMediaStatus] = createSignal('');
  const [mediaError, setMediaError] = createSignal('');
  const [playing, setPlaying] = createSignal(false);
  const muted = () => currentProject()?.dubbing.muteOriginal ?? false;
  const [volume, setVolume] = createSignal(1);
  const [playerTime, setPlayerTime] = createSignal(0);
  const [previewSubtitleSegmentId, setPreviewSubtitleSegmentId] = createSignal<string>();
  const [duration, setDuration] = createSignal(0);
  const [contentBounds, setContentBounds] = createSignal({ left: 0, top: 0, width: 0, height: 0 });
  let removeDropListener: (() => void) | undefined;
  let componentAlive = true;
  let previewAttemptedForSource: string | undefined;
  const nativePreview = createNativePreview(() => previewFrame, () => videoElement, (time, isPlaying) => {
    if (audioPreview.ready()) return;
    setPlayerTime(time); setPlaybackTime(time); setPlaying(isPlaying); setVideoPlaying(isPlaying);
  });
  const audioPreview = createAudioPreview((status) => {
    setPlayerTime(status.time); setPlaybackTime(status.time); setPlaying(status.playing); setVideoPlaying(status.playing);
    syncPreviewSubtitle(status.time);
    if (nativePreview.preferred()) {
      nativePreview.control('audio', status.time);
      nativePreview.control(status.playing && !status.buffering ? 'play' : 'pause');
    } else if (videoElement && videoElement.readyState >= HTMLMediaElement.HAVE_METADATA) {
      const local = Math.max(0, status.time - (activeAiPreview()?.startSeconds ?? 0));
      if (Math.abs(videoElement.currentTime - local) > 0.15 && !videoElement.seeking) {
        audioVisualSeek = true; videoElement.currentTime = local;
      }
      if (status.playing && videoElement.paused) void videoElement.play().catch(() => undefined);
      if (!status.playing && !videoElement.paused) videoElement.pause();
    }
  }, (message) => setNotice({ kind: 'error', message }));
  createEffect(() => {
    const gain = volume();
    if (audioPreview.ready()) void audioPreview.control('volume', undefined, gain);
  });
  createEffect(() => { if (segmentVoicePreviewActive()) { void audioPreview.control('pause'); nativePreview.control('pause'); } });
  const videoSourceKey = createMemo(() => {
    const project = currentProject();
    return JSON.stringify([
      project?.id ?? null,
      project?.source.videoPath ?? null,
      project?.source.videoCodec ?? null,
      project?.source.videoProfile ?? null,
      project?.source.videoPixelFormat ?? null,
      project?.source.audioCodec ?? null,
      project?.source.duration ?? null,
    ]);
  });

  const updateContentBounds = () => {
    if (!previewFrame) return;
    const project = currentProject();
    const width = videoElement?.videoWidth || project?.source.width || 16;
    const height = videoElement?.videoHeight || project?.source.height || 9;
    const fitted = fitContainRect(previewFrame.clientWidth, previewFrame.clientHeight, width, height);
    setContentBounds({ left: fitted.x, top: fitted.y, width: fitted.width, height: fitted.height });
  };

  let frameResizeObserver: ResizeObserver | undefined;
  type VideoWithFrameCallback = HTMLVideoElement & {
    requestVideoFrameCallback?: (callback: () => void) => number;
    cancelVideoFrameCallback?: (handle: number) => void;
  };
  let subtitleSyncFrameId: number | undefined;
  let subtitleSyncFrameKind: 'video' | 'animation' | undefined;
  let subtitleSyncActive = false;
  let awaitingSourceSeek = false;
  const activeAiPreview = () => aiRemovalPreview()?.projectId === currentProject()?.id
    && !(nativePreview.preferred() && aiRemovalPreview()?.nativeCache) ? aiRemovalPreview() : undefined;
  const activeVideoUrl = createMemo(() => {
    const preview = activeAiPreview();
    return preview ? convertFileSrc(preview.path) : videoUrl();
  });
  createEffect(on(() => activeAiPreview()?.path, () => {
    const preview = activeAiPreview();
    if (preview) { void audioPreview.control('pause'); void audioPreview.control('seek', preview.startSeconds); }
  }));
  const globalTimeForPlayer = (localTime: number) => {
    const preview = activeAiPreview();
    return preview ? preview.startSeconds + localTime : localTime;
  };

  const syncPreviewSubtitle = (time: number) => {
    const activeId = findTranslatedSegmentAtTime(
      currentTranscript()?.segments ?? [],
      time,
      currentProject()?.subtitle.enabled ?? false,
    )?.id;
    setPreviewSubtitleSegmentId((previous) => previous === activeId ? previous : activeId);
  };

  const stopPreviewSubtitleSync = () => {
    subtitleSyncActive = false;
    if (subtitleSyncFrameId === undefined || !videoElement) return;
    if (subtitleSyncFrameKind === 'video') {
      (videoElement as VideoWithFrameCallback).cancelVideoFrameCallback?.(subtitleSyncFrameId);
    } else {
      cancelAnimationFrame(subtitleSyncFrameId);
    }
    subtitleSyncFrameId = undefined;
    subtitleSyncFrameKind = undefined;
  };

  const schedulePreviewSubtitleSync = () => {
    if (!subtitleSyncActive || !videoElement || videoElement.paused || videoElement.ended) return;
    const frameVideo = videoElement as VideoWithFrameCallback;
    const onFrame = () => {
      subtitleSyncFrameId = undefined;
      subtitleSyncFrameKind = undefined;
      if (!subtitleSyncActive || videoElement.paused || videoElement.ended) return;
      syncPreviewSubtitle(globalTimeForPlayer(videoElement.currentTime));
      schedulePreviewSubtitleSync();
    };
    if (typeof frameVideo.requestVideoFrameCallback === 'function') {
      subtitleSyncFrameKind = 'video';
      subtitleSyncFrameId = frameVideo.requestVideoFrameCallback(onFrame);
    } else {
      subtitleSyncFrameKind = 'animation';
      subtitleSyncFrameId = requestAnimationFrame(onFrame);
    }
  };

  const startPreviewSubtitleSync = () => {
    if (nativePreview.preferred()) return;
    if (subtitleSyncActive) return;
    subtitleSyncActive = true;
    if (videoElement) syncPreviewSubtitle(globalTimeForPlayer(videoElement.currentTime));
    schedulePreviewSubtitleSync();
  };

  createEffect(() => {
    const segments = currentTranscript()?.segments;
    const subtitleEnabled = currentProject()?.subtitle.enabled ?? false;
    const time = videoElement && Number.isFinite(videoElement.currentTime) ? globalTimeForPlayer(videoElement.currentTime) : playbackTime();
    void segments;
    void subtitleEnabled;
    syncPreviewSubtitle(time);
  });

  const createCompatibilityPreview = async (
    projectId: string,
    videoPath: string,
    isCurrent: () => boolean,
    forceRebuild = false,
  ) => {
    const sourceKey = `${projectId}\0${videoPath}`;
    if (previewAttemptedForSource === sourceKey) return;
    previewAttemptedForSource = sourceKey;
    setCreatingProxy(true);
    setMediaError('');
    setMediaStatus('Đang chuẩn bị preview tương thích…');
    try {
      const path = await projectService.createPreviewProxy(projectId, forceRebuild);
      if (!isCurrent()) return;
      setUsingProxy(true);
      setVideoUrl(convertFileSrc(path));
      setMediaStatus('');
    } catch (error) {
      if (!isCurrent()) return;
      setMediaStatus('');
      setMediaError(error instanceof Error ? error.message : 'Không thể tạo preview H.264/AAC cho video này.');
    } finally {
      if (isCurrent()) {
        setCreatingProxy(false);
        setMediaStatus('');
      }
    }
  };

  createEffect(on(videoSourceKey, () => {
    const project = currentProject();
    const projectId = project?.id;
    const videoPath = project?.source.videoPath;
    setVideoUrl('');
    setAiRemovalPreview(undefined);
    awaitingSourceSeek = false;
    setMediaError('');
    setMediaStatus('');
    setUsingProxy(false);
    setCreatingProxy(false);
    setDuration(project?.source.duration ?? 0);
    setPlayerTime(0);
    setPlaybackTime(0);
    stopPreviewSubtitleSync();
    setPreviewSubtitleSegmentId(undefined);
    setVideoPlaying(false);
    setSeekRequest(null);
    if (!projectId || !videoPath) return;
    let active = true;
    const isCurrent = () => active && currentProject()?.id === projectId && currentProject()?.source.videoPath === videoPath;
    setLoadingVideo(true);
    if (!nativePreview.preferred() && needsCompatibilityPreview(
      project?.source.videoCodec,
      project?.source.videoProfile,
      project?.source.videoPixelFormat,
      project?.source.audioCodec,
    )) {
      void createCompatibilityPreview(projectId, videoPath, isCurrent)
        .finally(() => active && setLoadingVideo(false));
    } else {
      void projectService.getVideoAssetPath(projectId)
        .then((path) => { if (isCurrent()) setVideoUrl(convertFileSrc(path)); })
        .catch((error: unknown) => {
          if (isCurrent()) setMediaError(error instanceof Error ? error.message : 'Không thể đọc video trong dự án.');
        })
        .finally(() => active && setLoadingVideo(false));
    }
    onCleanup(() => { active = false; });
  }));

  createEffect(() => {
    const target = seekRequest();
    if (target === null || !videoElement || !Number.isFinite(target)) return;
    if (audioPreview.ready()) {
      void audioPreview.control('seek', target);
      if (activeAiPreview() && (target < activeAiPreview()!.startSeconds || target > activeAiPreview()!.endSeconds)) setAiRemovalPreview(undefined);
      nativePreview.control('seek', target);
      setPlayerTime(target); setPlaybackTime(target); syncPreviewSubtitle(target);
      setSeekRequest(null); return;
    }
    if (nativePreview.preferred()) nativePreview.control('seek', target);
    if (videoElement.readyState < HTMLMediaElement.HAVE_METADATA) {
      if (nativePreview.preferred()) setSeekRequest(null);
      return;
    }
    const preview = activeAiPreview();
    if (preview && !awaitingSourceSeek && target >= preview.startSeconds && target <= preview.endSeconds) {
      videoElement.currentTime = Math.max(0, Math.min(target - preview.startSeconds, videoElement.duration || target - preview.startSeconds));
    } else if (preview) {
      awaitingSourceSeek = true;
      setAiRemovalPreview(undefined);
      return;
    } else if (awaitingSourceSeek) {
      return;
    } else {
      videoElement.currentTime = Math.max(0, Math.min(target, duration() || target));
    }
    setSeekRequest(null);
  });

  createEffect(() => {
    currentProject()?.dubbing; audioPreview.ready();
    if (videoElement) { videoElement.volume = 0; videoElement.muted = true; }
  });
  onMount(() => {
    const nativeDrop = (event: Event) => {
      const paths = (event as CustomEvent<string[]>).detail;
      void importMediaPaths(paths);
    };
    window.addEventListener('native-preview-drop', nativeDrop);
    onCleanup(() => window.removeEventListener('native-preview-drop', nativeDrop));
    frameResizeObserver = new ResizeObserver(updateContentBounds);
    if (previewFrame) frameResizeObserver.observe(previewFrame);
    requestAnimationFrame(updateContentBounds);
    onCleanup(() => frameResizeObserver?.disconnect());
    if (videoElement) {
      videoElement.volume = 0;
      videoElement.muted = true;
    }
    void getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === 'over') setDragging(true);
      else if (event.payload.type === 'leave') setDragging(false);
      else if (event.payload.type === 'drop') {
        setDragging(false);
        void importMediaPaths(event.payload.paths);
      }
    }).then((unlisten) => {
      if (componentAlive) removeDropListener = unlisten;
      else unlisten();
    }).catch(() => setNotice({ kind: 'error', message: 'Không thể bật kéo thả tệp video.' }));
    let removeProgressListener: (() => void) | undefined;
    void listen<{ projectId: string; index: number; total: number; name: string }>('media-import-progress', event => {
      if (componentAlive && mediaBusy() && currentProject()?.id === event.payload.projectId) {
        setMediaProgress(`Đang nhập ${event.payload.index + 1}/${event.payload.total}: ${event.payload.name}`);
      }
    }).then(unlisten => { if (componentAlive) removeProgressListener = unlisten; else unlisten(); }).catch(() => undefined);
    onCleanup(() => removeProgressListener?.());
  });

  let observedProjectId: string | undefined;
  createEffect(() => {
    const projectId = currentProject()?.id;
    if (projectId !== observedProjectId) {
      observedProjectId = projectId;
      setSelectedOverlay(null);
    }
  });
  createEffect(on(() => currentProject()?.source.videoPath, (source) => { if (source) requestAnimationFrame(updateContentBounds); }));

  onCleanup(() => {
    componentAlive = false;
    setOcrRegionPicking(false);
    setBlurRegionPicking(false);
    setOcrRegionEditing(false);
    stopPreviewSubtitleSync();
    removeDropListener?.();
    setVideoPlaying(false);
    void audioPreview.control('pause');
  });

  const browse = async () => {
    try {
      const selection = await open({
        multiple: false,
        directory: false,
        filters: [{ name: 'Video', extensions: ['mp4', 'mov', 'mkv', 'webm'] }],
      });
      if (typeof selection === 'string') await selectVideo(selection);
    } catch (error) {
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể mở hộp thoại chọn video.' });
    }
  };

  const selectVideo = async (path: string) => {
    if (!currentProject()) return;
    if (currentProject()!.source.videoPath) {
      setReplacePath(path);
      return;
    }
    await importVideo(path, false);
  };

  const restoreSource = async () => {
    const project=currentProject(); if (!project || importing()) return;
    const selected=await open({multiple:false,title:'Chọn đúng video nguồn ban đầu',filters:[{name:'Video',extensions:['mp4','mov','mkv','webm']}]});
    if (typeof selected !== 'string') return;
    setImporting(true);
    try {
      await flushVisualSettings(project.id);
      const result=await projectService.importVideo(project.id,selected,true,true);
      setCurrentProject(result.project);
      setMediaError('');
      setNotice({kind:'success',message:'Đã khôi phục video nguồn. Transcript, bản dịch và giọng đọc được giữ nguyên.'});
    } catch (error) {setNotice({kind:'error',message:error instanceof Error?error.message:'Không khôi phục được video nguồn.'});}
    finally {setImporting(false);}
  };

  const importVideo = async (path: string, replace: boolean) => {
    const project = currentProject();
    if (!project || importing()) return;
    videoElement?.pause(); nativePreview.control('pause'); void audioPreview.control('pause');
    setImporting(true);
    try {
      const result = await projectService.importVideo(project.id, path, replace);
      setCurrentProject(result.project);
      if (result.replaced) setCurrentTranscript(undefined);
      setNotice({
        kind: 'success',
        message: result.duplicate ? 'Video này đã có trong dự án.' : result.replaced ? 'Đã thay video và xóa transcript phụ thuộc video cũ.' : 'Đã nhập video vào dự án.',
      });
    } catch (error) {
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể nhập video.' });
    } finally {
      setImporting(false);
      setReplacePath(undefined);
      nativePreview.reload();
    }
  };

  const togglePlayback = async () => {
    if (!videoUrl() || !audioPreview.ready()) return;
    if (segmentVoicePreviewActive()) setSegmentVoicePreviewActive(false);
    const next = !playing();
    if (next && playerTime() >= totalDuration() - 0.025) await audioPreview.control('seek', 0);
    if (await audioPreview.control(next ? 'play' : 'pause')) {
      nativePreview.control(next ? 'play' : 'pause'); setPlaying(next); setVideoPlaying(next);
    }
  };

  const toggleFullscreen = () => {
    const frame = previewFrame;
    if (!frame) return;
    if (document.fullscreenElement) void document.exitFullscreen();
    else void frame.requestFullscreen().catch(() => setNotice({ kind: 'error', message: 'Không thể mở video toàn màn hình.' }));
  };

  const handleMediaError = async () => {
    if (nativePreview.preferred()) return;
    if (creatingProxy()) return;
    if (usingProxy()) {
      const mediaError = videoElement?.error;
      const explanation = mediaError ? ({
        1: 'quá trình phát bị hủy',
        2: 'lỗi đọc dữ liệu preview',
        3: 'WebView không giải mã được video preview',
        4: 'WebView không hỗ trợ định dạng preview',
      } as Record<number, string>)[mediaError.code] ?? 'lỗi phát video không xác định' : 'không đọc được trạng thái phát video';
      const detail = mediaError?.message?.trim();
      setMediaError(`Không phát được preview H.264/AAC: ${explanation}${detail ? ` (${detail})` : ''}. Có thể thử tạo lại preview.`);
      return;
    }
    const project = currentProject();
    if (!project?.source.videoPath) {
      setMediaError('Không thể đọc tệp video trong project.');
      return;
    }
    const projectId = project.id;
    const videoPath = project.source.videoPath;
    await createCompatibilityPreview(
      projectId,
      videoPath,
      () => currentProject()?.id === projectId && currentProject()?.source.videoPath === videoPath,
    );
  };

  const retryPreview = () => {
    const project = currentProject();
    if (!project?.source.videoPath) return;
    previewAttemptedForSource = undefined;
    setVideoUrl('');
    void createCompatibilityPreview(
      project.id,
      project.source.videoPath,
      () => currentProject()?.id === project.id && currentProject()?.source.videoPath === project.source.videoPath,
      true,
    );
  };

  const onTimeUpdate = () => {
    if (audioPreview.ready()) return;
    const time = globalTimeForPlayer(videoElement.currentTime);
    if (nativePreview.preferred()) nativePreview.control('audio', time);
    setPlayerTime(time);
    setPlaybackTime(time);
    syncPreviewSubtitle(time);
  };

  const updateOriginalMute = (value: boolean) => {
    const project = currentProject();
    if (!project) return;
    const dubbing = { ...project.dubbing, muteOriginal: value };
    setCurrentProject({ ...project, dubbing });
    void projectService.updateDubbingSettings(project.id, dubbing)
      .then((updated) => setCurrentProject((current) => current?.id === project.id ? { ...current, dubbing: updated.dubbing, updatedAt: updated.updatedAt } : current))
      .catch((error: unknown) => setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể lưu trạng thái tắt âm thanh gốc.' }));
  };

  const addBlurRegion = () => {
    const project = currentProject();
    if (!project?.source.videoPath) return;
    setOcrRegionPicking(false);
    setOcrRegionEditing(false);
    setSubtitleRegionPicking(false);
    setBlurRegionPicking(true);
    setNotice({ kind: 'info', message: 'Kéo trên video để vẽ vùng blur. Nhấn Esc để hủy.' });
  };

  const createBlurRegion = (rect: { x: number; y: number; width: number; height: number }) => {
    const project = currentProject();
    if (!project) return;
    const region = { id: crypto.randomUUID(), ...rect, startMs: 0, endMs: null, enabled: true, strength: 24, role: 'generic' as const, edgeBlend: 10, pixelNoise: 0, textRemovalScanFps: 2 as const, textRemovalMode: 'ocrGuided' as const };
    setBlurRegionPicking(false);
    setSubtitleRegionPicking(false);
    updateVisualSettings({ blurRegions: [...project.blurRegions, region] }, true);
    setSelectedOverlay({ type: 'blur', id: region.id });
    setActiveWorkspacePanelTab('properties');
  };

  const addLogo = async () => {
    const project = currentProject();
    if (!project) return;
    const path = await open({ multiple: false, filters: [{ name: 'Image', extensions: ['png', 'jpg', 'jpeg', 'webp'] }] });
    if (typeof path !== 'string') return;
    try {
      const result = await projectService.importLogo(project.id, path);
      mergeVisualProject(result.project);
      if (result.logo) setSelectedOverlay({ type: 'logo', id: result.logo.id });
      setActiveWorkspacePanelTab('properties');
      setNotice({ kind: 'success', message: `Đã thêm logo “${result.logo?.originalName ?? 'logo'}”.` });
      if (result.cleanupWarning) setNotice({ kind: 'info', message: result.cleanupWarning });
    } catch (error) {
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể thêm ảnh logo.' });
    }
  };

  const selectSubtitleProperties = () => {
    setSelectedOverlay({ type: 'subtitle' });
    setActiveWorkspacePanelTab('properties');
  };

  const addTextOverlay = () => {
    const project = currentProject();
    if (!project) return;
    const overlay = { id: crypto.randomUUID(), text: 'Nhập văn bản', x: 0.25, y: 0.4, width: 0.5, height: 0.2, enabled: true, fontFamily: 'Arial', fontSize: 48, fontWeight: 700, color: '#FFFFFF', opacity: 1, alignment: 'center' as const };
    updateVisualSettings({ textOverlays: [...(project.textOverlays ?? []), overlay] }, true);
    setSelectedOverlay({ type: 'text', id: overlay.id });
    setActiveWorkspacePanelTab('properties');
  };

  const hasSubtitleBlurRegion = () => currentProject()?.blurRegions.some((region) => region.role === 'subtitle') ?? false;

  const addSubtitleRegion = () => {
    const project = currentProject();
    if (!project?.source.videoPath) return;
    if (hasSubtitleBlurRegion()) {
      setNotice({ kind: 'info', message: 'Đã có vùng Blur/STTN mang vai trò Subtitle Region. Hãy chỉnh vị trí trong vùng đó; vùng subtitle riêng hiện không khả dụng.' });
      return;
    }
    setOcrRegionPicking(false);
    setOcrRegionEditing(false);
    setBlurRegionPicking(false);
    setSelectedOverlay({ type: 'subtitle' });
    setActiveWorkspacePanelTab('properties');
    setSubtitleRegionPicking(true);
    setNotice({ kind: 'info', message: 'Kéo trên video để chọn vùng hiển thị phụ đề. Nhấn Esc để hủy.' });
  };

  const saveSubtitleRegion = async (region: { x: number; y: number; width: number; height: number }) => {
    const project = currentProject();
    if (!project) return;
    setSubtitleRegionPicking(false);
    try {
      await flushVisualSettings(project.id);
      const latest = currentProject();
      if (!latest || latest.id !== project.id) return;
      const updated = await projectService.updateSubtitleSettings(project.id, { ...latest.subtitle, region });
      mergeVisualProject(updated);
      setSelectedOverlay({ type: 'subtitle' });
      setActiveWorkspacePanelTab('properties');
      setNotice({ kind: 'success', message: 'Đã lưu vùng hiển thị phụ đề cho dự án.' });
    } catch (error) {
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể lưu vùng subtitle.' });
    }
  };

  const totalDuration = () => Number.isFinite(duration()) && duration() > 0 ? duration() : 0;

  const saveOcrRegion = async (region: { x: number; y: number; width: number; height: number }) => {
    const project = currentProject();
    if (!project) return;
    try {
      const updated = await projectService.updateTranscriptSettings(project.id, {
        mode: project.transcriptSettings?.mode ?? 'speechRecognition',
        ocrFps: project.transcriptSettings?.ocrFps ?? 2,
        ocrRegion: region,
      });
      setCurrentProject(updated);
      setOcrRegionPicking(false);
      setBlurRegionPicking(false);
      setSubtitleRegionPicking(false);
      setOcrRegionEditing(true);
      setSelectedOverlay(null);
      setActiveWorkspacePanelTab('transcript');
      setNotice({ kind: 'success', message: 'Đã lưu vùng subtitle để OCR cho dự án.' });
    } catch (error) {
      setOcrRegionPicking(false);
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể lưu vùng OCR.' });
    }
  };

  const updateOcrRegion = async (region: { x: number; y: number; width: number; height: number }) => {
    await saveOcrRegion(region);
  };

  const deleteOcrRegion = async () => {
    const project = currentProject();
    if (!project?.transcriptSettings) return;
    try {
      const updated = await projectService.updateTranscriptSettings(project.id, {
        ...project.transcriptSettings,
        ocrRegion: null,
      });
      setCurrentProject(updated);
      setOcrRegionEditing(false);
      setNotice({ kind: 'success', message: 'Đã xóa vùng OCR của dự án.' });
    } catch (error) {
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể xóa vùng OCR.' });
    }
  };

  return (
    <section class="video-stage" aria-label="Khu vực xem trước video">
      <header class="video-stage-toolbar">
        <div class="stage-title"><span class="stage-live-dot" /> VIDEO PREVIEW</div>
        <div class="stage-toolbar-right">
          <Show when={currentProject()?.source.width && currentProject()?.source.height}>
            <span class="stage-resolution">{currentProject()?.source.width} × {currentProject()?.source.height}</span>
          </Show>
          <Show when={activeAiPreview()}>
            <button class="stage-tool-button ai-preview-return" type="button" title="Quay lại video nguồn" onClick={() => { const time = playerTime(); awaitingSourceSeek = true; setSeekRequest(time); setAiRemovalPreview(undefined); }}><Film size={13} /> Video nguồn</button>
          </Show>
          <button class="stage-tool-button" type="button" disabled={!currentProject()?.source.videoPath} title="Kéo để vẽ vùng blur trên video" onClick={addBlurRegion}>{blurRegionPicking() ? 'Đang vẽ…' : '＋ Blur'}</button>
          <button class="stage-tool-button" type="button" disabled={!currentProject()} title="Thêm ảnh logo" onClick={() => void addLogo()}>＋ Logo</button>
          <button class="stage-tool-button" type="button" disabled={!currentProject()?.source.videoPath} title="Thêm chữ lên video" onClick={addTextOverlay}>＋ Chữ</button>
          <button class="stage-tool-button" type="button" disabled={!currentProject()?.source.videoPath || hasSubtitleBlurRegion()} title={hasSubtitleBlurRegion() ? 'Đang có một vùng Blur/STTN mang vai trò Subtitle Region' : 'Kéo để chọn vùng hiển thị phụ đề trên video'} onClick={addSubtitleRegion}>{subtitleRegionPicking() ? 'Đang chọn vùng…' : '＋ Vùng subtitle'}</button>
          <button class="stage-tool-button" type="button" title="Chỉnh kiểu phụ đề" onClick={selectSubtitleProperties}>Subtitle</button>
          <Show when={currentProject()?.source.videoPath}>
            <button class="stage-replace-button" type="button" disabled={importing()} onClick={browse}><Plus size={13} /> Replace</button>
          </Show>
        </div>
      </header>
      <div class="preview-well">
        <div
          ref={previewFrame!}
          class="preview-frame"
          classList={{ 'preview-frame-has-video': !!activeVideoUrl(), 'preview-drop-active': dragging(), 'preview-frame-ai-removal': !!activeAiPreview() }}
          onClick={(event) => {
            const target = event.target;
            if (target instanceof Element && target.closest('.video-content-overlay, .overlay-object, .overlay-subtitle-object, .stage-tool-button, button, input')) return;
            if (selectedOverlay()) setSelectedOverlay(null);
          }}
        >
          <Show when={activeVideoUrl()} fallback={
            <div class="no-video-content">
              <div class="no-video-symbol"><Film size={27} strokeWidth={1.5} /><span /></div>
              <h2>{mediaBusy() ? 'Đang nhập media…' : 'Thêm nguồn vào timeline'}</h2>
              <p>Nhập video, ảnh và audio ở kho nguồn, rồi chọn “Thêm vào timeline” để bắt đầu.</p>
              <button class="button button-outline" type="button" disabled={mediaBusy() || importing()} onClick={() => { setActiveWorkspacePanelTab('media'); void browseMedia(); }}>
                <Plus size={15} /> Nhập media
              </button>
              <span class="video-coming-note">VIDEO · PNG/JPG/WEBP · AUDIO</span>
              <Show when={loadingVideo()}><span class="media-loading-label"><span class="spin status-spinner media-loading-spinner" aria-hidden="true" /> {mediaStatus() || 'Đang mở video…'}</span></Show>
            </div>
          }>
            <Dynamic
              component={nativePreview.preferred() ? 'audio' : 'video'}
              ref={(element: HTMLMediaElement) => {
                if (videoElement && videoElement !== element) {
                  videoElement.pause();
                }
                videoElement = element as HTMLVideoElement;
              }}
              class="workspace-video"
              style={{ display: nativePreview.preferred() ? 'none' : undefined }}
              src={activeVideoUrl()}
              crossOrigin="anonymous"
              muted
              preload="metadata"
              playsinline
              onLoadedMetadata={() => {
                const preview = activeAiPreview();
                if (!preview && Number.isFinite(videoElement.duration)) {
                  setDuration(nativePreview.preferred() ? (currentProject()?.source.duration ?? videoElement.duration) : videoElement.duration);
                }
                updateContentBounds();
                if (preview) {
                  videoElement.currentTime = 0;
                  setPlayerTime(preview.startSeconds);
                  setPlaybackTime(preview.startSeconds);
                  awaitingSourceSeek = false;
                } else if (seekRequest() !== null) {
                  const target = seekRequest()!;
                  videoElement.currentTime = Math.max(0, Math.min(target, videoElement.duration || target));
                  setSeekRequest(null);
                  awaitingSourceSeek = false;
                }
                syncPreviewSubtitle(globalTimeForPlayer(videoElement.currentTime));
                setLoadingVideo(false);
              }}
              onTimeUpdate={onTimeUpdate}
              onPlay={() => { if (!audioPreview.ready()) return; startPreviewSubtitleSync(); }}
              onPause={() => { stopPreviewSubtitleSync(); }}
              onSeeking={() => { if (!audioVisualSeek && !nativePreview.preferred()) void audioPreview.control('seek', globalTimeForPlayer(videoElement.currentTime)); }}
              onSeeked={() => { audioVisualSeek = false; }}
              onEnded={() => { if (!audioPreview.ready()) { setPlaying(false); setVideoPlaying(false); } }}
              onError={() => void handleMediaError()}
            />
            <OverlayCanvas
              interactionOnly={nativePreview.preferred()}
              onPreviewDraft={nativePreview.setDraft}
              onPreviewPick={nativePreview.setPick}
              left={contentBounds().left}
              top={contentBounds().top}
              width={contentBounds().width}
              height={contentBounds().height}
              videoWidth={videoElement?.videoWidth || currentProject()?.source.width || 1920}
              videoHeight={videoElement?.videoHeight || currentProject()?.source.height || 1080}
              previewSubtitleSegmentId={previewSubtitleSegmentId()}
              regionPicking={ocrRegionPicking()}
              blurRegionPicking={blurRegionPicking()}
              subtitleRegionPicking={subtitleRegionPicking()}
              ocrRegionEditing={ocrRegionEditing()}
              ocrRegion={currentProject()?.transcriptSettings?.mode === 'ocr' ? currentProject()?.transcriptSettings?.ocrRegion : null}
              subtitleRegion={currentProject()?.subtitle.region ?? null}
              onRegionSelected={(region, kind) => kind === 'ocr' ? void saveOcrRegion(region) : kind === 'blur' ? createBlurRegion(region) : void saveSubtitleRegion(region)}
              onOcrRegionChange={(region) => void updateOcrRegion(region)}
              onSubtitleRegionChange={(region) => void saveSubtitleRegion(region)}
              onOcrRegionDelete={() => void deleteOcrRegion()}
            />
            <Show when={!nativePreview.preferred() && !playing() && !mediaError()}>
              <button class="video-center-play" type="button" aria-label="Phát video" onClick={togglePlayback}><Play size={22} fill="currentColor" /></button>
            </Show>
          </Show>
          <Show when={dragging()}><div class="drop-overlay"><div><Film size={25} /><strong>Thả video, ảnh hoặc audio vào kho nguồn</strong></div></div></Show>
          <Show when={activeAiPreview()}><div class="ai-removal-preview-badge" role="status">Đoạn đã xử lý bằng AI · {formatTime(activeAiPreview()!.startSeconds)}–{formatTime(activeAiPreview()!.endSeconds)}</div></Show>
          <Show when={mediaStatus() && videoUrl()}><div class="media-status-overlay" role="status"><span class="spin status-spinner media-status-spinner" aria-hidden="true" /><span class="media-status-message">{mediaStatus()}</span></div></Show>
          <Show when={mediaError()}><div class="media-error-overlay" role="alert"><AlertTriangle size={17} /><span class="media-error-message">{mediaError()}</span><Show when={currentProject()?.source.videoPath}><button class="media-error-retry" type="button" onClick={retryPreview}>{usingProxy() ? 'Tạo lại preview' : 'Thử tạo preview'}</button><button class="media-error-retry" type="button" disabled={importing()} onClick={() => void restoreSource()}>Tìm lại video nguồn</button></Show></div></Show>
        </div>
      </div>
      <Show when={currentProject()?.source.videoPath && !audioPreview.ready() && !audioPreview.loading()}><button class="button button-quiet" type="button" onClick={audioPreview.retry}>Thử lại âm thanh</button></Show>
      <Show when={audioPreview.loading()}><span class="preview-compatibility-note" role="status">Đang chuẩn bị âm thanh…</span></Show>
      <Show when={audioPreview.buffering()}><span class="preview-compatibility-note" role="status">Đang đợi âm thanh…</span></Show>
      <Show when={nativePreview.warning()}><span class="preview-compatibility-note" role="status">{nativePreview.preferred() ? nativePreview.warning() : 'Đang dùng preview tương thích.'}</span></Show>
      <div class="playback-controls" aria-label="Điều khiển phát video">
        <button class="play-button" type="button" aria-label={playing() ? 'Tạm dừng video' : 'Phát video'} disabled={!videoUrl() || loadingVideo() || creatingProxy() || !audioPreview.ready()} onClick={togglePlayback}>
          {playing() ? <Pause size={16} fill="currentColor" /> : <Play size={16} fill="currentColor" />}
        </button>
        <span class="playback-time"><span>{formatTime(playerTime())}</span><span class="playback-time-divider">/</span><span>{formatTime(totalDuration())}</span></span>
        <input
          class="playback-seek"
          type="range"
          min="0"
          max={Math.max(totalDuration(), 0.01)}
          step="0.05"
          value={Math.min(playerTime(), totalDuration())}
          aria-label="Tìm vị trí video"
          disabled={!activeVideoUrl() || !totalDuration()}
          onInput={(event) => {
            const value = Number(event.currentTarget.value);
            setPlayerTime(value);
            setPlaybackTime(value);
            syncPreviewSubtitle(value);
            setSeekRequest(value);
          }}
        />
        <button class="playback-volume" type="button" aria-label={muted() ? 'Bật âm thanh gốc' : 'Tắt âm thanh gốc'} disabled={!videoUrl()} onClick={() => updateOriginalMute(!muted())}>
              {muted() || volume() === 0 ? <VolumeX size={16} /> : volume() < 0.45 ? <Volume1 size={16} /> : <Volume2 size={16} />}
            </button>
        <input class="playback-volume-slider" type="range" min="0" max="1" step="0.01" value={volume()} aria-label="Âm lượng nghe thử" disabled={!videoUrl()} onInput={(event) => { setVolume(Number(event.currentTarget.value)); }} />
        <button class="playback-expand" type="button" aria-label="Toàn màn hình" disabled={!videoUrl()} onClick={toggleFullscreen}><Maximize2 size={15} /></button>
      </div>
      <div class="preview-footnote"><span><span class="footnote-dot" /> {activeAiPreview() ? 'AI REMOVAL PREVIEW' : usingProxy() ? 'PREVIEW TƯƠNG THÍCH' : 'TỶ LỆ GỐC'}</span><span>{currentProject()?.source.videoPath ? currentProject()?.source.originalName : 'Kéo thả video vào khu vực xem trước'}</span></div>

      <Show when={replacePath()}>
        <Modal title="Thay video nguồn?" description="Transcript và dữ liệu phụ thuộc video hiện tại sẽ được xóa sau khi video mới nhập thành công." onClose={() => setReplacePath(undefined)} danger>
          <div class="delete-confirmation">
            <div class="delete-warning-icon"><AlertTriangle size={20} /></div>
            <div class="delete-copy"><p class="delete-project-name">{currentProject()?.source.originalName ?? 'Video hiện tại'}</p><p>Video mới sẽ được sao chép vào dự án. Bản gốc trên máy của bạn không bị xóa.</p><p class="delete-irreversible">Transcript hiện tại sẽ không còn gắn với video mới.</p></div>
          </div>
          <footer class="modal-actions modal-actions-delete">
            <button class="button button-quiet" type="button" onClick={() => setReplacePath(undefined)}>Hủy</button>
            <button class="button button-danger" type="button" disabled={importing()} onClick={() => void importVideo(replacePath()!, true)}>
              <Show when={importing()}><span class="spin status-spinner" /></Show>{importing() ? 'Đang nhập…' : 'Thay video'}
            </button>
          </footer>
        </Modal>
      </Show>
    </section>
  );
}
