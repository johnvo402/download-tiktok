import { createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { RefreshCw } from 'lucide-solid';
import { renderService } from '../../app/services/render-service';
import { encoderBackends, encoderLabels, encoderStatus, encoderSupport, type ExportCapabilities, type VideoCodec, type VideoEncoder } from '../../app/services/video-encoding';

export default function VideoEncoderSelect(props: {
  codec: VideoCodec;
  value: VideoEncoder;
  disabled?: boolean;
  resolvedLabel?: string;
  onChange: (encoder: VideoEncoder) => void;
  onRefreshed?: () => void;
}) {
  const [caps, setCaps] = createSignal<ExportCapabilities>();
  const [checking, setChecking] = createSignal(true);
  const [error, setError] = createSignal('');
  let alive = true;
  async function check(refresh: boolean) {
    setChecking(true); setError('');
    try {
      const value = await renderService.capabilities(refresh);
      if (!alive) return;
      setCaps(value);
      if (refresh) props.onRefreshed?.();
    } catch (error) {
      if (alive) setError(typeof error === 'string' ? error : 'Không kiểm tra được bộ mã hóa. Hãy thử lại.');
    } finally { if (alive) setChecking(false); }
  }
  onMount(() => void check(false));
  onCleanup(() => { alive = false; });
  return <div class="video-encoder-control">
    <label class="video-encoder-field"><span>Mã hóa bằng</span><select class="settings-select" value={props.value} disabled={props.disabled} onChange={(event) => props.onChange(event.currentTarget.value as VideoEncoder)}>
      <option value="auto">{encoderLabels.auto}</option>
      <For each={encoderBackends}>{(id) => <option value={id} disabled={checking() || !encoderSupport(caps(), props.codec, id)}>
        {encoderLabels[id]}{!checking() && !encoderSupport(caps(), props.codec, id) ? ' · không khả dụng' : ''}
      </option>}</For>
    </select></label>
    <div class="video-encoder-status"><small role="status">{error() || (checking() ? 'Đang kiểm tra encoder…' : props.value === 'auto' && props.resolvedLabel ? `Tự động → ${props.resolvedLabel}` : encoderStatus(caps(), props.codec, props.value))}</small>
      <button type="button" class="button button-quiet" aria-label="Kiểm tra lại bộ mã hóa" title="Kiểm tra lại bộ mã hóa" disabled={props.disabled || checking()} onClick={() => void check(true)}><RefreshCw size={14} /></button>
    </div>
    <Show when={caps() && !checking()}><details class="video-encoder-details"><summary>Khả năng mã hóa trên máy này</summary><For each={caps()?.encoders.filter((item) => !item[props.codec])}>{(item) => <small>{item.label}: {item[props.codec === 'h264' ? 'h264Message' : 'hevcMessage']}</small>}</For><Show when={caps()?.encoders.every((item) => item[props.codec])}><small>Tất cả bộ mã hóa đều sẵn sàng.</small></Show></details></Show>
  </div>;
}
