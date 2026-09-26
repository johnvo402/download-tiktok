import { createMemo, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { open } from '@tauri-apps/plugin-dialog';
import {
  AlertTriangle, Check, ChevronUp, Clapperboard, FileVideo2, FolderOpen,
  Layers3, LoaderCircle, Play, Plus, RefreshCw, Trash2, X,
} from 'lucide-solid';
import Modal from '../../../shared/components/Modal';
import { setBatchProcessingActive, setCurrentProject, setCurrentTranscript, setNotice } from '../../../app/project-state';
import { batchVideoService } from '../../../app/services/batch-video-service';
import { projectService } from '../../../app/services/project-service';
import { renderService } from '../../../app/services/render-service';
import type { Project } from '../../projects/types';
import { BatchVideoRunner } from './batch-video-runner';
import { createEmptyBatchVideoQueue, orderBatchVideos, type BatchVideoItem, type BatchVideoQueue, type BatchVideoRunUpdate } from './types';

const supportedVideos = ['mp4', 'mov', 'mkv', 'webm'];
const fileName = (path: string) => path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;

export default function BatchVideoDialog(props: { project: Project; onClose: () => void }) {
  const [queue, setQueue] = createSignal<BatchVideoQueue>(createEmptyBatchVideoQueue(props.project.id));
  const [loading, setLoading] = createSignal(true);
  const [saving, setSaving] = createSignal(false);
  const [running, setRunning] = createSignal(false);
  const [cancelRequested, setCancelRequested] = createSignal(false);
  const [confirmReplace, setConfirmReplace] = createSignal(false);
  const [error, setError] = createSignal('');
  const [current, setCurrent] = createSignal<BatchVideoRunUpdate>();
  const runner = new BatchVideoRunner();
  let alive = true;

  const ordered = createMemo(() => orderBatchVideos(queue()));
  const sample = createMemo(() => queue().items.find((item) => item.id === queue().sampleId));
  const pendingCount = createMemo(() => queue().items.filter((item) => item.status !== 'completed').length);
  const completedCount = createMemo(() => queue().items.filter((item) => item.status === 'completed').length);
  const failedCount = createMemo(() => queue().items.filter((item) => item.status === 'failed').length);
  const allDone = createMemo(() => queue().items.length > 0 && completedCount() === queue().items.length);

  onMount(() => {
    void batchVideoService.get(props.project.id).then((saved) => {
      if (alive && saved) setQueue(saved);
    }).catch((reason: unknown) => {
      if (alive) setError(reason instanceof Error ? reason.message : 'Không thể đọc hàng đợi đã lưu.');
    }).finally(() => alive && setLoading(false));
  });
  onCleanup(() => {
    alive = false;
    if (running()) void runner.cancel().finally(() => setBatchProcessingActive(false));
    else setBatchProcessingActive(false);
  });

  const save = async (next: BatchVideoQueue) => {
    setSaving(true);
    setError('');
    try {
      const saved = await batchVideoService.save(props.project.id, next);
      if (alive) setQueue(saved);
      return saved;
    } catch (reason) {
      if (alive) setError(reason instanceof Error ? reason.message : 'Không thể lưu hàng đợi video.');
      throw reason;
    } finally { alive && setSaving(false); }
  };

  const addVideos = async () => {
    try {
      const selected = await open({
        multiple: true,
        directory: false,
        title: 'Thêm video vào hàng đợi',
        filters: [{ name: 'Video', extensions: supportedVideos }],
      });
      if (!selected) return;
      const paths = typeof selected === 'string' ? [selected] : selected;
      const known = new Set(queue().items.map((item) => normalizePath(item.sourcePath)));
      const additions: BatchVideoItem[] = paths
        .filter((path) => !known.has(normalizePath(path)))
        .map((path) => ({
          id: crypto.randomUUID(), sourcePath: path, fileName: fileName(path),
          status: 'queued', stage: null, progress: 0,
          message: 'Đang chờ xử lý.', outputPath: null,
        }));
      if (!additions.length) return;
      const prior = queue();
      const next = {
        ...prior,
        sampleId: prior.sampleId || additions[0].id,
        items: [...prior.items, ...additions],
      };
      await save(next);
    } catch (reason) {
      if (alive && !(reason instanceof Error && reason.message === 'cancelled')) {
        setError(reason instanceof Error ? reason.message : 'Không thể thêm video.');
      }
    }
  };

  const chooseOutputDirectory = async () => {
    try {
      const selected = await open({ directory: true, multiple: false, title: 'Chọn thư mục lưu video đã dịch' });
      if (typeof selected === 'string' && queue().items.length) {
        await save({ ...queue(), outputDirectory: selected });
      }
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Không thể chọn thư mục export.');
    }
  };

  const setSample = async (itemId: string) => {
    if (queue().sampleLoaded || queue().items.some((item) => item.status === 'completed')) return;
    setConfirmReplace(false);
    try { await save({ ...queue(), sampleId: itemId, sampleChecksum: null }); }
    catch { /* save() already exposes the backend error beside the queue. */ }
  };

  const removeItem = async (itemId: string) => {
    if (running()) return;
    const prior = queue();
    if (itemId === prior.sampleId && prior.sampleLoaded) return;
    const items = prior.items.filter((item) => item.id !== itemId);
    if (!items.length) {
      try {
        await batchVideoService.clear(props.project.id);
        setQueue(createEmptyBatchVideoQueue(props.project.id));
        setError('');
      } catch (reason) { setError(reason instanceof Error ? reason.message : 'Không thể xóa hàng đợi.'); }
      return;
    }
    const nextSample = itemId === prior.sampleId ? items[0].id : prior.sampleId;
    try {
      await save({ ...prior, sampleId: nextSample, sampleLoaded: itemId === prior.sampleId ? false : prior.sampleLoaded, sampleChecksum: itemId === prior.sampleId ? null : prior.sampleChecksum, items });
    } catch { /* save() already exposes the backend error beside the queue. */ }
  };

  const clearQueue = async () => {
    if (running() || !queue().items.length) return;
    if (!window.confirm('Xóa danh sách video và trạng thái batch đã lưu? Các file export hiện có sẽ được giữ nguyên.')) return;
    try {
      await batchVideoService.clear(props.project.id);
      setQueue(createEmptyBatchVideoQueue(props.project.id));
      setError('');
      setCurrent(undefined);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Không thể xóa hàng đợi.');
    }
  };

  const loadSample = async () => {
    const value = sample();
    if (!value || running()) return;
    setSaving(true); setError('');
    try {
      const latest = await projectService.get(props.project.id);
      const replacing = Boolean(latest.source.videoPath);
      if (replacing && !confirmReplace()) {
        setConfirmReplace(true);
        return;
      }
      const imported = await projectService.importVideo(props.project.id, value.sourcePath, replacing);
      setCurrentProject(imported.project);
      if (imported.replaced) setCurrentTranscript(undefined);
      await save({ ...queue(), sampleLoaded: true, sampleChecksum: imported.project.source.checksumSha256 ?? null });
      setNotice({ kind: 'success', message: imported.duplicate ? 'Video mẫu đã có sẵn trong project.' : 'Đã nạp video mẫu. Hãy chỉnh transcript mode, vùng blur/subtitle, giọng và âm thanh trong project trước khi chạy batch.' });
      props.onClose();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Không thể nạp video mẫu vào project.');
    } finally { setSaving(false); }
  };

  const updateProgress = (update: BatchVideoRunUpdate) => {
    if (!alive) return;
    setCurrent(update);
    const { itemId, ...patch } = update;
    setQueue((prior) => ({ ...prior, items: prior.items.map((item) => item.id === itemId ? { ...item, ...patch } : item) }));
  };

  const start = async () => {
    if (running() || !queue().sampleLoaded) return;
    if (queue().items.length < 2) { setError('Thêm ít nhất hai video để bắt đầu xử lý hàng loạt.'); return; }
    setRunning(true); setCancelRequested(false); setBatchProcessingActive(true); setError(''); setCurrent(undefined);
    try {
      await runner.run(queue(), (next) => alive && setQueue(next), updateProgress);
      if (cancelRequested() && alive) setError('Đã hủy. Video đã hoàn tất được giữ lại; các video còn lại có thể chạy tiếp sau.');
      const latest = await projectService.get(props.project.id);
      if (alive) setCurrentProject(latest);
      const transcript = await projectService.getTranscript(props.project.id);
      if (alive) setCurrentTranscript(transcript ?? undefined);
    } catch (reason) {
      if (alive) setError(reason instanceof Error ? reason.message : 'Hàng đợi xử lý video gặp lỗi.');
    } finally {
      setBatchProcessingActive(false);
      alive && setRunning(false);
    }
  };

  const cancel = async () => {
    setCancelRequested(true);
    setError('Đang dừng tác vụ hiện tại…');
    await runner.cancel();
  };

  const openOutput = async () => {
    try { await renderService.openExportFolder(props.project.id, queue().outputDirectory); }
    catch (reason) { setError(reason instanceof Error ? reason.message : 'Không thể mở thư mục export.'); }
  };

  const close = () => { if (!running()) props.onClose(); };

  return <Modal wide title="Dịch hàng loạt" description="Xử lý nhiều video lần lượt trong project này. Video mẫu xác định cấu hình được dùng lại cho cả hàng đợi." onClose={close}>
    <div class="batch-video-dialog-content">
      <div class="batch-video-intro">
        <span class="batch-video-intro-icon"><Layers3 size={18} /></span>
        <div><strong>{queue().items.length} video trong hàng đợi</strong><p>Không tạo project mới. Mỗi video được xử lý đến bước Export và lưu thành file riêng trong thư mục export của project.</p></div>
        <Show when={queue().items.length > 0}><span class="batch-video-counter">{completedCount()} / {queue().items.length}</span></Show>
      </div>

      <div class="batch-video-sample-note"><Clapperboard size={15} /><span><strong>Video mẫu:</strong> Chọn một video, nạp vào project rồi chỉnh Transcript mode, OCR/blur, subtitle, logo, giọng và âm thanh bằng màn hình hiện tại. Cấu hình project được dùng lại cho các video kế tiếp.</span></div>

      <div class="batch-video-list-header"><h3>VIDEO QUEUE</h3><div><Show when={queue().items.length > 0}><button class="button button-quiet batch-video-clear-button" type="button" disabled={loading() || running() || saving()} onClick={() => void clearQueue()}><Trash2 size={13} /> Xóa hàng đợi</button></Show><button class="button button-quiet" type="button" disabled={loading() || running() || saving()} onClick={() => void addVideos()}><Plus size={14} /> Thêm video</button></div></div>
      <Show when={!loading()} fallback={<div class="batch-video-empty"><LoaderCircle size={15} class="spin" /> Đang đọc hàng đợi…</div>}>
        <Show when={ordered().length > 0} fallback={<div class="batch-video-empty"><FileVideo2 size={20} /><strong>Chưa có video</strong><span>Thêm các video muốn dịch; video mẫu có thể chọn trong danh sách.</span><button class="button button-primary" type="button" onClick={() => void addVideos()}><Plus size={14} /> Chọn video</button></div>}>
          <div class="batch-video-list">
            <For each={ordered()}>{(item) => {
              const isSample = () => item.id === queue().sampleId;
              const stage = () => current()?.itemId === item.id ? current()!.stage : item.stage;
              const progress = () => current()?.itemId === item.id ? current()!.progress : item.progress;
              const message = () => current()?.itemId === item.id ? current()!.message : item.message;
              return <article class="batch-video-item" classList={{ 'batch-video-item-sample': isSample(), 'batch-video-item-running': item.status === 'running', 'batch-video-item-failed': item.status === 'failed' }}>
                <div class="batch-video-index">{isSample() ? <Clapperboard size={15} /> : <FileVideo2 size={15} />}</div>
                <div class="batch-video-details">
                  <div class="batch-video-name-row"><strong title={item.fileName}>{item.fileName}</strong><Show when={isSample()}><span class="batch-video-sample-badge">VIDEO MẪU</span></Show></div>
                  <small>{item.status === 'completed' && item.outputPath ? `Đã xuất: ${fileName(item.outputPath)}` : message()}</small>
                  <Show when={item.status === 'running'}><div class="batch-video-progress" role="progressbar" aria-label={`Tiến trình ${item.fileName}`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(progress() * 100)}><span style={{ width: `${Math.round(progress() * 100)}%` }} /></div></Show>
                </div>
                <span class={`batch-video-status batch-video-status-${item.status}`}>{item.status === 'completed' ? <><Check size={12} /> Xong</> : item.status === 'running' ? <><LoaderCircle size={12} class="spin" /> {stage() ?? 'Đang chạy'}</> : item.status === 'failed' ? <><AlertTriangle size={12} /> Lỗi</> : item.status === 'cancelled' ? 'Đã hủy' : 'Đang chờ'}</span>
                <div class="batch-video-item-actions">
                  <Show when={!isSample() && !queue().sampleLoaded && !queue().items.some((entry) => entry.status === 'completed')}><button class="icon-button batch-video-action" type="button" title="Đặt làm video mẫu" aria-label={`Đặt ${item.fileName} làm video mẫu`} onClick={() => void setSample(item.id)}><ChevronUp size={15} /></button></Show>
                  <Show when={item.status === 'completed' && item.outputPath}><button class="icon-button batch-video-action" type="button" title="Mở thư mục export" aria-label="Mở thư mục export" onClick={() => void openOutput()}><FolderOpen size={15} /></button></Show>
                  <Show when={!running() && !(isSample() && queue().sampleLoaded)}><button class="icon-button batch-video-action batch-video-remove" type="button" title="Bỏ video khỏi hàng đợi" aria-label={`Bỏ ${item.fileName} khỏi hàng đợi`} onClick={() => void removeItem(item.id)}><Trash2 size={14} /></button></Show>
                </div>
              </article>;
            }}</For>
          </div>
        </Show>
      </Show>

      <Show when={queue().items.length > 0}><div class="batch-video-output-row"><div><FolderOpen size={14} /><span><strong>Thư mục export</strong><small title={queue().outputDirectory ?? undefined}>{queue().outputDirectory ?? 'Mặc định · project/exports'}</small></span></div><button class="button button-quiet" type="button" disabled={running() || saving()} onClick={() => void chooseOutputDirectory()}>Chọn thư mục</button></div></Show>

      <Show when={confirmReplace()}><div class="batch-video-warning" role="alert"><AlertTriangle size={15} /><span>Nạp video mẫu sẽ thay video hiện tại trong project và xóa transcript/audio phụ thuộc video hiện tại. Cấu hình blur, subtitle, logo và voice vẫn được giữ.</span><button class="button button-danger" type="button" disabled={saving()} onClick={() => void loadSample()}>Xác nhận thay</button></div></Show>
      <Show when={queue().sampleLoaded && queue().items.length < 2}><div class="batch-video-hint"><Check size={14} /> Video mẫu đã sẵn sàng. Hãy chỉnh cấu hình project rồi thêm ít nhất một video nữa để chạy.</div></Show>
      <Show when={queue().sampleLoaded && queue().items.length >= 2 && !running()}><div class="batch-video-hint"><Check size={14} /> Video mẫu đã nạp. Thay đổi cấu hình project ngay trong Workspace; batch sẽ áp dụng cấu hình mới nhất khi bắt đầu.</div></Show>
      <Show when={running()}><div class="batch-video-running" role="status"><LoaderCircle size={15} class="spin" /><div><strong>{current()?.stage ?? 'Đang chuẩn bị'}</strong><span>{current()?.message ?? 'Đang khởi chạy tác vụ…'}</span></div><strong>{completedCount()} / {queue().items.length}</strong></div></Show>
      <Show when={allDone()}><div class="batch-video-complete"><Check size={15} /><span>Đã export đủ {queue().items.length} video.</span><button class="button button-quiet" type="button" onClick={() => void openOutput()}><FolderOpen size={14} /> Mở thư mục export</button></div></Show>
      <Show when={failedCount() > 0 && !running()}><p class="batch-video-failed-count">{failedCount()} video gặp lỗi. Sửa cấu hình hoặc file nguồn rồi có thể chạy lại.</p></Show>
      <Show when={error()}><div class="export-alert export-alert-error" role="alert"><AlertTriangle size={14} /><span>{error()}</span></div></Show>
      <Show when={saving()}><div class="batch-video-saving"><LoaderCircle size={12} class="spin" /> Đang lưu hàng đợi…</div></Show>
    </div>

    <footer class="modal-actions batch-video-actions">
      <Show when={running()}><button class="button button-danger" type="button" onClick={() => void cancel()}><X size={14} /> Hủy hàng đợi</button></Show>
      <button class="button button-quiet" type="button" disabled={running()} onClick={close}>Đóng</button>
      <Show when={!running()}>
        <Show when={!queue().sampleLoaded && sample()}>
          <button class="button button-primary" type="button" disabled={loading() || saving()} onClick={() => void loadSample()}><FileVideo2 size={14} /> Nạp video mẫu</button>
        </Show>
        <Show when={queue().sampleLoaded && !allDone()}>
          <button class="button button-primary" type="button" disabled={loading() || saving() || pendingCount() === 0 || queue().items.length < 2} onClick={() => void start()}><Play size={14} /> {failedCount() ? 'Chạy lại video chưa xong' : 'Chạy đến Export'}</button>
        </Show>
        <Show when={allDone()}><button class="button button-quiet" type="button" onClick={() => void addVideos()}><RefreshCw size={14} /> Thêm đợt video</button></Show>
      </Show>
    </footer>
  </Modal>;
}

function normalizePath(path: string) {
  const normalized = path.replace(/\\/g, '/');
  return /^[a-z]:\//i.test(normalized) ? normalized.toLocaleLowerCase('en-US') : normalized;
}
