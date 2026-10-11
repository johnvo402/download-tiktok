import { invoke } from '@tauri-apps/api/core';
import { createEffect, createSignal, onCleanup, untrack } from 'solid-js';
import { currentProject, currentTranscript } from '../../app/project-state';
import { videoJobEvents } from './video-context-state';
import { sourceWithDraft, type SourceMedia } from './audio-layout';

interface Entry { key: string; loading: boolean; data?: SourceMedia; error?: string; used: number }
const [entries, setEntries] = createSignal<Record<string, Entry>>({});
const [reload, setReload] = createSignal(0);
const consumers = new Map<symbol, Set<string>>();
let used = 0;
export const refreshSourceMedia = () => setReload(v => v + 1);
export const sourceMedia = (id?: string) => {
  const project = currentProject(); if (!id || !project) return;
  const entry = entries()[`${project.id}:${id}`], draft = currentTranscript();
  return entry?.data ? { ...entry, data: sourceWithDraft(entry.data, draft?.projectId === project.id ? draft : undefined) } : entry;
};
/** One in-flight manifest per source identity, shared by inspector/captions/audio. */
export function watchSourceMedia(ids: () => string[]) {
  let alive = true;
  const consumer = Symbol();
  onCleanup(() => { alive = false; consumers.delete(consumer); });
  createEffect(() => {
    const project = currentProject(); if (!project || !alive) return;
    reload();
    const wanted = new Set(ids());
    consumers.set(consumer, new Set([...wanted].map(id => `${project.id}:${id}`)));
    const pinned = new Set([...consumers.values()].flatMap(set => [...set]));
    for (const id of wanted) {
      const content = project.timeline.videoContents.find(v => v.id === id); if (!content) continue;
      const transcript = currentTranscript()?.videoContentId === id ? currentTranscript() : undefined;
      const jobs = Object.values(videoJobEvents()).filter(e => e.projectId === project.id && e.videoContentId === id && ['completed','partial','failed','cancelled'].includes(e.status)).map(e => [e.jobId,e.status]);
      const key = JSON.stringify([reload(), content.revision, content.settings.transcript, transcript?.segments.map(s => [s.id,s.start,s.end,s.translated,s.tts]), jobs]);
      const target = `${project.id}:${id}`;
      if (untrack(entries)[target]?.key === key) continue;
      setEntries(previous => {
        const next = { ...previous, [target]: { key, loading: true, used: ++used } }, keys = Object.keys(next);
        const old = keys.filter(k => !pinned.has(k)).sort((a, b) => next[a]!.used - next[b]!.used);
        let count = keys.length; while (count > 50 && old.length) { delete next[old.shift()!]; count--; }
        return next;
      });
      void invoke<SourceMedia>('get_timeline_source_media', { projectId: project.id, videoContentId: id }).then(data => {
        if (entries()[target]?.key === key) setEntries(p => ({ ...p, [target]: { key, loading: false, data, used: ++used } }));
      }).catch(e => { if (entries()[target]?.key === key) setEntries(p => ({ ...p, [target]: { key, loading: false, error: e instanceof Error ? e.message : String(e), used: ++used } })); });
    }
  });
}
