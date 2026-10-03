import { Dynamic } from 'solid-js/web';
import { createNativePreview } from './native-preview';
import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { open } from '@tauri-apps/plugin-dialog';
import { createEffect, createMemo, createSignal, on, onCleanup, onMount, Show } from 'solid-js';
import {
  AlertTriangle, Film, Maximize2, Pause, Play, Plus, Volume1, Volume2, VolumeX,
} from 'lucide-solid';
import { aiRemovalPreview, blurRegionPicking, currentProject, currentTranscript, ocrRegionEditing, ocrRegionPicking, playbackTime, seekRequest, segmentVoicePreviewActive, setActiveWorkspacePanelTab, setAiRemovalPreview, setBlurRegionPicking, setCurrentProject, setCurrentTranscript, setNotice, setOcrRegionEditing, setOcrRegionPicking, setPlaybackTime, setSeekRequest, setSelectedOverlay, setSegmentVoicePreviewActive, setSubtitleRegionPicking, setVideoPlaying, selectedOverlay, subtitleRegionPicking, videoPlaying } from '../../../app/project-state';
import { projectService } from '../../../app/services/project-service';
import { ttsService } from '../../../app/services/tts-service';
import { dialogueService } from '../../../app/services/dialogue-service';
import { findPlayableDubbingSegmentAtTime, findTranslatedSegmentAtTime, ttsWindowEnd } from '../../transcript/transcript-utils';
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

function isVideoPath(path: string) {
  return /\.(mp4|mov|mkv|webm)$/i.test(path);
}

