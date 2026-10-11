import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { convertFileSrc } from '@tauri-apps/api/core';
import { currentProject } from '../../app/project-state';
import { mediaService } from '../../app/services/media-service';
import { audioOffsetUs, audioRegions, regionAudible, regionGain } from './audio-layout';
import { sourceMedia } from './source-media';
import { setTimelinePlaying, timelinePlaying, timelineTimeUs } from './timeline-state';

/** Stream existing media into a shared gain/compressor graph; never render a preview file. */
export default function TimelineAudio() {
  let context: AudioContext | undefined, output: DynamicsCompressorNode | undefined;
  const regions = createMemo(() => {
    const doc = currentProject()?.timeline;
    return doc ? audioRegions(doc, new Map(doc.videoContents.flatMap(v => {
      const source = sourceMedia(v.id)?.data; return source ? [[v.id, source] as const] : [];
    }))) : [];
  });
  const active = createMemo(() => regions().filter(r => {
    const doc = currentProject()?.timeline;
    return doc && regionAudible(r, doc) && r.startUs <= timelineTimeUs() && r.endUs > timelineTimeUs();
  }));
  const ids = createMemo(() => active().map(r => r.id));
  function graph(media: HTMLAudioElement) {
    if (!context) {
      context = new AudioContext(); output = context.createDynamicsCompressor();
      output.threshold.value = -1; output.knee.value = 0; output.ratio.value = 20;
      output.attack.value = .003; output.release.value = .1; output.connect(context.destination);
    }
    const gain = context.createGain(), source = context.createMediaElementSource(media);
    gain.gain.value = 0;
    source.connect(gain); gain.connect(output!);
    return { context, gain, source };
  }
  onCleanup(() => { output?.disconnect(); if (context) void context.close(); });
  function Player(props: { id: string }) {
    const region = () => active().find(r => r.id === props.id)!;
    const [url, setUrl] = createSignal(''), [error, setError] = createSignal('');
    let media!: HTMLAudioElement, nodes: ReturnType<typeof graph> | undefined, disposed = false;
    const identity = createMemo(() => `${currentProject()?.id}:${region()?.assetId ?? ''}:${currentProject()?.timeline.assets.find(a => a.id === region()?.assetId)?.relativePath ?? ''}:${region()?.path ?? ''}`);
    createEffect(on(identity, () => {
      const r = region(), project = currentProject(); if (!r || !project) return;
      let alive = true;
      setUrl(''); setError(''); media?.pause();
      void (r.path ? Promise.resolve(r.path) : mediaService.path(project.id, r.assetId!))
        .then(path => { if (alive) setUrl(convertFileSrc(path)); }).catch(e => { if (alive) fail(e instanceof Error ? e.message : String(e)); });
      onCleanup(() => { alive = false; media?.pause(); });
    }));
    const gain = () => { const r = region(), doc = currentProject()?.timeline; return r && doc ? regionGain(r, timelineTimeUs(), regions(), doc) : 0; };
    function fail(message: string) { if (disposed) return; setError(message); media?.pause(); setTimelinePlaying(false); }
    function sync() {
      const r = region(); if (!r || !media || media.readyState < 1 || !url()) return;
      const offset = audioOffsetUs(r, timelineTimeUs()); if (offset === null) { media.pause(); return; }
      const position = Math.min(Math.max(0, offset / 1e6), Number.isFinite(media.duration) ? media.duration : Infinity);
      media.playbackRate = Math.max(.0625, Math.min(16, r.clip.rate.numerator / r.clip.rate.denominator * r.speed));
      if (!timelinePlaying() || Math.abs(media.currentTime - position) > .15) media.currentTime = position;
      if (!timelinePlaying()) { media.pause(); return; }
      try {
        nodes ??= graph(media);
        nodes.gain.gain.setTargetAtTime(gain(), nodes.context.currentTime, .005);
        if (nodes.context.state === 'suspended') void nodes.context.resume().catch(e => fail(String(e)));
        if (media.paused && position < media.duration) void media.play().catch(e => { if (e?.name !== 'AbortError') fail('Không phát được audio. Thử phát lại hoặc kiểm tra nguồn.'); });
      } catch (e) { fail(e instanceof Error ? e.message : String(e)); }
    }
    createEffect(() => { timelineTimeUs(); timelinePlaying(); url(); gain(); region(); sync(); });
    onCleanup(() => { disposed = true; media?.pause(); nodes?.source.disconnect(); nodes?.gain.disconnect(); });
    return <div class="timeline-audio-region" data-audio-region={props.id} data-audio-clip-id={region()?.clip.id} data-gain={gain()}>
      <audio ref={media} src={url() || undefined} crossOrigin="anonymous" preload="auto" onLoadedMetadata={sync} onError={() => { if (url()) fail('Không đọc được audio trên timeline. Kiểm tra file nguồn hoặc tạo lại đoạn giọng bị lỗi.'); }} />
      <Show when={error()}><span role="alert">{error()}</span></Show>
    </div>;
  }
  return <div class="timeline-audio-mixer" aria-label="Audio timeline"><For each={ids()}>{id => <Player id={id} />}</For></div>;
}
