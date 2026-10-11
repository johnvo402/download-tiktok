import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from 'solid-js';
import { convertFileSrc } from '@tauri-apps/api/core';
import { Eye, EyeOff, Lock, Unlock, Volume2, VolumeX, Play, Pause, ZoomIn, ZoomOut, Scan, SkipBack } from 'lucide-solid';
import { currentProject, playbackTime } from '../../app/project-state';
import { selectedClipId, selectedVideoId } from './video-context-state';
import { clipDurationUs, clipEndUs, sourceToTimelineUs, timelineDurationUs } from './time';
import { activeClips, fitScale, formatTimelineTime, limitScale, orderedTracks, RULER_HEIGHT, rulerTicks, TRACK_HEIGHT, timeAtPixel, visibleClips, zoomAt } from './navigation';
import { seekTimeline, selectTimelineClip, setTimelinePlaying, timelinePlaying, timelinePreviewActive, timelineSelectedClipId, timelineTimeUs, toggleTrack, trackSaving } from './timeline-state';
import { timelineCache } from './timeline-cache';
import type { TimelineClip } from './types';
import './timeline.css';

function ClipVisual(props: { clip: TimelineClip; scale: number; fromUs: number; toUs: number; playing: boolean }) {
  const [thumbnail, setThumbnail] = createSignal('');
  const [peaks, setPeaks] = createSignal<number[]>([]);
  const [error, setError] = createSignal('');
  const asset = () => { const content = props.clip.content; return 'assetId' in content ? currentProject()?.timeline.assets.find(a => a.id === content.assetId) : undefined; };
  const name = () => asset()?.metadata.originalName ?? (props.clip.content.kind === 'text' ? props.clip.content.text : props.clip.content.kind === 'voice' ? 'Giọng đọc' : 'Phụ đề');
  const identity = createMemo(() => `${currentProject()?.id}:${asset()?.id}:${asset()?.metadata.checksumSha256}`);
  createEffect(on(identity, () => {
    const source = asset(), id = currentProject()?.id; if (!source || !id) return;
    let alive = true; setError(''); setThumbnail(''); setPeaks([]);
    const kind = source.kind === 'audio' ? 'waveform' : 'thumbnail';
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
  const selected = () => (timelineSelectedClipId() ?? selectedClipId() ?? currentProject()?.timeline.legacyBinding?.clipId) === props.clip.id;
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
  return <button type="button" class="timeline-clip" classList={{ selected: selected(), 'at-playhead': playing(), 'is-audio': props.clip.content.kind === 'audio' || props.clip.content.kind === 'voice' }}
    style={{ left: `${left() / 1e6 * props.scale}px`, width: `${width()}px` }} aria-label={`Chọn ${name()}, bắt đầu ${formatTimelineTime(props.clip.startUs)}, dài ${formatTimelineTime(clipDurationUs(props.clip))}`} aria-pressed={selected()}
    title={`${name()} · ${formatTimelineTime(clipDurationUs(props.clip))}${error() ? ` · ${error()}` : ''}`}
    onPointerDown={event => event.stopPropagation()} onClick={() => void selectTimelineClip(props.clip.id)} onDblClick={() => seekTimeline(props.clip.startUs)}>
    <Show when={thumbnail()}><div class="timeline-thumbnails" style={{ 'background-image': `url("${thumbnail()}")` }} /></Show>
    <Show when={peaks().length}><svg class="timeline-waveform" viewBox={`0 0 ${Math.max(1, bars().length * 4)} 32`} preserveAspectRatio="none" aria-hidden="true"><For each={bars()}>{(peak, i) => <rect x={i() * 4} y={16 - Math.max(1, peak * 15)} width="2" height={Math.max(2, peak * 30)} />}</For></svg></Show>
    <span class="timeline-clip-name">{playing() ? '▶ ' : ''}{name()}</span><small>{formatTimelineTime(clipDurationUs(props.clip))}</small>
  </button>;
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
  const gutter = () => size().width < 540 ? 128 : 172;
  const usableWidth = () => Math.max(1, size().width - gutter());
  const duration = createMemo(() => timelineDurationUs(currentProject()?.timeline.clips ?? []));
  const tracks = createMemo(() => currentProject() ? orderedTracks(currentProject()!.timeline) : []);
  const playingIds = createMemo(() => new Set(timelinePreviewActive() && currentProject() ? activeClips(currentProject()!.timeline, timelineTimeUs()).map(c => c.id) : []));
  const canvasWidth = () => Math.max(usableWidth(), duration() / 1e6 * scale() + 24);
  const fromUs = () => timeAtPixel(scroll().x, scale(), duration());
  const toUs = () => timeAtPixel(scroll().x + usableWidth(), scale(), duration()) + 1;
  const indexedRows = createMemo(() => tracks().map((track, index) => ({ track, index })));
  const rows = () => indexedRows().filter(row => (row.index + 1) * TRACK_HEIGHT > scroll().y - TRACK_HEIGHT && row.index * TRACK_HEIGHT < scroll().y + size().height);
  const ticks = () => rulerTicks(scale(), scroll().x, usableWidth()).filter(time => time <= duration());
  const displayTime = () => {
    if (timelinePreviewActive()) return timelineTimeUs();
    const clip = currentProject()?.timeline.clips.find(c => c.id === selectedClipId()) ?? currentProject()?.timeline.clips.find(c => c.content.kind === 'video' && c.content.videoContentId === selectedVideoId());
    const sourceTime = Math.round(Math.max(0, playbackTime()) * 1e6);
    return clip ? sourceTime >= clip.sourceOutUs ? clipEndUs(clip) : sourceToTimelineUs(clip, Math.max(clip.sourceInUs, sourceTime)) ?? clip.startUs : timelineTimeUs();
  };
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
    if (event.key === ' ' && event.target instanceof HTMLElement && event.target.closest('button')) return;
    if (event.target instanceof HTMLElement && (event.target.matches('input,textarea,select') || event.target.isContentEditable) || event.ctrlKey || event.metaKey || event.altKey) return;
    if (['ArrowLeft', 'ArrowRight', 'Home', 'End', ' ', '+', '=', '-', 'f', 'F'].includes(event.key)) event.preventDefault();
    if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') seekTimeline(displayTime() + (event.key === 'ArrowLeft' ? -1 : 1) * (event.shiftKey ? 10e6 : 1e6));
    if (event.key === 'Home') seekTimeline(0);
    if (event.key === 'End') seekTimeline(duration());
    if (event.key === ' ') { if (!timelinePreviewActive()) seekTimeline(displayTime()); if (displayTime() >= duration()) seekTimeline(0); setTimelinePlaying(!timelinePlaying()); }
    if (event.key === '+' || event.key === '=') zoom(1.5);
    if (event.key === '-') zoom(1 / 1.5);
    if (event.key.toLowerCase() === 'f') fit();
  }
  let resizing: { y: number; height: number } | undefined;
  return <section class="timeline-editor" aria-label="Timeline dự án" onKeyDown={keys} style={{ height: `${height()}px` }}>
    <div class="timeline-height-handle" role="separator" aria-label="Đổi chiều cao timeline" aria-orientation="horizontal" aria-valuemin={160} aria-valuemax={Math.round(maxHeight())} aria-valuenow={Math.round(Math.min(height(), maxHeight()))} tabindex="0"
      onKeyDown={event => { if (event.key === 'ArrowUp' || event.key === 'ArrowDown') { event.preventDefault(); event.stopPropagation(); setHeight(h => Math.min(maxHeight(), Math.max(160, h + (event.key === 'ArrowUp' ? 24 : -24)))); } }}
      onPointerDown={event => { event.preventDefault(); resizing = { y: event.clientY, height: Math.min(height(), maxHeight()) }; event.currentTarget.setPointerCapture(event.pointerId); }}
      onPointerMove={event => { if (resizing) setHeight(Math.min(maxHeight(), Math.max(160, resizing.height + resizing.y - event.clientY))); }}
      onPointerUp={() => { resizing = undefined; }} onPointerCancel={() => { resizing = undefined; }}><span /></div>
    <div class="timeline-toolbar">
      <strong>Timeline</strong>
      <button type="button" aria-label="Về đầu timeline" title="Về đầu (Home)" onClick={() => seekTimeline(0)}><SkipBack size={15} /></button>
      <button type="button" aria-label={timelinePlaying() ? 'Tạm dừng timeline' : 'Phát timeline'} title="Phát / tạm dừng (Space)" disabled={!duration()} onClick={() => { if (!timelinePreviewActive()) seekTimeline(displayTime()); if (displayTime() >= duration()) seekTimeline(0); setTimelinePlaying(!timelinePlaying()); }}>{timelinePlaying() ? <Pause size={16} /> : <Play size={16} />}</button>
      <span class="timeline-time">{formatTimelineTime(displayTime(), true)} <small>/ {formatTimelineTime(duration())}</small></span>
      <span class="timeline-toolbar-spacer" />
      <button type="button" aria-label="Thu nhỏ timeline" title="Thu nhỏ (-), Ctrl + cuộn quanh con trỏ" onClick={() => zoom(1 / 1.5)}><ZoomOut size={16} /></button>
      <button type="button" aria-label="Phóng to timeline" title="Phóng to (+), Ctrl + cuộn quanh con trỏ" onClick={() => zoom(1.5)}><ZoomIn size={16} /></button>
      <button type="button" aria-label="Vừa toàn bộ dự án" title="Vừa toàn bộ dự án (F)" onClick={fit}><Scan size={16} /></button>
    </div>
    <div class="timeline-viewport" ref={viewport} tabindex="0" aria-label="Thước thời gian và các track; dùng mũi tên để seek, Space để phát" onScroll={() => setScroll({ x: viewport.scrollLeft, y: viewport.scrollTop })}
      onWheel={event => { if (event.ctrlKey || event.metaKey) { event.preventDefault(); zoom(Math.exp(-event.deltaY * .003), Math.max(0, event.clientX - viewport.getBoundingClientRect().left - gutter())); } else if (event.shiftKey) { event.preventDefault(); viewport.scrollLeft += event.deltaY || event.deltaX; } }}
      onPointerDown={event => { if (event.button !== 0 || event.clientX - viewport.getBoundingClientRect().left < gutter()) return; event.preventDefault(); viewport.focus(); setScrubbing(true); viewport.setPointerCapture(event.pointerId); seekTimeline(pointerTime(event)); }}
      onPointerMove={event => { if (scrubbing()) seekTimeline(pointerTime(event)); }} onPointerUp={() => setScrubbing(false)} onPointerCancel={() => setScrubbing(false)}>
      <div class="timeline-canvas" style={{ width: `${gutter() + canvasWidth()}px`, height: `${RULER_HEIGHT + Math.max(1, tracks().length) * TRACK_HEIGHT}px` }}>
        <div class="timeline-ruler"><span class="timeline-ruler-label" style={{ width: `${gutter()}px` }}>Lớp / nguồn</span><For each={ticks()}>{time => <span class="timeline-tick" style={{ left: `${gutter() + time / 1e6 * scale()}px` }}>{formatTimelineTime(time, time < 1e6 || scale() > 100)}</span>}</For></div>
        <For each={rows()}>{row => <div class="timeline-track" classList={{ disabled: !row.track.enabled, locked: row.track.locked }} style={{ top: `${RULER_HEIGHT + row.index * TRACK_HEIGHT}px` }}>
          <div class="timeline-track-header" style={{ width: `${gutter()}px` }} onPointerDown={event => event.stopPropagation()}><span title={row.track.name}>{row.track.name}</span><div>
            <button type="button" disabled={trackSaving()} aria-label={`${row.track.locked ? 'Mở khóa' : 'Khóa'} ${row.track.name}`} aria-pressed={row.track.locked} onClick={() => void toggleTrack(row.track.id, 'locked')}>{row.track.locked ? <Lock size={13} /> : <Unlock size={13} />}</button>
            <button type="button" disabled={trackSaving()} aria-label={`${row.track.enabled ? 'Tắt' : 'Bật'} ${row.track.name}`} aria-pressed={row.track.enabled} onClick={() => void toggleTrack(row.track.id, 'enabled')}>{row.track.enabled ? <Eye size={13} /> : <EyeOff size={13} />}</button>
            <Show when={row.track.kind !== 'subtitle'}><button type="button" disabled={trackSaving()} aria-label={`${row.track.muted ? 'Bật tiếng' : 'Tắt tiếng'} ${row.track.name}`} aria-pressed={row.track.muted} onClick={() => void toggleTrack(row.track.id, 'muted')}>{row.track.muted ? <VolumeX size={13} /> : <Volume2 size={13} />}</button><button type="button" disabled={trackSaving()} aria-label={`Solo ${row.track.name}`} aria-pressed={row.track.solo} onClick={() => void toggleTrack(row.track.id, 'solo')}>S</button></Show>
          </div></div>
          <div class="timeline-track-clips" style={{ left: `${gutter()}px` }}><For each={visibleClips(currentProject()?.timeline.clips ?? [], row.track.id, fromUs(), toUs())}>{clip => <ClipVisual clip={clip} scale={scale()} fromUs={fromUs()} toUs={toUs()} playing={playingIds().has(clip.id)} />}</For></div>
        </div>}</For>
        <div class="timeline-playhead" style={{ left: `${gutter() + displayTime() / 1e6 * scale()}px` }} aria-hidden="true"><span /></div>
        <Show when={!tracks().length}><p class="timeline-empty" style={{ left: `${gutter() + 12}px` }}>Thêm nguồn từ Kho nguồn để bắt đầu dựng.</p></Show>
      </div>
    </div>
  </section>;
}
