import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from 'solid-js';
import { convertFileSrc } from '@tauri-apps/api/core';
import { currentProject, currentTranscript, segmentVoicePreviewActive, setActiveWorkspacePanelTab } from '../../app/project-state';
import { mediaService } from '../../app/services/media-service';
import { activeClips, formatTimelineTime } from './navigation';
import { timelineDurationUs, timelineToSourceUs } from './time';
import { reviewSelectedSource, seekTimeline, selectTimelineClip, selectedTimelineIds, setTimelinePlaying, timelinePlaying, timelineTimeUs } from './timeline-state';
import type { TimelineClip } from './types';
import { captionAt, clipWarnings, contentId, usesSourceVoice } from './audio-layout';
import { sourceMedia, watchSourceMedia } from './source-media';
import TimelineAudio from './TimelineAudio';
import { clipTypography, layerTransform, sourceGeometry } from './visual-layout';
import { previewClip, previewSettings } from './visual-state';

/** Browser compositor; native rendering and prefetch/clock refinements follow in T08. */
function SourceLayer(props: { clip: TimelineClip; width: number; height: number; guides: boolean }) {
  const [url, setUrl] = createSignal('');
  const [error, setError] = createSignal('');
  let media: HTMLMediaElement | undefined;
  const clip = () => previewClip(props.clip);
  const content = () => { const c = clip().content; return 'videoContentId' in c ? currentProject()?.timeline.videoContents.find(v => v.id === c.videoContentId) : undefined; };
  const asset = () => { const content = props.clip.content; return 'assetId' in content ? currentProject()?.timeline.assets.find(a => a.id === content.assetId) : undefined; };
  const identity = createMemo(() => `${currentProject()?.id}:${asset()?.id}:${asset()?.relativePath}`);
  createEffect(on(identity, () => {
    const source = asset(), project = currentProject();
    if (!source || !project) return;
    let alive = true;
    setUrl(''); setError('');
    void mediaService.path(project.id, source.id).then(path => { if (alive) setUrl(convertFileSrc(path)); })
      .catch(error => { if (alive) { setError(error instanceof Error ? error.message : 'Không mở được nguồn.'); setTimelinePlaying(false); } });
    onCleanup(() => { alive = false; media?.pause(); });
  }));
  function sync() {
    const project = currentProject();
    const time = timelineToSourceUs(props.clip, timelineTimeUs());
    if (!media || !project || media.readyState < 1 || time === null) return;
    media.muted = true; // The dedicated timeline mixer owns all audio, including the video's original track.
    media.playbackRate = Math.max(.0625, Math.min(16, props.clip.rate.numerator / props.clip.rate.denominator));
    if (!timelinePlaying() || Math.abs(media.currentTime - time / 1e6) > .15) media.currentTime = time / 1e6;
    if (timelinePlaying() && media.paused) void media.play().catch(() => { setError('Nguồn chưa phát được. Có thể kéo playhead để xem từng thời điểm.'); setTimelinePlaying(false); });
    else if (!timelinePlaying()) media.pause();
  }
  createEffect(() => { url(); timelineTimeUs(); timelinePlaying(); currentProject(); sync(); });
  onCleanup(() => { media?.pause(); });
  const geometry = () => sourceGeometry(props.width, props.height, asset()?.metadata.width || props.width, asset()?.metadata.height || props.height, clip().transform);
  const planeStyle = () => { const g = geometry(), crop = clip().transform.crop; return { width: `${g.width}px`, height: `${g.height}px`, left: `${g.left}px`, top: `${g.top}px`, 'clip-path': `inset(${crop.y * 100}% ${(1 - crop.x - crop.width) * 100}% ${(1 - crop.y - crop.height) * 100}% ${crop.x * 100}%)` }; };
  const layerStyle = () => { const t = clip().transform; return { left: `${t.x * 100}%`, top: `${t.y * 100}%`, opacity: t.opacity, transform: layerTransform(t), 'z-index': currentProject()?.timeline.tracks.find(track => track.id === props.clip.trackId)?.order ?? 0 }; };
  const typography = () => clipTypography(clip(), currentProject()?.timeline);
  const textStyle = () => { const s = typography(); return { 'font-family': s.fontFamily, 'font-size': `${s.fontSize * props.width / (previewSettings()?.width ?? props.width)}px`, 'font-weight': s.fontWeight, color: s.color, 'text-align': s.alignment, 'background-color': `${s.backgroundColor}${Math.round(s.backgroundOpacity * 255).toString(16).padStart(2, '0')}` }; };
  const caption = () => {
    const id = content()?.id;
    const data = currentTranscript()?.videoContentId === id ? currentTranscript() : sourceMedia(id)?.data?.transcript;
    return captionAt(clip(), data, timelineTimeUs());
  };
  async function select() { setTimelinePlaying(false); await selectTimelineClip(props.clip.id); setActiveWorkspacePanelTab('properties'); }
  const regionStyle = (r: { x: number; y: number; width: number; height: number }) => ({ left: `${r.x * 100}%`, top: `${r.y * 100}%`, width: `${r.width * 100}%`, height: `${r.height * 100}%` });
  return <div class="timeline-preview-layer" classList={{ 'is-selected': selectedTimelineIds().includes(props.clip.id), 'is-audio': clip().content.kind === 'audio' }} data-clip-id={props.clip.id} style={layerStyle()}>
    <Show when={error()}><p class="timeline-source-error" role="alert">{asset()?.metadata.originalName}: {error()}</p></Show>
    <Show when={asset()}><div class="timeline-source-plane" style={planeStyle()} onClick={() => void select()}>
    <Show when={url()}>
      <Show when={asset()?.kind === 'video'}><video ref={element => { media = element; }} src={url()} muted preload="auto" onLoadedMetadata={sync} onError={() => { setError('Không giải mã được video nguồn.'); setTimelinePlaying(false); }} /></Show>
      <Show when={asset()?.kind === 'image'}><img src={url()} alt={asset()?.metadata.originalName ?? 'Ảnh trên timeline'} onError={() => setError('Không mở được ảnh nguồn.')} /></Show>
    </Show>
    <Show when={props.guides && clip().content.kind === 'video'}>
      <Show when={content()?.settings.transcriptSettings.ocrRegion}>{r => <div class="timeline-source-region ocr" style={regionStyle(r())}><span>OCR · nguồn</span></div>}</Show>
      <For each={content()?.settings.blurRegions.filter(r => r.enabled && r.startMs * 1000 <= (timelineToSourceUs(clip(), timelineTimeUs()) ?? -1) && (r.endMs === null || r.endMs * 1000 > (timelineToSourceUs(clip(), timelineTimeUs()) ?? -1))) ?? []}>{r => <div class="timeline-source-region" style={regionStyle(r)}><span>{r.effect === 'sttn' ? 'AI Remove' : 'Blur'} · nguồn</span></div>}</For>
    </Show>
    </div></Show>
    <Show when={clip().content.kind === 'text' || clip().content.kind === 'subtitles'}><div class="timeline-preview-typography" style={{ 'clip-path': planeStyle()['clip-path'] }} onClick={() => void select()}><span style={textStyle()}>{clip().content.kind === 'text' ? (clip().content as { text: string }).text : caption()}</span></div></Show>
  </div>;
}
export default function TimelinePreview() {
  let body!: HTMLDivElement;
  const [bounds, setBounds] = createSignal({ width: 640, height: 360 });
  const [guides, setGuides] = createSignal(false);
  const aspect = () => (previewSettings()?.width ?? 16) / (previewSettings()?.height ?? 9);
  const width = () => Math.min(bounds().width, bounds().height * aspect());
  const height = () => Math.min(bounds().height, bounds().width / aspect());
  watchSourceMedia(() => (currentProject()?.timeline.clips ?? []).flatMap(c => contentId(c) ? [contentId(c)!] : []));
  const active = createMemo(() => currentProject() ? activeClips(currentProject()!.timeline, timelineTimeUs()) : []);
  const clips = createMemo(() => active().filter(c => !['voice', 'audio', 'sourceAudio'].includes(c.content.kind)));
  const warningCount = () => {
    const c = currentProject()?.timeline.clips.find(c => c.id === selectedTimelineIds()[0]);
    return c ? clipWarnings(c, sourceMedia(contentId(c))?.data, usesSourceVoice(c, currentProject()?.timeline)).length : 0;
  };
  // Keep layer instances stable across project saves and transport ticks.
  const ids = createMemo(() => clips().map(c => c.id));
  let frame = 0;
  onMount(() => {
    const observer = new ResizeObserver(() => setBounds({ width: body.clientWidth, height: body.clientHeight }));
    observer.observe(body);
    onCleanup(() => observer.disconnect());
    let previous = performance.now();
    function tick(now: number) {
      const delta = Math.min(100_000, Math.round((now - previous) * 1000)); previous = now;
      if (timelinePlaying()) {
        const end = timelineDurationUs(currentProject()?.timeline.clips ?? []);
        const next = Math.min(end, timelineTimeUs() + delta);
        seekTimeline(next);
        if (next >= end) setTimelinePlaying(false);
      }
      frame = requestAnimationFrame(tick);
    }
    frame = requestAnimationFrame(tick);
  });
  createEffect(() => { if (segmentVoicePreviewActive()) setTimelinePlaying(false); });
  onCleanup(() => { cancelAnimationFrame(frame); setTimelinePlaying(false); });
  return <section class="timeline-preview" aria-label="Xem trước theo thời gian dự án">
    <div class="timeline-preview-heading"><span>Timeline · {formatTimelineTime(timelineTimeUs(), true)}</span><label><input type="checkbox" checked={guides()} onChange={e => setGuides(e.currentTarget.checked)} /> Vùng OCR/xóa chữ</label><button type="button" class="button button-quiet" onClick={reviewSelectedSource}>Xem video đang chọn</button></div>
    <Show when={warningCount()}><button class="timeline-audio-warning-link" type="button" onClick={() => setActiveWorkspacePanelTab('properties')}>{warningCount()} cảnh báo lời/giọng · Chỉnh trong Âm thanh</button></Show>
    <TimelineAudio />
    <div class="timeline-preview-body" ref={body}><div class="timeline-preview-stage" style={{ 'background-color': previewSettings()?.backgroundColor ?? '#000', width: `${width()}px`, height: `${height()}px` }}>
      <For each={ids()}>{id => <SourceLayer clip={clips().find(c => c.id === id)!} width={width()} height={height()} guides={guides()} />}</For>
      <Show when={!clips().some(c => ['video', 'image', 'text'].includes(c.content.kind))}><span class="timeline-preview-gap">{active().length ? 'Audio / phụ đề' : 'Khoảng trống'}</span></Show>
    </div></div>
  </section>;
}
