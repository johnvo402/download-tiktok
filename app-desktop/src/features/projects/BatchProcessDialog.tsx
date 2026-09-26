import { createMemo, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { AlertTriangle, Check, Layers3, LoaderCircle, Play, ShieldCheck, X } from 'lucide-solid';
import Modal from '../../shared/components/Modal';
import type { Project } from './types';
import { BatchWorkflowRunner, validateBatchProjects, type BatchProgress, type BatchSteps } from './batch-workflow';

const defaultSteps: BatchSteps = { transcriptIfMissing: true, translateMissing: true, generateMissingVoices: true, export: true };

export default function BatchProcessDialog(props: { projectIds: string[]; onClose: () => void }) {
  const [steps, setSteps] = createSignal<BatchSteps>({ ...defaultSteps });
  const [checks, setChecks] = createSignal<Array<{ project: Project; valid: boolean; reasons: string[] }>>([]);
  const [checking, setChecking] = createSignal(true);
  const [running, setRunning] = createSignal(false);
  const [done, setDone] = createSignal(false);
  const [progress, setProgress] = createSignal<BatchProgress[]>([]);
  const [current, setCurrent] = createSignal<BatchProgress>();
  const [error, setError] = createSignal('');
  const runner = new BatchWorkflowRunner();
  let active = true;

  const validate = async (nextSteps = steps()) => {
    setChecking(true); setError('');
    try { setChecks(await validateBatchProjects(props.projectIds, nextSteps)); }
    catch (reason) { setError(reason instanceof Error ? reason.message : String(reason)); }
    finally { setChecking(false); }
  };
  onMount(() => void validate());
  onCleanup(() => { active = false; if (running()) void runner.cancel(); });

  const valid = createMemo(() => checks().filter((item) => item.valid));
  const invalid = createMemo(() => checks().filter((item) => !item.valid));
  const completedCount = createMemo(() => progress().filter((item) => item.status === 'completed').length);
  const failedCount = createMemo(() => progress().filter((item) => item.status === 'failed').length);
  const updateStep = (key: keyof BatchSteps, value: boolean) => {
    const next = { ...steps(), [key]: value };
    setSteps(next);
    void validate(next);
  };

  const start = async () => {
    if (!valid().length) return;
    setRunning(true); setDone(false); setError(''); setProgress([]);
    try {
      await runner.run(valid().map((item) => item.project), steps(), (item) => {
        setCurrent(item);
        if (item.status !== 'running') setProgress((all) => [...all.filter((entry) => entry.projectId !== item.projectId), item]);
      });
      if (!active) return;
      setDone(true);
    } catch (reason) {
      if (active) setError(reason instanceof Error ? reason.message : String(reason));
      setDone(true);
    } finally { if (active) setRunning(false); }
  };

  const cancel = async () => {
    await runner.cancel();
    setRunning(false); setDone(true);
  };

  const close = () => { if (running()) void cancel(); props.onClose(); };

  return <Modal title="Xử lý nhiều project" description={`${props.projectIds.length} project riêng được chọn. Mỗi project dùng nguồn video và cấu hình của chính nó; một project lỗi không dừng project kế tiếp.`} onClose={close}>
    <div class="batch-dialog-content">
      <div class="batch-workflow-card"><div class="batch-workflow-icon"><Layers3 size={17} /></div><div><strong>Full Localization</strong><small>Mỗi project dùng cấu hình ngôn ngữ, voice, subtitle và export riêng.</small></div><span class="batch-project-count">{props.projectIds.length} PROJECT</span></div>
      <section class="batch-step-section"><h3>WORKFLOW STEPS</h3>
        <label><input type="checkbox" checked={steps().transcriptIfMissing} disabled={running()} onChange={(event) => updateStep('transcriptIfMissing', event.currentTarget.checked)} /><span><strong>Tạo transcript nếu chưa có</strong><small>Ưu tiên một text subtitle track; nếu không, dùng faster-whisper đã cài.</small></span></label>
        <label><input type="checkbox" checked={steps().translateMissing} disabled={running()} onChange={(event) => updateStep('translateMissing', event.currentTarget.checked)} /><span><strong>Dịch phần còn thiếu</strong><small>Dùng provider và target language đang cấu hình trong project/settings.</small></span></label>
        <label><input type="checkbox" checked={steps().generateMissingVoices} disabled={running()} onChange={(event) => updateStep('generateMissingVoices', event.currentTarget.checked)} /><span><strong>Tạo voice còn thiếu</strong><small>Chỉ tạo audio cho segment chưa có bản TTS hợp lệ.</small></span></label>
        <label><input type="checkbox" checked={steps().export} disabled={running()} onChange={(event) => updateStep('export', event.currentTarget.checked)} /><span><strong>Export video</strong><small>Dùng quality mặc định và các thiết lập visual/audio của từng project.</small></span></label>
      </section>

      <div class="batch-validation-header"><h3>PROJECT VALIDATION</h3><button class="button button-quiet" type="button" disabled={checking() || running()} onClick={() => void validate()}>{checking() ? 'Đang kiểm tra…' : 'Kiểm tra lại'}</button></div>
      <div class="batch-validation-list">
        <Show when={!checking()} fallback={<div class="batch-checking"><LoaderCircle size={14} class="spin" /> Đang kiểm tra project, transcript và vùng blur…</div>}>
          <For each={checks()}>{(item) => <article class="batch-validation-item" classList={{ 'batch-invalid': !item.valid }}><span class="batch-validation-mark">{item.valid ? <Check size={13} /> : <AlertTriangle size={13} />}</span><div><strong>{item.project.name}</strong><Show when={!item.valid}><small>{item.reasons.join(' ')}</small></Show><Show when={item.valid}><small>Sẵn sàng đưa vào workflow.</small></Show></div><span class="batch-validation-state">{item.valid ? 'READY' : 'CHECK'}</span></article>}</For>
          <Show when={invalid().length > 0}><p class="batch-partial-note"><ShieldCheck size={13} /> Chỉ project hợp lệ được chạy. Project lỗi sẽ được bỏ qua để các project còn lại tiếp tục.</p></Show>
          <Show when={checks().length === 0}><p class="batch-empty-note">Không có project để xử lý.</p></Show>
        </Show>
      </div>

      <Show when={running() && current()}>{(item) => <div class="batch-running-state" role="status"><LoaderCircle size={14} class="spin" /><div><strong>{item().projectName} · {item().step}</strong><small>{item().message} · project {item().projectIndex}/{item().projectCount}</small></div></div>}</Show>
      <Show when={done()}><div class="batch-complete-state"><Check size={15} /><span>{completedCount()} hoàn tất{failedCount() ? ` · ${failedCount()} lỗi` : ''}{invalid().length ? ` · ${invalid().length} bỏ qua` : ''}</span></div></Show>
      <Show when={error()}><div class="export-alert export-alert-error" role="alert"><AlertTriangle size={14} /><span>{error()}</span></div></Show>
    </div>
    <footer class="modal-actions batch-actions">
      <Show when={running()}><button class="button button-danger" type="button" onClick={() => void cancel()}><X size={14} /> Hủy batch</button></Show>
      <button class="button button-quiet" type="button" onClick={close}>{running() ? 'Đóng và hủy' : 'Đóng'}</button>
      <Show when={!running()}><button class="button button-primary" type="button" disabled={checking() || !valid().length || !Object.values(steps()).some(Boolean) || done()} onClick={() => void start()}><Play size={14} /> {done() ? 'Đã xử lý' : `Bắt đầu ${valid().length} project`}</button></Show>
    </footer>
  </Modal>;
}
