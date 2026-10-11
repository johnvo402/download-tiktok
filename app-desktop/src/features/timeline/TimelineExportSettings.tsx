import { Show } from 'solid-js';
import type { TimelineExportOptions } from '../../app/services/render-service';
import type { TimelineDocument } from './types';
import { selectedTimelineIds } from './timeline-state';
import { defaultTimelineExport, selectedExportRange } from './export-settings';
import { timelineDurationUs } from './time';

export default function TimelineExportSettings(props: { document: TimelineDocument; value: TimelineExportOptions; onChange: (value: TimelineExportOptions) => void }) {
  const change = (patch: Partial<TimelineExportOptions>) => props.onChange({ ...props.value, ...patch });
  const range = () => props.value.startUs !== null;
  const selected = () => selectedExportRange(props.document, selectedTimelineIds());
  const fps = () => props.value.frameRate.numerator / props.value.frameRate.denominator;
  return <div class="export-section">
    <h3>TIMELINE</h3>
    <p class="export-encoder-note">Xuất các lớp đang bật theo preview. Âm thanh dùng âm lượng, mute/solo, fade và ducking của từng clip. Vùng OCR/xóa chữ là hướng dẫn của nguồn; xử lý nguồn trong “Video nguồn” trước khi dựng.</p>
    <label class="export-quality"><span>Phạm vi xuất</span><select class="settings-select" value={range() ? 'range' : 'all'} onChange={e => change(e.currentTarget.value === 'all' ? { startUs: null, endUs: null } : selected() ?? { startUs: 0, endUs: timelineDurationUs(props.document.clips) })}>
      <option value="all">Toàn bộ timeline</option><option value="range">Vùng thời gian</option>
    </select></label>
    <Show when={range()}>
      <label class="export-quality"><span>Bắt đầu (giây)</span><input class="text-input" type="number" min="0" step="0.001" value={(props.value.startUs ?? 0) / 1e6} onInput={e => change({ startUs: Math.round(Number(e.currentTarget.value) * 1e6) })} /></label>
      <label class="export-quality"><span>Kết thúc (giây)</span><input class="text-input" type="number" min="0" step="0.001" value={(props.value.endUs ?? 0) / 1e6} onInput={e => change({ endUs: Math.round(Number(e.currentTarget.value) * 1e6) })} /></label>
      <button type="button" class="button button-quiet" disabled={!selected()} onClick={() => { const value = selected(); if (value) change(value); }}>Lấy vùng các clip đã chọn</button>
    </Show>
    <label class="export-quality"><span>Chiều rộng (pixel)</span><input class="text-input" type="number" min="2" max="8192" step="2" value={props.value.width} onInput={e => change({ width: Number(e.currentTarget.value) })} /></label>
    <label class="export-quality"><span>Chiều cao (pixel)</span><input class="text-input" type="number" min="2" max="8192" step="2" value={props.value.height} onInput={e => change({ height: Number(e.currentTarget.value) })} /></label>
    <label class="export-quality"><span>FPS đầu ra</span><select class="settings-select" value={String(fps())} onChange={e => {
      const v = Number(e.currentTarget.value);
      const projectRate = props.document.settings.frameRate;
      change({ frameRate: v === projectRate.numerator / projectRate.denominator ? { ...projectRate } : v === 30000 / 1001 ? { numerator: 30000, denominator: 1001 } : v === 60000 / 1001 ? { numerator: 60000, denominator: 1001 } : { numerator: v, denominator: 1 } });
    }}>
      <option value={String(props.document.settings.frameRate.numerator / props.document.settings.frameRate.denominator)}>Theo dự án ({(props.document.settings.frameRate.numerator / props.document.settings.frameRate.denominator).toFixed(3)})</option>
      <option value="24">24</option><option value="25">25</option><option value={String(30000 / 1001)}>29.97</option><option value="30">30</option><option value="50">50</option><option value={String(60000 / 1001)}>59.94</option><option value="60">60</option>
    </select></label>
    <div class="export-bitrate-actions"><button class="button button-quiet" type="button" onClick={() => props.onChange({ ...defaultTimelineExport(props.document), startUs: props.value.startUs, endUs: props.value.endUs })}>Theo khung hình dự án</button></div>
    <small class="export-encoder-note">Đổi tỷ lệ chiều rộng/cao sẽ kéo giãn toàn bộ khung hình. Chỉnh tỷ lệ canvas trong timeline để bố cục lại.</small>
  </div>;
}
