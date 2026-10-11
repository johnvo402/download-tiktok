import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount } from 'solid-js';
import { convertFileSrc } from '@tauri-apps/api/core';
import { currentProject } from '../../app/project-state';
import { mediaService } from '../../app/services/media-service';
import { audioOffsetUs, regionAudible, regionGain, type AudioRegion } from './audio-layout';
import { timelineTimeUs } from './timeline-state';
import { PREFETCH_US } from './scene-plan';
import { mediaPlayer } from './media-player';
import type { PreviewPlayers } from './preview-clock';

/** Existing files stream into one gain/compressor graph. The shared preview
 * transport gates all nodes together and immediately mutes old seeks. */
export default function TimelineAudio(props: { players: PreviewPlayers; regions: AudioRegion[]; fail(id: string, error: string): void }) {
  let context: AudioContext | undefined, output: DynamicsCompressorNode | undefined;
  const regions = () => props.regions;
  const resident = createMemo(() => {
    const doc = currentProject()?.timeline, time = timelineTimeUs();
    const audible = regions().filter(r => doc && regionAudible(r, doc));
    const active = audible.filter(r => r.startUs <= time && r.endUs > time);
    const next = audible.filter(r => r.startUs > time && r.startUs <= time + PREFETCH_US).sort((a, b) => a.startUs - b.startUs).slice(0, 8);
    return [...active, ...next];
  });
  const ids = createMemo(() => resident().map(r => r.id));
  function graph(media: HTMLAudioElement) {
    if (!context) {
      context = new AudioContext(); output = context.createDynamicsCompressor();
      output.threshold.value = -1; output.knee.value = 0; output.ratio.value = 20;
      output.attack.value = .003; output.release.value = .1; output.connect(context.destination);
    }
    const gain = context.createGain(), source = context.createMediaElementSource(media);
    gain.gain.value = 0; source.connect(gain); gain.connect(output!);
    return { context, gain, source };
  }
  onCleanup(() => { output?.disconnect(); if (context) void context.close(); });
  function Player(p: { id: string }) {
    const region = () => resident().find(r => r.id === p.id)!;
    const [url, setUrl] = createSignal('');
    let media!: HTMLAudioElement, nodes: ReturnType<typeof graph> | undefined, player: ReturnType<typeof mediaPlayer> | undefined;
    const identity = createMemo(() => `${currentProject()?.id}:${region()?.assetId ?? ''}:${currentProject()?.timeline.assets.find(a => a.id === region()?.assetId)?.relativePath ?? ''}:${region()?.path ?? ''}`);
    function fail(message: string) { props.fail(`audio:${p.id}`, message); }
    createEffect(on(identity, () => {
      const r = region(), project = currentProject(); if (!r || !project) return;
      let alive = true; setUrl(''); media?.pause();
      void (r.path ? Promise.resolve(r.path) : mediaService.path(project.id, r.assetId!))
        .then(path => { if (alive) setUrl(convertFileSrc(path)); }).catch(e => { if (alive) fail(e instanceof Error ? e.message : String(e)); });
      onCleanup(() => { alive = false; });
    }));
    const gain = () => { const r = region(), doc = currentProject()?.timeline; return r && doc ? regionGain(r, timelineTimeUs(), regions(), doc) : 0; };
    function Element() {
      onMount(() => {
        player = mediaPlayer(media, {
          position: t => { const r = region(); return r ? audioOffsetUs(r, t) : null; },
          rate: () => { const r = region(); return r.clip.rate.numerator / r.clip.rate.denominator * r.speed; }, frameUs: () => 2000,
          gain: playing => {
            if (!playing) { if (nodes) { nodes.gain.gain.cancelScheduledValues(nodes.context.currentTime); nodes.gain.gain.setValueAtTime(0, nodes.context.currentTime); } return; }
            try {
              nodes ??= graph(media); nodes.gain.gain.setTargetAtTime(gain(), nodes.context.currentTime, .005);
              if (nodes.context.state === 'suspended') void nodes.context.resume().catch(e => fail(String(e)));
            } catch (e) { fail(String(e)); }
          }, fail,
        });
        const release = props.players.register(`audio:${p.id}`, player);
        onCleanup(() => { release(); player?.dispose(); nodes?.source.disconnect(); nodes?.gain.disconnect(); nodes = undefined; });
      });
      return <audio ref={media} src={url()} crossOrigin="anonymous" preload="auto" onLoadedData={() => player?.prefetch(region().startUs)} />;
    }
    return <div class="timeline-audio-region" data-audio-region={p.id} data-audio-clip-id={region()?.clip.id} data-gain={gain()} data-prefetch={region()?.startUs > timelineTimeUs()}>
      <For each={url() ? [url()] : []}>{() => <Element />}</For>
    </div>;
  }
  return <div class="timeline-audio-mixer" aria-label="Audio timeline"><For each={ids()}>{id => <Player id={id} />}</For></div>;
}
