import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from 'solid-js';
import { convertFileSrc } from '@tauri-apps/api/core';
import { currentProject, segmentVoicePreviewActive, setActiveWorkspacePanelTab } from '../../app/project-state';
import { mediaService } from '../../app/services/media-service';
import { formatTimelineTime } from './navigation';
import { timelineToSourceUs } from './time';
import { publishTimelineTime, reviewSelectedSource, seekTimeline, selectTimelineClip, selectedTimelineIds, setTimelineBuffering, setTimelinePlaying, timelineAdvancing, timelineBuffering, timelineSeekRevision, timelineTimeUs } from './timeline-state';
import type { TimelineClip } from './types';
import { audioRegions, clipWarnings, contentId, usesSourceVoice } from './audio-layout';
import { refreshSourceMedia, sourceMedia, watchSourceMedia } from './source-media';
import TimelineAudio from './TimelineAudio';
import { clipTypography, layerTransform, sourceGeometry } from './visual-layout';
import { previewClip, previewSettings } from './visual-state';
import { nativeSceneClip, residentClips, timelineScene } from './scene-plan';
import { PreviewClock, PreviewPlayers } from './preview-clock';
import { mediaPlayer } from './media-player';
import { createTimelineNativePreview } from './native-preview';

function SourceLayer(props: { clip: TimelineClip; width: number; height: number; guides: boolean; active: boolean; native: boolean; text: string; players: PreviewPlayers; fail(id: string, error: string): void }) {
  const [url, setUrl] = createSignal('');
  const [error, setError] = createSignal('');
  const [imageReady, setImageReady] = createSignal(false);
  const [frameReady, setFrameReady] = createSignal(false);
  const clip = () => previewClip(props.clip);
  const content = () => { const c = clip().content; return 'videoContentId' in c ? currentProject()?.timeline.videoContents.find(v => v.id === c.videoContentId) : undefined; };
  const asset = () => { const content = props.clip.content; return 'assetId' in content ? currentProject()?.timeline.assets.find(a => a.id === content.assetId) : undefined; };
  const identity = createMemo(() => `${currentProject()?.id}:${asset()?.id}:${asset()?.relativePath}`);
  createEffect(on(identity, () => {
    const source = asset(), project = currentProject();
    if (!source || !project) return;
    let alive = true;
    setUrl(''); setError(''); setImageReady(false); setFrameReady(false);
    void mediaService.path(project.id, source.id).then(path => { if (alive) setUrl(convertFileSrc(path)); })
      .catch(error => { if (alive) fail(error instanceof Error ? error.message : 'Không mở được nguồn.'); });
    onCleanup(() => { alive = false; });
  }));
  function fail(message: string) { setError(message); props.fail(`visual:${props.clip.id}`, message); }
  function Video() {
    let element!: HTMLVideoElement;
    let player: ReturnType<typeof mediaPlayer> | undefined;
    onMount(() => {
      player = mediaPlayer(element, { position: t => timelineToSourceUs(props.clip, t), rate: () => props.clip.rate.numerator / props.clip.rate.denominator,
        frameUs: () => 1e6 / (asset()?.metadata.fps ?? 25), visibility: setFrameReady, fail });
      const release = props.players.register(`visual:${props.clip.id}`, player);
      onCleanup(() => { release(); player?.dispose(); });
    });
    return <video ref={element} src={url()} muted preload="auto" onLoadedData={() => player?.prefetch(props.clip.startUs)} />;
  }
  onMount(() => {
    if (asset()?.kind === 'image') {
      const release = props.players.register(`visual:${props.clip.id}`, { prepare: () => imageReady(), play: () => {}, error: () => error() || undefined });
      onCleanup(release);
    }
  });
  const geometry = () => sourceGeometry(props.width, props.height, asset()?.metadata.width || props.width, asset()?.metadata.height || props.height, clip().transform);
  const planeStyle = () => { const g = geometry(), crop = clip().transform.crop; return { width: `${g.width}px`, height: `${g.height}px`, left: `${g.left}px`, top: `${g.top}px`, 'clip-path': `inset(${crop.y * 100}% ${(1 - crop.x - crop.width) * 100}% ${(1 - crop.y - crop.height) * 100}% ${crop.x * 100}%)` }; };
  const layerStyle = () => { const t = clip().transform; return { left: `${t.x * 100}%`, top: `${t.y * 100}%`, opacity: t.opacity, transform: layerTransform(t), 'z-index': currentProject()?.timeline.tracks.find(track => track.id === props.clip.trackId)?.order ?? 0 }; };
  const typography = () => clipTypography(clip(), currentProject()?.timeline);
  const textStyle = () => { const s = typography(); return { 'font-family': s.fontFamily, 'font-size': `${s.fontSize * props.width / (previewSettings()?.width ?? props.width)}px`, 'font-weight': s.fontWeight, color: s.color, 'text-align': s.alignment, 'background-color': `${s.backgroundColor}${Math.round(s.backgroundOpacity * 255).toString(16).padStart(2, '0')}` }; };
  async function select() { setTimelinePlaying(false); await selectTimelineClip(props.clip.id); setActiveWorkspacePanelTab('properties'); }
  const regionStyle = (r: { x: number; y: number; width: number; height: number }) => ({ left: `${r.x * 100}%`, top: `${r.y * 100}%`, width: `${r.width * 100}%`, height: `${r.height * 100}%` });
  return <div class="timeline-preview-layer" classList={{ 'is-selected': selectedTimelineIds().includes(props.clip.id), 'is-audio': clip().content.kind === 'audio' }} data-clip-id={props.clip.id} data-prefetch={!props.active} style={{ ...layerStyle(), display: props.active ? undefined : 'none', visibility: asset()?.kind === 'video' && !frameReady() ? 'hidden' : undefined }}>
    <Show when={error()}><p class="timeline-source-error" role="alert">{asset()?.metadata.originalName}: {error()}</p></Show>
    <Show when={asset()}><div class="timeline-source-plane" style={planeStyle()} onClick={() => void select()}>
    <Show when={url()}>
      <Show when={asset()?.kind === 'video' && !props.native}><Video /></Show>
      <Show when={asset()?.kind === 'image'}><img src={url()} alt={asset()?.metadata.originalName ?? 'Ảnh trên timeline'} onLoad={() => setImageReady(true)} onError={() => fail('Không mở được ảnh nguồn.')} /></Show>
    </Show>
    <Show when={props.guides && clip().content.kind === 'video'}>
      <Show when={content()?.settings.transcriptSettings.ocrRegion}>{r => <div class="timeline-source-region ocr" style={regionStyle(r())}><span>OCR · nguồn</span></div>}</Show>
      <For each={content()?.settings.blurRegions.filter(r => r.enabled && r.startMs * 1000 <= (timelineToSourceUs(clip(), timelineTimeUs()) ?? -1) && (r.endMs === null || r.endMs * 1000 > (timelineToSourceUs(clip(), timelineTimeUs()) ?? -1))) ?? []}>{r => <div class="timeline-source-region" style={regionStyle(r)}><span>{r.effect === 'sttn' ? 'AI Remove' : 'Blur'} · nguồn</span></div>}</For>
    </Show>
    </div></Show>
    <Show when={clip().content.kind === 'text' || clip().content.kind === 'subtitles'}><div class="timeline-preview-typography" style={{ 'clip-path': planeStyle()['clip-path'] }} onClick={() => void select()}><span style={textStyle()}>{props.text}</span></div></Show>
  </div>;
}
export default function TimelinePreview() {
  let body!: HTMLDivElement;
  const [bounds, setBounds] = createSignal({ width: 640, height: 360 });
  const [guides, setGuides] = createSignal(false);
  const [retry, setRetry] = createSignal(1), [failures, setFailures] = createSignal<Record<string, string>>({});
  const [failure, setFailure] = createSignal('');
  const players = new PreviewPlayers(), clock = new PreviewClock();
  const aspect = () => (previewSettings()?.width ?? 16) / (previewSettings()?.height ?? 9);
  const width = () => Math.min(bounds().width, bounds().height * aspect());
  const height = () => Math.min(bounds().height, bounds().width / aspect());
  const resident = createMemo(() => currentProject() ? residentClips(currentProject()!.timeline, timelineTimeUs()) : []);
  watchSourceMedia(() => [...resident(), ...(currentProject()?.timeline.clips.filter(c => selectedTimelineIds().includes(c.id)) ?? [])].flatMap(c => contentId(c) ? [contentId(c)!] : []));
  const sources = createMemo(() => new Map((currentProject()?.timeline.videoContents ?? []).flatMap(v => {
    const data = sourceMedia(v.id)?.data; return data ? [[v.id, data] as const] : [];
  })));
  const regions = createMemo(() => currentProject() ? audioRegions(currentProject()!.timeline, sources()) : []);
  const visualDocument = createMemo(() => {
    const doc = currentProject()?.timeline;
    return doc ? { ...doc, settings: previewSettings() ?? doc.settings, clips: doc.clips.map(previewClip) } : undefined;
  });
  const scene = createMemo(() => visualDocument() ? timelineScene(visualDocument()!, sources(), timelineTimeUs(), regions()) : undefined);
  const clips = createMemo(() => resident().filter(c => ['video', 'image', 'text', 'subtitles'].includes(c.content.kind)));
  const activeIds = () => new Set(scene()?.layers.map(l => l.clip.id) ?? []);
  const nativeClip = createMemo(() => {
    const doc = visualDocument(), frame = scene();
    return doc && frame ? nativeSceneClip(doc, frame.layers, guides()) : undefined;
  });
  const native = createTimelineNativePreview(() => body?.querySelector<HTMLElement>('.timeline-preview-stage') ?? undefined, nativeClip, () => timelineAdvancing() && !timelineBuffering(), id => {
    setTimelinePlaying(false); void selectTimelineClip(id).then(() => setActiveWorkspacePanelTab('properties'));
  });
  createEffect(() => {
    const id = native.active() ? nativeClip()?.id : undefined; if (!id) return;
    const release = players.register(`native:${id}`, { prepare: () => native.ready(), play: () => {}, error: () => undefined });
    onCleanup(release);
  });
  const warningCount = () => {
    const c = currentProject()?.timeline.clips.find(c => c.id === selectedTimelineIds()[0]);
    return c ? clipWarnings(c, sourceMedia(contentId(c))?.data, usesSourceVoice(c, currentProject()?.timeline)).length : 0;
  };
  // Keep layer instances stable across project saves and transport ticks.
  const ids = createMemo(() => clips().map(c => c.id));
  function fail(id: string, error: string) { setFailures(p => p[id] === error ? p : { ...p, [id]: error }); }
  let frame = 0;
  function synchronize() {
    const value = scene(); if (!value) return;
    const required = [...value.layers.filter(l => l.asset).map(l => `${native.active() && nativeClip()?.id === l.clip.id ? 'native' : 'visual'}:${l.clip.id}`), ...value.audio.map(r => `audio:${r.id}`)];
    const manifests = value.active.flatMap(c => contentId(c) ? [sourceMedia(contentId(c))] : []);
    const error = required.map(id => failures()[id]).find(Boolean) || manifests.find(m => m?.error)?.error;
    const result = players.sync(required, timelineTimeUs(), timelineSeekRevision(), timelineAdvancing() && !error, manifests.every(m => !!m && !m.loading));
    const message = error || result.error || '';
    setFailure(message); setTimelineBuffering(!result.ready && !message);
    if (message) setTimelinePlaying(false);
    return { ...result, ready: result.ready && !message };
  }
  // User seek pauses every participant immediately, before the next animation frame.
  createEffect(on(() => [timelineSeekRevision(), timelineAdvancing(), currentProject()?.timeline, sources()], () => synchronize()));
  onMount(() => {
    const observer = new ResizeObserver(() => setBounds({ width: body.clientWidth, height: body.clientHeight }));
    observer.observe(body);
    onCleanup(() => observer.disconnect());
    function tick(now: number) {
      const result = synchronize(), value = scene();
      if (value) {
        const next = clock.advance(now, timelineTimeUs(), timelineSeekRevision(), timelineAdvancing(), !!result?.ready, value.nextBoundaryUs);
        publishTimelineTime(next);
        if (next >= value.endUs) setTimelinePlaying(false);
        synchronize();
      }
      frame = requestAnimationFrame(tick);
    }
    frame = requestAnimationFrame(tick);
    const hidden = () => { if (document.hidden) { setTimelinePlaying(false); players.pause(); } };
    document.addEventListener('visibilitychange', hidden);
    onCleanup(() => document.removeEventListener('visibilitychange', hidden));
  });
  createEffect(() => { if (segmentVoicePreviewActive()) setTimelinePlaying(false); });
  onCleanup(() => { cancelAnimationFrame(frame); players.pause(); setTimelinePlaying(false); setTimelineBuffering(false); });
  return <section class="timeline-preview" aria-label="Xem trước theo thời gian dự án">
    <div class="timeline-preview-heading"><span>Timeline · {formatTimelineTime(timelineTimeUs(), true)}</span><label><input type="checkbox" checked={guides()} onChange={e => setGuides(e.currentTarget.checked)} /> Vùng OCR/xóa chữ</label><button type="button" class="button button-quiet" onClick={reviewSelectedSource}>Xem video đang chọn</button></div>
    <Show when={warningCount()}><button class="timeline-audio-warning-link" type="button" onClick={() => setActiveWorkspacePanelTab('properties')}>{warningCount()} cảnh báo lời/giọng · Chỉnh trong Âm thanh</button></Show>
    <Show when={timelineBuffering()}><p class="timeline-preview-status" role="status">Đang nạp hình và âm thanh tại playhead…</p></Show>
    <Show when={failure()}><p class="timeline-preview-status error" role="alert">{failure()} <button type="button" onClick={() => { players.pause(); setFailures({}); setFailure(''); refreshSourceMedia(); setRetry(v => v + 1); seekTimeline(timelineTimeUs()); }}>Thử lại preview</button></p></Show>
    <Show when={native.warning()}><p class="timeline-preview-status" role="status">{native.warning()}</p></Show>
    <For each={[retry()]}>{() => <TimelineAudio players={players} regions={regions()} fail={fail} />}</For>
    <div class="timeline-preview-body" ref={body}><div class="timeline-preview-stage" style={{ 'background-color': previewSettings()?.backgroundColor ?? '#000', width: `${width()}px`, height: `${height()}px` }}>
      <For each={[retry()]}>{() => <For each={ids()}>{id => <SourceLayer clip={clips().find(c => c.id === id)!} width={width()} height={height()} guides={guides()} active={activeIds().has(id)} native={native.active() && nativeClip()?.id === id} text={scene()?.layers.find(l => l.clip.id === id)?.text ?? ''} players={players} fail={fail} />}</For>}</For>
      <Show when={!scene()?.layers.some(l => ['video', 'image', 'text'].includes(l.clip.content.kind))}><span class="timeline-preview-gap">{scene()?.active.length ? 'Audio / phụ đề' : 'Khoảng trống'}</span></Show>
    </div></div>
  </section>;
}
