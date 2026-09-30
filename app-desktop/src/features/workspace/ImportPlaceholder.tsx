import { createEffect, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { open } from '@tauri-apps/plugin-dialog';
import {
  ArrowDown, ArrowUp, Check, CircleAlert, Clapperboard, Download, FileVideo,
  FolderOpen, GripVertical, LoaderCircle, Merge, RefreshCw, ShieldCheck,
  X,
} from 'lucide-solid';
import { currentProject, setNotice } from '../../app/project-state';
import { jobService } from '../../app/services/job-service';
import { videoToolsService, type MergeFrameMode, type MergeVideoInfo, type TikTokCookieInfo, type TikTokDramaInfo, type VideoToolProgress } from '../../app/services/video-tools-service';

type Tab = 'tiktok' | 'merge';

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

export default function ImportPlaceholder() {
  const [tab, setTab] = createSignal<Tab>('tiktok');
  const [tiktokOutput, setTiktokOutput] = createSignal('');
  const [mergeOutput, setMergeOutput] = createSignal('');
  const [mergeFrameMode, setMergeFrameMode] = createSignal<MergeFrameMode>('source');
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

  let unlisten: (() => void) | undefined;
  let folderGeneration = 0;
  const projectId = () => currentProject()?.id;
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

  const startMerge = async () => {
    const id = projectId();
    if (!id || !sourceDirectory() || videos().length < 2 || !mergeOutput() || taskRunning()) return;
    setMergeStarting(true);
    setMergeProgress({ projectId: id, jobId: '', status: 'queued', progress: 0, message: 'Đang chuẩn bị FFmpeg…', stage: 'queued', details: null, output: null });
    setMergeJobId(undefined);
    setMergeError('');
    try {
      const job = await videoToolsService.startMerge(id, sourceDirectory(), videos().map((video) => video.path), mergeOutput(), mergeFrameMode());
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
          <div class="video-tools-heading-icon"><Clapperboard size={19} /></div>
          <div><span class="eyebrow">VIDEO WORKFLOWS</span><h1>Kéo video</h1><p>Tải series TikTok hoặc sắp xếp và gộp các video trong một thư mục.</p></div>
        </div>
        <span class="video-tools-project-chip"><span />{currentProject()?.name ?? 'Dự án'}</span>
      </header>

      <div class="video-tools-tabs" role="tablist" aria-label="Chọn công cụ video">
        <button classList={{ active: tab() === 'tiktok' }} type="button" role="tab" aria-selected={tab() === 'tiktok'} onClick={() => setTab('tiktok')}><Download size={15} /><span>TikTok Drama Downloader</span></button>
        <button classList={{ active: tab() === 'merge' }} type="button" role="tab" aria-selected={tab() === 'merge'} onClick={() => setTab('merge')}><Merge size={15} /><span>Gộp video</span></button>
      </div>

      <Show when={tab() === 'tiktok'}>
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

      <Show when={tab() === 'merge'}>
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
