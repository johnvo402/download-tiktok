import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from 'solid-js';
import { convertFileSrc } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { AudioLines, Film, Image, Link, Plus, RefreshCw, Search } from 'lucide-solid';
import { currentProject, playbackTime, setNotice } from '../../app/project-state';
import { flushPendingSaves } from '../../app/save-guard';
import { mediaService, type MediaAssetStatus } from '../../app/services/media-service';
import { mergeVisualProject } from '../overlays/overlay-state';
import type { MediaAsset } from '../timeline/types';
import { secondsToTimeUs, timelineDurationUs } from '../timeline/time';
import { timelinePreviewActive, timelineTimeUs } from '../timeline/timeline-state';
import { browseMedia, importMediaPaths, mediaBusy, mediaProgress, mediaResults, setMediaBusy } from './media-state';
import { filterMedia, mediaDuration, mediaName, mediaUsage, MEDIA_EXTENSIONS } from './media-utils';
import './media-library.css';

function AssetCard(props: { asset: MediaAsset; projectId: string; status?: MediaAssetStatus; refresh: () => Promise<void> }) {
  const [thumbnail, setThumbnail] = createSignal('');
  const [peaks, setPeaks] = createSignal<number[]>();
  const [cacheError, setCacheError] = createSignal('');
  const [waveLoading, setWaveLoading] = createSignal(false);
  let element!: HTMLElement;
  let alive = true;
  const name = () => props.asset.metadata.originalName || mediaName(props.asset.relativePath);
  const Icon = props.asset.kind === 'video' ? Film : props.asset.kind === 'image' ? Image : AudioLines;
  const available = () => props.status?.state === 'available';
  onMount(() => {
    if (props.asset.kind === 'audio') return;
    let requested = false;
    let sourcePath = props.asset.relativePath;
    const observer = new IntersectionObserver(entries => {
      if (requested || !entries.some(entry => entry.isIntersecting) || !available()) return;
      requested = true;
      observer.disconnect();
      void mediaService.cache(props.projectId, props.asset.id, 'thumbnail').then(result => {
        if (alive && result.thumbnailPath) setThumbnail(convertFileSrc(result.thumbnailPath));
      }).catch(error => { if (alive) setCacheError(error instanceof Error ? error.message : 'Không tạo được thumbnail.'); });
    });
    observer.observe(element);
    // Status may arrive after the card was first intersected.
    createEffect(() => {
      if (sourcePath !== props.asset.relativePath) { sourcePath = props.asset.relativePath; requested = false; setThumbnail(''); setCacheError(''); }
      if (!requested && available()) { observer.unobserve(element); observer.observe(element); }
    });
    onCleanup(() => { alive = false; observer.disconnect(); });
  });
  onCleanup(() => { alive = false; });
  async function wave() {
    if (waveLoading() || !available()) return;
    setWaveLoading(true); setCacheError('');
    try { const result = await mediaService.cache(props.projectId, props.asset.id, 'waveform'); if (alive) setPeaks(result.waveform?.peaks); }
    catch (error) { if (alive) setCacheError(error instanceof Error ? error.message : 'Không tạo được waveform.'); }
    finally { if (alive) setWaveLoading(false); }
  }
  async function add() {
    if (mediaBusy() || !available()) return;
    const projectId = props.projectId, assetId = props.asset.id, sourceName = name(), sourceKind = props.asset.kind;
    setMediaBusy(true);
    try {
      const atUs = sourceKind === 'video' ? null : timelinePreviewActive() ? timelineTimeUs() : secondsToTimeUs(Math.max(0, playbackTime()));
      await flushPendingSaves();
      const updated = await mediaService.append(projectId, assetId, atUs);
      if (currentProject()?.id === projectId) { mergeVisualProject(updated); setNotice({ kind: 'success', message: `Đã thêm ${sourceName} vào timeline.` }); }
    } catch (error) { if (currentProject()?.id === projectId) setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thêm được clip.' }); }
    finally { setMediaBusy(false); }
  }
  async function relink() {
    const projectId = props.projectId, assetId = props.asset.id, sourceName = name();
    try {
      const path = await open({ multiple: false, title: `Tìm lại ${sourceName}`, filters: [{ name: 'Media', extensions: MEDIA_EXTENSIONS }] });
      if (typeof path !== 'string' || mediaBusy()) return;
      setMediaBusy(true);
      try {
        await flushPendingSaves();
        const updated = await mediaService.relink(projectId, assetId, path);
        if (currentProject()?.id === projectId) { mergeVisualProject(updated); await props.refresh(); if (currentProject()?.id === projectId) setNotice({ kind: 'success', message: 'Đã tìm lại đúng nguồn; dữ liệu dịch và giọng đọc được giữ nguyên.' }); }
      } finally { setMediaBusy(false); }
    } catch (error) { if (currentProject()?.id === projectId) setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không liên kết lại được nguồn.' }); }
  }
  return <article ref={element} class="library-asset" classList={{ 'library-asset-unavailable': !!props.status && !available() }}>
    <div class="library-thumbnail"><Show when={thumbnail()} fallback={<Icon size={26} aria-hidden="true" />}><img src={thumbnail()} alt={`Hình xem trước ${name()}`} loading="lazy" onError={() => setThumbnail('')} /></Show><span>{props.asset.kind === 'video' ? 'VIDEO' : props.asset.kind === 'image' ? 'ẢNH' : 'AUDIO'}</span></div>
    <div class="library-asset-body"><strong class="library-asset-name" title={name()}>{name()}</strong>
      <p>{mediaDuration(props.asset.durationUs)}<Show when={props.asset.metadata.width}> · {props.asset.metadata.width} × {props.asset.metadata.height}</Show></p>
      <p>{mediaUsage(props.asset.id, currentProject()?.timeline.clips ?? [])} clip sử dụng</p>
      <Show when={!available()}><p class="library-asset-status" role="status">{props.status?.state === 'missing' ? 'Thiếu tệp nguồn' : props.status?.state === 'invalid' ? 'Nguồn bị lỗi hoặc thay đổi' : 'Đang kiểm tra nguồn…'}</p></Show>
      <Show when={props.status?.message}><p class="library-asset-error">{props.status!.message}</p></Show>
      <Show when={cacheError()}><p class="library-asset-error">{cacheError()}</p></Show>
      <Show when={peaks()}>{values => <svg class="library-waveform" viewBox="0 0 512 48" role="img" aria-label={`Sóng âm ${name()}`}><For each={values()}>{(peak, index) => <line x1={index()} x2={index()} y1={24 - Math.max(1, peak * 23)} y2={24 + Math.max(1, peak * 23)} />}</For></svg>}</Show>
      <div class="library-asset-actions"><button class="button button-outline" type="button" disabled={mediaBusy() || !available()} onClick={() => void add()}><Plus size={14} aria-hidden="true" /> Thêm vào timeline</button>
        <Show when={props.status && !available()}><button class="button button-quiet" type="button" disabled={mediaBusy()} onClick={() => void relink()}><Link size={14} aria-hidden="true" /> Tìm lại tệp</button></Show>
        <Show when={props.asset.metadata.audioStreams?.length && available()}><button class="button button-quiet" type="button" disabled={waveLoading() || mediaBusy()} onClick={() => void wave()}><AudioLines size={14} aria-hidden="true" /> {waveLoading() ? 'Đang tạo sóng âm…' : 'Hiện sóng âm'}</button></Show>
      </div>
    </div>
  </article>;
}

export default function MediaLibrary() {
  const [kind, setKind] = createSignal('all');
  const [query, setQuery] = createSignal('');
  const [statuses, setStatuses] = createSignal<MediaAssetStatus[]>([]);
  const [checking, setChecking] = createSignal(false);
  const [error, setError] = createSignal('');
  let generation = 0;
  onCleanup(() => { generation += 1; });
  async function refresh(verify = false) {
    const id = currentProject()?.id; if (!id) return;
    const token = ++generation; setChecking(true); setError('');
    try { const result = await mediaService.inspect(id, verify); if (token === generation && currentProject()?.id === id) setStatuses(result); }
    catch (error) { if (token === generation) setError(error instanceof Error ? error.message : 'Không kiểm tra được nguồn.'); }
    finally { if (token === generation) setChecking(false); }
  }
  const inventory = createMemo(() => `${currentProject()?.id}:${currentProject()?.timeline.assets.map(a => `${a.id}:${a.relativePath}`).join('|')}`);
  createEffect(on(inventory, () => {
    // Reinspect only when the source inventory changes, not on every preference save.
    void refresh();
  }));
  const assets = createMemo(() => filterMedia(currentProject()?.timeline.assets ?? [], kind(), query()));
  const failures = () => {
    const result = mediaResults();
    return result && result.projectId === currentProject()?.id ? result.items.filter(item => item.status === 'failed') : [];
  };
  return <section class="media-library" aria-label="Kho nguồn media">
    <div class="library-toolbar"><div><h3>Kho nguồn</h3><p>Nhập video, ảnh và audio để dùng trong dự án.</p></div><button class="button button-primary" type="button" disabled={mediaBusy()} onClick={() => void browseMedia()}><Plus size={15} aria-hidden="true" /> Nhập media</button></div>
    <div class="library-filters"><label class="library-search"><Search size={14} aria-hidden="true" /><input aria-label="Tìm nguồn theo tên" type="search" placeholder="Tìm nguồn…" value={query()} onInput={event => setQuery(event.currentTarget.value)} /></label>
      <select aria-label="Lọc loại nguồn" value={kind()} onChange={event => setKind(event.currentTarget.value)}><option value="all">Tất cả</option><option value="video">Video</option><option value="image">Ảnh</option><option value="audio">Audio</option></select>
      <button class="button button-quiet" type="button" title="Kiểm tra tệp và định danh nguồn" disabled={checking() || mediaBusy()} onClick={() => void refresh(true)}><RefreshCw size={14} aria-hidden="true" /> {checking() ? 'Đang kiểm tra…' : 'Kiểm tra'}</button>
    </div>
    <Show when={mediaBusy()}><p class="library-progress" role="status">{mediaProgress() || 'Đang lưu thay đổi kho nguồn…'}</p></Show>
    <Show when={error()}><p class="library-asset-error" role="alert">{error()}</p></Show>
    <Show when={failures().length}><div class="library-failures" role="status"><strong>{failures().length} tệp chưa nhập được</strong><For each={failures()}>{item => <p><b>{mediaName(item.sourcePath)}</b> — {item.error}</p>}</For><button class="button button-outline" type="button" disabled={mediaBusy()} onClick={() => void importMediaPaths(failures().map(item => item.sourcePath))}>Thử lại các tệp lỗi</button></div></Show>
    <Show when={assets().length} fallback={<div class="library-empty"><Film size={25} aria-hidden="true" /><strong>{currentProject()?.timeline.assets.length ? 'Không có nguồn phù hợp' : 'Chưa có nguồn media'}</strong><p>Kéo thả tệp vào màn hình Dịch hoặc nhấn Nhập media.</p></div>}>
      <div class="library-grid"><For each={assets().map(source => source.id)}>{sourceId => <AssetCard asset={assets().find(source => source.id === sourceId)!} projectId={currentProject()!.id} status={statuses().find(s => s.assetId === sourceId)} refresh={() => refresh()} />}</For></div>
    </Show>
    <Show when={currentProject()?.timeline.clips.length}><p class="library-summary">{currentProject()!.timeline.clips.length} clip · Timeline {mediaDuration(timelineDurationUs(currentProject()!.timeline.clips))}</p></Show>
  </section>;
}
