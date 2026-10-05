import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { convertFileSrc } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import {
  ArrowDown, ArrowUp, Check, CircleAlert, Download, FileVideo,
  FolderOpen, GripVertical, ImagePlus, LoaderCircle, Merge, RefreshCw, ShieldCheck,
  X,
} from 'lucide-solid';
import { currentProject, setNotice } from '../../app/project-state';
import { jobService } from '../../app/services/job-service';
import { projectService } from '../../app/services/project-service';
import VideoEncoderSelect from '../../shared/components/VideoEncoderSelect';
import { videoToolsService, type MergeEncoder, type MergeFrameMode, type MergeLogoSettings, type MergeTextSettings, type MergeVideoCodec, type MergeVideoInfo, type TikTokCookieInfo, type TikTokDramaInfo, type VideoToolProgress } from '../../app/services/video-tools-service';
import { mergeVisualProject } from '../overlays/overlay-state';

type VideoToolsProps = { workflow: 'tiktok' | 'merge' };

function formatTime(value: number) {
  const seconds = Math.max(0, Math.floor(value));
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const remainder = seconds % 60;
  return hours > 0
    ? `${String(hours).padStart(2, '0')}:${String(minutes).padStart(2, '0')}:${String(remainder).padStart(2, '0')}`
    : `${String(minutes).padStart(2, '0')}:${String(remainder).padStart(2, '0')}`;
}

