import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { convertFileSrc } from '@tauri-apps/api/core';
import { currentProject, currentTranscript, selectedOverlay, setActiveWorkspacePanelTab, setBlurRegionPicking, setCurrentProject, setNotice, setOcrRegionEditing, setOcrRegionPicking, setSelectedOverlay } from '../../app/project-state';
import { projectService } from '../../app/services/project-service';
import { mergeVisualProject, updateVisualSettings } from './overlay-state';
import { blurRect, logoRect, moveRect, resizeRect, type Rect } from './overlay-utils';
import type { BlurRegion, ProjectLogo } from '../projects/types';

interface OverlayCanvasProps {
  left: number;
  top: number;
  width: number;
  height: number;
  videoWidth: number;
  videoHeight: number;
  previewSubtitleSegmentId?: string;
  regionPicking?: boolean;
  blurRegionPicking?: boolean;
  ocrRegionEditing?: boolean;
  ocrRegion?: Rect | null;
  onRegionSelected?: (rect: Rect, kind: 'ocr' | 'blur') => void;
  onOcrRegionChange?: (rect: Rect) => void;
  onOcrRegionDelete?: () => void;
}

type DragState = { kind: 'blur' | 'logo' | 'ocr'; id: string; mode: 'move' | 'resize'; pointerX: number; pointerY: number; rect: Rect; aspect?: number };

function layoutPreviewSubtitle(text: string, fontFamily: string, configuredSize: number, minimumSize: number, weight: number, maxWidth: number, maxHeight: number, lineHeight: number, autoFit: boolean) {
  const canvas = document.createElement('canvas');
  const context = canvas.getContext('2d');
  if (!context) return { text, fontSize: configuredSize };
  const floor = autoFit ? minimumSize : configuredSize;
  for (let fontSize = configuredSize; fontSize >= floor; fontSize -= 1) {
    context.font = `${weight} ${fontSize}px "${fontFamily.replaceAll('"', '')}", sans-serif`;
    const lines: string[] = [];
    for (const manualLine of text.replaceAll('\r\n', '\n').replaceAll('\r', '\n').split('\n')) {
      let line = '';
      for (const word of manualLine.split(/\s+/u).filter(Boolean)) {
        const candidate = line ? `${line} ${word}` : word;
        if (!line || context.measureText(candidate).width <= maxWidth * 0.9) line = candidate;
        else { lines.push(line); line = word; }
        if (context.measureText(line).width > maxWidth * 0.9) {
          const chars = Array.from(line); line = '';
          for (const char of chars) {
            if (line && context.measureText(line + char).width > maxWidth * 0.9) { lines.push(line); line = ''; }
            line += char;
          }
        }
      }
      lines.push(line);
    }
    if (lines.length * fontSize * lineHeight <= maxHeight * 0.96 && lines.every((line) => context.measureText(line).width <= maxWidth * 0.96)) return { text: lines.join('\n'), fontSize };
    if (!autoFit) break;
  }
  return { text, fontSize: autoFit ? minimumSize : configuredSize };
}

