import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from 'solid-js';
import { convertFileSrc } from '@tauri-apps/api/core';
import { Eye, EyeOff, Lock, Unlock, Volume2, VolumeX, Play, Pause, ZoomIn, ZoomOut, Scan, SkipBack, StepBack, StepForward, ChevronUp, ChevronDown } from 'lucide-solid';
import { currentProject } from '../../app/project-state';
import { clipDurationUs, clipEndUs, timelineDurationUs } from './time';
import { fitScale, formatTimelineTime, limitScale, orderedTracks, RULER_HEIGHT, rulerTicks, TRACK_HEIGHT, timeAtPixel, zoomAt } from './navigation';
import { seekTimeline, selectTimelineClip, selectedTimelineIds, setTimelineSelection, setTimelinePlaying, setTimelineScrubbing, stepTimeline, timelinePlaying, timelinePreviewActive, timelineCursorUs, timelineTimeUs, toggleTrack, trackSaving } from './timeline-state';
import { clearTimelineCache, timelineCache } from './timeline-cache';
import { IntervalIndex } from './interval-index';
import type { TimelineClip, TimelineDocument } from './types';
import { applyEdit, editClosure, editLayout, snapMove, snapTime, type EditCommand, type EditLayout } from './edit-commands';
import { canPaste, commitEdit, copyTimelineClips, editBusy, editNotice, pasteTimelineClips, redoTimeline, setSnapEnabled, snapEnabled, undoTimeline } from './edit-state';
import EditToolbar from './EditToolbar';
import ClipContextMenu, { openEffectsProperties } from './ClipContextMenu';
import './timeline.css';

