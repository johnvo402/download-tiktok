import { createEffect, createMemo, createResource, createSignal, Index, onCleanup, Show } from 'solid-js';
import { convertFileSrc } from '@tauri-apps/api/core';
import { Pause, Play, Image, Volume2 } from 'lucide-solid';
import { studioService } from '../../app/services/studio-service';
import { audioCurrent, formatTime, timeline } from './studio-utils';
import type { StudioDraft, StudioOverlay } from './types';

export default function StudioPreview(props: {
  draft: StudioDraft; sceneId: string; disabled: boolean; onScene: (id: string) => void;
  onSubtitlePosition: (y: number) => void; onOverlayPosition: (id: string, x: number, y: number) => void;
}) {
  const [playing, setPlaying] = createSignal(false);
  const [localTime, setLocalTime] = createSignal(0);
  const [scope, setScope] = createSignal<'chapter' | 'all'>('chapter');
  const [assetError, setAssetError] = createSignal('');
  const entries = createMemo(() => timeline(props.draft));
  const selected = createMemo(() => props.draft.scenes.find(s => s.id === props.sceneId));
  const entry = createMemo(() => entries().find(e => e.scene.id === props.sceneId));
  const playable = createMemo(() => entries().filter(e => e.duration > 0 && (scope() === 'all' || e.scene.chapterId === selected()?.chapterId)));
  const globalTime = () => (entry()?.start ?? 0) + localTime();
  const activeOverlays = createMemo(() => props.draft.overlays.filter(o => globalTime() >= o.start && globalTime() < o.end));
  let audio!: HTMLAudioElement;
  let stage!: HTMLDivElement;
  let pendingSeek = 0;
  let previousScene = '';
  let previousAudioPath = '';
  let disposed = false;
  const url = async (path: string) => {
    try { setAssetError(''); return convertFileSrc(await studioService.asset(props.draft.projectId, path)); }
    catch (error) { setAssetError(error instanceof Error ? error.message : 'Không mở được media.'); return ''; }
  };
  const [imageUrl] = createResource(() => selected()?.image ?? '', path => path ? url(path) : Promise.resolve(''));
  const [audioUrl] = createResource(() => {
    const scene = selected(); return scene && audioCurrent(scene, props.draft) ? scene.audio!.path : '';
  }, path => path ? url(path) : Promise.resolve(''));
  const [logos] = createResource(() => props.draft.overlays.filter(o => o.kind === 'logo' && o.path).map(o => o.path!).join('|'), async paths => {
    const pairs = await Promise.all(paths.split('|').filter(Boolean).map(async path => [path, await url(path)] as const));
    return Object.fromEntries(pairs);
  });
  createEffect(() => {
    const id = props.sceneId;
    if (id !== previousScene) {
      previousScene = id; setLocalTime(0);
      const path = selected()?.audio?.path ?? '';
      // Restored scenes can share one asset; no new metadata event fires then.
      if (path && path === previousAudioPath && audio && audioCurrent(selected()!, props.draft)) {
        audio.currentTime = pendingSeek; setLocalTime(pendingSeek); pendingSeek = 0;
        if (playing()) void play();
      }
      previousAudioPath = path;
    }
    if (!audioUrl()) { audio?.pause(); }
    if (!selected() || !audioCurrent(selected()!, props.draft)) setPlaying(false);
  });
  createEffect(() => { if (props.disabled) { setPlaying(false); audio?.pause(); } });
  onCleanup(() => { disposed = true; audio?.pause(); });
  const play = async () => {
    try { await audio.play(); if (!disposed) setPlaying(true); }
    catch { if (!disposed) { setPlaying(false); setAssetError('Không phát được giọng đọc. Hãy kiểm tra tệp audio của cảnh.'); } }
  };
  const toggle = () => {
    if (playing()) { audio.pause(); setPlaying(false); } else { if (audio.ended) audio.currentTime = 0; void play(); }
  };
  const seek = (time: number) => {
    const list = playable();
    const next = list.find(e => time >= e.start && time < e.end) ?? list.at(-1);
    if (!next) return;
    pendingSeek = Math.max(0, Math.min(next.duration, time - next.start));
    setLocalTime(pendingSeek);
    if (next.scene.id === props.sceneId) { audio.currentTime = pendingSeek; pendingSeek = 0; }
    else props.onScene(next.scene.id);
  };
  const ended = () => {
    const list = playable(); const index = list.findIndex(e => e.scene.id === props.sceneId);
    const next = list[index + 1];
    if (playing() && next) { pendingSeek = 0; props.onScene(next.scene.id); }
    else setPlaying(false);
  };
  const subtitle = createMemo(() => selected()?.audio?.cues.find(c => localTime() >= c.start && localTime() < c.end)?.text ?? '');
  const dimensions = () => props.draft.aspectRatio === '9:16' ? [1080, 1920] : [1920, 1080];
  const drag = (event: PointerEvent, overlay?: StudioOverlay) => {
    if (props.disabled) return;
    event.preventDefault();
    const element = event.currentTarget as HTMLElement;
    element.setPointerCapture(event.pointerId);
    const bounds = stage.getBoundingClientRect();
    const startX = event.clientX; const startY = event.clientY;
    const originX = overlay?.x ?? .5; const originY = overlay?.y ?? props.draft.subtitle.y;
    const move = (next: PointerEvent) => {
      const y = Math.max(0.02, Math.min(.96, originY + (next.clientY - startY) / bounds.height));
      if (overlay) props.onOverlayPosition(overlay.id, Math.max(0, Math.min(1 - overlay.width, originX + (next.clientX - startX) / bounds.width)), y);
      else props.onSubtitlePosition(y);
    };
    const finish = () => { element.removeEventListener('pointermove', move); element.removeEventListener('pointerup', finish); element.removeEventListener('pointercancel', finish); };
    element.addEventListener('pointermove', move); element.addEventListener('pointerup', finish); element.addEventListener('pointercancel', finish);
  };
  return <section class="studio-preview" aria-label="Bản xem trước video">
    <div class="studio-panel-heading"><span><Play size={15} /> Bản xem trước</span><span class="studio-muted">{props.draft.aspectRatio} · Chưa xuất MP4</span></div>
    <div class="studio-stage-area">
      <div ref={stage} class="studio-stage" classList={{ 'studio-stage-portrait': props.draft.aspectRatio === '9:16' }} style={{ 'aspect-ratio': props.draft.aspectRatio.replace(':', ' / '), '--studio-output-width': `${dimensions()[0]}` }}>
        <Show when={imageUrl()} fallback={<div class="studio-preview-empty"><Image size={36} /><strong>{selected()?.title || 'Chọn một cảnh để xem trước'}</strong><span>Nhập ảnh tạo từ Codex vào cảnh này.</span></div>}>
          <img class="studio-scene-image" src={imageUrl()} alt={selected()?.title ?? ''} style={{ transform: props.draft.motion ? `scale(${1 + .06 * localTime() / Math.max(1, entry()?.duration ?? 1)})` : undefined }} />
        </Show>
        <Index each={activeOverlays()}>{overlay => <div class="studio-preview-overlay" onPointerDown={event => drag(event, overlay())} style={{ left: `${overlay().x * 100}%`, top: `${overlay().y * 100}%`, width: `${overlay().width * 100}%`, opacity: overlay().opacity, color: overlay().color, 'font-size': `calc(100cqw * ${overlay().fontSize} / ${dimensions()[0]})` }}>
          <Show when={overlay().kind === 'logo'} fallback={overlay().text}><img src={logos()?.[overlay().path ?? '']} alt="Logo" draggable={false} /></Show>
        </div>}</Index>
        <Show when={props.draft.subtitle.enabled && subtitle() && selected() && audioCurrent(selected()!, props.draft)}>
          <div class="studio-preview-subtitle" classList={{ 'studio-subtitle-background': props.draft.subtitle.background }} onPointerDown={event => drag(event)} style={{ top: `${props.draft.subtitle.y * 100}%`, color: props.draft.subtitle.color, 'font-size': `calc(100cqw * ${props.draft.subtitle.fontSize} / ${dimensions()[0]})` }}>{subtitle()}</div>
        </Show>
      </div>
    </div>
    <audio ref={audio} src={audioUrl() || undefined} preload="metadata" onLoadedMetadata={() => { audio.currentTime = pendingSeek; pendingSeek = 0; if (playing()) void play(); }} onTimeUpdate={() => setLocalTime(audio.currentTime)} onEnded={ended} onError={() => { if (audioUrl()) { setPlaying(false); setAssetError('Không mở được giọng đọc của cảnh.'); } }} />
    <div class="studio-player-controls">
      <button type="button" class="studio-icon-button" aria-label={playing() ? 'Tạm dừng' : 'Phát bản xem trước'} disabled={!audioUrl() || props.disabled} onClick={toggle}>{playing() ? <Pause size={19} /> : <Play size={19} />}</button>
      <span class="studio-time">{formatTime(globalTime())}</span>
      <input type="range" aria-label="Tua bản xem trước" min={playable()[0]?.start ?? 0} max={playable().at(-1)?.end ?? 1} step="0.1" value={globalTime()} disabled={!audioUrl() || props.disabled} onInput={event => seek(Number(event.currentTarget.value))} />
      <span class="studio-time">{formatTime(playable().at(-1)?.end ?? 0)}</span><Volume2 size={15} />
    </div>
    <div class="studio-preview-footer"><label>Phạm vi phát <select value={scope()} onChange={event => setScope(event.currentTarget.value as 'all' | 'chapter')}><option value="chapter">Chương hiện tại</option><option value="all">Toàn video</option></select></label><span>Kéo phụ đề, logo và text để đổi vị trí.</span></div>
    <Show when={assetError()}><p role="alert" class="studio-error">{assetError()}</p></Show>
  </section>;
}