export default function TranslationView() {
  let videoElement!: HTMLVideoElement;
  let previewFrame!: HTMLDivElement;
  let dubbingAudio: HTMLAudioElement | undefined;
  let dialogueAudio: HTMLAudioElement | undefined;
  let audioContext: AudioContext | undefined;
  let originalGain: GainNode | undefined;
  let dubbingGain: GainNode | undefined;
  let dialogueGain: GainNode | undefined;
  let activeDubId: string | undefined;
  let finishedDubId: string | undefined;
  let lastOverflowNoticeId: string | undefined;
  let dubLookupToken = 0;
  let dialogueStemLookupToken = 0;
  const [dialogueStemPath, setDialogueStemPath] = createSignal('');
  const [dialogueStemLoading, setDialogueStemLoading] = createSignal(false);
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
  const [volume, setVolume] = createSignal(0.82);
  const [playerTime, setPlayerTime] = createSignal(0);
  const [previewSubtitleSegmentId, setPreviewSubtitleSegmentId] = createSignal<string>();
  const [duration, setDuration] = createSignal(0);
  const [contentBounds, setContentBounds] = createSignal({ left: 0, top: 0, width: 0, height: 0 });
  const [activeDubbingSegment, setActiveDubbingSegment] = createSignal<string>();
  let removeDropListener: (() => void) | undefined;
  let componentAlive = true;
  let previewAttemptedForSource: string | undefined;
  const nativePreview = createNativePreview(() => previewFrame, () => videoElement, (time, isPlaying) => {
    setPlayerTime(time); setPlaybackTime(time); setPlaying(isPlaying); setVideoPlaying(isPlaying);
  });
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
  const activeAiPreview = () => aiRemovalPreview()?.projectId === currentProject()?.id ? aiRemovalPreview() : undefined;
  const activeVideoUrl = createMemo(() => {
    const preview = activeAiPreview();
    return preview ? convertFileSrc(preview.path) : videoUrl();
  });
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

  const setPreviewGain = (node: GainNode | undefined, value: number) => {
    if (!node || !audioContext) return;
    const now = audioContext.currentTime;
    const target = Math.max(-1, Math.min(1, value));
    node.gain.cancelScheduledValues(now);
    if (Math.abs(target) < 0.000001) node.gain.setValueAtTime(0, now);
    else node.gain.setTargetAtTime(target, now, 0.025);
  };

  createEffect(() => {
    const project = currentProject();
    const sourceVolume = volume() * (project?.dubbing.originalVolume ?? 1);
    const ducking = project?.dubbing.duckingEnabled && activeDubbingSegment() ? 1 - (project.dubbing.duckingAmount ?? 0.75) : 1;
    const originalLevel = muted() ? 0 : Math.max(0, Math.min(1, sourceVolume * ducking));
    if (originalGain) {
      setPreviewGain(originalGain, originalLevel);
      if (videoElement) { videoElement.volume = 1; videoElement.muted = false; }
    } else if (videoElement) {
      videoElement.volume = originalLevel;
      videoElement.muted = muted();
    }
    if (dubbingGain) setPreviewGain(dubbingGain, project?.dubbing.enabled ? Math.max(0, Math.min(1, project.dubbing.dubbedVolume ?? 1)) : 0);
    if (dialogueGain) {
      const reduction = project?.dubbing.dialogueReductionEnabled && dialogueStemPath()
        ? Math.max(0, Math.min(1, project.dubbing.dialogueReductionAmount ?? 0.65))
        : 0;
      setPreviewGain(dialogueGain, -originalLevel * reduction);
    }
  });

  onMount(() => {
    const nativeDrop = (event: Event) => {
      const paths = (event as CustomEvent<string[]>).detail;
      const path = paths.find(isVideoPath);
      if (path) void selectVideo(path);
      else setNotice({ kind: 'error', message: 'Hãy thả một tệp MP4, MOV, MKV hoặc WEBM.' });
    };
    window.addEventListener('native-preview-drop', nativeDrop);
    onCleanup(() => window.removeEventListener('native-preview-drop', nativeDrop));
    frameResizeObserver = new ResizeObserver(updateContentBounds);
    if (previewFrame) frameResizeObserver.observe(previewFrame);
    requestAnimationFrame(updateContentBounds);
    onCleanup(() => frameResizeObserver?.disconnect());
    if (videoElement) {
      videoElement.volume = volume() * (currentProject()?.dubbing.originalVolume ?? 1);
      videoElement.muted = muted();
    }
    void getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === 'over') setDragging(true);
      else if (event.payload.type === 'leave') setDragging(false);
      else if (event.payload.type === 'drop') {
        setDragging(false);
        const filePath = event.payload.paths.find(isVideoPath);
        if (!filePath) {
          setNotice({ kind: 'error', message: 'Hãy thả một tệp MP4, MOV, MKV hoặc WEBM.' });
          return;
        }
        void selectVideo(filePath);
      }
    }).then((unlisten) => {
      if (componentAlive) removeDropListener = unlisten;
      else unlisten();
    }).catch(() => setNotice({ kind: 'error', message: 'Không thể bật kéo thả tệp video.' }));
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

  const ensureAudioGraph = () => {
    if (!videoElement || audioContext) return;
    try {
      audioContext = new AudioContext();
      const videoSource = audioContext.createMediaElementSource(videoElement);
      originalGain = audioContext.createGain();
      videoSource.connect(originalGain).connect(audioContext.destination);
      dubbingAudio = new Audio();
      dubbingAudio.crossOrigin = 'anonymous';
      dubbingAudio.preload = 'auto';
      const dubSource = audioContext.createMediaElementSource(dubbingAudio);
      dubbingGain = audioContext.createGain();
      dubSource.connect(dubbingGain).connect(audioContext.destination);
      dialogueAudio = new Audio();
      dialogueAudio.crossOrigin = 'anonymous';
      dialogueAudio.preload = 'auto';
      const dialogueSource = audioContext.createMediaElementSource(dialogueAudio);
      dialogueGain = audioContext.createGain();
      dialogueSource.connect(dialogueGain).connect(audioContext.destination);
      originalGain.gain.value = muted() ? 0 : volume() * (currentProject()?.dubbing.originalVolume ?? 1);
      dubbingGain.gain.value = currentProject()?.dubbing.enabled ? currentProject()?.dubbing.dubbedVolume ?? 1 : 0;
      dialogueGain.gain.value = 0;
      void audioContext.resume();
      videoElement.volume = 1;
      videoElement.muted = false;
    } catch {
      audioContext = undefined;
      originalGain = undefined;
      dubbingGain = undefined;
      dialogueGain = undefined;
      dialogueAudio = undefined;
      setNotice({ kind: 'error', message: 'Web Audio không khả dụng trong WebView; lồng tiếng preview chưa thể phát.' });
    }
  };

  const clearDialogueAudio = () => {
    if (!dialogueAudio) return;
    dialogueAudio.pause();
    dialogueAudio.onloadedmetadata = null;
    dialogueAudio.onerror = null;
    delete dialogueAudio.dataset.stemPath;
    if (dialogueAudio.hasAttribute('src')) {
      dialogueAudio.removeAttribute('src');
      dialogueAudio.load();
    }
  };

  const dialogueStemKey = createMemo(() => {
    const project = currentProject();
    return [project?.id ?? '', project?.source.videoPath ?? '', project?.dubbing.dialogueReductionEnabled ? '1' : '0'].join('\0');
  });

  createEffect(() => {
    if (dialogueStemLoading()) setMediaStatus('Đang chuẩn bị BandIt preview…');
    else if (mediaStatus() === 'Đang chuẩn bị BandIt preview…') setMediaStatus('');
  });
  createEffect(on(dialogueStemKey, () => {
    const project = currentProject();
    const projectId = project?.id;
    const enabled = project?.dubbing.dialogueReductionEnabled ?? false;
    dialogueStemLookupToken += 1;
    const token = dialogueStemLookupToken;
    setDialogueStemPath('');
    setDialogueStemLoading(false);
    clearDialogueAudio();
    if (!projectId || !project?.source.videoPath || !enabled) return;
    setDialogueStemLoading(true);
    void dialogueService.preparePreviewStem(projectId).then((path) => {
      if (token !== dialogueStemLookupToken || currentProject()?.id !== projectId) return;
      setDialogueStemPath(path);
    }).catch((error: unknown) => {
      if (token !== dialogueStemLookupToken) return;
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể chuẩn bị BandIt preview.' });
    }).finally(() => {
      if (token === dialogueStemLookupToken) setDialogueStemLoading(false);
    });
  }));

  createEffect(() => {
    const project = currentProject();
    const path = dialogueStemPath();
    const enabled = project?.dubbing.dialogueReductionEnabled ?? false;
    const time = playbackTime();
    const playingNow = videoPlaying();
    const voicePreviewActive = segmentVoicePreviewActive();
    if (!enabled || !path || !videoElement) {
      if (dialogueAudio && !dialogueAudio.paused) dialogueAudio.pause();
      return;
    }
    if (playingNow) ensureAudioGraph();
    if (!dialogueAudio || !audioContext) return;
    if (dialogueAudio.dataset.stemPath !== path) {
      dialogueAudio.dataset.stemPath = path;
      dialogueAudio.onloadedmetadata = () => {
        if (!dialogueAudio || dialogueAudio.dataset.stemPath !== path) return;
        dialogueAudio.currentTime = Math.max(0, Math.min(videoElement.currentTime, dialogueAudio.duration || videoElement.currentTime));
        if (videoPlaying() && !segmentVoicePreviewActive()) {
          void audioContext?.resume().then(() => dialogueAudio?.play()).catch(() => undefined);
        }
      };
      dialogueAudio.onerror = () => setNotice({ kind: 'error', message: 'Không thể đọc stem thoại BandIt để preview.' });
      dialogueAudio.src = convertFileSrc(path);
      dialogueAudio.load();
    }
    if (voicePreviewActive) {
      if (!dialogueAudio.paused) dialogueAudio.pause();
      return;
    }
    if (dialogueAudio.readyState >= HTMLMediaElement.HAVE_METADATA) {
      if (Math.abs(dialogueAudio.currentTime - time) > 0.12) dialogueAudio.currentTime = Math.max(0, Math.min(time, dialogueAudio.duration || time));
      dialogueAudio.playbackRate = videoElement.playbackRate || 1;
      if (playingNow && dialogueAudio.paused) void audioContext.resume().then(() => dialogueAudio?.play()).catch(() => undefined);
      if (!playingNow && !dialogueAudio.paused) dialogueAudio.pause();
    }
  });
  const clearDubbingAudio = () => {
    dubLookupToken += 1;
    if (!dubbingAudio) return;
    dubbingAudio.pause();
    dubbingAudio.onended = null;
    dubbingAudio.onerror = null;
    dubbingAudio.onloadedmetadata = null;
    if (dubbingAudio.hasAttribute('src')) {
      dubbingAudio.removeAttribute('src');
      dubbingAudio.load();
    }
  };

  createEffect(() => {
    const project = currentProject();
    const projectId = project?.id;
    const enabled = project?.dubbing.enabled ?? false;
    const time = playbackTime();
    const playingNow = videoPlaying();
    const voicePreviewActive = segmentVoicePreviewActive();
    const segments = currentTranscript()?.segments ?? [];
    if (!videoElement || !projectId) return;
    if (voicePreviewActive) {
      if (activeDubId || dubbingAudio?.hasAttribute('src')) clearDubbingAudio();
      if (!videoElement.paused) videoElement.pause();
      activeDubId = undefined;
      finishedDubId = undefined;
      setActiveDubbingSegment(undefined);
      return;
    }
    if (playingNow) ensureAudioGraph();
    const overflow = enabled ? segments.find((item) => item.tts?.status === 'durationOverflow' && time >= item.start && time < ttsWindowEnd(segments, item)) : undefined;
    if (overflow?.id && overflow.id !== lastOverflowNoticeId) {
      setNotice({ kind: 'info', message: 'Dubbing preview bỏ qua audio vượt thời lượng để tránh chồng hoặc cắt tiếng. Nghe thử segment riêng hoặc chạy Fit Audio.' });
      lastOverflowNoticeId = overflow.id;
    } else if (!overflow) {
      lastOverflowNoticeId = undefined;
    }
    const segment = findPlayableDubbingSegmentAtTime(segments, time, enabled, finishedDubId);
    const segmentId = segment?.id;
    if (!segment || !dubbingAudio || !audioContext) {
      if (activeDubId || dubbingAudio?.hasAttribute('src')) clearDubbingAudio();
      activeDubId = undefined;
      setActiveDubbingSegment(undefined);
      return;
    }
    if (activeDubId !== segmentId) {
      clearDubbingAudio();
      activeDubId = segmentId;
      const token = dubLookupToken;
      setActiveDubbingSegment(segmentId);
      void ttsService.getAudioPath(projectId, segmentId!).then((path) => {
        if (token !== dubLookupToken || !dubbingAudio || activeDubId !== segmentId) return;
        dubbingAudio.onended = () => { if (token === dubLookupToken) finishedDubId = segmentId; };
        dubbingAudio.onerror = () => {
          if (token !== dubLookupToken || activeDubId !== segmentId) return;
          finishedDubId = segmentId;
          setNotice({ kind: 'error', message: 'Không thể đọc hoặc giải mã file audio lồng tiếng của segment này.' });
        };
        dubbingAudio.onloadedmetadata = () => {
          if (token !== dubLookupToken || !dubbingAudio) return;
          const elapsed = Math.max(0, playbackTime() - segment!.start);
          if (elapsed >= dubbingAudio.duration) {
            finishedDubId = segmentId;
            return;
          }
          // Loading a local WAV can still take a few frames. Seeking by that tiny
          // delay chops Vietnamese initial consonants (e.g. "Ma" -> "a").
          // Preserve the attack for normal segment entry; only seek when the user
          // actually starts well inside the segment.
          dubbingAudio.currentTime = elapsed <= 0.25 ? 0 : elapsed;
          if (videoPlaying()) void dubbingAudio.play().catch(() => setNotice({ kind: 'error', message: 'Không thể phát audio lồng tiếng trên preview.' }));
        };
        dubbingAudio.src = convertFileSrc(path);
        dubbingAudio.load();
      }).catch((error: unknown) => {
        if (token === dubLookupToken && activeDubId === segmentId) {
          finishedDubId = segmentId;
          setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể mở audio lồng tiếng.' });
        }
      });
    } else if (dubbingAudio.src) {
      const offset = Math.max(0, time - segment.start);
      if (Math.abs(dubbingAudio.currentTime - offset) > 0.32) dubbingAudio.currentTime = offset;
      dubbingAudio.playbackRate = videoElement.playbackRate || 1;
      if (playingNow && dubbingAudio.paused) void audioContext.resume().then(() => dubbingAudio?.play()).catch(() => undefined);
      if (!playingNow && !dubbingAudio.paused) dubbingAudio.pause();
    }
  });

  onCleanup(() => {
    componentAlive = false;
    setOcrRegionPicking(false);
    setBlurRegionPicking(false);
    setOcrRegionEditing(false);
    stopPreviewSubtitleSync();
    removeDropListener?.();
    setVideoPlaying(false);
    dialogueStemLookupToken += 1;
    clearDubbingAudio();
    clearDialogueAudio();
    void audioContext?.close();
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

  const importVideo = async (path: string, replace: boolean) => {
    const project = currentProject();
    if (!project || importing()) return;
    videoElement?.pause(); nativePreview.control('pause');
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
    if (!videoElement || !videoUrl()) return;
    if (nativePreview.preferred() && !(currentProject()?.source.audioStreams?.length)) {
      if (segmentVoicePreviewActive()) setSegmentVoicePreviewActive(false);
      ensureAudioGraph(); await audioContext?.resume();
      nativePreview.control(playing() ? 'pause' : 'play');
      setPlaying(!playing()); setVideoPlaying(playing());
      return;
    }
    if (videoElement.paused) {
      if (segmentVoicePreviewActive()) setSegmentVoicePreviewActive(false);
      // Create and unlock the Web Audio graph while the Play click still carries
      // user activation. Creating it from a later onPlay effect can leave the
      // preview graph suspended, making gain controls appear ineffective.
      ensureAudioGraph();
      try {
        if (audioContext?.state === 'suspended') {
          try {
            await audioContext.resume();
          } catch {
            setNotice({ kind: 'error', message: 'Không thể kích hoạt bộ trộn âm thanh preview của WebView.' });
            return;
          }
        }
      } catch {
        setMediaError('WebView không thể phát định dạng video này.');
      }
      try {
        await videoElement.play();
      } catch {
        setMediaError('WebView không thể phát định dạng video này.');
      }
    }
    else videoElement.pause();
  };

  const toggleFullscreen = () => {
    const frame = previewFrame;
    if (!frame) return;
    if (document.fullscreenElement) void document.exitFullscreen();
    else void frame.requestFullscreen().catch(() => setNotice({ kind: 'error', message: 'Không thể mở video toàn màn hình.' }));
  };

  const handleMediaError = async () => {
    if (nativePreview.preferred()) {
      if (!currentProject()?.source.audioStreams?.length) return;
      const p = currentProject()!;
      try { const path = await invoke<string>('preview_audio_path', { projectId: p.id }); if (currentProject()?.id === p.id) setVideoUrl(convertFileSrc(path)); }
      catch { setMediaError('Không đọc được âm thanh nguồn. Hãy thử preview tương thích.'); }
      return;
    }
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
              <h2>{importing() ? 'Đang nhập video…' : 'Thêm video của bạn'}</h2>
              <p>Kéo video vào đây hoặc chọn tệp từ máy tính. Video sẽ được sao chép vào dự án.</p>
              <button class="button button-outline" type="button" disabled={importing()} onClick={browse}>
                <Plus size={15} /> Chọn video
              </button>
              <span class="video-coming-note">MP4 · MOV · MKV · WEBM</span>
              <Show when={loadingVideo()}><span class="media-loading-label"><span class="spin status-spinner media-loading-spinner" aria-hidden="true" /> {mediaStatus() || 'Đang mở video…'}</span></Show>
            </div>
          }>
            <Dynamic
              component={nativePreview.preferred() ? 'audio' : 'video'}
              ref={(element: HTMLMediaElement) => {
                if (videoElement && videoElement !== element) {
                  videoElement.pause(); clearDubbingAudio(); clearDialogueAudio(); void audioContext?.close();
                  audioContext = undefined; originalGain = undefined; dubbingGain = undefined; dialogueGain = undefined;
                  dubbingAudio = undefined; dialogueAudio = undefined;
                }
                videoElement = element as HTMLVideoElement;
              }}
              class="workspace-video"
              style={{ display: nativePreview.preferred() ? 'none' : undefined }}
              src={activeVideoUrl()}
              crossOrigin="anonymous"
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
              onPlay={() => { nativePreview.control('play'); setPlaying(true); setVideoPlaying(true); startPreviewSubtitleSync(); }}
              onPause={() => { nativePreview.control('pause'); setPlaying(false); setVideoPlaying(false); stopPreviewSubtitleSync(); syncPreviewSubtitle(globalTimeForPlayer(videoElement.currentTime)); dubbingAudio?.pause(); }}
              onSeeking={() => { syncPreviewSubtitle(globalTimeForPlayer(videoElement.currentTime)); clearDubbingAudio(); activeDubId = undefined; finishedDubId = undefined; setActiveDubbingSegment(undefined); }}
              onSeeked={() => { if (nativePreview.preferred()) nativePreview.control('seek', globalTimeForPlayer(videoElement.currentTime)); onTimeUpdate(); }}
              onEnded={() => { nativePreview.control('pause'); setPlaying(false); setVideoPlaying(false); stopPreviewSubtitleSync(); syncPreviewSubtitle(globalTimeForPlayer(videoElement.currentTime)); clearDubbingAudio(); activeDubId = undefined; finishedDubId = undefined; }}
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
          <Show when={dragging()}><div class="drop-overlay"><div><Film size={25} /><strong>Thả video để nhập vào dự án</strong></div></div></Show>
          <Show when={activeAiPreview()}><div class="ai-removal-preview-badge" role="status">Đoạn đã xử lý bằng AI · {formatTime(activeAiPreview()!.startSeconds)}–{formatTime(activeAiPreview()!.endSeconds)}</div></Show>
          <Show when={mediaStatus() && videoUrl()}><div class="media-status-overlay" role="status"><span class="spin status-spinner media-status-spinner" aria-hidden="true" /><span class="media-status-message">{mediaStatus()}</span></div></Show>
          <Show when={mediaError()}><div class="media-error-overlay" role="alert"><AlertTriangle size={17} /><span class="media-error-message">{mediaError()}</span><Show when={currentProject()?.source.videoPath}><button class="media-error-retry" type="button" onClick={retryPreview}>{usingProxy() ? 'Tạo lại preview' : 'Thử tạo preview'}</button></Show></div></Show>
        </div>
      </div>
      <Show when={nativePreview.warning()}><span class="preview-compatibility-note" role="status">{nativePreview.preferred() ? nativePreview.warning() : 'Đang dùng preview tương thích.'}</span></Show>
      <div class="playback-controls" aria-label="Điều khiển phát video">
        <button class="play-button" type="button" aria-label={playing() ? 'Tạm dừng video' : 'Phát video'} disabled={!videoUrl() || loadingVideo() || creatingProxy()} onClick={togglePlayback}>
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
        <button class="playback-volume" type="button" aria-label={muted() ? 'Bật âm thanh' : 'Tắt âm thanh'} disabled={!videoUrl()} onClick={() => updateOriginalMute(!muted())}>
              {muted() || volume() === 0 ? <VolumeX size={16} /> : volume() < 0.45 ? <Volume1 size={16} /> : <Volume2 size={16} />}
            </button>
        <input class="playback-volume-slider" type="range" min="0" max="1" step="0.01" value={muted() ? 0 : volume()} aria-label="Âm lượng video" disabled={!videoUrl()} onInput={(event) => { setVolume(Number(event.currentTarget.value)); if (muted()) updateOriginalMute(false); }} />
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
