import { createEffect, createSignal, For, on, Show } from 'solid-js';
import { currentProject, setNotice } from '../../app/project-state';
import { flushPendingSaves } from '../../app/save-guard';
import { ttsService } from '../../app/services/tts-service';
import { clipAudio, clipWarnings, contentId, hasAudio, usesSourceVoice, type ClipWarning } from './audio-layout';
import { commitEdit, editBusy, editNotice } from './edit-state';
import { sourceMedia, watchSourceMedia, refreshSourceMedia } from './source-media';
import { selectedTimelineIds, setTimelinePlaying } from './timeline-state';
import { clipEndUs, secondsToTimeUs } from './time';
import type { ClipAudio } from './types';
import { rememberVideoJob, videoJobEvents, isRunningVideoJob } from './video-context-state';

export default function AudioProperties() {
  const clip = () => currentProject()?.timeline.clips.find(c => c.id === selectedTimelineIds()[0]);
  const id = () => clip() ? contentId(clip()!) : undefined;
  watchSourceMedia(() => id() ? [id()!] : []);
  const [audio, setAudio] = createSignal<ClipAudio>();
  const [speed, setSpeed] = createSignal(1), [edge, setEdge] = createSignal<'start' | 'end'>('end'), [cut, setCut] = createSignal(0);
  const [starting, setStarting] = createSignal(false), [message, setMessage] = createSignal('');
  const [ttsIds, setTtsIds] = createSignal<string[]>([]);
  createEffect(on(() => JSON.stringify([currentProject()?.id, clip(), clip() ? clipAudio(clip()!, currentProject()?.timeline) : null]), () => {
    const c = clip(); setAudio(c ? { ...clipAudio(c, currentProject()?.timeline) } : undefined);
    setSpeed(c ? c.rate.numerator / c.rate.denominator : 1); setCut(c ? clipEndUs(c) / 1e6 : 0); setEdge('end'); setMessage('');
  }));
  const locked = () => currentProject()?.timeline.tracks.find(t => t.id === clip()?.trackId)?.locked;
  const disabled = () => editBusy() || starting() || locked();
  const warnings = () => clip() ? clipWarnings(clip()!, sourceMedia(id())?.data, usesSourceVoice(clip()!, currentProject()?.timeline)) : [];
  const affected = () => [...new Set(warnings().map(w => w.segmentId))];
  createEffect(on(() => JSON.stringify([currentProject()?.id, clip()?.id, affected()]), () => setTtsIds(affected())));
  const linkedVoice = () => currentProject()?.timeline.clips.some(c => c.content.kind === 'voice' && c.linkedClipId === clip()?.id);
  const running = () => Object.values(videoJobEvents()).find(e => e.projectId === currentProject()?.id && e.videoContentId === id() && isRunningVideoJob(e));
  function update(changes: Partial<ClipAudio>) { setAudio(a => a ? { ...a, ...changes } : undefined); }
  async function apply() {
    if (!clip() || !audio()) return;
    setTimelinePlaying(false);
    if (await commitEdit({ kind: 'audio', id: clip()!.id, audio: audio()! }, 'Chỉnh âm thanh clip')) setMessage('Đã lưu âm thanh. Phát timeline để nghe.');
  }
  async function regenerate() {
    const project = currentProject(), videoContentId = id(), segmentIds = ttsIds().filter(id => affected().includes(id));
    if (!project || !videoContentId || !segmentIds.length) return;
    setStarting(true); setTimelinePlaying(false);
    try {
      await flushPendingSaves();
      if (currentProject()?.id !== project.id) return;
      const jobId = await ttsService.start(project.id, { scope: 'selected', segmentIds, videoContentId });
      rememberVideoJob('tts', { projectId: project.id, videoContentId, jobId, status: 'queued', message: 'Đang tạo lại các đoạn giọng đã chọn.' });
      setNotice({ kind: 'success', message: `Đã yêu cầu tạo lại ${segmentIds.length} đoạn của video này.` });
    } catch (e) { editNotice(e); } finally { setStarting(false); }
  }
  function fit(w: ClipWarning) { if (w.requiredSpeed && w.requiredSpeed >= .5 && w.requiredSpeed <= 2) update({ voiceSpeed: Math.ceil(w.requiredSpeed * 100) / 100 }); }
  return <Show when={clip()}><section class="property-group timeline-audio-properties" aria-label="Âm thanh clip">
    <h4>Âm thanh / thời gian clip</h4>
    <p class="overlay-selection-hint">Mỗi lần dùng nguồn có cấu hình riêng. Thay đổi được lưu bằng Áp dụng và có thể hoàn tác.</p>
    <Show when={locked()}><p role="status">Track đang khóa.</p></Show>
    <Show when={hasAudio(clip()) && audio()}><form onSubmit={e => { e.preventDefault(); void apply(); }}>
      <fieldset disabled={disabled()} class="timeline-audio-fields">
        <label>Âm lượng (%)<input aria-label="Âm lượng clip (%)" type="number" required min="0" max="200" step="1" value={Math.round((audio()?.volume ?? 1) * 100)} onInput={e => update({ volume: e.currentTarget.valueAsNumber / 100 })} /></label>
        <label><input type="checkbox" checked={audio()?.muted} onChange={e => update({ muted: e.currentTarget.checked })} /> Tắt tiếng clip{clip()?.content.kind === 'video' ? ' gốc' : ''}</label>
        <div class="timeline-property-grid"><label>Fade vào (giây)<input aria-label="Fade vào (giây)" type="number" required min="0" step="0.001" value={(audio()?.fadeInUs ?? 0) / 1e6} onInput={e => update({ fadeInUs: Math.round(e.currentTarget.valueAsNumber * 1e6) })} /></label><label>Fade ra (giây)<input aria-label="Fade ra (giây)" type="number" required min="0" step="0.001" value={(audio()?.fadeOutUs ?? 0) / 1e6} onInput={e => update({ fadeOutUs: Math.round(e.currentTarget.valueAsNumber * 1e6) })} /></label></div>
        <Show when={clip()?.content.kind !== 'voice'}>
          <label><input type="checkbox" checked={audio()?.duckingEnabled} onChange={e => update({ duckingEnabled: e.currentTarget.checked })} /> Giảm nền khi có giọng (ducking)</label>
          <label>Mức giảm nền (%)<input aria-label="Mức ducking (%)" type="number" required min="0" max="100" value={Math.round((audio()?.duckingAmount ?? .75) * 100)} onInput={e => update({ duckingAmount: e.currentTarget.valueAsNumber / 100 })} /></label>
          <div class="timeline-property-grid"><label>Attack (ms)<input aria-label="Ducking attack (ms)" type="number" required min="0" max="500" value={(audio()?.duckAttackUs ?? 60000) / 1000} onInput={e => update({ duckAttackUs: Math.round(e.currentTarget.valueAsNumber * 1000) })} /></label><label>Release (ms)<input aria-label="Ducking release (ms)" type="number" required min="0" max="2000" value={(audio()?.duckReleaseUs ?? 180000) / 1000} onInput={e => update({ duckReleaseUs: Math.round(e.currentTarget.valueAsNumber * 1000) })} /></label></div>
        </Show>
        <Show when={clip()?.content.kind === 'video'}><Show when={!linkedVoice()} fallback={<p class="overlay-selection-hint">Giọng nằm ở lớp liên kết riêng. Chọn lớp Giọng để chỉnh âm lượng và tốc độ.</p>}><label><input type="checkbox" checked={audio()?.voiceEnabled} onChange={e => update({ voiceEnabled: e.currentTarget.checked })} /> Phát giọng của video (khi bật lồng tiếng)</label><p class="overlay-selection-hint">Âm lượng giọng dùng cấu hình lồng tiếng nguồn. Thêm lớp Giọng để chỉnh âm lượng riêng.</p></Show></Show>
        <Show when={clip()?.content.kind === 'voice' || (clip()?.content.kind === 'video' && !linkedVoice())}><label>Tốc độ giọng<input aria-label="Tốc độ giọng clip" type="number" required min="0.5" max="2" step="0.01" value={audio()?.voiceSpeed ?? 1} onInput={e => update({ voiceSpeed: e.currentTarget.valueAsNumber })} /></label></Show>
        <div class="timeline-property-actions"><button class="button button-quiet" type="button" onClick={() => setAudio({ ...clipAudio(clip()!, currentProject()?.timeline) })}>Hủy chỉnh âm thanh</button><button class="button button-outline" type="submit">Áp dụng âm thanh</button></div>
      </fieldset>
    </form></Show>
    <form onSubmit={e => { e.preventDefault(); if (clip()) void commitEdit({ kind: 'rate', id: clip()!.id, rate: { numerator: Math.round(speed() * 1000), denominator: 1000 } }, 'Đổi tốc độ clip'); }}>
      <fieldset disabled={disabled()} class="timeline-audio-fields"><label>Tốc độ clip<input aria-label="Tốc độ clip" type="number" required min="0.25" max="4" step="0.001" value={speed()} onInput={e => setSpeed(e.currentTarget.valueAsNumber)} /></label><p class="overlay-selection-hint">Hình, phụ đề và audio liên kết cùng đổi tốc độ. Khoảng nguồn và file giọng được giữ.</p><button class="button button-outline" type="submit">Áp dụng tốc độ clip</button></fieldset>
    </form>
    <div class="timeline-property-actions">
      <Show when={clip()?.content.kind === 'video'}><button type="button" class="button button-outline" disabled={disabled()} onClick={() => void commitEdit({ kind: 'extractAudio', videoClipId: clip()!.id }, 'Tách audio gốc')}>Tách audio gốc</button></Show>
      <Show when={hasAudio(clip()) && clip()?.content.kind !== 'video' && (clip()?.linkedClipId || clip()?.groupId)}><button type="button" class="button button-outline" disabled={disabled()} onClick={() => void commitEdit({ kind: 'detach', id: clip()!.id }, 'Tách liên kết audio')}>Tách liên kết audio</button></Show>
    </div>
    <Show when={sourceMedia(id())?.loading}><p role="status">Đang đọc phụ đề / giọng nguồn…</p></Show>
    <Show when={sourceMedia(id())?.error}><p role="alert" class="timeline-property-error">{sourceMedia(id())?.error}</p><button type="button" class="button button-quiet" onClick={refreshSourceMedia}>Đọc lại nguồn</button></Show>
    <Show when={warnings().length}><div class="timeline-audio-warnings" role="status">
      <h4>{warnings().length} cảnh báo lời / giọng</h4>
      <For each={warnings()}>{w => <p>{w.message} <small>Đoạn {w.segmentId} · {(w.startUs / 1e6).toFixed(2)}–{(w.endUs / 1e6).toFixed(2)}s nguồn</small><Show when={hasAudio(clip()) && w.requiredSpeed && w.requiredSpeed >= .5 && w.requiredSpeed <= 2}><button type="button" class="button button-quiet" disabled={disabled()} onClick={() => fit(w)}>Đặt tốc độ giọng {Math.ceil(w.requiredSpeed! * 100) / 100}×</button></Show></p>}</For>
      <form onSubmit={e => { e.preventDefault(); try { if (clip()) void commitEdit({ kind: 'trim', id: clip()!.id, edge: edge(), atUs: secondsToTimeUs(cut()) }, 'Chỉnh mép clip'); } catch (error) { editNotice(error); } }}><fieldset disabled={disabled()} class="timeline-audio-fields"><label>Mép clip<select aria-label="Mép cần chỉnh" value={edge()} onChange={e => { const edge = e.currentTarget.value as 'start' | 'end'; setEdge(edge); setCut((edge === 'start' ? clip()!.startUs : clipEndUs(clip()!)) / 1e6); }}><option value="start">Đầu</option><option value="end">Cuối</option></select></label><label>Thời điểm trên dự án (giây)<input aria-label="Điểm cắt audio (giây)" type="number" required min="0" step="0.001" value={cut()} onInput={e => setCut(e.currentTarget.valueAsNumber)} /></label><button type="submit" class="button button-outline">Chỉnh điểm cắt</button></fieldset></form>
      <Show when={clip()?.content.kind === 'video' || clip()?.content.kind === 'voice'}><p class="overlay-selection-hint">Chọn các đoạn cần tạo lại của nguồn này; các bản sao cùng nội dung sẽ dùng giọng mới.</p><fieldset disabled={disabled() || !!running()} class="timeline-audio-fields"><For each={affected()}>{id => <label><input type="checkbox" checked={ttsIds().includes(id)} onChange={e => setTtsIds(ids => e.currentTarget.checked ? [...ids, id] : ids.filter(s => s !== id))} /> Tạo lại đoạn {id}</label>}</For><button type="button" class="button button-outline" disabled={!ttsIds().length} onClick={() => void regenerate()}>Tạo lại {ttsIds().length} đoạn này</button></fieldset><Show when={running()}>{job => <p>{job().message}<button type="button" class="button button-quiet" onClick={() => void ttsService.cancel(job().jobId).catch(editNotice)}>Hủy tạo giọng</button></p>}</Show></Show>
    </div></Show>
    <Show when={message()}><p role="status">{message()}</p></Show>
  </section></Show>;
}