function formatBytes(bytes: number) {
  if (!Number.isFinite(bytes) || bytes <= 0) return '—';
  const units = ['B', 'KB', 'MB', 'GB'];
  let value = bytes;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) { value /= 1024; index += 1; }
  return `${value.toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
}

function directoryLabel(path: string) {
  const parts = path.replace(/\\/g, '/').split('/').filter(Boolean);
  return parts.length > 2 ? `…/${parts.slice(-2).join('/')}` : path;
}

export default function VideoToolsView(props: VideoToolsProps) {
  const [tiktokOutput, setTiktokOutput] = createSignal('');
  const [mergeOutput, setMergeOutput] = createSignal('');
  const [mergeFrameMode, setMergeFrameMode] = createSignal<MergeFrameMode>('source');
  const [mergeVideoCodec, setMergeVideoCodec] = createSignal<MergeVideoCodec>('h264');
  const [mergeEncoder, setMergeEncoder] = createSignal<MergeEncoder>('auto');
  const [mergeVideoPreviewUrl, setMergeVideoPreviewUrl] = createSignal('');
  const [mergeLogoEnabled, setMergeLogoEnabled] = createSignal(false);
  const [mergeLogoId, setMergeLogoId] = createSignal('');
  const [mergeLogo, setMergeLogo] = createSignal<MergeLogoSettings | null>(null);
  const [mergeLogoUrl, setMergeLogoUrl] = createSignal('');
  const [mergeLogoImageAspect, setMergeLogoImageAspect] = createSignal(0);
  const [mergeLogoDragging, setMergeLogoDragging] = createSignal(false);
  const [mergeLogoError, setMergeLogoError] = createSignal('');
  const [mergeTextEnabled, setMergeTextEnabled] = createSignal(false);
  const [mergeText, setMergeText] = createSignal<MergeTextSettings>({ text: 'Nhập văn bản', x: 0.15, y: 0.4, width: 0.7, height: 0.2, opacity: 1, fontFamily: 'Arial', fontSize: 48, fontWeight: 700, color: '#FFFFFF', alignment: 'center' });
  const [mergeTextDragging, setMergeTextDragging] = createSignal(false);
  const [tiktokUrl, setTiktokUrl] = createSignal('');
  const [cookieFile, setCookieFile] = createSignal('');
  const [cookieInfo, setCookieInfo] = createSignal<TikTokCookieInfo>();
  const [cookieError, setCookieError] = createSignal('');
  const [cookieChecking, setCookieChecking] = createSignal(false);
  const [browserSessionReady, setBrowserSessionReady] = createSignal(false);
  const [cookieLoginStarting, setCookieLoginStarting] = createSignal(false);
  const [cookieLoginJobId, setCookieLoginJobId] = createSignal<string>();
  const [cookieLoginProgress, setCookieLoginProgress] = createSignal<VideoToolProgress>();
  const [tiktokStarting, setTiktokStarting] = createSignal(false);
  const [tiktokJobId, setTiktokJobId] = createSignal<string>();
  const [tiktokProgress, setTiktokProgress] = createSignal<VideoToolProgress>();
  const [dramaInfo, setDramaInfo] = createSignal<TikTokDramaInfo>();
  const [tiktokError, setTiktokError] = createSignal('');

  const [sourceDirectory, setSourceDirectory] = createSignal('');
  const [videos, setVideos] = createSignal<MergeVideoInfo[]>([]);
  const [skippedCount, setSkippedCount] = createSignal(0);
  const [scanError, setScanError] = createSignal('');
  const [scanning, setScanning] = createSignal(false);
  const [mergeStarting, setMergeStarting] = createSignal(false);
  const [mergeJobId, setMergeJobId] = createSignal<string>();
  const [mergeProgress, setMergeProgress] = createSignal<VideoToolProgress>();
  const [mergeError, setMergeError] = createSignal('');
  const [draggedPath, setDraggedPath] = createSignal<string>();

  let mergePreviewStage: HTMLDivElement | undefined;
  let mergeLogoDrag: { pointerId: number; mode: 'move' | 'resize'; startX: number; startY: number; stageWidth: number; stageHeight: number; initial: MergeLogoSettings } | undefined;
  let mergeTextDrag: { pointerId: number; mode: 'move' | 'resize'; startX: number; startY: number; stageWidth: number; stageHeight: number; initial: MergeTextSettings } | undefined;
  let unlisten: (() => void) | undefined;
  let folderGeneration = 0;
  const projectId = () => currentProject()?.id;
  const mergeCanvasAspect = createMemo(() => {
    if (mergeFrameMode() === 'portrait') return 9 / 16;
    if (mergeFrameMode() === 'landscape') return 16 / 9;
    const reference = videos().reduce<MergeVideoInfo | undefined>((best, video) => !best || video.width * video.height > best.width * best.height ? video : best, undefined);
    return reference && reference.height > 0 ? reference.width / reference.height : 16 / 9;
  });
  const mergeCanvasWidth = createMemo(() => {
    const reference = videos().reduce<MergeVideoInfo | undefined>((best, video) => !best || video.width * video.height > best.width * best.height ? video : best, undefined);
    if (!reference) return 1920;
    if (mergeFrameMode() === 'source') return reference.width + reference.width % 2;
    const longest = Math.max(...videos().map((video) => Math.max(video.width, video.height)));
    const longEdge = longest + longest % 2;
    if (mergeFrameMode() === 'landscape') return longEdge;
    return Math.max(2, Math.floor(Math.round(longEdge * 9 / 16) / 2) * 2);
  });
  createEffect(() => {
    const path = videos()[0]?.path;
    setMergeVideoPreviewUrl(path ? convertFileSrc(path) : '');
  });
  createEffect(() => {
    const logo = mergeLogo();
    const imageAspect = mergeLogoImageAspect();
    const canvasAspect = mergeCanvasAspect();
    if (!logo?.maintainAspectRatio || imageAspect <= 0 || canvasAspect <= 0) return;
    const height = Math.min(1 - logo.y, logo.width * canvasAspect / imageAspect);
    if (Math.abs(height - logo.height) > 0.001) setMergeLogo({ ...logo, height });
  });
  const taskRunning = () => tiktokStarting() || cookieLoginStarting() || mergeStarting() || ['queued', 'waiting', 'running'].includes(tiktokProgress()?.status ?? '') || ['queued', 'waiting', 'running'].includes(mergeProgress()?.status ?? '');

  createEffect(() => {
    const id = projectId();
    if (!id) return;
    const generation = ++folderGeneration;
    void Promise.all([
      videoToolsService.outputDirectory(id, 'tiktok'),
      videoToolsService.outputDirectory(id, 'merge'),
      videoToolsService.hasTikTokBrowserSession(),
    ]).then(([tiktok, merge, hasSession]) => {
      if (generation !== folderGeneration) return;
      setTiktokOutput(tiktok);
      setMergeOutput(merge);
      setBrowserSessionReady(hasSession);
    }).catch((error: unknown) => {
      if (generation === folderGeneration) setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không đọc được thư mục lưu trong dự án.' });
    });
    onCleanup(() => { folderGeneration += 1; });
  });

  onMount(() => {
    void videoToolsService.onProgress((event) => {
      if (event.projectId !== projectId()) return;
      if (cookieLoginStarting() && (!cookieLoginJobId() || cookieLoginJobId() === event.jobId)) {
        setCookieLoginJobId(event.jobId);
        setCookieLoginProgress(event);
        if (event.status === 'completed') {
          setBrowserSessionReady(true);
          setCookieError('');
        }
        if (event.status === 'failed' || event.status === 'cancelled') {
          setCookieError(event.message);
        }
        if (['completed', 'failed', 'cancelled'].includes(event.status)) setCookieLoginStarting(false);
      }
      if (tiktokStarting() && (!tiktokJobId() || tiktokJobId() === event.jobId)) {
        setTiktokJobId(event.jobId);
        setTiktokProgress(event);
        if (event.details) setDramaInfo(event.details);
        if (event.status === 'failed' || event.status === 'cancelled') setTiktokError(event.message);
        if (event.status === 'completed') setTiktokError('');
        if (['completed', 'failed', 'cancelled'].includes(event.status)) setTiktokStarting(false);
      }
      if (mergeStarting() && (!mergeJobId() || mergeJobId() === event.jobId)) {
        setMergeJobId(event.jobId);
        setMergeProgress(event);
        if (event.status === 'failed' || event.status === 'cancelled') setMergeError(event.message);
        if (['completed', 'failed', 'cancelled'].includes(event.status)) setMergeStarting(false);
      }
    }).then((dispose) => { unlisten = dispose; });
  });

  onCleanup(() => unlisten?.());

  const chooseDirectory = async (purpose: 'tiktok' | 'merge') => {
    try {
      const selected = await open({ directory: true, multiple: false, title: purpose === 'tiktok' ? 'Chọn thư mục lưu series' : 'Chọn thư mục lưu video gộp' });
      if (typeof selected !== 'string') return;
      if (purpose === 'tiktok') setTiktokOutput(selected);
      else setMergeOutput(selected);
    } catch (error) { setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không mở được hộp thoại thư mục.' }); }
  };

  const chooseCookieFile = async () => {
    try {
      const selected = await open({ directory: false, multiple: false, title: 'Chọn Netscape cookies.txt', filters: [{ name: 'Netscape cookies', extensions: ['txt', 'cookies'] }] });
      if (typeof selected === 'string') {
        setCookieFile(selected);
        void inspectCookieFile(selected);
      }
    } catch (error) { setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không mở được file cookies.' }); }
  };

  const inspectCookieFile = async (path: string) => {
    setCookieChecking(true);
    setCookieInfo(undefined);
    setCookieError('');
    try { setCookieInfo(await videoToolsService.inspectTikTokCookies(path)); }
    catch (error) { setCookieError(error instanceof Error ? error.message : 'Không đọc được file cookies.'); }
    finally { setCookieChecking(false); }
  };

  const connectTikTokBrowser = async () => {
    const id = projectId();
    if (!id || taskRunning()) return;
    setCookieLoginStarting(true);
    setCookieLoginJobId(undefined);
    setCookieLoginProgress({ projectId: id, jobId: '', status: 'queued', progress: 0, message: 'Đang mở cửa sổ đăng nhập…', stage: 'queued', details: null, output: null });
    setCookieError('');
    try {
      setCookieLoginJobId(await videoToolsService.startTikTokBrowserLogin(id));
    } catch (error) {
      setCookieError(error instanceof Error ? error.message : 'Không thể mở đăng nhập TikTok.');
      setCookieLoginStarting(false);
    }
  };

  const startTikTokDownload = async (event: SubmitEvent) => {
    event.preventDefault();
    const id = projectId();
    if (!id || !tiktokUrl().trim() || !tiktokOutput() || taskRunning()) return;
    setTiktokStarting(true);
    setTiktokProgress({ projectId: id, jobId: '', status: 'queued', progress: 0, message: 'Đang chuẩn bị downloader…', stage: 'queued', details: null, output: null });
    setTiktokJobId(undefined);
    setTiktokError('');
    setDramaInfo(undefined);
    try {
      const job = await videoToolsService.startTikTokDownload(id, tiktokUrl(), tiktokOutput(), cookieFile() || null, browserSessionReady() && !cookieFile());
      setTiktokJobId(job);
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Không thể bắt đầu tải TikTok Drama.';
      setTiktokError(message);
      setTiktokProgress({ projectId: id, jobId: '', status: 'failed', progress: 0, message, stage: 'failed', details: null, output: null });
      setTiktokStarting(false);
    }
  };

  const cancelTikTok = async () => {
    const job = tiktokJobId();
    if (!job) return;
    try { await jobService.cancel(job); }
    catch (error) { setTiktokError(error instanceof Error ? error.message : 'Không thể hủy tải TikTok.'); }
  };

  const cancelTikTokBrowserLogin = async () => {
    const job = cookieLoginJobId();
    if (!job) return;
    try { await jobService.cancel(job); }
    catch (error) { setCookieError(error instanceof Error ? error.message : 'Không thể hủy đăng nhập TikTok.'); }
  };

  const scanSourceFolder = async (directory: string) => {
    setScanning(true);
    setScanError('');
    setVideos([]);
    setSkippedCount(0);
    try {
      const result = await videoToolsService.scanVideos(directory);
      setVideos(result.videos);
      setSkippedCount(result.skipped.length);
      if (result.videos.length === 0 && result.skipped.length) setScanError('Không tìm thấy video hợp lệ. Các tệp lỗi hoặc không hỗ trợ đã bị bỏ qua.');
    } catch (error) {
      setScanError(error instanceof Error ? error.message : 'Không thể quét thư mục video.');
    } finally { setScanning(false); }
  };

  const chooseSourceFolder = async () => {
    try {
      const selected = await open({ directory: true, multiple: false, title: 'Chọn thư mục chứa video cần gộp' });
      if (typeof selected !== 'string') return;
      setSourceDirectory(selected);
      await scanSourceFolder(selected);
    } catch (error) { setScanError(error instanceof Error ? error.message : 'Không mở được hộp thoại chọn video.'); }
  };

  const selectMergeLogo = async (logoId: string) => {
    setMergeLogoId(logoId);
    setMergeLogoError('');
    setMergeLogoImageAspect(0);
    const logo = currentProject()?.logos.find((item) => item.id === logoId);
    if (!logo) {
      setMergeLogo(null);
      setMergeLogoUrl('');
      setMergeLogoImageAspect(0);
      return;
    }
    setMergeLogo({ logoId, x: logo.x, y: logo.y, width: logo.width, height: logo.height, opacity: logo.opacity, maintainAspectRatio: logo.maintainAspectRatio });
    setMergeLogoEnabled(true);
    try {
      const path = await projectService.getLogoAssetPath(currentProject()!.id, logoId);
      setMergeLogoUrl(convertFileSrc(path));
    } catch (error) {
      setMergeLogoUrl('');
      setMergeLogoError(error instanceof Error ? error.message : 'Không thể đọc ảnh logo trong preview.');
    }
  };

  const addMergeLogo = async () => {
    const project = currentProject();
    if (!project || taskRunning()) return;
    try {
      const path = await open({ multiple: false, title: 'Chọn ảnh logo', filters: [{ name: 'Ảnh logo', extensions: ['png', 'jpg', 'jpeg', 'webp'] }] });
      if (typeof path !== 'string') return;
      const result = await projectService.importLogo(project.id, path);
      if (result.logo) {
        mergeVisualProject(result.project);
        const latestProject = currentProject() ?? result.project;
        const mergeOnlyProject = await projectService.saveVisualSettings(project.id, {
          subtitle: latestProject.subtitle,
          blurRegions: latestProject.blurRegions,
          logos: latestProject.logos.map((logo) => logo.id === result.logo!.id ? { ...logo, enabled: false, mergeOnly: true } : logo),
          textOverlays: latestProject.textOverlays ?? [],
        });
        mergeVisualProject(mergeOnlyProject);
        await selectMergeLogo(result.logo.id);
      }
      if (result.cleanupWarning) setNotice({ kind: 'info', message: result.cleanupWarning });
    } catch (error) {
      setMergeLogoError(error instanceof Error ? error.message : 'Không thể thêm ảnh logo vào dự án.');
    }
  };

  const beginMergeLogoPointer = (event: PointerEvent & { currentTarget: HTMLElement }, mode: 'move' | 'resize') => {
    const logo = mergeLogo();
    const bounds = mergePreviewStage?.getBoundingClientRect();
    if (!logo || !bounds || event.button !== 0 || taskRunning()) return;
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    mergeLogoDrag = { pointerId: event.pointerId, mode, startX: event.clientX, startY: event.clientY, stageWidth: bounds.width, stageHeight: bounds.height, initial: { ...logo } };
    setMergeLogoDragging(true);
  };

  const moveMergeLogoPointer = (event: PointerEvent) => {
    const drag = mergeLogoDrag;
    if (!drag || drag.pointerId !== event.pointerId) return;
    const dx = (event.clientX - drag.startX) / Math.max(drag.stageWidth, 1);
    const dy = (event.clientY - drag.startY) / Math.max(drag.stageHeight, 1);
    const start = drag.initial;
    if (drag.mode === 'move') {
      setMergeLogo({ ...start, x: Math.min(1 - start.width, Math.max(0, start.x + dx)), y: Math.min(1 - start.height, Math.max(0, start.y + dy)) });
      return;
    }
    let width = Math.min(1 - start.x, Math.max(0.025, start.width + dx));
    let height = Math.min(1 - start.y, Math.max(0.025, start.height + dy));
    if (start.maintainAspectRatio) {
      const normalizedAspect = start.width / Math.max(start.height, 0.001);
      width = Math.min(width, height * normalizedAspect);
      height = width / normalizedAspect;
    }
    setMergeLogo({ ...start, width, height });
  };

  const endMergeLogoPointer = (event: PointerEvent) => {
    if (mergeLogoDrag?.pointerId === event.pointerId) {
      mergeLogoDrag = undefined;
      setMergeLogoDragging(false);
    }
  };

  const beginMergeTextPointer = (event: PointerEvent & { currentTarget: HTMLElement }, mode: 'move' | 'resize') => {
    const text = mergeText();
    const bounds = mergePreviewStage?.getBoundingClientRect();
    if (!bounds || event.button !== 0 || taskRunning()) return;
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    mergeTextDrag = { pointerId: event.pointerId, mode, startX: event.clientX, startY: event.clientY, stageWidth: bounds.width, stageHeight: bounds.height, initial: { ...text } };
    setMergeTextDragging(true);
  };

  const moveMergeTextPointer = (event: PointerEvent) => {
    const drag = mergeTextDrag;
    if (!drag || drag.pointerId !== event.pointerId) return;
    const dx = (event.clientX - drag.startX) / Math.max(drag.stageWidth, 1);
    const dy = (event.clientY - drag.startY) / Math.max(drag.stageHeight, 1);
    const start = drag.initial;
    if (drag.mode === 'move') {
      setMergeText({ ...start, x: Math.min(1 - start.width, Math.max(0, start.x + dx)), y: Math.min(1 - start.height, Math.max(0, start.y + dy)) });
    } else {
      setMergeText({ ...start, width: Math.min(1 - start.x, Math.max(0.025, start.width + dx)), height: Math.min(1 - start.y, Math.max(0.025, start.height + dy)) });
    }
  };

  const endMergeTextPointer = (event: PointerEvent) => {
    if (mergeTextDrag?.pointerId === event.pointerId) {
      mergeTextDrag = undefined;
      setMergeTextDragging(false);
    }
  };

  const nudgeMergeText = (event: KeyboardEvent) => {
    const text = mergeText();
    if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) return;
    event.preventDefault();
    const step = event.shiftKey ? 0.05 : 0.01;
    setMergeText({
      ...text,
      x: Math.min(1 - text.width, Math.max(0, text.x + (event.key === 'ArrowLeft' ? -step : event.key === 'ArrowRight' ? step : 0))),
      y: Math.min(1 - text.height, Math.max(0, text.y + (event.key === 'ArrowUp' ? -step : event.key === 'ArrowDown' ? step : 0))),
    });
  };

  const resizeMergeLogoWidth = (value: number) => {
    const logo = mergeLogo();
    if (!logo) return;
    let width = Math.min(1 - logo.x, Math.max(0.025, value / 100));
    let height = logo.height;
    if (logo.maintainAspectRatio) height = Math.min(1 - logo.y, width / Math.max(logo.width / Math.max(logo.height, 0.001), 0.001));
    setMergeLogo({ ...logo, width, height });
  };

  const nudgeMergeLogo = (event: KeyboardEvent) => {
    const logo = mergeLogo();
    if (!logo || !['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) return;
    event.preventDefault();
    const step = event.shiftKey ? 0.05 : 0.01;
    setMergeLogo({
      ...logo,
      x: Math.min(1 - logo.width, Math.max(0, logo.x + (event.key === 'ArrowLeft' ? -step : event.key === 'ArrowRight' ? step : 0))),
      y: Math.min(1 - logo.height, Math.max(0, logo.y + (event.key === 'ArrowUp' ? -step : event.key === 'ArrowDown' ? step : 0))),
    });
  };

  const startMerge = async () => {
    const id = projectId();
    if (!id || !sourceDirectory() || videos().length < 2 || !mergeOutput() || taskRunning()) return;
    setMergeStarting(true);
    setMergeProgress({ projectId: id, jobId: '', status: 'queued', progress: 0, message: 'Đang chuẩn bị FFmpeg…', stage: 'queued', details: null, output: null });
    setMergeJobId(undefined);
    setMergeError('');
    try {
      const logos = mergeLogoEnabled() && mergeLogo() ? [mergeLogo()!] : [];
      const texts = mergeTextEnabled() && mergeText().text.trim() ? [mergeText()] : [];
      const job = await videoToolsService.startMerge(id, sourceDirectory(), videos().map((video) => video.path), mergeOutput(), mergeFrameMode(), mergeVideoCodec(), mergeEncoder(), logos, texts);
      setMergeJobId(job);
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Không thể bắt đầu gộp video.';
      setMergeError(message);
      setMergeProgress({ projectId: id, jobId: '', status: 'failed', progress: 0, message, stage: 'failed', details: null, output: null });
      setMergeStarting(false);
    }
  };

  const cancelMerge = async () => {
    const job = mergeJobId();
    if (!job) return;
    try { await jobService.cancel(job); }
    catch (error) { setMergeError(error instanceof Error ? error.message : 'Không thể hủy gộp video.'); }
  };

  const moveVideo = (path: string, direction: -1 | 1) => {
    const current = [...videos()];
    const index = current.findIndex((video) => video.path === path);
    const destination = index + direction;
    if (index < 0 || destination < 0 || destination >= current.length) return;
    [current[index], current[destination]] = [current[destination], current[index]];
    setVideos(current);
  };

  const reorderVideo = (sourcePath: string, targetPath: string) => {
    if (!sourcePath || sourcePath === targetPath) return;
    const current = [...videos()];
    const from = current.findIndex((video) => video.path === sourcePath);
    const to = current.findIndex((video) => video.path === targetPath);
    if (from < 0 || to < 0) return;
    const [video] = current.splice(from, 1);
    current.splice(to, 0, video);
    setVideos(current);
  };

  return (
    <main class="video-tools-page">
      <header class="video-tools-header">
        <div class="video-tools-heading">
          <div class="video-tools-heading-icon" aria-hidden="true">{props.workflow === 'merge' ? <Merge size={19} /> : <Download size={19} />}</div>
          <div><span class="eyebrow">VIDEO WORKFLOWS</span><h1>{props.workflow === 'merge' ? 'Gộp Video' : 'Tải video'}</h1><p>{props.workflow === 'merge' ? 'Sắp xếp và gộp các video trong một thư mục.' : 'Tải trọn bộ series TikTok và lưu video theo thứ tự tập.'}</p></div>
        </div>
        <span class="video-tools-project-chip"><span />{currentProject()?.name ?? 'Dự án'}</span>
      </header>

      <Show when={props.workflow === 'tiktok'}>
        <div class="video-tools-body">
          <section class="video-tool-card tiktok-tool-card">
            <div class="video-tool-card-header"><div><span class="eyebrow">TIKTOK SHORT DRAMA</span><h2>Tải trọn bộ series</h2><p>Dán liên kết một tập. App sẽ đọc danh sách tập, thông tin series và lưu các video theo thứ tự EP.</p></div><div class="video-tool-header-icon"><Download size={18} /></div></div>
            <form class="video-tool-form" onSubmit={(event) => void startTikTokDownload(event)}>
              <label class="video-tool-field"><span>Liên kết TikTok</span><input type="url" required value={tiktokUrl()} disabled={taskRunning()} placeholder="https://www.tiktok.com/@.../video/..." onInput={(event) => setTiktokUrl(event.currentTarget.value)} /></label>
              <div class="video-tool-field"><span>Thư mục lưu</span><div class="video-tool-directory-row"><div class="video-tool-directory-value" title={tiktokOutput()}><FolderOpen size={15} /><span>{tiktokOutput() ? directoryLabel(tiktokOutput()) : 'Đang lấy thư mục dự án…'}</span></div><button class="button button-quiet" type="button" disabled={taskRunning()} onClick={() => void chooseDirectory('tiktok')}>Chọn thư mục</button></div><small>Mặc định lưu tại thư mục TikTok bên trong project.</small></div>
              <details class="video-tool-advanced" open><summary>Đăng nhập TikTok (tùy chọn)</summary><p>Nhấn nút bên dưới để mở cửa sổ Chrome riêng của JC AI. Đăng nhập TikTok một lần; các lần tải sau sẽ tự dùng lại phiên này. App không đọc cookie từ hồ sơ Chrome cá nhân.</p><div class="video-tool-directory-row"><div class="video-tool-directory-value"><ShieldCheck size={15} /><span>{browserSessionReady() ? 'Đã kết nối phiên TikTok' : 'Chưa kết nối trình duyệt'}</span></div><button class="button button-quiet" type="button" disabled={taskRunning()} onClick={() => void connectTikTokBrowser()}>{cookieLoginStarting() ? <><LoaderCircle size={14} class="spin" /> Đang chờ đăng nhập…</> : browserSessionReady() ? 'Kết nối lại' : 'Tự lấy cookie từ trình duyệt'}</button></div><Show when={cookieLoginStarting()}><div class="video-tool-cookie-status" role="status"><LoaderCircle size={13} class="spin" /><span>{cookieLoginProgress()?.message ?? 'Đăng nhập trong cửa sổ Chrome vừa mở.'}</span><button class="button button-quiet" type="button" disabled={!cookieLoginJobId()} onClick={() => void cancelTikTokBrowserLogin()}>Hủy</button></div></Show><div class="video-tool-directory-row video-tool-cookie-file-row"><div class="video-tool-directory-value" title={cookieFile()}><FolderOpen size={15} /><span>{cookieFile() ? directoryLabel(cookieFile()) : 'Hoặc dùng file cookies.txt đã có'}</span></div><button class="button button-quiet" type="button" disabled={taskRunning() || cookieChecking()} onClick={() => void chooseCookieFile()}>{cookieFile() ? 'Đổi file' : 'Chọn file'}</button><Show when={cookieFile()}><button class="video-tool-clear-cookie" type="button" aria-label="Xóa lựa chọn cookies" disabled={taskRunning() || cookieChecking()} onClick={() => { setCookieFile(''); setCookieInfo(undefined); setCookieError(''); }}><X size={14} /></button></Show></div><Show when={cookieChecking()}><div class="video-tool-cookie-status" role="status"><LoaderCircle size={13} class="spin" /><span>Đang kiểm tra file cookies…</span></div></Show><Show when={cookieInfo()} keyed>{(info) => <div class="video-tool-cookie-status" classList={{ 'video-tool-cookie-warning': info.tiktokCookies === 0 }} role="status"><ShieldCheck size={13} /><span>{info.tiktokCookies > 0 ? `Đã đọc ${info.totalCookies} cookie, có ${info.tiktokCookies} cookie TikTok.` : `Đã đọc ${info.totalCookies} cookie nhưng không tìm thấy cookie thuộc tiktok.com.`}</span></div>}</Show><Show when={cookieError()}><div class="video-tool-inline-error"><CircleAlert size={14} /><span>{cookieError()}</span></div></Show><small>Phiên trình duyệt được lưu riêng cho ứng dụng; cookie không hiển thị trong giao diện.</small></details>
              <div class="video-tool-form-footer"><span><ShieldCheck size={14} />Dùng phiên trình duyệt hoặc file cookie đã chọn</span><button class="button button-primary" type="submit" disabled={!tiktokUrl().trim() || !tiktokOutput() || taskRunning() || cookieChecking() || (!!cookieFile() && (!cookieInfo() || cookieInfo()!.tiktokCookies === 0))}><Show when={tiktokStarting()} fallback={<Download size={15} />}><LoaderCircle size={15} class="spin" /></Show>{tiktokStarting() ? 'Đang xử lý…' : 'Đọc info & tải series'}</button></div>
            </form>
          </section>

          <section class="video-tool-card video-tool-status-card" aria-live="polite">
            <div class="video-tool-card-header compact"><div><span class="eyebrow">THÔNG TIN & TIẾN TRÌNH</span><h2>{dramaInfo()?.title ?? 'Chưa có tác vụ'}</h2><p>{tiktokProgress()?.message ?? 'Thông tin series và trạng thái tải sẽ hiển thị tại đây.'}</p></div><Show when={tiktokStarting()}><LoaderCircle size={18} class="spin video-tool-live-icon" /></Show><Show when={tiktokProgress()?.status === 'completed'}><Check size={19} class="video-tool-success-icon" /></Show></div>
            <Show when={tiktokStarting() || tiktokProgress()?.status === 'completed'}><div class="video-tool-progress-track"><span style={{ width: `${Math.round((tiktokProgress()?.progress ?? 0) * 100)}%` }} /></div><div class="video-tool-progress-meta"><span>{tiktokProgress()?.stage === 'download' ? 'Đang xử lý tập' : tiktokProgress()?.stage ?? 'Đang chờ'}</span><strong>{Math.round((tiktokProgress()?.progress ?? 0) * 100)}%</strong></div></Show>
            <Show when={tiktokStarting()}><button class="button button-quiet video-tool-cancel" type="button" disabled={!tiktokJobId()} onClick={() => void cancelTikTok()}><X size={14} /> Hủy tải</button></Show>
            <Show when={tiktokError()}><div class="video-tool-inline-error"><CircleAlert size={15} /><span>{tiktokError()}</span></div></Show>
            <Show when={dramaInfo()} keyed>{(info) => <div class="drama-info-grid"><div class="drama-info-main"><span class="drama-info-count">{info.totalEpisodes} tập được phát hiện</span><Show when={info.tags.length}><div class="drama-info-tags"><For each={info.tags.slice(0, 6)}>{(tag) => <span>{tag}</span>}</For></div></Show><Show when={info.description}><p>{info.description}</p></Show></div><div class="drama-info-side"><span>LƯỢT XEM</span><strong>{info.playCount ?? '—'}</strong><small>Thư mục series</small><code title={info.outputDirectory}>{directoryLabel(info.outputDirectory)}</code></div></div>}</Show>
            <Show when={tiktokProgress()?.status === 'completed' && tiktokProgress()?.output}><div class="video-tool-output-path"><Check size={14} /><span>Đã lưu:</span><code title={tiktokProgress()?.output ?? ''}>{directoryLabel(tiktokProgress()?.output ?? '')}</code></div></Show>
            <Show when={!dramaInfo() && !tiktokStarting() && !tiktokError()}><div class="video-tool-empty-info"><FileVideo size={19} /><span>Sau khi đọc được series, danh sách tập, tags, lượt xem và thư mục lưu sẽ hiện ở đây.</span></div></Show>
          </section>
        </div>
      </Show>

      <Show when={props.workflow === 'merge'}>
        <div class="video-tools-body merge-tool-body">
          <section class="video-tool-card merge-source-card">
            <div class="video-tool-card-header"><div><span class="eyebrow">01 · VIDEO NGUỒN</span><h2>Chọn thư mục video</h2><p>Các video ở ngay trong thư mục sẽ được quét và sắp xếp ban đầu theo tên A–Z.</p></div><div class="video-tool-header-icon"><FileVideo size={18} /></div></div>
            <div class="video-tool-directory-row"><div class="video-tool-directory-value" title={sourceDirectory()}><FolderOpen size={15} /><span>{sourceDirectory() ? directoryLabel(sourceDirectory()) : 'Chưa chọn thư mục chứa video'}</span></div><button class="button button-outline" type="button" disabled={taskRunning() || scanning()} onClick={() => void chooseSourceFolder()}><Show when={scanning()} fallback={<FolderOpen size={14} />}><LoaderCircle size={14} class="spin" /></Show>{scanning() ? 'Đang quét…' : sourceDirectory() ? 'Đổi thư mục' : 'Chọn thư mục'}</button></div>
            <Show when={scanError()}><div class="video-tool-inline-error"><CircleAlert size={15} /><span>{scanError()}</span></div></Show>
            <Show when={skippedCount() > 0}><div class="video-tool-skip-note"><CircleAlert size={14} />Đã bỏ qua {skippedCount()} tệp không đọc được hoặc không phải video hợp lệ.</div></Show>
            <Show when={videos().length > 0}>
              <div class="merge-list-heading"><strong>{videos().length} video</strong><span>Kéo để đổi thứ tự · thứ tự này sẽ được giữ khi gộp</span></div>
              <ol class="merge-video-list" aria-label="Thứ tự video sẽ gộp">
                <For each={videos()}>{(video, index) => <li class="merge-video-row" draggable="true" onDragStart={(event) => { setDraggedPath(video.path); if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move'; }} onDragOver={(event) => { event.preventDefault(); if (event.dataTransfer) event.dataTransfer.dropEffect = 'move'; }} onDrop={(event) => { event.preventDefault(); reorderVideo(draggedPath() ?? '', video.path); setDraggedPath(undefined); }} onDragEnd={() => setDraggedPath(undefined)} classList={{ 'merge-video-dragging': draggedPath() === video.path }}>
                  <span class="merge-video-position">{String(index() + 1).padStart(2, '0')}</span><GripVertical size={15} class="merge-video-grip" aria-hidden="true" /><div class="merge-video-copy"><strong title={video.fileName}>{video.fileName}</strong><small>{video.width}×{video.height} · {formatTime(video.duration)} · {video.videoCodec.toUpperCase()}{video.audioCodec ? ` / ${video.audioCodec.toUpperCase()}` : ' · Không có audio'}</small></div><span class="merge-video-size">{formatBytes(video.sizeBytes)}</span><div class="merge-video-order-actions"><button type="button" aria-label={`Đưa ${video.fileName} lên trên`} disabled={taskRunning() || index() === 0} onClick={() => moveVideo(video.path, -1)}><ArrowUp size={14} /></button><button type="button" aria-label={`Đưa ${video.fileName} xuống dưới`} disabled={taskRunning() || index() === videos().length - 1} onClick={() => moveVideo(video.path, 1)}><ArrowDown size={14} /></button></div>
                </li>}</For>
              </ol>
            </Show>
            <Show when={sourceDirectory() && !scanning() && videos().length === 0 && !scanError()}><div class="video-tool-empty-info merge-empty-info"><FileVideo size={19} /><span>Thư mục chưa có video hỗ trợ. Hỗ trợ MP4, MOV, MKV, WEBM, M4V và AVI.</span><button class="button button-quiet" type="button" onClick={() => void scanSourceFolder(sourceDirectory())}><RefreshCw size={13} /> Quét lại</button></div></Show>
          </section>

          <section class="video-tool-card merge-output-card">
            <div class="video-tool-card-header compact"><div><span class="eyebrow">02 · FILE ĐẦU RA</span><h2>Thư mục lưu video gộp</h2><p>Tên file được tạo tự động theo thời gian; file cũ sẽ không bị ghi đè.</p></div></div>
            <div class="video-tool-directory-row"><div class="video-tool-directory-value" title={mergeOutput()}><FolderOpen size={15} /><span>{mergeOutput() ? directoryLabel(mergeOutput()) : 'Đang lấy thư mục dự án…'}</span></div><button class="button button-quiet" type="button" disabled={taskRunning()} onClick={() => void chooseDirectory('merge')}>Chọn thư mục</button></div>
            <label class="merge-frame-mode"><span><strong>Tỷ lệ khung hình</strong><small>{mergeFrameMode() === 'source' ? 'Tự động giữ khung hình của video có độ phân giải lớn nhất.' : 'Video được giữ nguyên tỷ lệ, căn giữa trên nền đen nếu không vừa khung.'}</small></span><select class="settings-select" value={mergeFrameMode()} disabled={taskRunning()} onChange={(event) => setMergeFrameMode(event.currentTarget.value as MergeFrameMode)}><option value="source">Tự động theo video</option><option value="portrait">Dọc · 9:16</option><option value="landscape">Ngang · 16:9</option></select></label>
            <div class="merge-encoding-options">
              <label class="merge-frame-mode"><span><strong>Codec video</strong><small>HEVC thường tạo file nhỏ hơn ở chất lượng tương đương.</small></span><select class="settings-select" value={mergeVideoCodec()} disabled={taskRunning()} onChange={(event) => setMergeVideoCodec(event.currentTarget.value as MergeVideoCodec)}><option value="h264">H.264 · tương thích rộng</option><option value="hevc">HEVC / H.265 · file gọn hơn</option></select></label>
              <VideoEncoderSelect codec={mergeVideoCodec()} value={mergeEncoder()} disabled={taskRunning()} onChange={setMergeEncoder} />
            </div>
            <small class="merge-encoding-note">CPU/GPU chỉ dùng khi cần mã hóa lại. Video cùng codec, cùng thông số và không có logo/chữ hoặc đổi tỷ lệ sẽ được nối trực tiếp, giữ nguyên chất lượng.</small>
            <section class="merge-logo-section" aria-label="Logo video gộp">
              <label class="merge-logo-toggle"><input type="checkbox" checked={mergeLogoEnabled()} disabled={taskRunning()} onChange={(event) => setMergeLogoEnabled(event.currentTarget.checked)} /><span><strong>Chèn logo khi gộp video</strong><small>Logo được ghép lên toàn bộ video đầu ra.</small></span></label>
              <Show when={mergeLogoEnabled()}>
                <div class="merge-logo-toolbar">
                  <label class="video-tool-field"><span>Ảnh logo</span><select class="settings-select" value={mergeLogoId()} disabled={taskRunning()} onChange={(event) => void selectMergeLogo(event.currentTarget.value)}><option value="">Chọn logo trong dự án</option><For each={currentProject()?.logos ?? []}>{(logo) => <option value={logo.id}>{logo.originalName}</option>}</For></select></label>
                  <button class="button button-quiet" type="button" disabled={taskRunning()} onClick={() => void addMergeLogo()}><ImagePlus size={14} /> Thêm logo</button>
                </div>
                <Show when={mergeLogoError()}><div class="video-tool-inline-error"><CircleAlert size={14} /><span>{mergeLogoError()}</span></div></Show>
                <Show when={!mergeLogo() && videos().length > 0}><div class="merge-logo-empty"><ImagePlus size={15} /><span>Chọn một logo đã có hoặc thêm ảnh mới để xem trước và chỉnh vị trí.</span></div></Show>
              </Show>
            </section>
            <section class="merge-logo-section merge-text-section" aria-label="Chữ trên video gộp">
              <label class="merge-logo-toggle"><input type="checkbox" checked={mergeTextEnabled()} disabled={taskRunning()} onChange={(event) => setMergeTextEnabled(event.currentTarget.checked)} /><span><strong>Chèn chữ khi gộp video</strong><small>Chữ được ghép lên toàn bộ video đầu ra.</small></span></label>
              <Show when={mergeTextEnabled()}>
                <label class="video-tool-field merge-text-content-field"><span>Nội dung chữ</span><textarea class="settings-textarea" rows="3" maxLength={4000} value={mergeText().text} disabled={taskRunning()} onInput={(event) => setMergeText({ ...mergeText(), text: event.currentTarget.value })} /></label>
                <div class="merge-logo-controls merge-text-controls">
                  <label><span>Cỡ chữ <strong>{mergeText().fontSize}px</strong></span><input type="range" min="12" max="180" value={mergeText().fontSize} disabled={taskRunning()} onInput={(event) => setMergeText({ ...mergeText(), fontSize: Number(event.currentTarget.value) })} /></label>
                  <label><span>Độ mờ <strong>{Math.round(mergeText().opacity * 100)}%</strong></span><input type="range" min="0" max="100" value={Math.round(mergeText().opacity * 100)} disabled={taskRunning()} onInput={(event) => setMergeText({ ...mergeText(), opacity: Number(event.currentTarget.value) / 100 })} /></label>
                  <label><span>Màu chữ</span><input type="color" value={mergeText().color} disabled={taskRunning()} onInput={(event) => setMergeText({ ...mergeText(), color: event.currentTarget.value.toUpperCase() })} /></label>
                  <label><span>Font family</span><input class="settings-text-input" value={mergeText().fontFamily} maxLength={80} disabled={taskRunning()} onInput={(event) => setMergeText({ ...mergeText(), fontFamily: event.currentTarget.value })} /></label>
                  <label><span>Độ đậm</span><select class="settings-select" value={mergeText().fontWeight} disabled={taskRunning()} onChange={(event) => setMergeText({ ...mergeText(), fontWeight: Number(event.currentTarget.value) })}><option value="400">Regular</option><option value="500">Medium</option><option value="600">Semibold</option><option value="700">Bold</option><option value="800">Extra bold</option></select></label>
                  <label><span>Căn lề</span><select class="settings-select" value={mergeText().alignment} disabled={taskRunning()} onChange={(event) => setMergeText({ ...mergeText(), alignment: event.currentTarget.value as MergeTextSettings['alignment'] })}><option value="left">Trái</option><option value="center">Giữa</option><option value="right">Phải</option></select></label>
                </div>
              </Show>
            </section>
            <Show when={videos().length > 0 && ((mergeLogoEnabled() && !!mergeLogo()) || (mergeTextEnabled() && !!mergeText().text.trim()))}>
              <div class="merge-preview-shell">
                <div class="merge-preview-heading"><strong>Xem trước video gộp</strong><span>Kéo chữ/logo để di chuyển · kéo góc để đổi kích thước</span></div>
                <div class="merge-preview-stage" ref={mergePreviewStage} style={{ 'aspect-ratio': String(mergeCanvasAspect()), '--merge-canvas-aspect': String(mergeCanvasAspect()) }} onPointerMove={(event) => { moveMergeLogoPointer(event); moveMergeTextPointer(event); }} onPointerUp={(event) => { endMergeLogoPointer(event); endMergeTextPointer(event); }} onPointerCancel={(event) => { endMergeLogoPointer(event); endMergeTextPointer(event); }}>
                  <Show when={mergeVideoPreviewUrl()} fallback={<div class="merge-preview-empty"><FileVideo size={20} /><span>Chưa có video xem trước</span></div>}>
                    {(src) => <video class="merge-preview-video" src={src()} controls muted playsinline preload="metadata" />}
                  </Show>
                  <Show when={mergeLogoEnabled() && mergeLogo() && mergeLogoUrl()}><div class="merge-preview-logo" classList={{ dragging: mergeLogoDragging() }} role="group" tabindex="0" aria-label="Logo trong preview gộp video" title="Kéo để di chuyển · kéo góc để đổi kích thước · dùng phím mũi tên để di chuyển" style={{ left: `${(mergeLogo()?.x ?? 0) * 100}%`, top: `${(mergeLogo()?.y ?? 0) * 100}%`, width: `${(mergeLogo()?.width ?? 0) * 100}%`, height: `${(mergeLogo()?.height ?? 0) * 100}%`, opacity: String(mergeLogo()?.opacity ?? 1) }} onKeyDown={nudgeMergeLogo} onPointerDown={(event) => beginMergeLogoPointer(event, 'move')} onPointerUp={endMergeLogoPointer} onPointerCancel={endMergeLogoPointer}>
                    <img src={mergeLogoUrl()} alt="Logo trên video gộp" draggable={false} onLoad={(event) => { const image = event.currentTarget; if (image.naturalHeight > 0) setMergeLogoImageAspect(image.naturalWidth / image.naturalHeight); }} />
                    <button class="merge-preview-resize" type="button" aria-label="Kéo để đổi kích thước logo" onPointerDown={(event) => beginMergeLogoPointer(event, 'resize')} onPointerUp={endMergeLogoPointer} onPointerCancel={endMergeLogoPointer} />
                  </div></Show>
                  <Show when={mergeTextEnabled() && mergeText().text.trim()}><div class="merge-preview-text" classList={{ dragging: mergeTextDragging() }} role="group" tabindex="0" aria-label="Chữ trong preview gộp video" title="Kéo để di chuyển · kéo góc để đổi kích thước · dùng phím mũi tên để di chuyển" style={{ left: `${mergeText().x * 100}%`, top: `${mergeText().y * 100}%`, width: `${mergeText().width * 100}%`, height: `${mergeText().height * 100}%`, opacity: String(mergeText().opacity), 'font-family': `"${mergeText().fontFamily}", sans-serif`, 'font-size': `${mergeText().fontSize * 100 / Math.max(mergeCanvasWidth(), 1)}cqw`, 'font-weight': String(mergeText().fontWeight), color: mergeText().color, 'text-align': mergeText().alignment }} onKeyDown={nudgeMergeText} onPointerDown={(event) => beginMergeTextPointer(event, 'move')} onPointerUp={endMergeTextPointer} onPointerCancel={endMergeTextPointer}>
                    <span>{mergeText().text}</span><button class="merge-preview-resize" type="button" aria-label="Kéo để đổi kích thước chữ" onPointerDown={(event) => beginMergeTextPointer(event, 'resize')} onPointerUp={endMergeTextPointer} onPointerCancel={endMergeTextPointer} />
                  </div></Show>
                </div>
              </div>
              <Show when={mergeLogoEnabled() && mergeLogo()}><div class="merge-logo-controls"><label><span>Độ mờ logo <strong>{Math.round((mergeLogo()?.opacity ?? 1) * 100)}%</strong></span><input type="range" min="0" max="100" step="1" value={Math.round((mergeLogo()?.opacity ?? 1) * 100)} disabled={taskRunning()} onInput={(event) => { const logo = mergeLogo(); if (logo) setMergeLogo({ ...logo, opacity: Number(event.currentTarget.value) / 100 }); }} /></label><label><span>Kích thước logo <strong>{Math.round((mergeLogo()?.width ?? 0) * 100)}% khung</strong></span><input type="range" min="3" max="100" step="1" value={Math.round((mergeLogo()?.width ?? 0.16) * 100)} disabled={taskRunning()} onInput={(event) => resizeMergeLogoWidth(Number(event.currentTarget.value))} /></label><label class="merge-logo-aspect"><input type="checkbox" checked={mergeLogo()?.maintainAspectRatio ?? true} disabled={taskRunning()} onChange={(event) => { const logo = mergeLogo(); if (logo) setMergeLogo({ ...logo, maintainAspectRatio: event.currentTarget.checked }); }} /><span>Giữ tỷ lệ ảnh</span></label></div></Show>
            </Show>
            <div class="merge-output-hint"><span>File mặc định</span><code>merged_{new Date().toISOString().slice(0, 10).replaceAll('-', '')}_HHMMSS.mp4</code></div>
            <div class="merge-action-row"><div><strong>{videos().length >= 2 ? `${videos().length} video · ${formatTime(videos().reduce((total, video) => total + video.duration, 0))}` : 'Cần ít nhất 2 video'}</strong><small>{videos().length >= 2 ? 'Stream tương thích sẽ được ghép không mã hóa lại; video khác định dạng được chuẩn hóa.' : 'Chọn thư mục và sắp thứ tự các video trước khi gộp.'}</small></div><button class="button button-primary" type="button" disabled={videos().length < 2 || !mergeOutput() || taskRunning()} onClick={() => void startMerge()}><Show when={mergeStarting()} fallback={<Merge size={15} />}><LoaderCircle size={15} class="spin" /></Show>{mergeStarting() ? 'Đang gộp…' : 'Gộp video'}</button></div>
            <Show when={mergeStarting() || mergeProgress()?.status === 'completed'}><div class="video-tool-progress-track"><span style={{ width: `${Math.round((mergeProgress()?.progress ?? 0) * 100)}%` }} /></div><div class="video-tool-progress-meta"><span>{mergeProgress()?.message ?? 'Đang chuẩn bị…'}</span><strong>{Math.round((mergeProgress()?.progress ?? 0) * 100)}%</strong></div></Show>
            <Show when={mergeStarting()}><button class="button button-quiet video-tool-cancel" type="button" disabled={!mergeJobId()} onClick={() => void cancelMerge()}><X size={14} /> Hủy gộp</button></Show>
            <Show when={mergeError()}><div class="video-tool-inline-error"><CircleAlert size={15} /><span>{mergeError()}</span></div></Show>
            <Show when={mergeProgress()?.status === 'completed' && mergeProgress()?.output}><div class="video-tool-output-path"><Check size={14} /><span>Video đã tạo:</span><code title={mergeProgress()?.output ?? ''}>{directoryLabel(mergeProgress()?.output ?? '')}</code></div></Show>
          </section>
        </div>
      </Show>
    </main>
  );
}
