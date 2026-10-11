import { createEffect, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { currentProject, setNotice } from '../../app/project-state';
import { commitEdit, editBusy } from './edit-state';
import { defaultEffects, motionAt, motionKeys, validateEffects } from './effects';
import { clipDurationUs, rescaleTime } from './time';
import { seekTimeline, selectedTimelineIds, setTimelinePlaying, timelineCursorUs } from './timeline-state';
import { setEffectsDraft } from './visual-state';
import type { ClipTransition, MotionValues, VisualEffects } from './types';

export default function EffectsProperties() {
  const clip = () => currentProject()?.timeline.clips.find(c => c.id === selectedTimelineIds()[0]);
  const visual = () => clip() && !['audio', 'voice', 'sourceAudio'].includes(clip()!.content.kind);
  const locked = () => currentProject()?.timeline.tracks.find(t => t.id === clip()?.trackId)?.locked;
  const [effects, setEffects] = createSignal<VisualEffects>(defaultEffects());
  const [position, setPosition] = createSignal(0);
  const [values, setValues] = createSignal<MotionValues>({ x: .5, y: .5, scaleX: 1, scaleY: 1, opacity: 1 });
  const [error, setError] = createSignal('');
  const [transition, setTransition] = createSignal<ClipTransition['kind'] | 'none'>('none');
  const [transitionSeconds, setTransitionSeconds] = createSignal(.5);
  const [transitionError, setTransitionError] = createSignal('');
  function sourceAtPosition() {
    const c = clip();
    if (!c || !Number.isFinite(position()) || position() < 0 || position() * 1e6 > clipDurationUs(c)) throw new Error('Vị trí keyframe phải nằm trong clip.');
    return Math.min(c.sourceOutUs, c.sourceInUs + rescaleTime(Math.round(position() * 1e6), c.rate));
  }
  function loadPosition(seconds: number) {
    const c = clip(); if (!c) return;
    setPosition(seconds);
    try { setValues({ ...motionAt({ ...c, effects: effects() }, sourceAtPosition()) }); setError(''); }
    catch (e) { setError(e instanceof Error ? e.message : String(e)); }
  }
  createEffect(on(() => JSON.stringify([currentProject()?.id, clip()]), () => {
    const c = clip(); setEffects(structuredClone(c?.effects ?? defaultEffects())); setEffectsDraft(undefined); setError('');
    setTransition(c?.transitionIn?.kind ?? 'none'); setTransitionSeconds((c?.transitionIn?.durationUs ?? 500000) / 1e6); setTransitionError('');
    loadPosition(c ? Math.max(0, Math.min(clipDurationUs(c), timelineCursorUs() - c.startUs)) / 1e6 : 0);
  }));
  onCleanup(() => setEffectsDraft(undefined));
  function draft(next: VisualEffects) {
    const c = clip(), p = currentProject(); if (!c || !p || locked()) return;
    setEffects(next); setTimelinePlaying(false);
    try { validateEffects({ ...c, effects: next }); setEffectsDraft({ projectId: p.id, clipId: c.id, effects: next }); setError(''); }
    catch (e) { setEffectsDraft(undefined); setError(e instanceof Error ? e.message : String(e)); }
  }
  function putKeyframe() {
    const c = clip(); if (!c) return;
    try {
      const sourceUs = sourceAtPosition();
      const next = { ...effects(), keyframes: [...effects().keyframes.filter(k => k.sourceUs !== sourceUs), { sourceUs, ...values() }].sort((a, b) => a.sourceUs - b.sourceUs) };
      validateEffects({ ...c, effects: next }); draft(next); seekTimeline(c.startUs + Math.min(clipDurationUs(c) - 1, Math.round(position() * 1e6)));
    } catch (e) { setError(e instanceof Error ? e.message : String(e)); }
  }
  function removeKeyframe() {
    try { const at = sourceAtPosition(); draft({ ...effects(), keyframes: effects().keyframes.filter(k => k.sourceUs !== at) }); }
    catch (e) { setError(e instanceof Error ? e.message : String(e)); }
  }
  async function apply() {
    const c = clip(); if (!c) return;
    try {
      validateEffects({ ...c, effects: effects() });
      if (await commitEdit({ kind: 'effects', id: c.id, effects: effects() }, 'Fade hình và keyframe')) {
        setEffectsDraft(undefined); setNotice({ kind: 'success', message: 'Đã lưu fade và keyframe.' });
      }
    } catch (e) { setError(e instanceof Error ? e.message : String(e)); }
  }
  async function applyTransition() {
    const c = clip(); if (!c) return;
    const kind = transition(); setTransitionError('');
    const ok = await commitEdit({ kind: 'transition', id: c.id, transition: kind === 'none' ? null : { kind, durationUs: Math.round(transitionSeconds() * 1e6) } }, 'Chuyển cảnh');
    if (!ok) setTransitionError('Chưa áp dụng được. Kiểm tra hai clip liền nhau, thời lượng, nhóm và track khóa; xem thông báo lỗi.');
  }
  const names: Record<keyof MotionValues, string> = { x: 'Keyframe tâm X (%)', y: 'Keyframe tâm Y (%)', scaleX: 'Keyframe kích thước X (%)', scaleY: 'Keyframe kích thước Y (%)', opacity: 'Keyframe độ mờ (%)' };
  return <Show when={visual()}>
    <form class="property-group timeline-effects-properties" id="timeline-effects-properties" onSubmit={e => { e.preventDefault(); void apply(); }}>
      <h4>Fade hình & keyframe</h4><p class="overlay-selection-hint">Keyframe bám thời gian nguồn, nội suy tuyến tính. Nhập giá trị, thêm/cập nhật điểm rồi Áp dụng để lưu một bước undo. Âm thanh có fade riêng trong Âm thanh.</p>
      <fieldset disabled={editBusy() || locked()}>
        <div class="timeline-property-grid"><label>Fade vào (giây)<input aria-label="Fade hình vào" type="number" min="0" step="0.05" value={effects().fadeInUs / 1e6} onInput={e => draft({ ...effects(), fadeInUs: Math.round(e.currentTarget.valueAsNumber * 1e6) })} /></label><label>Fade ra (giây)<input aria-label="Fade hình ra" type="number" min="0" step="0.05" value={effects().fadeOutUs / 1e6} onInput={e => draft({ ...effects(), fadeOutUs: Math.round(e.currentTarget.valueAsNumber * 1e6) })} /></label></div>
        <label>Vị trí keyframe (giây từ đầu clip)<input aria-label="Vị trí keyframe" type="number" min="0" max={clip() ? clipDurationUs(clip()!) / 1e6 : 0} step="0.001" value={position()} onInput={e => loadPosition(e.currentTarget.valueAsNumber)} /></label>
        <button type="button" class="button button-quiet" onClick={() => { const c = clip(); if (c) loadPosition(Math.max(0, Math.min(clipDurationUs(c), timelineCursorUs() - c.startUs)) / 1e6); }}>Lấy vị trí playhead</button>
        <div class="timeline-property-grid"><For each={motionKeys}>{key => <label>{names[key]}<input aria-label={names[key]} type="number" step="0.1" min={key.startsWith('scale') ? .1 : key === 'opacity' ? 0 : undefined} max={key === 'opacity' ? 100 : key.startsWith('scale') ? 10000 : undefined} value={values()[key] * 100} onInput={e => { setValues(v => ({ ...v, [key]: e.currentTarget.valueAsNumber / 100 })); setTimelinePlaying(false); }} /></label>}</For></div>
        <div class="timeline-property-actions"><button type="button" class="button button-outline" onClick={putKeyframe}>Thêm/cập nhật keyframe</button><button type="button" class="button button-quiet" onClick={removeKeyframe}>Xóa điểm tại vị trí</button><button type="button" class="button button-quiet" disabled={!effects().keyframes.length} onClick={() => draft({ ...effects(), keyframes: [] })}>Xóa toàn bộ keyframe</button></div>
        <ul class="timeline-keyframe-list" aria-label="Các keyframe"><For each={effects().keyframes}>{k => {
          const seconds = () => (k.sourceUs - clip()!.sourceInUs) / 1e6 * clip()!.rate.denominator / clip()!.rate.numerator;
          const outside = () => k.sourceUs < clip()!.sourceInUs || k.sourceUs > clip()!.sourceOutUs;
          return <li><button type="button" disabled={outside()} onClick={() => { loadPosition(seconds()); seekTimeline(clip()!.startUs + Math.min(clipDurationUs(clip()!) - 1, Math.round(seconds() * 1e6))); }}>◆ {seconds().toFixed(3)}s{outside() ? ' · Ngoài trim' : ''}</button><button type="button" aria-label={`Xóa keyframe nguồn ${k.sourceUs}`} onClick={() => draft({ ...effects(), keyframes: effects().keyframes.filter(p => p.sourceUs !== k.sourceUs) })}>Xóa</button></li>;
        }}</For></ul>
        <Show when={error()}><p role="alert" class="timeline-property-error">{error()}</p></Show>
        <div class="timeline-property-actions"><button type="button" class="button button-quiet" onClick={() => { draft(structuredClone(clip()?.effects ?? defaultEffects())); setEffectsDraft(undefined); loadPosition(position()); }}>Hủy xem thử hiệu ứng</button><button type="submit" class="button button-primary" disabled={!!error()}>Áp dụng hiệu ứng</button></div>
      </fieldset>
    </form>
    <Show when={clip()?.content.kind === 'video' || clip()?.content.kind === 'image'}>
      <form class="property-group" onSubmit={e => { e.preventDefault(); void applyTransition(); }}>
        <h4>Chuyển cảnh từ clip trước</h4><p class="overlay-selection-hint">Hai video/ảnh liền nhau trên cùng track. Chồng phần cuối/đầu trong khoảng nguồn đã trim; B và các clip phía sau cùng track dịch lên, lớp liên kết đi theo. Các lớp độc lập giữ vị trí.</p>
        <fieldset disabled={editBusy() || locked()}>
          <label>Kiểu chuyển cảnh<select aria-label="Kiểu chuyển cảnh" value={transition()} onChange={e => setTransition(e.currentTarget.value as ClipTransition['kind'] | 'none')}><option value="none">Không có</option><option value="dissolve">Hòa tan</option><option value="wipeLeft">Quét từ trái sang phải</option></select></label>
          <label>Thời lượng (giây)<input aria-label="Thời lượng chuyển cảnh" type="number" min="0.001" step="0.001" value={transitionSeconds()} disabled={transition() === 'none'} onInput={e => setTransitionSeconds(e.currentTarget.valueAsNumber)} /></label>
          <p class="overlay-selection-hint">Tiếng gốc/giọng được crossfade. Gỡ chuyển cảnh trước khi cắt, đổi tốc độ hoặc di chuyển riêng một phía; gỡ sẽ trả lại vị trí các clip phía sau.</p>
          <Show when={transitionError()}><p role="alert" class="timeline-property-error">{transitionError()}</p></Show>
          <button type="submit" class="button button-outline" disabled={transition() === 'none' && !clip()?.transitionIn}>Áp dụng chuyển cảnh</button>
        </fieldset>
      </form>
    </Show>
  </Show>;
}
