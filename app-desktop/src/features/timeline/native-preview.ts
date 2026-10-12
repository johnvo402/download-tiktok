import { createEffect, createMemo, createSignal, on, onCleanup, onMount, untrack } from 'solid-js';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { currentProject } from '../../app/project-state';
import type { TimelineClip } from './types';
import { timelineToSourceUs } from './time';
import { timelineSeekRevision, timelineTimeUs } from './timeline-state';

interface Status { sessionId: string; state: string; frameGeneration: number; frames: number; error?: string }
/** Native acceleration for a single unobstructed video. Its decoder samples the
 * SAME timeline clock; native audio/source transport must never take ownership. */
export function createTimelineNativePreview(viewport: () => HTMLElement | undefined, clip: () => TimelineClip | undefined, advancing: () => boolean, select: (clipId: string) => void) {
  const [mounted, setMounted] = createSignal(false), [session, setSession] = createSignal<string>();
  const [ready, setReady] = createSignal(false), [warning, setWarning] = createSignal('');
  const [active, setActive] = createSignal(false);
  const failed = new Set<string>();
  let alive = true, generation = 0, frame = 0, lastBounds = '';
  const listeners: UnlistenFn[] = [];
  let seekCount = 0, seekRevision = -1, lastTime = -1, operations = Promise.resolve();
  let sending = false, pending: { id: string; time: number; revision: number } | undefined;
  const identity = createMemo(() => {
    const c = clip(), p = currentProject(), assetId = c && 'assetId' in c.content ? c.content.assetId : undefined;
    const a = p?.timeline.assets.find(a => a.id === assetId);
    return mounted() && c && p ? `${p.id}:${c.id}:${a?.relativePath}:${a?.metadata.checksumSha256}` : '';
  });
  function bounds() {
    const element = viewport(); if (!element) return;
    const r = element.getBoundingClientRect();
    const occluded = !!document.querySelector('[role="dialog"],[role="menu"],.modal-backdrop,.jobs-dock-panel,[data-kb-popper-content-wrapper]');
    return { x: r.left, y: r.top, width: Math.max(1, r.width), height: Math.max(1, r.height), dpr: window.devicePixelRatio,
      visible: ready() && !document.hidden && !occluded && r.width > 0 && r.height > 0 };
  }
  async function resize() {
    const id = session(), b = bounds(); if (!id || !b) return;
    const key = JSON.stringify([id, b]); if (key === lastBounds) return; lastBounds = key;
    await invoke('preview_resize', { sessionId: id, bounds: b });
  }
  function hide(id: string) {
    const b = bounds(); if (b) void invoke('preview_resize', { sessionId: id, bounds: { ...b, visible: false } }).catch(() => {});
  }
  function fallback(message: string) {
    failed.add(identity()); setReady(false); setActive(false); setWarning(`${message} Đang dùng preview trình duyệt.`);
    const id = session(); setSession(undefined); pending = undefined;
    if (id) { hide(id); void invoke('preview_close', { sessionId: id }).catch(() => {}); }
  }
  async function send() {
    if (sending || !pending) return;
    sending = true;
    try {
      while (pending && alive) {
        const request = pending; pending = undefined;
        if (request.id !== session()) continue;
        try {
          const seek = request.revision !== seekRevision || request.time < lastTime - .000001 || Math.abs(request.time - lastTime) > .15;
          if (seek) { seekCount++; setReady(false); await resize(); }
          if (request.id !== session()) continue;
          await invoke('preview_control', { sessionId: request.id, action: seek ? 'seek' : 'audio', time: request.time });
          if (request.id !== session()) continue;
          if (seek) seekRevision = request.revision;
          lastTime = request.time;
        } catch (e) {
          // A closed decoder can reject while its replacement already has a
          // queued seek. Only the request's current owner may trigger fallback.
          if (alive && request.id === session()) fallback(e instanceof Error ? e.message : String(e));
        }
      }
    } finally { sending = false; }
  }
  createEffect(on(identity, key => {
    const token = ++generation, old = untrack(session), c = clip(), projectId = currentProject()?.id;
    if (old) hide(old);
    setSession(undefined); setReady(false); setActive(false); setWarning(''); pending = undefined; lastBounds = '';
    operations = operations.catch(() => {}).then(async () => {
      if (old) await invoke('preview_close', { sessionId: old }).catch(() => {});
      if (!alive || token !== generation || !key || failed.has(key) || !isTauri() || import.meta.env.VITE_PREVIEW_ENGINE === 'compatibility') return;
      const b = bounds(); if (!b || !c || !projectId) return;
      setActive(true);
      try {
        const id = await invoke<string>('preview_open', { projectId, bounds: { ...b, visible: false }, clean: null, timelineClipId: c.id });
        if (!alive || token !== generation) { await invoke('preview_close', { sessionId: id }).catch(() => {}); return; }
        seekCount = 0; seekRevision = -1; lastTime = -1; setSession(id);
      } catch (e) { if (alive && token === generation) fallback(e instanceof Error ? e.message : String(e)); }
    });
  }));
  createEffect(() => {
    const id = session(), c = clip(), revision = timelineSeekRevision(); timelineTimeUs(); advancing();
    if (!id || !c) return;
    const source = timelineToSourceUs(c, timelineTimeUs()); if (source === null) return;
    if (revision !== seekRevision) setReady(false);
    if (revision !== seekRevision || source / 1e6 !== lastTime) { pending = { id, time: source / 1e6, revision }; void send(); }
  });
  onMount(() => {
    void listen<Status>('preview-status', ({ payload: s }) => {
      if (!alive || s.sessionId !== session()) return;
      if (s.error || s.state === 'error' || s.state === 'closed') { fallback(s.error || 'Native preview đã đóng.'); return; }
      setReady(seekRevision === timelineSeekRevision() && s.frames > 0 && s.frameGeneration >= seekCount && s.state !== 'suspended');
    }).then(stop => { if (alive) { listeners.push(stop); setMounted(true); } else stop(); }).catch(() => {});
    void listen<{ sessionId: string; kind: string }>('preview-pointer', ({ payload }) => {
      const c = clip(); if (alive && c && payload.sessionId === session() && payload.kind === 'pointerdown') select(c.id);
    }).then(stop => { if (alive) listeners.push(stop); else stop(); }).catch(() => {});
    function tick() {
      if (!alive) return;
      const element = viewport(); if (element) { element.dataset.previewEngine = ready() ? 'native' : 'compatibility'; element.dataset.previewSession = session() ?? ''; }
      void resize().catch(() => {}); frame = requestAnimationFrame(tick);
    }
    frame = requestAnimationFrame(tick);
  });
  onCleanup(() => {
    alive = false; generation++; cancelAnimationFrame(frame); listeners.forEach(stop => stop()); pending = undefined;
    const id = session(); if (id) { hide(id); void invoke('preview_close', { sessionId: id }).catch(() => {}); }
  });
  return { active, ready, warning };
}
