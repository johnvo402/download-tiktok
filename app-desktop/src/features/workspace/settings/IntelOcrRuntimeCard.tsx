import { createSignal, onCleanup, onMount, Show } from 'solid-js';
import { Cpu } from 'lucide-solid';
import { optionalRuntimeService } from '../../../app/services/optional-runtime-service';
import type { OcrEnvironment } from '../../transcript/types';
import Modal from '../../../shared/components/Modal';

export default function IntelOcrRuntimeCard(props: {
  environment?: OcrEnvironment;
  confirm: boolean;
  onConfirmChange: (value: boolean) => void;
  onInstalled: () => Promise<void>;
}) {
  const [installing, setInstalling] = createSignal(false);
  const [progress, setProgress] = createSignal('');
  const [error, setError] = createSignal('');
  let unlisten: (() => void) | undefined;
  let disposed = false;
  onMount(() => {
    void optionalRuntimeService.onProgress(message => {
      if (installing()) setProgress(message);
    }).then(remove => { if (disposed) remove(); else unlisten = remove; });
  });
  onCleanup(() => { disposed = true; unlisten?.(); });
  const capability = () => props.environment?.backends?.openvino;
  const install = async () => {
    setInstalling(true);
    setError('');
    setProgress('Đang chuẩn bị tăng tốc Intel…');
    try {
      await optionalRuntimeService.install('ocr_openvino', 'openvino');
      await props.onInstalled();
      props.onConfirmChange(false);
    } catch (cause) {
      setError(typeof cause === 'string' ? cause : 'Không thể cài tăng tốc Intel. Kiểm tra kết nối, dung lượng trống và driver Intel.');
    } finally { setInstalling(false); }
  };
  return <section class="ocr-install-callout" aria-label="Tăng tốc Intel cho đọc chữ">
    <div class="ocr-install-copy">
      <strong><Cpu size={15} /> Tăng tốc Intel · OpenVINO</strong>
      <span>{capability()?.available ? `Sẵn sàng · ${capability()?.label} · ${capability()?.device}`
        : capability()?.installed ? capability()?.message ?? 'Đã cài, GPU Intel chưa sẵn sàng.'
        : 'Dùng Intel Iris Xe / Arc để đọc chữ. Cài khi cần; model OCR hiện có được dùng lại.'}</span>
      <small>Khoảng 85 MB cho tăng tốc. Lần đầu có thể cần thêm 150–250 MB cho các thành phần dùng chung.</small>
    </div>
    <button class="button button-quiet" type="button" disabled={installing()}
      onClick={() => props.onConfirmChange(true)}>{capability()?.available ? 'Kiểm tra bản cài' : capability()?.installed ? 'Sửa chữa tăng tốc Intel' : 'Cài tăng tốc Intel'}</button>
    <Show when={props.confirm}><Modal title="Cài tăng tốc Intel cho đọc chữ?"
      description="Cài các thành phần cần cho GPU Intel và kiểm tra tính toán thực tế trước khi sử dụng. Không cần CUDA; các model đã có được giữ nguyên."
      onClose={() => { if (!installing()) props.onConfirmChange(false); }}>
      <p class="provider-help">Cần Internet và dung lượng trống. Nếu đã cài tính năng AI khác, ứng dụng cần thêm dung lượng tạm để bảo vệ bản cài đang dùng.</p>
      <Show when={installing()}><div class="tts-install-progress" role="status"><span>{progress()}</span><div class="job-progress-track"><span class="job-progress-indeterminate" /></div></div></Show>
      <Show when={error()}><p class="provider-feedback provider-feedback-error" role="alert">{error()}</p></Show>
      <footer class="modal-actions"><button class="button button-quiet" type="button" disabled={installing()} onClick={() => props.onConfirmChange(false)}>Hủy</button>
        <button class="button button-primary" type="button" disabled={installing()} onClick={() => void install()}>{installing() ? 'Đang cài…' : 'Cài và kiểm tra Intel GPU'}</button></footer>
    </Modal></Show>
  </section>;
}