export default function OverlayCanvas(props: OverlayCanvasProps) {
  let layer!: HTMLDivElement;
  const [assetUrls, setAssetUrls] = createSignal<Record<string, string>>({});
  const [draft, setDraft] = createSignal<{ kind: 'blur' | 'logo' | 'ocr'; id: string; rect: Rect }>();
  const [regionDraft, setRegionDraft] = createSignal<Rect>();
  let pickingPointer: 'ocr' | 'blur' | null = null;
  let drag: DragState | undefined;
  let assetRequest = 0;

  createEffect(() => {
    const project = currentProject();
    const logos = project?.logos ?? [];
    const token = ++assetRequest;
    if (!project || !logos.length) {
      setAssetUrls({});
      return;
    }
    void Promise.all(logos.map(async (logo) => {
      try {
        const path = await projectService.getLogoAssetPath(project.id, logo.id);
        return [logo.id, convertFileSrc(path)] as const;
      } catch {
        return [logo.id, ''] as const;
      }
    })).then((pairs) => {
      if (token === assetRequest) setAssetUrls(Object.fromEntries(pairs));
    });
  });

  const blurDraftRect = (region: BlurRegion): Rect => draft()?.kind === 'blur' && draft()?.id === region.id ? draft()!.rect : blurRect(region);
  const logoDraftRect = (logo: ProjectLogo): Rect => draft()?.kind === 'logo' && draft()?.id === logo.id ? draft()!.rect : logoRect(logo);
  const ocrDraftRect = (): Rect | null => draft()?.kind === 'ocr' ? draft()!.rect : props.ocrRegion ?? null;

  const beginPointer = (event: PointerEvent & { currentTarget: HTMLElement }, kind: 'blur' | 'logo' | 'ocr', id: string, mode: 'move' | 'resize') => {
    if (event.button !== 0 || !layer) return;
    event.preventDefault();
    event.stopPropagation();
    const project = currentProject();
    const object = kind === 'blur' ? project?.blurRegions.find((value) => value.id === id) : kind === 'logo' ? project?.logos.find((value) => value.id === id) : undefined;
    const rect = kind === 'ocr' ? props.ocrRegion ?? undefined : kind === 'blur' ? object && blurRect(object as BlurRegion) : object && logoRect(object as ProjectLogo);
    if (!rect) return;
    if (kind === 'ocr') setOcrRegionEditing(true);
    else {
      setSelectedOverlay({ type: kind, id });
      setActiveWorkspacePanelTab('properties');
    }
    let aspect: number | undefined;
    if (kind === 'logo') {
      const logo = object as ProjectLogo;
      const srcWidth = props.videoWidth || project?.source.width || 1920;
      const srcHeight = props.videoHeight || project?.source.height || 1080;
      const image = layer.querySelector<HTMLImageElement>(`[data-logo-id="${CSS.escape(logo.id)}"] img`);
      const imageAspect = image?.naturalWidth && image.naturalHeight ? image.naturalWidth / image.naturalHeight : 1;
      if (logo.maintainAspectRatio) aspect = (srcWidth / srcHeight) / imageAspect;
    }
    drag = { kind, id, mode, pointerX: event.clientX, pointerY: event.clientY, rect, aspect };
    setDraft({ kind, id, rect: drag.rect });
    // Capture on the object that received the pointer down. Capturing on the
    // pointer-events:none overlay layer retargets the follow-up click to that
    // layer, which the preview interprets as a blank-area click and deselects.
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const onLayerPointerMove = (event: PointerEvent) => {
    if (pickingPointer && layer) {
      const bounds = layer.getBoundingClientRect();
      if (!bounds.width || !bounds.height) return;
      const x = Math.max(0, Math.min(1, (event.clientX - bounds.left) / bounds.width));
      const y = Math.max(0, Math.min(1, (event.clientY - bounds.top) / bounds.height));
      const start = (layer as HTMLDivElement & { __regionStart?: { x: number; y: number } }).__regionStart;
      if (start) setRegionDraft({ x: Math.min(start.x, x), y: Math.min(start.y, y), width: Math.abs(x - start.x), height: Math.abs(y - start.y) });
      return;
    }
    if (!drag || !layer) return;
    const bounds = layer.getBoundingClientRect();
    if (!bounds.width || !bounds.height) return;
    const dx = (event.clientX - drag.pointerX) / bounds.width;
    const dy = (event.clientY - drag.pointerY) / bounds.height;
    const rect = drag.mode === 'move'
      ? moveRect(drag.rect, dx, dy)
      : resizeRect(drag.rect, dx, dy, drag.aspect);
    setDraft({ kind: drag.kind, id: drag.id, rect });
  };

  const onLayerPointerUp = () => {
    if (pickingPointer) {
      const kind = pickingPointer;
      pickingPointer = null;
      const rect = regionDraft();
      setRegionDraft(undefined);
      (layer as HTMLDivElement & { __regionStart?: { x: number; y: number } }).__regionStart = undefined;
      const minimum = kind === 'blur' ? 0.02 : 0.01;
      if (rect && rect.width >= minimum && rect.height >= minimum) props.onRegionSelected?.(rect, kind);
      else setNotice({ kind: 'error', message: kind === 'blur' ? 'Vùng blur quá nhỏ. Hãy kéo chọn vùng lớn hơn.' : 'Vùng OCR quá nhỏ. Hãy kéo chọn vùng subtitle lớn hơn.' });
      return;
    }
    if (!drag) return;
    const current = draft();
    const changed = current && (
      Math.abs(current.rect.x - drag.rect.x) > 0.000001
      || Math.abs(current.rect.y - drag.rect.y) > 0.000001
      || Math.abs(current.rect.width - drag.rect.width) > 0.000001
      || Math.abs(current.rect.height - drag.rect.height) > 0.000001
    );
    // A click only selects the object. Writing an unchanged rectangle here
    // replaces its keyed DOM node before the browser dispatches `click`, which
    // can retarget that click to the preview and clear the new selection.
    if (current && changed) commitRect(drag.kind, drag.id, current.rect);
    drag = undefined;
    setDraft(undefined);
  };

  const beginRegionPick = (event: PointerEvent) => {
    const kind = props.regionPicking ? 'ocr' : props.blurRegionPicking ? 'blur' : null;
    if (!kind || event.button !== 0 || !layer) return;
    event.preventDefault();
    event.stopPropagation();
    const bounds = layer.getBoundingClientRect();
    if (!bounds.width || !bounds.height) return;
    const x = Math.max(0, Math.min(1, (event.clientX - bounds.left) / bounds.width));
    const y = Math.max(0, Math.min(1, (event.clientY - bounds.top) / bounds.height));
    (layer as HTMLDivElement & { __regionStart?: { x: number; y: number } }).__regionStart = { x, y };
    pickingPointer = kind;
    setRegionDraft({ x, y, width: 0, height: 0 });
    layer.setPointerCapture(event.pointerId);
  };

  const commitRect = (kind: 'blur' | 'logo' | 'ocr', id: string, rect: Rect) => {
    const project = currentProject();
    if (!project) return;
    if (kind === 'ocr') {
      props.onOcrRegionChange?.(rect);
    } else if (kind === 'blur') {
      setCurrentProject({ ...project, blurRegions: project.blurRegions.map((item) => item.id === id ? { ...item, ...rect } : item) });
      updateVisualSettings({ blurRegions: currentProject()?.blurRegions ?? [] }, true);
    } else {
      setCurrentProject({ ...project, logos: project.logos.map((item) => item.id === id ? { ...item, ...rect } : item) });
      updateVisualSettings({ logos: currentProject()?.logos ?? [] }, true);
    }
  };

  const nudge = (kind: 'blur' | 'logo' | 'ocr', id: string, key: string, shift: boolean, resize: boolean) => {
    const project = currentProject();
    const item = kind === 'blur' ? project?.blurRegions.find((value) => value.id === id) : kind === 'logo' ? project?.logos.find((value) => value.id === id) : undefined;
    const rect = kind === 'ocr' ? props.ocrRegion ?? undefined : kind === 'blur' ? item && blurRect(item as BlurRegion) : item && logoRect(item as ProjectLogo);
    if (!project || !rect) return;
    const step = (shift ? 10 : 1) / Math.max(layer?.clientWidth ?? 1000, 1);
    const stepY = (shift ? 10 : 1) / Math.max(layer?.clientHeight ?? 600, 1);
    const dx = key === 'ArrowLeft' ? -step : key === 'ArrowRight' ? step : 0;
    const dy = key === 'ArrowUp' ? -stepY : key === 'ArrowDown' ? stepY : 0;
    let next: Rect;
    if (!resize) next = moveRect(rect, dx, dy);
    else {
      const logo = kind === 'logo' ? item as ProjectLogo : undefined;
      const aspect = logo?.maintainAspectRatio ? rect.width / rect.height : undefined;
      next = resizeRect(rect, key === 'ArrowRight' ? step : key === 'ArrowLeft' ? -step : 0, key === 'ArrowDown' ? stepY : key === 'ArrowUp' ? -stepY : 0, aspect);
    }
    commitRect(kind, id, next);
  };

  const handleGlobalKey = (event: KeyboardEvent) => {
    if ((props.regionPicking || props.blurRegionPicking) && event.key === 'Escape') {
      event.preventDefault();
      const picking = props.regionPicking ? 'ocr' : 'blur';
      pickingPointer = null;
      setRegionDraft(undefined);
      if (picking === 'ocr') setOcrRegionPicking(false);
      else setBlurRegionPicking(false);
      setNotice({ kind: 'info', message: picking === 'ocr' ? 'Đã hủy chọn vùng OCR.' : 'Đã hủy vẽ vùng blur.' });
      return;
    }
    const target = event.target;
    if (target instanceof HTMLElement && target.closest('input, textarea, select, button, [contenteditable="true"]')) return;
    const selected = selectedOverlay();
    if (!selected) return;
    if (event.key === 'Escape') {
      setSelectedOverlay(null);
      return;
    }
    if (event.key === 'Delete' || event.key === 'Backspace') {
      if (selected.type === 'subtitle') return;
      event.preventDefault();
      if (selected.type === 'blur') {
        const project = currentProject();
        if (!project) return;
        setCurrentProject({ ...project, blurRegions: project.blurRegions.filter((region) => region.id !== selected.id) });
        updateVisualSettings({ blurRegions: currentProject()?.blurRegions ?? [] }, true);
        setSelectedOverlay(null);
      } else {
        void removeLogoById(selected.id);
      }
      return;
    }
    if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) return;
    if (selected.type === 'subtitle') return;
    event.preventDefault();
    nudge(selected.type, selected.id, event.key, event.shiftKey, false);
  };

  const removeLogoById = async (id: string) => {
    const project = currentProject();
    if (!project) return;
    try {
      const result = await projectService.removeLogo(project.id, id);
      mergeVisualProject(result.project);
      setSelectedOverlay(null);
      if (result.cleanupWarning) setNotice({ kind: 'info', message: result.cleanupWarning });
    } catch (error) {
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể xóa logo.' });
    }
  };

  onMount(() => {
    window.addEventListener('keydown', handleGlobalKey);
    onCleanup(() => window.removeEventListener('keydown', handleGlobalKey));
  });

  const translated = () => {
    const id = props.previewSubtitleSegmentId;
    if (!id) return undefined;
    return currentTranscript()?.segments.find((segment) => segment.id === id && segment.translated.trim());
  };
  const subtitle = () => currentProject()?.subtitle;
  const subtitleRegion = () => currentProject()?.blurRegions.find((region) => region.role === 'subtitle' && region.enabled);

  return <div
    ref={layer!}
    class="video-content-overlay"
    classList={{ 'region-picking': !!props.regionPicking || !!props.blurRegionPicking }}
    style={{ left: `${props.left}px`, top: `${props.top}px`, width: `${props.width}px`, height: `${props.height}px`, 'pointer-events': props.regionPicking || props.blurRegionPicking || props.ocrRegionEditing ? 'auto' : 'none' }}
    onPointerDown={beginRegionPick}
    onPointerMove={onLayerPointerMove}
    onPointerUp={onLayerPointerUp}
    onPointerCancel={onLayerPointerUp}
  >
    <Show when={props.regionPicking || props.blurRegionPicking}>
      <div class="ocr-pick-instructions">{props.blurRegionPicking ? 'Kéo để vẽ vùng blur' : 'Kéo để khoanh vùng subtitle'} · Esc để hủy</div>
      <Show when={regionDraft()}>{(rect) => <div class="ocr-region-selection" style={{ left: `${rect().x * 100}%`, top: `${rect().y * 100}%`, width: `${rect().width * 100}%`, height: `${rect().height * 100}%` }} />}</Show>
    </Show>
    <Show when={props.ocrRegionEditing && props.ocrRegion}>{(savedRegion) => {
      const rect = () => ocrDraftRect() ?? savedRegion();
      return <div
        class="overlay-object overlay-ocr-region selected"
        role="button"
        tabindex="0"
        aria-label="Vùng nhận dạng OCR"
        title="Kéo để di chuyển · góc phải dưới để đổi kích thước · Delete để xóa"
        style={{ left: `${rect().x * 100}%`, top: `${rect().y * 100}%`, width: `${rect().width * 100}%`, height: `${rect().height * 100}%` }}
        onPointerDown={(event) => beginPointer(event, 'ocr', 'ocr-region', 'move')}
        onClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key === 'Delete' || event.key === 'Backspace') { event.preventDefault(); event.stopPropagation(); props.onOcrRegionDelete?.(); }
          else if (event.key.startsWith('Arrow')) { event.preventDefault(); event.stopPropagation(); nudge('ocr', 'ocr-region', event.key, event.shiftKey, false); }
        }}
      >
        <span class="overlay-label">Vùng OCR</span>
        <button class="overlay-resize-handle" type="button" aria-label="Đổi kích thước vùng OCR" onPointerDown={(event) => beginPointer(event, 'ocr', 'ocr-region', 'resize')} onClick={(event) => event.stopPropagation()} onKeyDown={(event) => { if (event.key.startsWith('Arrow')) { event.preventDefault(); event.stopPropagation(); nudge('ocr', 'ocr-region', event.key, event.shiftKey, true); } }} />
      </div>;
    }}</Show>
    <For each={currentProject()?.blurRegions ?? []}>{(region) => {
      const rect = () => blurDraftRect(region);
      const active = () => { const current = selectedOverlay(); return current?.type === 'blur' && current.id === region.id; };
      return <div
        class="overlay-object overlay-blur"
        classList={{ selected: active(), disabled: !region.enabled }}
        data-overlay-id={region.id}
        role="button"
        tabindex="0"
        aria-label={`Vùng blur ${region.id.slice(0, 8)}${region.enabled ? '' : ', đang tắt'}`}
        title="Kéo để di chuyển · góc phải dưới để đổi kích thước"
        style={{ left: `${rect().x * 100}%`, top: `${rect().y * 100}%`, width: `${rect().width * 100}%`, height: `${rect().height * 100}%`, '--blur-strength': `${region.strength * (props.height / Math.max(props.videoHeight, 1))}px`, '--blur-edge-x': `${(region.edgeBlend ?? 0) * Math.min(rect().width, rect().height) / Math.max(rect().width, 0.0001)}%`, '--blur-edge-y': `${(region.edgeBlend ?? 0) * Math.min(rect().width, rect().height) / Math.max(rect().height, 0.0001)}%` }}
        onPointerDown={(event) => beginPointer(event, 'blur', region.id, 'move')}
        onClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key === 'Enter' || event.key === ' ') { setSelectedOverlay({ type: 'blur', id: region.id }); setActiveWorkspacePanelTab('properties'); }
          if (event.key.startsWith('Arrow')) { event.preventDefault(); event.stopPropagation(); nudge('blur', region.id, event.key, event.shiftKey, event.currentTarget.classList.contains('resize-focused')); }
        }}
      >
        <Show when={active()}><span class="overlay-label">Blur region</span><button class="overlay-resize-handle" type="button" aria-label="Đổi kích thước vùng blur" onPointerDown={(event) => beginPointer(event, 'blur', region.id, 'resize')} onClick={(event) => event.stopPropagation()} onKeyDown={(event) => { if (event.key.startsWith('Arrow')) { event.preventDefault(); event.stopPropagation(); nudge('blur', region.id, event.key, event.shiftKey, true); } }} /></Show>
      </div>;
    }}</For>
    <For each={currentProject()?.logos ?? []}>{(logo) => {
      const rect = () => logoDraftRect(logo);
      const active = () => { const current = selectedOverlay(); return current?.type === 'logo' && current.id === logo.id; };
      return <div
        class="overlay-object overlay-logo"
        classList={{ selected: active(), disabled: !logo.enabled, 'maintain-ratio': logo.maintainAspectRatio }}
        data-logo-id={logo.id}
        role="button"
        tabindex="0"
        aria-label={`Logo ${logo.originalName}${logo.enabled ? '' : ', đang tắt'}`}
        title="Kéo để di chuyển · góc phải dưới để đổi kích thước"
        style={{ left: `${rect().x * 100}%`, top: `${rect().y * 100}%`, width: `${rect().width * 100}%`, height: `${rect().height * 100}%`, opacity: String(logo.opacity) }}
        onPointerDown={(event) => beginPointer(event, 'logo', logo.id, 'move')}
        onClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key === 'Enter' || event.key === ' ') { setSelectedOverlay({ type: 'logo', id: logo.id }); setActiveWorkspacePanelTab('properties'); }
          if (event.key.startsWith('Arrow')) { event.preventDefault(); event.stopPropagation(); nudge('logo', logo.id, event.key, event.shiftKey, event.currentTarget.classList.contains('resize-focused')); }
        }}
      >
        <Show when={assetUrls()[logo.id]} fallback={<span class="overlay-logo-missing" aria-label="Không thể đọc tệp logo">LOGO</span>}>
          {(src) => <img src={src()} alt={logo.originalName} draggable={false} />}
        </Show>
        <Show when={active()}><span class="overlay-label">{logo.originalName}</span><button class="overlay-resize-handle" type="button" aria-label="Đổi kích thước logo" onPointerDown={(event) => beginPointer(event, 'logo', logo.id, 'resize')} onClick={(event) => event.stopPropagation()} onKeyDown={(event) => { if (event.key.startsWith('Arrow')) { event.preventDefault(); event.stopPropagation(); nudge('logo', logo.id, event.key, event.shiftKey, true); } }} /></Show>
      </div>;
    }}</For>
    <Show when={translated()}>{(segment) => {
      const style = createMemo(() => subtitle());
      const region = createMemo(() => subtitleRegion());
      const selected = createMemo(() => selectedOverlay()?.type === 'subtitle');
      const fontScaleX = createMemo(() => props.width / Math.max(props.videoWidth, 1));
      const fit = createMemo(() => {
        const activeStyle = style();
        const activeRegion = region();
        const text = segment().translated;
        if (!activeStyle || !activeRegion || !text.trim()) return undefined;
        const regionWidth = activeRegion.width * props.width;
        const regionHeight = activeRegion.height * props.height;
        const paddingX = regionWidth * (activeStyle.paddingX / 100);
        const paddingY = regionHeight * (activeStyle.paddingY / 100);
        const previewFontScale = props.width / Math.max(props.videoWidth, 1);
        return layoutPreviewSubtitle(
          text,
          activeStyle.fontFamily,
          activeStyle.fontSize * previewFontScale,
          Math.min(activeStyle.minFontSize, activeStyle.fontSize) * previewFontScale,
          activeStyle.fontWeight,
          Math.max(1, regionWidth - paddingX * 2),
          Math.max(1, regionHeight - paddingY * 2),
          activeStyle.lineHeight,
          activeStyle.autoFit,
        );
      });
      return <Show when={style()?.enabled && region() && segment().translated.trim()}>
      <div
        class="translated-subtitle-overlay overlay-subtitle-object"
        classList={{ 'subtitle-selected': selected() }}
        role="button"
        tabindex="0"
        aria-label="Phụ đề dịch hiện tại"
        aria-live="off"
        style={{
          top: `${region()!.y * 100}%`, left: `${region()!.x * 100}%`, transform: 'none',
          width: `${region()!.width * 100}%`, height: `${region()!.height * 100}%`, 'box-sizing': 'border-box',
          padding: `${region()!.height * props.height * (style()!.paddingY / 100)}px ${region()!.width * props.width * (style()!.paddingX / 100)}px`, display: 'flex', 'flex-direction': 'column', 'justify-content': style()!.verticalAlignment === 'top' ? 'flex-start' : style()!.verticalAlignment === 'bottom' ? 'flex-end' : 'center', 'align-items': style()!.alignment === 'left' ? 'flex-start' : style()!.alignment === 'right' ? 'flex-end' : 'center',
          'font-family': `"${style()!.fontFamily.replaceAll('"', '')}", sans-serif`,
          'font-size': `${fit()?.fontSize ?? style()!.fontSize * fontScaleX()}px`, 'font-weight': style()!.fontWeight,
          color: style()!.textColor, 'text-align': style()!.alignment, 'line-height': style()!.lineHeight,
          'max-height': '100%', 'overflow': 'hidden',
          'text-shadow': style()!.outline ? `0 0 ${Math.max(1, style()!.outlineWidth * fontScaleX())}px ${style()!.outlineColor}, 1px 1px ${Math.max(1, style()!.outlineWidth * fontScaleX())}px ${style()!.outlineColor}, -1px -1px ${Math.max(1, style()!.outlineWidth * fontScaleX())}px ${style()!.outlineColor}` : 'none',
          'background-color': style()!.backgroundEnabled ? `${style()!.backgroundColor}${Math.round(style()!.backgroundOpacity * 255).toString(16).padStart(2, '0')}` : 'transparent',
        }}
        onPointerDown={(event) => { event.stopPropagation(); setSelectedOverlay({ type: 'subtitle' }); setActiveWorkspacePanelTab('properties'); }}
        onClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => { if (event.key === 'Enter' || event.key === ' ') { setSelectedOverlay({ type: 'subtitle' }); setActiveWorkspacePanelTab('properties'); } }}
      >{fit()?.text ?? segment().translated}</div>
      </Show>;
    }}</Show>
  </div>;
}