function ClipVisual(props: { clip: TimelineClip; scale: number; fromUs: number; toUs: number; playing: boolean; startDrag: (event: PointerEvent, clip: TimelineClip, edge?: 'start' | 'end') => void; suppressClick: () => boolean; context: (clip: TimelineClip, x: number, y: number, restore: HTMLElement) => void }) {
  const [thumbnail, setThumbnail] = createSignal('');
  const [peaks, setPeaks] = createSignal<number[]>([]);
  const [error, setError] = createSignal('');
  const asset = () => { const content = props.clip.content; return 'assetId' in content ? currentProject()?.timeline.assets.find(a => a.id === content.assetId) : undefined; };
  const name = () => asset()?.metadata.originalName ?? (props.clip.content.kind === 'text' ? props.clip.content.text : props.clip.content.kind === 'voice' ? 'Giọng đọc' : 'Phụ đề');
  const identity = createMemo(() => `${currentProject()?.id}:${asset()?.id}:${asset()?.metadata.checksumSha256}`);
  createEffect(on(identity, () => {
    const source = asset(), id = currentProject()?.id; if (!source || !id) return;
    let alive = true; setError(''); setThumbnail(''); setPeaks([]);
    const kind = source.kind === 'audio' || props.clip.content.kind === 'sourceAudio' ? 'waveform' : 'thumbnail';
    const request = timelineCache(id, source.id, source.metadata.checksumSha256, kind);
    void request.promise.then(cache => {
      if (!alive) return;
      if (cache.thumbnailPath) setThumbnail(convertFileSrc(cache.thumbnailPath));
      if (cache.waveform) setPeaks(cache.waveform.peaks);
    }).catch(() => { if (alive) setError('Chưa có thumbnail/waveform. Kiểm tra nguồn trong Kho nguồn.'); });
    onCleanup(() => { alive = false; request.release(); });
  }));
  const left = () => Math.max(props.clip.startUs, props.fromUs);
  const end = () => Math.min(props.clip.startUs + clipDurationUs(props.clip), props.toUs);
  const width = () => Math.max(1, (end() - left()) / 1e6 * props.scale);
  const selected = () => selectedTimelineIds().includes(props.clip.id);
  const playing = () => props.playing;
  const bars = () => {
    const values = peaks(); if (!values.length) return [];
    const count = Math.min(128, Math.max(1, Math.floor(width() / 4)));
    const duration = asset()?.durationUs ?? props.clip.sourceOutUs;
    const rate = props.clip.rate.numerator / props.clip.rate.denominator;
    return Array.from({ length: count }, (_, i) => {
      const sourceTime = props.clip.sourceInUs + (left() - props.clip.startUs + (end() - left()) * i / count) * rate;
      return values[Math.min(values.length - 1, Math.max(0, Math.floor(sourceTime / duration * values.length)))]!;
    });
  };
  return <div class="timeline-clip" classList={{ selected: selected(), 'at-playhead': playing(), 'is-audio': ['audio', 'voice', 'sourceAudio'].includes(props.clip.content.kind) }}
    style={{ left: `${left() / 1e6 * props.scale}px`, width: `${width()}px` }} aria-label={`Chọn ${name()}, bắt đầu ${formatTimelineTime(props.clip.startUs)}, dài ${formatTimelineTime(clipDurationUs(props.clip))}`}
    title={`${name()} · ${formatTimelineTime(clipDurationUs(props.clip))}${error() ? ` · ${error()}` : ''}`}
    onContextMenu={e => { e.preventDefault(); e.stopPropagation(); props.context(props.clip, e.clientX, e.clientY, e.currentTarget.querySelector('button')!); }}
    onKeyDown={e => { if (e.key === 'ContextMenu' || e.shiftKey && e.key === 'F10') { e.preventDefault(); e.stopPropagation(); const r = e.currentTarget.getBoundingClientRect(); props.context(props.clip, r.left, r.bottom, e.currentTarget.querySelector('button')!); } }}>
    <button type="button" class="timeline-clip-body" disabled={editBusy()} aria-label={`Chọn ${name()}, bắt đầu ${formatTimelineTime(props.clip.startUs)}, dài ${formatTimelineTime(clipDurationUs(props.clip))}`} aria-pressed={selected()}
      onPointerDown={event => props.startDrag(event, props.clip)} onClick={event => { if (!props.suppressClick()) void selectTimelineClip(props.clip.id, event.ctrlKey || event.metaKey || event.shiftKey); }} onDblClick={() => seekTimeline(props.clip.startUs)}>
    <Show when={thumbnail()}><div class="timeline-thumbnails" style={{ 'background-image': `url("${thumbnail()}")` }} /></Show>
    <Show when={peaks().length}><svg class="timeline-waveform" viewBox={`0 0 ${Math.max(1, bars().length * 4)} 32`} preserveAspectRatio="none" aria-hidden="true"><For each={bars()}>{(peak, i) => <rect x={i() * 4} y={16 - Math.max(1, peak * 15)} width="2" height={Math.max(2, peak * 30)} />}</For></svg></Show>
    <span class="timeline-clip-name">{playing() ? '▶ ' : ''}{name()}</span><small>{formatTimelineTime(clipDurationUs(props.clip))}{props.clip.effects?.keyframes.length ? ' · ◆' : ''}{props.clip.transitionIn ? ' · ⇄' : ''}</small>
    <Show when={props.clip.transitionIn}><span class="timeline-transition-band" aria-hidden="true" style={{ width: `${Math.min(width(), props.clip.transitionIn!.durationUs / 1e6 * props.scale)}px` }} /></Show>
    </button>
    <Show when={selected() && props.clip.startUs >= props.fromUs}><span class="timeline-trim-handle start" title="Kéo trim đầu; dùng nút Trim đầu để nhập thời gian" aria-hidden="true" onPointerDown={event => props.startDrag(event, props.clip, 'start')} onClick={event => event.stopPropagation()} /></Show>
    <Show when={selected() && clipEndUs(props.clip) <= props.toUs}><span class="timeline-trim-handle end" title="Kéo trim cuối; dùng nút Trim cuối để nhập thời gian" aria-hidden="true" onPointerDown={event => props.startDrag(event, props.clip, 'end')} onClick={event => event.stopPropagation()} /></Show>
  </div>;
}

