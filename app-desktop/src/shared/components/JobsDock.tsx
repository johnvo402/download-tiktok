import { createMemo, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { Activity, CheckCircle2, ChevronDown, CircleAlert, Clock3, LoaderCircle, RotateCcw, X } from 'lucide-solid';
import { jobService, type UnifiedJob, type UnifiedJobStatus } from '../../app/services/job-service';
import { setNotice } from '../../app/project-state';

type Filter = 'all' | 'active' | 'failed' | 'completed';

const jobLabel = (type: string) => ({
  studio: 'Tạo video AI',
  transcribe: 'Nhận dạng lời thoại', translate: 'Dịch', tts: 'Tạo giọng đọc', export: 'Xuất video', runtime_install: 'Cài bộ xử lý', model_install: 'Cài model',
} as Record<string, string>)[type] ?? type;
const statusLabel = (status: UnifiedJobStatus) => ({
  queued: 'Đang chờ', waiting: 'Chờ tài nguyên', running: 'Đang chạy', cancelling: 'Đang dừng…', completed: 'Hoàn tất',
  partial: 'Hoàn tất một phần', failed: 'Thất bại', cancelled: 'Đã hủy', interrupted: 'Bị gián đoạn',
}[status]);
const active = (job: UnifiedJob) => ['queued', 'waiting', 'running', 'cancelling'].includes(job.status);
const terminalSuccess = (job: UnifiedJob) => job.status === 'completed';

export default function JobsDock() {
  const [jobs, setJobs] = createSignal<UnifiedJob[]>([]);
  const [open, setOpen] = createSignal(false);
  const [filter, setFilter] = createSignal<Filter>('all');
  const [busy, setBusy] = createSignal<string>();
  let unlisten: (() => void) | undefined;

  const refresh = async () => {
    try { setJobs(await jobService.list()); } catch { /* Browser preview has no native queue. */ }
  };
  const merge = (next: UnifiedJob) => setJobs((current) => {
    const index = current.findIndex((job) => job.id === next.id);
    const updated = [...current];
    if (index >= 0) updated[index] = next;
    else updated.unshift(next);
    return updated.sort((a, b) => b.createdAt.localeCompare(a.createdAt)).slice(0, 220);
  });

  onMount(() => {
    void refresh();
    void jobService.onUpdated(merge).then((dispose) => { unlisten = dispose; });
  });
  onCleanup(() => unlisten?.());

  const activeCount = createMemo(() => jobs().filter(active).length);
  const filtered = createMemo(() => jobs().filter((job) => {
    if (filter() === 'active') return active(job);
    if (filter() === 'failed') return ['failed', 'partial', 'interrupted'].includes(job.status);
    if (filter() === 'completed') return ['completed', 'cancelled'].includes(job.status);
    return true;
  }));

  const cancel = async (job: UnifiedJob) => {
    setBusy(job.id);
    try {
      if (await jobService.cancel(job.id)) setNotice({ kind: 'info', message: `Đang dừng ${jobLabel(job.type)}…` });
      else setNotice({ kind: 'info', message: 'Job đã dừng hoặc không còn hoạt động.' });
    } catch (error) { setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể hủy job.' }); }
    finally { setBusy(undefined); }
  };

  const retry = async (job: UnifiedJob) => {
    setBusy(job.id);
    try {
      const id = await jobService.retry(job.id);
      await refresh();
      setOpen(true);
      setNotice({ kind: 'success', message: `Đã tạo lần thử mới (${id.slice(0, 8)}).` });
    } catch (error) { setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể thử lại job.' }); }
    finally { setBusy(undefined); }
  };

  return <div class="jobs-dock">
    <button class="jobs-dock-trigger" type="button" aria-expanded={open()} onClick={() => { setOpen(!open()); if (!open()) void refresh(); }}>
      <Activity size={15} /><span>Jobs</span><Show when={activeCount() > 0}><span class="jobs-dock-count">{activeCount()}</span></Show><ChevronDown size={14} classList={{ 'jobs-dock-chevron-open': open() }} />
    </button>
    <Show when={open()}>
      <section class="jobs-dock-panel" aria-label="Hàng đợi tác vụ">
        <header class="jobs-dock-header"><div><strong>Hàng đợi tác vụ</strong><small>{activeCount()} đang hoạt động · giữ 200 job gần nhất</small></div><button class="icon-button" type="button" aria-label="Đóng hàng đợi" onClick={() => setOpen(false)}><X size={15} /></button></header>
        <nav class="jobs-dock-filters" aria-label="Lọc job">
          <For each={[["all", "Tất cả"], ["active", "Đang chạy"], ["failed", "Cần chú ý"], ["completed", "Đã dừng"]] as const}>{([value, label]) => <button type="button" classList={{ active: filter() === value }} onClick={() => setFilter(value)}>{label}</button>}</For>
        </nav>
        <div class="jobs-dock-list">
          <Show when={filtered().length > 0} fallback={<div class="jobs-dock-empty"><CheckCircle2 size={20} /><span>Chưa có tác vụ trong mục này.</span></div>}>
            <For each={filtered().slice(0, 80)}>{(job) => <article class="jobs-dock-item">
              <div class="jobs-dock-item-icon" classList={{ 'job-icon-active': active(job), 'job-icon-error': ['failed', 'partial', 'interrupted'].includes(job.status), 'job-icon-success': terminalSuccess(job) }}>
                <Show when={active(job)} fallback={<Show when={['failed', 'partial', 'interrupted'].includes(job.status)} fallback={<CheckCircle2 size={15} />}><CircleAlert size={15} /></Show>}><LoaderCircle class="spin" size={15} /></Show>
              </div>
              <div class="jobs-dock-item-main">
                <div class="jobs-dock-item-title"><strong>{job.projectName ?? 'Ứng dụng'}</strong><span>{statusLabel(job.status)}</span></div>
                <div class="jobs-dock-item-subtitle"><span>{jobLabel(job.type)}</span><Show when={job.attempt > 1}><span>· Lần {job.attempt}</span></Show><Show when={job.resources.length}><span>· {[...new Set(job.resources.map(resource => (({cpu:'CPU',gpu:'GPU',ffmpeg:'Video',python:'AI','python-worker':'AI',disk:'Lưu trữ',network:'Mạng',browser:'Trình duyệt'} as Record<string,string>)[resource] ?? resource)))].join(', ')}</span></Show></div>
                <Show when={active(job)}><div class="jobs-dock-progress"><span style={{ width: `${Math.round(job.progress * 100)}%` }} /></div></Show>
                <small class="jobs-dock-message">{job.message}</small>
                <Show when={job.error}><small class="jobs-dock-error">{job.error!.code}: {job.error!.message}</small></Show>
                <small class="jobs-dock-time"><Clock3 size={11} /> {new Date(job.createdAt).toLocaleString('vi-VN')}</small>
              </div>
              <div class="jobs-dock-actions">
                <Show when={active(job) && job.status !== 'cancelling' && !['runtime_install', 'model_install'].includes(job.type)}><button class="icon-button" type="button" disabled={busy() === job.id} aria-label="Hủy job" title="Hủy" onClick={() => void cancel(job)}><X size={14} /></button></Show>
                <Show when={['failed', 'partial', 'cancelled', 'interrupted'].includes(job.status) && job.retryPayload}><button class="icon-button" type="button" disabled={busy() === job.id} aria-label="Thử lại job" title="Thử lại" onClick={() => void retry(job)}><Show when={busy() === job.id} fallback={<RotateCcw size={14} />}><LoaderCircle size={14} class="spin" /></Show></button></Show>
              </div>
            </article>}</For>
          </Show>
        </div>
      </section>
    </Show>
  </div>;
}

