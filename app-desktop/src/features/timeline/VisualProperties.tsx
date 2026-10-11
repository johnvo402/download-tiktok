import { createEffect, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { currentProject, setNotice } from '../../app/project-state';
import { commitEdit, editBusy } from './edit-state';
import { seekTimeline, selectedTimelineIds, setTimelinePlaying, timelineCursorUs, timelinePreviewActive } from './timeline-state';
import { clipTypography, defaultStyle, validateCanvas, validateStyle, validateTransform } from './visual-layout';
import { setCanvasDraft, setVisualDraft } from './visual-state';
import type { ClipStyle, ClipTransform, TimelineSettings } from './types';
import EffectsProperties from './EffectsProperties';

export default function VisualProperties() {
  const clip = () => currentProject()?.timeline.clips.find(c => c.id === selectedTimelineIds()[0]);
  const visual = () => clip() && !['audio', 'voice', 'sourceAudio'].includes(clip()!.content.kind);
  const typography = () => clip()?.content.kind === 'text' || clip()?.content.kind === 'subtitles';
  const locked = () => currentProject()?.timeline.tracks.find(t => t.id === clip()?.trackId)?.locked;
  const [transform, setTransform] = createSignal<ClipTransform>();
  const [style, setStyle] = createSignal(defaultStyle());
  const [text, setText] = createSignal('');
  const [canvas, setCanvas] = createSignal<TimelineSettings>();
  const [clipError, setClipError] = createSignal(''), [canvasError, setCanvasError] = createSignal('');
  let errorElement!: HTMLParagraphElement, canvasErrorElement!: HTMLParagraphElement;
  createEffect(on(() => JSON.stringify([currentProject()?.id, clip(), clip() ? clipTypography(clip()!, currentProject()?.timeline) : null]), () => {
    const c = clip(); setTransform(c ? structuredClone(c.transform) : undefined); setStyle(structuredClone(c ? clipTypography(c, currentProject()?.timeline) : defaultStyle()));
    setText(c?.content.kind === 'text' ? c.content.text : ''); setClipError(''); setVisualDraft(undefined);
  }));
  createEffect(on(() => JSON.stringify([currentProject()?.id, currentProject()?.timeline.settings]), () => {
    setCanvas(currentProject() ? structuredClone(currentProject()!.timeline.settings) : undefined); setCanvasError(''); setCanvasDraft(undefined);
  }));
  onCleanup(() => { setVisualDraft(undefined); setCanvasDraft(undefined); });
  function draft() {
    const c = clip(), t = transform(), project = currentProject(); if (!c || !t || !project || locked()) return;
    try {
      validateTransform(t); if (typography()) validateStyle(style());
      if (!timelinePreviewActive()) seekTimeline(timelineCursorUs()); setTimelinePlaying(false);
      setVisualDraft({ projectId: project.id, clip: { ...c, transform: structuredClone(t), ...(typography() ? { style: structuredClone(style()) } : {}), content: c.content.kind === 'text' ? { kind: 'text', text: text() } : c.content } }); setClipError('');
    } catch (e) { setClipError(e instanceof Error ? e.message : String(e)); }
  }
  function update(changes: Partial<ClipTransform>) { setTransform(t => t ? { ...t, ...changes } : t); draft(); }
  function updateStyle(changes: Partial<ClipStyle>) { setStyle(s => ({ ...s, ...changes })); draft(); }
  function updateCanvas(changes: Partial<TimelineSettings>) {
    const s = { ...canvas()!, ...changes }; setCanvas(s);
    try { validateCanvas(s); if (!timelinePreviewActive()) seekTimeline(timelineCursorUs()); setTimelinePlaying(false); setCanvasDraft({ projectId: currentProject()!.id, settings: s }); setCanvasError(''); }
    catch (e) { setCanvasError(e instanceof Error ? e.message : String(e)); }
  }
  async function applyClip() {
    const c = clip(), t = transform(); if (!c || !t) return;
    if (JSON.stringify(c.transform) === JSON.stringify(t) && (!typography() || JSON.stringify(clipTypography(c, currentProject()?.timeline)) === JSON.stringify(style())) && (c.content.kind !== 'text' || c.content.text === text())) { setVisualDraft(undefined); setClipError(''); return; }
    const done = await commitEdit({ kind: 'visual', id: c.id, transform: t, ...(typography() ? { style: style() } : {}), ...(c.content.kind === 'text' ? { text: text() } : {}) }, 'Chỉnh hình ảnh clip');
    if (done) { setVisualDraft(undefined); setNotice({ kind: 'success', message: 'Đã lưu thuộc tính clip.' }); }
    else { setClipError('Chưa lưu được. Kiểm tra thông báo và các giá trị đã nhập.'); errorElement?.focus(); }
  }
  async function applyCanvas() {
    if (!canvas()) return;
    if (JSON.stringify(canvas()) === JSON.stringify(currentProject()?.timeline.settings)) { setCanvasDraft(undefined); return; }
    if (await commitEdit({ kind: 'canvas', settings: canvas()! }, 'Chỉnh khung hình dự án')) { setCanvasDraft(undefined); setNotice({ kind: 'success', message: 'Đã lưu khung hình dự án.' }); }
    else { setCanvasError('Chưa lưu được khung hình. Kiểm tra thông báo lỗi.'); canvasErrorElement?.focus(); }
  }
  const preset = () => canvas()?.width === 1920 && canvas()?.height === 1080 ? '16:9' : canvas()?.width === 1080 && canvas()?.height === 1920 ? '9:16' : canvas()?.width === 1080 && canvas()?.height === 1080 ? '1:1' : 'custom';
  return <div class="timeline-visual-properties">
    <form class="property-group" onSubmit={event => { event.preventDefault(); void applyCanvas(); }}>
      <h4>Khung hình dự án</h4><p class="overlay-selection-hint">Tỷ lệ đầu ra riêng với vùng crop của mỗi clip. Áp dụng để lưu; các giá trị đang nhập được xem thử.</p>
      <label>Tỷ lệ khung hình<select aria-label="Tỷ lệ khung hình dự án" value={preset()} disabled={editBusy()} onChange={event => { const sizes: Record<string, [number, number]> = { '16:9': [1920, 1080], '9:16': [1080, 1920], '1:1': [1080, 1080] }; const size = sizes[event.currentTarget.value]; if (size) updateCanvas({ width: size[0], height: size[1] }); }}><option value="custom">Tùy chỉnh</option><option>16:9</option><option>9:16</option><option>1:1</option></select></label>
      <div class="timeline-property-grid"><label>Rộng (px)<input aria-label="Rộng khung hình" type="number" required min="1" max="16384" value={canvas()?.width ?? 1920} disabled={editBusy()} onInput={e => updateCanvas({ width: e.currentTarget.valueAsNumber })} /></label><label>Cao (px)<input aria-label="Cao khung hình" type="number" required min="1" max="16384" value={canvas()?.height ?? 1080} disabled={editBusy()} onInput={e => updateCanvas({ height: e.currentTarget.valueAsNumber })} /></label></div>
      <label>Màu nền<input aria-label="Màu nền khung hình" type="color" value={canvas()?.backgroundColor ?? '#000000'} disabled={editBusy()} onInput={e => updateCanvas({ backgroundColor: e.currentTarget.value })} /></label>
      <Show when={canvasError()}><p ref={canvasErrorElement} tabindex="-1" role="alert" class="timeline-property-error">{canvasError()}</p></Show>
      <div class="timeline-property-actions"><button type="button" class="button button-quiet" disabled={editBusy()} onClick={() => { setCanvas(structuredClone(currentProject()!.timeline.settings)); setCanvasDraft(undefined); setCanvasError(''); }}>Hủy xem thử</button><button class="button button-outline" type="submit" disabled={editBusy() || !!canvasError()}>Áp dụng khung hình</button></div>
    </form>
    <Show when={visual() && transform()} fallback={<p class="overlay-selection-hint">Chọn clip video, ảnh/logo, chữ hoặc phụ đề trên timeline để chỉnh hình ảnh.</p>}>
      <form class="property-group" onSubmit={e => { e.preventDefault(); void applyClip(); }}>
        <h4>Hình ảnh clip {locked() ? '· Đang khóa' : ''}</h4><p class="overlay-selection-hint">Chỉnh clip đầu tiên trong lựa chọn. Logo dùng ảnh PNG trong kho nguồn; dữ liệu OCR và xóa chữ vẫn theo nguồn.</p>
        <Show when={clip()?.effects?.keyframes.length}><p class="overlay-selection-hint">Vị trí, scale và độ mờ đang do keyframe điều khiển. Chỉnh các điểm trong Hiệu ứng; xóa toàn bộ keyframe để dùng giá trị hình ảnh cơ bản.</p></Show>
        <fieldset disabled={editBusy() || locked()}>
          <div class="timeline-property-grid"><For each={[['x', 'Tâm X (%)'], ['y', 'Tâm Y (%)'], ['scaleX', 'Kích thước X (%)'], ['scaleY', 'Kích thước Y (%)'], ['rotation', 'Xoay (độ)'], ['opacity', 'Độ mờ (%)']] as const}>{([key, name]) => <label>{name}<input aria-label={name} type="number" required step="0.1" min={key.startsWith('scale') ? .1 : key === 'opacity' ? 0 : undefined} max={key === 'opacity' ? 100 : key.startsWith('scale') ? 10000 : undefined} value={transform()![key] * (key === 'rotation' ? 1 : 100)} onInput={e => update({ [key]: e.currentTarget.valueAsNumber / (key === 'rotation' ? 1 : 100) })} /></label>}</For></div>
          <label>Cách lấp khung<select aria-label="Cách lấp khung clip" value={transform()!.fit} onChange={e => update({ fit: e.currentTarget.value as ClipTransform['fit'] })}><option value="contain">Fit · giữ toàn bộ</option><option value="cover">Fill · lấp đầy</option><option value="stretch">Kéo giãn</option></select></label>
          <div class="timeline-property-grid"><label class="timeline-property-check"><input type="checkbox" checked={transform()!.flipX} onChange={e => update({ flipX: e.currentTarget.checked })} /> Lật ngang</label><label class="timeline-property-check"><input type="checkbox" checked={transform()!.flipY} onChange={e => update({ flipY: e.currentTarget.checked })} /> Lật dọc</label></div>
          <h5>Crop theo nguồn (%)</h5><div class="timeline-property-grid"><For each={[['x', 'Crop X'], ['y', 'Crop Y'], ['width', 'Crop rộng'], ['height', 'Crop cao']] as const}>{([key, name]) => <label>{name}<input aria-label={name} type="number" required min={key === 'x' || key === 'y' ? 0 : .1} max="100" step="0.1" value={transform()!.crop[key] * 100} onInput={e => update({ crop: { ...transform()!.crop, [key]: e.currentTarget.valueAsNumber / 100 } })} /></label>}</For></div>
          <Show when={clip()?.content.kind === 'text'}><label>Nội dung chữ<textarea aria-label="Nội dung lớp chữ" required maxlength="10000" value={text()} onInput={e => { setText(e.currentTarget.value); draft(); }} /></label></Show>
          <Show when={typography()}><h5>Chữ / phụ đề của clip</h5><label>Font<input aria-label="Font lớp chữ" maxlength="96" required value={style().fontFamily} onInput={e => updateStyle({ fontFamily: e.currentTarget.value })} /></label><div class="timeline-property-grid"><label>Cỡ chữ (px đầu ra)<input aria-label="Cỡ chữ clip" type="number" min="8" max="240" required value={style().fontSize} onInput={e => updateStyle({ fontSize: e.currentTarget.valueAsNumber })} /></label><label>Độ đậm<select aria-label="Độ đậm chữ clip" value={style().fontWeight} onChange={e => updateStyle({ fontWeight: Number(e.currentTarget.value) })}><option value="400">Thường</option><option value="700">Đậm</option><option value="900">Rất đậm</option></select></label><label>Màu chữ<input aria-label="Màu chữ clip" type="color" value={style().color} onInput={e => updateStyle({ color: e.currentTarget.value })} /></label><label>Màu nền chữ<input aria-label="Màu nền chữ clip" type="color" value={style().backgroundColor} onInput={e => updateStyle({ backgroundColor: e.currentTarget.value })} /></label></div><label>Độ mờ nền chữ (%)<input aria-label="Độ mờ nền chữ" type="number" required min="0" max="100" value={style().backgroundOpacity * 100} onInput={e => updateStyle({ backgroundOpacity: e.currentTarget.valueAsNumber / 100 })} /></label><label>Căn chữ<select aria-label="Căn chữ clip" value={style().alignment} onChange={e => updateStyle({ alignment: e.currentTarget.value as ClipStyle['alignment'] })}><option value="left">Trái</option><option value="center">Giữa</option><option value="right">Phải</option></select></label></Show>
          <Show when={clipError()}><p ref={errorElement} tabindex="-1" role="alert" class="timeline-property-error">{clipError()}</p></Show>
          <div class="timeline-property-actions"><button type="button" class="button button-quiet" onClick={() => { const c = clip()!; setTransform(structuredClone(c.transform)); setStyle(structuredClone(clipTypography(c, currentProject()?.timeline))); setText(c.content.kind === 'text' ? c.content.text : ''); setVisualDraft(undefined); setClipError(''); }}>Hủy xem thử</button><button class="button button-primary" type="submit" disabled={!!clipError()}>Áp dụng clip</button></div>
        </fieldset>
      </form>
    </Show>
    <EffectsProperties />
  </div>;
}
