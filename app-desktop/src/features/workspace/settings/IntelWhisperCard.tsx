import { createResource, createSignal, onCleanup, onMount, Show } from 'solid-js';
import { listen } from '@tauri-apps/api/event';
import { optionalRuntimeService } from '../../../app/services/optional-runtime-service';
import { projectService } from '../../../app/services/project-service';
import type { AsrEnvironment } from '../../transcript/types';
import Modal from '../../../shared/components/Modal';

export default function IntelWhisperCard(props: { model: string; environment?: AsrEnvironment; onChanged: () => Promise<void> }) {
  const [action, setAction] = createSignal<'runtime' | 'model' | 'remove' | 'uninstall'>();
  const [busy, setBusy] = createSignal(false);
  const [plan] = createResource(() => action() === 'runtime', runtime => runtime ? optionalRuntimeService.plan('whisper_openvino', 'openvino') : undefined);
  const [message, setMessage] = createSignal('');
  const [error, setError] = createSignal('');
  let removers: (() => void)[] = [];
  let disposed = false;
  onMount(() => {
    const add = (remove: () => void) => disposed ? remove() : removers.push(remove);
    void optionalRuntimeService.onProgress(text => { if (busy()) setMessage(text); }).then(add);
    void listen<{ job: string; message?: string }>('asr-model-progress', event => {
      if (busy() && event.payload.message) setMessage(event.payload.message);
    }).then(add);
  });
  onCleanup(() => { disposed = true; removers.forEach(remove => remove()); });
  const size = () => `${((props.environment?.intelEstimatedDownloadBytes ?? 0) / 1048576).toFixed(0)} MB`;
  const execute = async () => {
    setBusy(true); setError(''); setMessage('Đang chuẩn bị…');
    try {
      if (action() === 'runtime') await optionalRuntimeService.install('whisper_openvino', 'openvino');
      else if (action() === 'uninstall') await optionalRuntimeService.uninstall('whisper_openvino');
      else if (action() === 'remove') await projectService.removeAsrModel(props.model, 'openvino');
      else await projectService.installAsrModel(props.model, 'openvino');
      await props.onChanged(); setAction(undefined);
    } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { setBusy(false); }
  };
  return <section class="ocr-install-callout" aria-label="Whisper Intel">
    <div class="ocr-install-copy"><strong>Whisper · Intel GPU</strong>
      <span>{props.environment?.intelGpuAvailable ? 'Tăng tốc Intel sẵn sàng.' : props.environment?.intelRuntimeInstalled ? 'Đã cài tăng tốc; GPU Intel chưa sẵn sàng. Kiểm tra driver Intel.' : 'Cài tăng tốc Intel khi cần. Dùng chung với tính năng đọc chữ; không cần NVIDIA.'}</span>
      <span>{props.environment?.intelModelSupported ? `Model ${props.model}: ${props.environment.intelModelCached ? 'đã cài' : `chưa cài · ${size()}`}` : 'Intel hiện hỗ trợ tiny, base và small.'}</span>
      <small>Model Intel và model CPU/NVIDIA là hai định dạng khác nhau. Chỉ tải bản bạn cần dùng.</small>
    </div>
    <div class="provider-action-row">
      <button class="button button-quiet" type="button" disabled={busy()} onClick={() => setAction('runtime')}>{props.environment?.intelRuntimeInstalled ? 'Kiểm tra / sửa tăng tốc Intel' : 'Cài tăng tốc Intel'}</button>
      <Show when={props.environment?.intelRuntimeInstalled}><button class="button button-quiet" type="button" disabled={busy()} onClick={() => setAction('uninstall')}>Gỡ bộ xử lý Intel</button></Show>
      <Show when={props.environment?.intelGpuAvailable && props.environment?.intelModelSupported}>
        <button class="button button-quiet" type="button" disabled={busy()} onClick={() => setAction(props.environment?.intelModelCached ? 'remove' : 'model')}>
          {props.environment?.intelModelCached ? `Gỡ model Intel ${props.model}` : `Tải model Intel ${props.model} · ${size()}`}
        </button>
      </Show>
    </div>
    <Show when={action()}><Modal title={action() === 'runtime' ? 'Cài tăng tốc Intel cho Whisper?' : action() === 'uninstall' ? 'Gỡ bộ nhận dạng Intel?' : action() === 'remove' ? `Gỡ Whisper Intel ${props.model}?` : `Tải Whisper Intel ${props.model}?`}
      description={action() === 'runtime' ? 'Chỉ tải các thành phần còn thiếu và kiểm tra trước khi thay bản đang dùng. Model được cài riêng.' : action() === 'uninstall' ? 'Chỉ gỡ bộ xử lý nhận dạng Intel. Model, bộ đọc chữ và transcript được giữ nguyên.' : action() === 'remove' ? 'Chỉ xóa model Intel. Model CPU/NVIDIA và transcript dự án được giữ nguyên.' : `Tải ${size()} rồi nạp và kiểm tra model bằng GPU Intel. Sau khi cài, nhận dạng chạy offline.`}
      onClose={() => { if (!busy()) setAction(undefined); }}>
      <Show when={action() === 'runtime' && plan()}>{value => <p class="provider-help">Tải thêm khoảng {((value().estimatedDownloadBytes) / 1048576).toFixed(0)} MiB · Cài thêm {((value().estimatedInstalledBytes) / 1048576).toFixed(0)} MiB · Bản sao tạm {((value().stagingBytes) / 1048576).toFixed(0)} MiB.</p>}</Show>
      <Show when={plan.error && action() === 'runtime'}><p class="provider-feedback provider-feedback-error">{String(plan.error)}</p></Show>
      <Show when={busy()}><div class="tts-install-progress" role="status"><span>{message()}</span><div class="job-progress-track"><span class="job-progress-indeterminate" /></div></div></Show>
      <Show when={error()}><p class="provider-feedback provider-feedback-error" role="alert">{error()}</p></Show>
      <footer class="modal-actions"><button class="button button-quiet" type="button" disabled={busy()} onClick={() => setAction(undefined)}>Hủy</button><button class="button button-primary" type="button" disabled={busy() || action() === 'runtime' && (plan.loading || !plan() || !!plan.error)} onClick={() => void execute()}>{busy() ? 'Đang xử lý…' : 'Xác nhận'}</button></footer>
    </Modal></Show>
  </section>;
}
