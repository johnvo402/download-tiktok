import { createEffect, createSignal, createMemo, on, onCleanup, onMount, Show } from 'solid-js';
import { convertFileSrc } from '@tauri-apps/api/core';
import { currentProject, currentTranscript, ocrRegionPicking, ocrRegionEditing, segmentVoicePreviewActive, setOcrRegionEditing, setOcrRegionPicking, setPlaybackTime, setCurrentProject, setNotice, seekRequest, setSeekRequest, setVideoPlaying } from '../../app/project-state';
import { projectService } from '../../app/services/project-service';
import { projectForVideo } from './video-context';
import { selectedVideoId } from './video-context-state';
import { findTranslatedSegmentAtTime } from '../transcript/transcript-utils';

/** Source-local review for T03. Timeline compositing/transport belongs to T08. */
export default function SourceVideoPreview() {
  const [url, setUrl] = createSignal('');
  const [error, setError] = createSignal('');
  const [time, setTime] = createSignal(0);
  const [region, setRegion] = createSignal<{ x: number; y: number; width: number; height: number }>();
  const [bounds, setBounds] = createSignal({ left:0, top:0, width:0, height:0 });
  let video!: HTMLVideoElement;
  let stage!: HTMLDivElement;
  let start: { x: number; y: number } | undefined;
  let initial: ReturnType<typeof region>;
  let dragMode: 'draw' | 'move' | 'resize' = 'draw';
  const view = () => currentProject() && projectForVideo(currentProject()!, selectedVideoId());
  const identity = createMemo(() => `${currentProject()?.id}:${selectedVideoId()}:${view()?.source.videoPath}`);
  createEffect(on(identity, () => {
    const project = currentProject(); const id = selectedVideoId();
    if (!project || !id) return;
    // Identity deliberately excludes updatedAt and clip position.
    const source = projectForVideo(project, id).source.videoPath;
    void source;
    let active = true;
    setUrl(''); setError(''); setTime(0); setRegion(undefined); setPlaybackTime(0); setVideoPlaying(false);
    void projectService.getVideoAssetPath(project.id, id).then((path) => {
      if (active) setUrl(convertFileSrc(path));
    }).catch((error) => { if (active) setError(error instanceof Error ? error.message : 'Không mở được nguồn video.'); });
    onCleanup(() => { active = false; video?.pause(); });
  }));
  createEffect(() => {
    if (segmentVoicePreviewActive()) video?.pause();
  });
  createEffect(() => {
    const seek = seekRequest();
    if (seek === null || !video || !url()) return;
    video.currentTime = Math.max(0, Math.min(seek, Number.isFinite(video.duration) ? video.duration : seek));
    setSeekRequest(null);
  });
  const caption = () => findTranslatedSegmentAtTime(currentTranscript()?.segments ?? [], time(), view()?.subtitle.enabled ?? true)?.translated;
  const crop = () => region() ?? view()?.transcriptSettings.ocrRegion ?? undefined;
  function measure() {
    if (!video || !stage) return;
    const frame = video.getBoundingClientRect(); const parent = stage.getBoundingClientRect();
    setBounds({ left:frame.left-parent.left, top:frame.top-parent.top, width:frame.width, height:frame.height });
  }
  onMount(() => {
    const observer = new ResizeObserver(measure); observer.observe(stage);
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape') { start=undefined;setOcrRegionPicking(false);setOcrRegionEditing(false); } };
    window.addEventListener('keydown', escape);
    onCleanup(() => { observer.disconnect(); window.removeEventListener('keydown', escape); });
  });
  function point(event: PointerEvent) {
    const bounds = video.getBoundingClientRect();
    return { x: Math.max(0, Math.min(1, (event.clientX - bounds.left) / bounds.width)), y: Math.max(0, Math.min(1, (event.clientY - bounds.top) / bounds.height)) };
  }
  function move(event: PointerEvent) {
    if (!start) return;
    const end = point(event);
    if (initial && dragMode === 'move') {
      setRegion({ ...initial, x:Math.max(0, Math.min(1-initial.width, initial.x+end.x-start.x)), y:Math.max(0, Math.min(1-initial.height, initial.y+end.y-start.y)) });
    } else if (initial && dragMode === 'resize') {
      setRegion({ ...initial, width:Math.max(.01,Math.min(1-initial.x,initial.width+end.x-start.x)),height:Math.max(.01,Math.min(1-initial.y,initial.height+end.y-start.y)) });
    } else setRegion({ x: Math.min(start.x, end.x), y: Math.min(start.y, end.y), width: Math.abs(end.x - start.x), height: Math.abs(end.y - start.y) });
  }
  async function finish(event: PointerEvent) {
    move(event); start = undefined;
    const crop = region(); const project = currentProject(); const id = selectedVideoId();
    if (!project || !id || !crop || crop.width < .01 || crop.height < .01) return;
    try {
      const settings = projectForVideo(project, id).transcriptSettings;
      const saved = await projectService.updateTranscriptSettings(project.id, { ...settings, ocrRegion: crop }, id);
      if (currentProject()?.id === project.id && saved.updatedAt >= currentProject()!.updatedAt) setCurrentProject(saved);
      if (selectedVideoId() === id) setOcrRegionPicking(false);
    } catch (error) { setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không lưu được vùng OCR.' }); }
  }
  onCleanup(() => { video?.pause(); setVideoPlaying(false); });
  return <div class="source-video-preview"><p>Xem nguồn · {view()?.source.originalName} · Thời gian của video nguồn</p>
    <Show when={error()}><p role="alert">{error()}</p></Show>
    <div class="source-video-stage" ref={stage}><Show when={url()}><video ref={video} src={url()} controls preload="metadata" style={{ 'object-fit': 'contain' }} onLoadedMetadata={measure}
      onTimeUpdate={() => { setTime(video.currentTime); setPlaybackTime(video.currentTime); }} onPlay={() => setVideoPlaying(true)} onPause={() => setVideoPlaying(false)} />
      <Show when={view()?.subtitle.enabled && caption()}><div class="source-video-caption" style={{ left:`${bounds().left+bounds().width*.1}px`,width:`${bounds().width*.8}px`,top:`${bounds().top+bounds().height-80}px`,right:'auto',bottom:'auto' }}>{caption()}</div></Show>
      <Show when={ocrRegionPicking() || ocrRegionEditing()}><svg class="source-video-ocr" style={{left:`${bounds().left}px`,top:`${bounds().top}px`,width:`${bounds().width}px`,height:`${bounds().height}px`,right:'auto',bottom:'auto'}} viewBox="0 0 1000 1000" preserveAspectRatio="none" aria-label="Kéo chọn vùng OCR trên video" onPointerDown={(event) => {
        initial=crop(); dragMode=ocrRegionEditing() && event.target instanceof SVGElement && event.target.tagName==='circle' ? 'resize'
          : ocrRegionEditing() && event.target instanceof SVGElement && event.target.tagName==='rect' ? 'move' : 'draw';
        start = point(event); event.currentTarget.setPointerCapture(event.pointerId); video.pause();
      }} onPointerMove={move} onPointerUp={(event) => void finish(event)}>
        <Show when={crop()}>{(value) => <><rect x={value().x * 1000} y={value().y * 1000} width={value().width * 1000} height={value().height * 1000} />
          <Show when={ocrRegionEditing()}><circle cx={(value().x+value().width)*1000} cy={(value().y+value().height)*1000} r="10" fill="#59d2e8" /></Show></>}</Show>
      </svg></Show>
    </Show></div>
  </div>;
}
