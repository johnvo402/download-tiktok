import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from 'solid-js';
import { convertFileSrc } from '@tauri-apps/api/core';
import { currentProject, segmentVoicePreviewActive } from '../../app/project-state';
import { mediaService } from '../../app/services/media-service';
import { activeClips, formatTimelineTime, trackAudible } from './navigation';
import { timelineDurationUs, timelineToSourceUs } from './time';
import { reviewSelectedSource, seekTimeline, setTimelinePlaying, timelinePlaying, timelineTimeUs } from './timeline-state';
import type { TimelineClip } from './types';

/** T04 source navigation. Full transforms, captions/TTS and native compositing follow in T06–T08. */
function SourceLayer(props: { clip: TimelineClip }) {
  const [url, setUrl] = createSignal('');
  const [error, setError] = createSignal('');
  let media: HTMLMediaElement | undefined;
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
    const track = project.timeline.tracks.find(t => t.id === props.clip.trackId);
    media.muted = !track || !trackAudible(track, project.timeline.tracks);
    media.playbackRate = Math.max(.0625, Math.min(16, props.clip.rate.numerator / props.clip.rate.denominator));
    if (!timelinePlaying() || Math.abs(media.currentTime - time / 1e6) > .15) media.currentTime = time / 1e6;
    if (timelinePlaying() && media.paused) void media.play().catch(() => { setError('Nguồn chưa phát được. Có thể kéo playhead để xem từng thời điểm.'); setTimelinePlaying(false); });
    else if (!timelinePlaying()) media.pause();
  }
  createEffect(() => { url(); timelineTimeUs(); timelinePlaying(); currentProject(); sync(); });
  onCleanup(() => { media?.pause(); });
  return <>
    <Show when={error()}><p class="timeline-source-error" role="alert">{asset()?.metadata.originalName}: {error()}</p></Show>
    <Show when={url()}>
      <Show when={asset()?.kind === 'video'}><video ref={element => { media = element; }} src={url()} preload="auto" onLoadedMetadata={sync} onError={() => { setError('Không giải mã được video nguồn.'); setTimelinePlaying(false); }} /></Show>
      <Show when={asset()?.kind === 'image'}><img src={url()} alt={asset()?.metadata.originalName ?? 'Ảnh trên timeline'} onError={() => setError('Không mở được ảnh nguồn.')} /></Show>
      <Show when={asset()?.kind === 'audio'}><audio ref={element => { media = element; }} src={url()} preload="auto" onLoadedMetadata={sync} onError={() => { setError('Không giải mã được audio nguồn.'); setTimelinePlaying(false); }} /></Show>
    </Show>
    <Show when={props.clip.content.kind === 'text'}><div class="timeline-preview-text">{props.clip.content.kind === 'text' ? props.clip.content.text : ''}</div></Show>
  </>;
}
export default function TimelinePreview() {
  let body!: HTMLDivElement;
  const [bounds, setBounds] = createSignal({ width: 640, height: 360 });
  const aspect = () => (currentProject()?.timeline.settings.width ?? 16) / (currentProject()?.timeline.settings.height ?? 9);
  const clips = createMemo(() => currentProject() ? activeClips(currentProject()!.timeline, timelineTimeUs()).filter(c => !['voice', 'subtitles'].includes(c.content.kind)) : []);
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
    <div class="timeline-preview-heading"><span>Timeline · {formatTimelineTime(timelineTimeUs(), true)}</span><button type="button" class="button button-quiet" onClick={reviewSelectedSource}>Xem video đang chọn</button></div>
    <div class="timeline-preview-body" ref={body}><div class="timeline-preview-stage" style={{ 'background-color': currentProject()?.timeline.settings.backgroundColor ?? '#000', width: `${Math.min(bounds().width, bounds().height * aspect())}px`, height: `${Math.min(bounds().height, bounds().width / aspect())}px` }}>
      <For each={ids()}>{id => <SourceLayer clip={clips().find(c => c.id === id)!} />}</For>
      <Show when={!clips().some(c => ['video', 'image', 'text'].includes(c.content.kind))}><span class="timeline-preview-gap">{clips().length ? 'Audio' : 'Khoảng trống'}</span></Show>
    </div></div>
  </section>;
}
