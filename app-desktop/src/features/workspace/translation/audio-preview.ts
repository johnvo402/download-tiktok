import { invoke } from '@tauri-apps/api/core';
import { createEffect, createMemo, createSignal, onCleanup, onMount, untrack } from 'solid-js';
import { currentProject, currentTranscript } from '../../../app/project-state';

export interface AudioStatus {
  sessionId: string; projectId: string; time: number; playing: boolean; buffering: boolean;
  error: string | null; queuedFrames: number;
}

/** No media/PCM crosses IPC; the native audio callback owns the playback clock. */
export function createAudioPreview(position: (status: AudioStatus) => void, error: (message: string) => void) {
  const [ready, setReady] = createSignal(false);
  const [loading, setLoading] = createSignal(false);
  const [buffering, setBuffering] = createSignal(false);
  const [mounted, setMounted] = createSignal(false);
  const [reload, setReload] = createSignal(0);
  let alive = true, generation = 0, projectId = '', sessionId = '', sourceIdentity = '', lastTime = 0, lastPlaying = false;
  let chain: Promise<unknown> = Promise.resolve();
  let reported = '', polling = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const fail = (e: unknown) => {
    const message = e instanceof Error ? e.message : String(e);
    if (alive && message !== reported) { reported = message; error(message); }
  };
  const identity = createMemo(() => JSON.stringify([
    reload(),
    currentProject()?.id, currentProject()?.source.checksumSha256, currentProject()?.source.videoPath,
    currentProject()?.dubbing.dialogueReductionEnabled,
    (currentTranscript()?.segments ?? []).map(s => [s.id, s.start, s.end, s.tts?.audioPath, s.tts?.status]),
  ]));
  createEffect(() => {
    const key = identity();
    if (!mounted()) return;
    const project = untrack(currentProject);
    const token = ++generation;
    setReady(false); setBuffering(false); setLoading(!!project?.source.videoPath);
    clearTimeout(timer);
    if (projectId) void invoke('audio_preview_control', {projectId, sessionId, action:'pause'}).catch(() => undefined);
    timer = setTimeout(() => {
      chain = chain.catch(() => undefined).then(async () => {
        if (!alive || token !== generation) return;
        if (projectId) await invoke('audio_preview_close', { projectId, sessionId });
        const previousId = projectId;
        const previousSource = sourceIdentity;
        projectId = ''; sessionId = '';
        if (!project?.source.videoPath) { setLoading(false); return; }
        const status = await invoke<AudioStatus>('audio_preview_open', { projectId: project.id, dubbing: untrack(() => currentProject()?.dubbing) });
        projectId = project.id; sessionId = status.sessionId;
        sourceIdentity = `${project.source.checksumSha256 ?? ''}:${project.source.videoPath}`;
        if (!alive || token !== generation || key !== identity()) {
          await invoke('audio_preview_close', { projectId, sessionId }); projectId = ''; sessionId = ''; return;
        }
        if (previousId === project.id && previousSource === sourceIdentity && lastTime > 0) {
          await invoke('audio_preview_control', { projectId, sessionId, action: 'seek', time: Math.min(lastTime, project.source.duration ?? lastTime) });
        } else { lastTime = 0; lastPlaying = false; }
        setReady(true); setLoading(false); reported = '';
        await invoke('audio_preview_control', { projectId, sessionId, action: 'settings', dubbing: untrack(() => currentProject()?.dubbing) });
        if (lastPlaying) await invoke('audio_preview_control', { projectId, sessionId, action: 'play' });
        position({ ...status, time: lastTime, playing: lastPlaying });
      }).catch(e => { if (alive && token === generation) { setLoading(false); setReady(false); fail(e); } });
    }, 150);
  });
  createEffect(() => {
    const dubbing = currentProject()?.dubbing;
    if (!ready() || !projectId || !dubbing) return;
    const timer = setTimeout(() => void invoke('audio_preview_control', { projectId, sessionId, action: 'settings', dubbing }).catch(fail), 60);
    onCleanup(() => clearTimeout(timer));
  });
  const control = async (action: 'play' | 'pause' | 'seek' | 'volume', time?: number, volume?: number) => {
    if (!ready() || !projectId) return false;
    try {
      await invoke('audio_preview_control', { projectId, sessionId, action, time: time ?? null, volume: volume ?? null });
      if (action === 'play') lastPlaying = true;
      if (action === 'pause') lastPlaying = false;
      if (action === 'seek' && time !== undefined) lastTime = time;
      return true;
    } catch (e) { fail(e); return false; }
  };
  onMount(() => {
    setMounted(true);
    const interval = setInterval(() => {
      if (!ready() || !projectId || polling) return;
      const id = projectId, token = generation;
      polling = true;
      void invoke<AudioStatus>('audio_preview_status', { projectId: id }).then(status => {
        if (!alive || token !== generation || id !== projectId) return;
        lastTime = status.time; lastPlaying = status.playing;
        setBuffering(status.playing && status.buffering);
        if (status.error) { fail(status.error); setReady(false); }
        position(status);
      }).catch(fail).finally(() => { polling = false; });
    }, 50);
    onCleanup(() => clearInterval(interval));
  });
  onCleanup(() => {
    alive = false; ++generation; clearTimeout(timer);
    chain = chain.catch(() => undefined).then(async () => {
      if (projectId) await invoke('audio_preview_close', { projectId, sessionId });
      projectId = ''; sessionId = '';
    }).catch(() => undefined);
  });
  return { ready, loading, buffering, control, retry: () => setReload(n => n + 1) };
}