export default function TimelineEditor() {
  let viewport!: HTMLDivElement;
  const [size, setSize] = createSignal({ width: 800, height: 200 });
  const [scroll, setScroll] = createSignal({ x: 0, y: 0 });
  const [scale, setScale] = createSignal(30);
  const [fitting, setFitting] = createSignal(true);
  const [scrubbing, setScrubbing] = createSignal(false);
  const [height, setHeight] = createSignal(260);
  const [maxHeight, setMaxHeight] = createSignal(600);
  const [draft, setDraft] = createSignal<{ next?: EditLayout; ids: Set<string>; error?: string }>();
  const [context, setContext] = createSignal<{ x: number; y: number; restore: HTMLElement }>();
  async function openContext(c: TimelineClip, x: number, y: number, restore: HTMLElement) {
    if (editBusy()) return;
    setTimelinePlaying(false);
    if (!selectedTimelineIds().includes(c.id)) await selectTimelineClip(c.id);
    if (selectedTimelineIds().includes(c.id)) setContext({ x, y, restore });
  }
  let gesture: { doc: TimelineDocument; expected: EditLayout; clip: TimelineClip; ids: string[]; x: number; y: number; scrollX: number; pointerId: number; target: HTMLElement; edge?: 'start' | 'end'; command?: EditCommand; moved: boolean; projectId: string } | undefined;
  let ignoreClick = false;
  const gutter = () => size().width < 540 ? 128 : 172;
  const usableWidth = () => Math.max(1, size().width - gutter());
  const duration = createMemo(() => timelineDurationUs(currentProject()?.timeline.clips ?? []));
  const tracks = createMemo(() => currentProject() ? orderedTracks(currentProject()!.timeline) : []);
  const clipIndex = createMemo(() => new IntervalIndex(currentProject()?.timeline.clips ?? [], c => c.startUs, clipEndUs));
  const enabledTracks = createMemo(() => new Set(tracks().filter(t => t.enabled).map(t => t.id)));
  const playingIds = createMemo(() => new Set(timelinePreviewActive() ? clipIndex().at(timelineTimeUs()).filter(c => enabledTracks().has(c.trackId)).map(c => c.id) : []));
  createEffect(on(() => currentProject()?.id, id => { if (id) onCleanup(() => clearTimelineCache(id)); }));
  const canvasWidth = () => Math.max(usableWidth(), Math.max(duration(), timelineDurationUs(draft()?.next?.clips ?? [])) / 1e6 * scale() + 24);
  const fromUs = () => timeAtPixel(scroll().x, scale(), duration());
  const toUs = () => timeAtPixel(scroll().x + usableWidth(), scale(), duration()) + 1;
  const visible = createMemo(() => clipIndex().intersect(fromUs(), toUs()));
  const indexedRows = createMemo(() => tracks().map((track, index) => ({ track, index })));
  const rows = () => indexedRows().filter(row => (row.index + 1) * TRACK_HEIGHT > scroll().y - TRACK_HEIGHT && row.index * TRACK_HEIGHT < scroll().y + size().height);
  const ticks = () => rulerTicks(scale(), scroll().x, usableWidth()).filter(time => time <= duration());
  const displayTime = timelineCursorUs;
  function fit() { setFitting(true); setScale(limitScale(fitScale(duration(), usableWidth()), duration(), usableWidth())); viewport.scrollLeft = 0; }
  let fittedProject = '';
  createEffect(() => {
    const id = currentProject()?.id; duration();
    if (id && id !== fittedProject && viewport) { fittedProject = id; fit(); viewport.scrollTop = 0; }
  });
  createEffect(() => {
    const value = fitting() ? fitScale(duration(), usableWidth()) : scale();
    setScale(limitScale(value, duration(), usableWidth()));
  });
  onMount(() => {
    const container = viewport.closest('.translation-workspace');
    const observer = new ResizeObserver(() => {
      setSize({ width: viewport.clientWidth, height: viewport.clientHeight });
      if (container) setMaxHeight(Math.max(160, Math.min(600, container.clientHeight - 180, container.clientHeight * .65)));
    });
    observer.observe(viewport); if (container) observer.observe(container);
    setSize({ width: viewport.clientWidth, height: viewport.clientHeight }); fit();
    onCleanup(() => observer.disconnect());
  });
  function zoom(factor: number, anchor = Math.max(0, Math.min(usableWidth(), displayTime() / 1e6 * scale() - viewport.scrollLeft))) {
    setFitting(false);
    const old = scale(), next = limitScale(old * factor, duration(), usableWidth());
    const x = zoomAt(old, next, viewport.scrollLeft, anchor, Math.max(0, duration() / 1e6 * next + 24 - usableWidth()));
    setScale(next); queueMicrotask(() => { viewport.scrollLeft = x; });
  }
  function pointerTime(event: PointerEvent) { return timeAtPixel(event.clientX - viewport.getBoundingClientRect().left - gutter() + viewport.scrollLeft, scale(), duration()); }
  function keys(event: KeyboardEvent) {
    if (event.target instanceof HTMLElement && event.target.closest('[role="dialog"],[role="menu"]')) return;
    if (event.key === ' ' && event.target instanceof HTMLElement && event.target.closest('button')) return;
    if (event.target instanceof HTMLElement && (event.target.matches('input,textarea,select') || event.target.isContentEditable)) return;
    if (event.ctrlKey || event.metaKey) {
      const key = event.key.toLowerCase();
      if (['z', 'y', 'c', 'v', 'd', 'g', 'a'].includes(key)) event.preventDefault();
      if (editBusy()) return;
      if (key === 'z') void (event.shiftKey ? redoTimeline() : undoTimeline());
      if (key === 'y') void redoTimeline();
      if (key === 'c') copyTimelineClips();
      if (key === 'v' && canPaste()) void pasteTimelineClips(displayTime());
      if (key === 'd') void commitEdit({ kind: 'duplicate', ids: selectedTimelineIds(), atUs: duration() }, 'Nhân bản clip');
      if (key === 'g') void commitEdit({ kind: event.shiftKey ? 'ungroup' : 'group', ids: selectedTimelineIds() }, event.shiftKey ? 'Tách nhóm clip' : 'Nhóm clip');
      if (key === 'a') setTimelineSelection(currentProject()?.timeline.clips.map(c => c.id) ?? []);
      return;
    }
    if (event.altKey) return;
    if (event.key.toLowerCase() === 'k') { event.preventDefault(); openEffectsProperties(); return; }
    if (['Delete', 'Backspace', 's', 'S', 'n', 'N', 'Escape'].includes(event.key)) event.preventDefault();
    if (event.key === 'Escape') { if (gesture) cancelDrag(); else setTimelineSelection([]); return; }
    if (event.key.toLowerCase() === 'n') setSnapEnabled(value => !value);
    if (!editBusy() && ['Delete', 'Backspace'].includes(event.key)) void commitEdit({ kind: 'delete', ids: selectedTimelineIds(), ripple: event.shiftKey }, event.shiftKey ? 'Xóa ripple' : 'Xóa clip');
    if (!editBusy() && event.key.toLowerCase() === 's') void commitEdit({ kind: 'split', ids: selectedTimelineIds(), atUs: displayTime() }, 'Cắt clip');
    if (['ArrowLeft', 'ArrowRight', 'Home', 'End', ' ', '+', '=', '-', 'f', 'F'].includes(event.key)) event.preventDefault();
    if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
      const direction = event.key === 'ArrowLeft' ? -1 : 1;
      if (event.shiftKey) seekTimeline(displayTime() + direction * 1e6); else stepTimeline(direction);
    }
    if (event.key === 'Home') seekTimeline(0);
    if (event.key === 'End') seekTimeline(duration());
    if (event.key === ' ') { if (!timelinePreviewActive()) seekTimeline(displayTime()); if (displayTime() >= duration()) seekTimeline(0); setTimelinePlaying(!timelinePlaying()); }
    if (event.key === '+' || event.key === '=') zoom(1.5);
    if (event.key === '-') zoom(1 / 1.5);
    if (event.key.toLowerCase() === 'f') fit();
  }
  function startDrag(event: PointerEvent, clip: TimelineClip, edge?: 'start' | 'end') {
    event.stopPropagation();
    if (event.button !== 0 || editBusy() || trackSaving()) return;
    const project = currentProject(); if (!project) return;
    if (project.timeline.tracks.find(t => t.id === clip.trackId)?.locked) { if (edge) editNotice(new Error('Mở khóa track trước khi trim.')); return; }
    const chosen = selectedTimelineIds();
    const ids = chosen.includes(clip.id) ? chosen : [clip.id];
    gesture = { doc: structuredClone(project.timeline), expected: editLayout(project.timeline), clip, ids, x: event.clientX, y: event.clientY, scrollX: viewport.scrollLeft, pointerId: event.pointerId, target: event.currentTarget as HTMLElement, edge, moved: false, projectId: project.id };
    gesture.target.setPointerCapture(event.pointerId);
    window.addEventListener('pointermove', windowDragMove);
    window.addEventListener('pointerup', windowDragEnd);
    window.addEventListener('pointercancel', windowDragCancel);
    if (edge) event.preventDefault();
  }
  function cancelDrag() {
    if (gesture?.target.hasPointerCapture(gesture.pointerId)) gesture.target.releasePointerCapture(gesture.pointerId);
    gesture = undefined; setDraft(undefined);
    window.removeEventListener('pointermove', windowDragMove);
    window.removeEventListener('pointerup', windowDragEnd);
    window.removeEventListener('pointercancel', windowDragCancel);
  }
  function windowDragMove(event: PointerEvent) { if (gesture?.pointerId === event.pointerId) dragMove(event); }
  function windowDragEnd(event: PointerEvent) { if (gesture?.pointerId === event.pointerId) void finishDrag(); }
  function windowDragCancel(event: PointerEvent) { if (gesture?.pointerId === event.pointerId) cancelDrag(); }
  function dragMove(event: PointerEvent) {
    const g = gesture; if (!g) return;
    if (currentProject()?.id !== g.projectId) { cancelDrag(); return; }
    if (!g.moved && Math.hypot(event.clientX - g.x, event.clientY - g.y) < 4) return;
    g.moved = true;
    const rect = viewport.getBoundingClientRect();
    if (event.clientX > rect.right - 24) viewport.scrollLeft += 18;
    if (event.clientX < rect.left + gutter() + 24) viewport.scrollLeft -= 18;
    if (event.clientY > rect.bottom - 12) viewport.scrollTop += 12;
    if (event.clientY < rect.top + RULER_HEIGHT + 12) viewport.scrollTop -= 12;
    let delta = Math.round((event.clientX - g.x + viewport.scrollLeft - g.scrollX) / scale() * 1e6);
    const selected = editClosure(g.doc, g.ids), tolerance = 8 / scale() * 1e6;
    const snapping = snapEnabled() && !event.altKey;
    if (g.edge) {
      let at = Math.max(0, (g.edge === 'start' ? g.clip.startUs : clipEndUs(g.clip)) + delta);
      if (snapping) at = snapTime(g.doc, g.ids, at, displayTime(), tolerance);
      g.command = { kind: 'trim', id: g.clip.id, edge: g.edge, atUs: at };
    } else {
      const min = Math.min(...g.doc.clips.filter(c => selected.has(c.id)).map(c => c.startUs));
      delta = Math.max(-min, delta); if (snapping) delta = snapMove(g.doc, g.ids, delta, displayTime(), tolerance);
      const index = Math.floor((event.clientY - rect.top + viewport.scrollTop - RULER_HEIGHT) / TRACK_HEIGHT);
      const target = tracks()[index]?.id ?? g.clip.trackId;
      g.command = { kind: 'move', ids: g.ids, anchorId: g.clip.id, deltaUs: delta, trackId: target };
    }
    try { setDraft({ next: applyEdit(g.doc, g.command), ids: selected }); }
    catch (error) { setDraft({ ids: selected, error: error instanceof Error ? error.message : 'Không thể đặt clip ở đây.' }); }
  }
  async function finishDrag() {
    const g = gesture; if (!g) return;
    const candidate = draft(); cancelDrag();
    if (!g.moved) return;
    ignoreClick = true; setTimeout(() => { ignoreClick = false; }, 0);
    if (candidate?.error) { editNotice(new Error(candidate.error)); return; }
    if (g.command && candidate?.next) {
      // Selecting first honours the transcript save barrier, then one CAS is one undo step.
      if (!selectedTimelineIds().includes(g.clip.id)) { await selectTimelineClip(g.clip.id); if (!selectedTimelineIds().includes(g.clip.id)) return; }
      await commitEdit(g.command, g.edge ? 'Trim clip' : 'Di chuyển clip', g.expected);
    }
  }
  onCleanup(cancelDrag);
  onCleanup(() => setTimelineScrubbing(false));
  let resizing: { y: number; height: number } | undefined;
  return <section class="timeline-editor" aria-label="Timeline dự án" onKeyDown={keys} style={{ height: `${height()}px` }}>
    <Show when={context()}>{value => <ClipContextMenu {...value()} close={() => setContext(undefined)} />}</Show>
    <div class="timeline-height-handle" role="separator" aria-label="Đổi chiều cao timeline" aria-orientation="horizontal" aria-valuemin={160} aria-valuemax={Math.round(maxHeight())} aria-valuenow={Math.round(Math.min(height(), maxHeight()))} tabindex="0"
      onKeyDown={event => { if (event.key === 'ArrowUp' || event.key === 'ArrowDown') { event.preventDefault(); event.stopPropagation(); setHeight(h => Math.min(maxHeight(), Math.max(160, h + (event.key === 'ArrowUp' ? 24 : -24)))); } }}
      onPointerDown={event => { event.preventDefault(); resizing = { y: event.clientY, height: Math.min(height(), maxHeight()) }; event.currentTarget.setPointerCapture(event.pointerId); }}
      onPointerMove={event => { if (resizing) setHeight(Math.min(maxHeight(), Math.max(160, resizing.height + resizing.y - event.clientY))); }}
      onPointerUp={() => { resizing = undefined; }} onPointerCancel={() => { resizing = undefined; }}><span /></div>
    <div class="timeline-toolbar">
      <strong>Timeline</strong>
      <button type="button" aria-label="Về đầu timeline" title="Về đầu (Home)" onClick={() => seekTimeline(0)}><SkipBack size={15} /></button>
      <button type="button" aria-label="Lùi một khung hình" title="Lùi một frame (←); Shift + ← lùi 1 giây" disabled={!duration()} onClick={() => stepTimeline(-1)}><StepBack size={15} /></button>
      <button type="button" aria-label={timelinePlaying() ? 'Tạm dừng timeline' : 'Phát timeline'} title="Phát / tạm dừng (Space)" disabled={!duration()} onClick={() => { if (!timelinePreviewActive()) seekTimeline(displayTime()); if (displayTime() >= duration()) seekTimeline(0); setTimelinePlaying(!timelinePlaying()); }}>{timelinePlaying() ? <Pause size={16} /> : <Play size={16} />}</button>
      <button type="button" aria-label="Tiến một khung hình" title="Tiến một frame (→); Shift + → tiến 1 giây" disabled={!duration()} onClick={() => stepTimeline(1)}><StepForward size={15} /></button>
      <span class="timeline-time">{formatTimelineTime(displayTime(), true)} <small>/ {formatTimelineTime(duration())}</small></span>
      <span class="timeline-toolbar-spacer" />
      <button type="button" aria-label="Thu nhỏ timeline" title="Thu nhỏ (-), Ctrl + cuộn quanh con trỏ" onClick={() => zoom(1 / 1.5)}><ZoomOut size={16} /></button>
      <button type="button" aria-label="Phóng to timeline" title="Phóng to (+), Ctrl + cuộn quanh con trỏ" onClick={() => zoom(1.5)}><ZoomIn size={16} /></button>
      <button type="button" aria-label="Vừa toàn bộ dự án" title="Vừa toàn bộ dự án (F)" onClick={fit}><Scan size={16} /></button>
    </div>
    <EditToolbar cursorUs={displayTime()} />
    <Show when={draft()}><p class="timeline-drag-status" role="status" classList={{ invalid: !!draft()?.error }}>{draft()?.error ?? (snapEnabled() ? 'Thả để lưu · Alt bỏ hít mép · Esc hủy' : 'Thả để lưu · Esc hủy')}</p></Show>
    <div class="timeline-viewport" ref={viewport} tabindex="0" aria-label="Thước thời gian và các track; dùng mũi tên để seek, Space để phát" onScroll={() => setScroll({ x: viewport.scrollLeft, y: viewport.scrollTop })}
      onWheel={event => { if (event.ctrlKey || event.metaKey) { event.preventDefault(); zoom(Math.exp(-event.deltaY * .003), Math.max(0, event.clientX - viewport.getBoundingClientRect().left - gutter())); } else if (event.shiftKey) { event.preventDefault(); viewport.scrollLeft += event.deltaY || event.deltaX; } }}
      onPointerDown={event => { if (event.button !== 0 || editBusy() || event.clientX - viewport.getBoundingClientRect().left < gutter()) return; event.preventDefault(); viewport.focus(); setScrubbing(true); setTimelineScrubbing(true); viewport.setPointerCapture(event.pointerId); seekTimeline(pointerTime(event)); }}
      onPointerMove={event => { if (scrubbing()) seekTimeline(pointerTime(event)); }} onPointerUp={() => { setScrubbing(false); setTimelineScrubbing(false); }} onPointerCancel={() => { setScrubbing(false); setTimelineScrubbing(false); cancelDrag(); }}>
      <div class="timeline-canvas" style={{ width: `${gutter() + canvasWidth()}px`, height: `${RULER_HEIGHT + Math.max(1, tracks().length) * TRACK_HEIGHT}px` }}>
        <div class="timeline-ruler"><span class="timeline-ruler-label" style={{ width: `${gutter()}px` }}>Lớp / nguồn</span><For each={ticks()}>{time => <span class="timeline-tick" style={{ left: `${gutter() + time / 1e6 * scale()}px` }}>{formatTimelineTime(time, time < 1e6 || scale() > 100)}</span>}</For></div>
        <For each={rows()}>{row => <div class="timeline-track" classList={{ disabled: !row.track.enabled, locked: row.track.locked }} style={{ top: `${RULER_HEIGHT + row.index * TRACK_HEIGHT}px` }}>
          <div class="timeline-track-header" style={{ width: `${gutter()}px` }} onPointerDown={event => event.stopPropagation()}><div class="timeline-track-title"><span title={row.track.name}>{row.track.name}</span><button type="button" disabled={editBusy() || row.track.locked} aria-label={`Đưa ${row.track.name} lên trên`} onClick={() => void commitEdit({ kind: 'track', action: 'reorder', id: row.track.id, direction: 1 }, 'Đổi thứ tự track')}><ChevronUp size={12} /></button><button type="button" disabled={editBusy() || row.track.locked} aria-label={`Đưa ${row.track.name} xuống dưới`} onClick={() => void commitEdit({ kind: 'track', action: 'reorder', id: row.track.id, direction: -1 }, 'Đổi thứ tự track')}><ChevronDown size={12} /></button></div><div>
            <button type="button" disabled={trackSaving()} aria-label={`${row.track.locked ? 'Mở khóa' : 'Khóa'} ${row.track.name}`} aria-pressed={row.track.locked} onClick={() => void toggleTrack(row.track.id, 'locked')}>{row.track.locked ? <Lock size={13} /> : <Unlock size={13} />}</button>
            <button type="button" disabled={trackSaving()} aria-label={`${row.track.enabled ? 'Tắt' : 'Bật'} ${row.track.name}`} aria-pressed={row.track.enabled} onClick={() => void toggleTrack(row.track.id, 'enabled')}>{row.track.enabled ? <Eye size={13} /> : <EyeOff size={13} />}</button>
            <Show when={row.track.kind !== 'subtitle'}><button type="button" disabled={trackSaving()} aria-label={`${row.track.muted ? 'Bật tiếng' : 'Tắt tiếng'} ${row.track.name}`} aria-pressed={row.track.muted} onClick={() => void toggleTrack(row.track.id, 'muted')}>{row.track.muted ? <VolumeX size={13} /> : <Volume2 size={13} />}</button><button type="button" disabled={trackSaving()} aria-label={`Solo ${row.track.name}`} aria-pressed={row.track.solo} onClick={() => void toggleTrack(row.track.id, 'solo')}>S</button></Show>
          </div></div>
          <div class="timeline-track-clips" style={{ left: `${gutter()}px` }}><For each={visible().filter(c => c.trackId === row.track.id)}>{clip => <ClipVisual clip={clip} scale={scale()} fromUs={fromUs()} toUs={toUs()} playing={playingIds().has(clip.id)} startDrag={startDrag} suppressClick={() => ignoreClick} context={(c, x, y, restore) => void openContext(c, x, y, restore)} />}</For></div>
        </div>}</For>
        <For each={draft()?.next?.clips.filter(c => draft()?.ids.has(c.id)) ?? []}>{clip => <div class="timeline-drag-ghost" style={{ left: `${gutter() + clip.startUs / 1e6 * scale()}px`, top: `${RULER_HEIGHT + tracks().findIndex(t => t.id === clip.trackId) * TRACK_HEIGHT + 8}px`, width: `${clipDurationUs(clip) / 1e6 * scale()}px` }} />}</For>
        <div class="timeline-playhead" style={{ left: `${gutter() + displayTime() / 1e6 * scale()}px` }} aria-hidden="true"><span /></div>
        <Show when={!tracks().length}><p class="timeline-empty" style={{ left: `${gutter() + 12}px` }}>Thêm nguồn từ Kho nguồn để bắt đầu dựng.</p></Show>
      </div>
    </div>
  </section>;
}
