import { createSignal } from 'solid-js';
import { currentProject } from '../../app/project-state';
import type { TimelineClip, TimelineSettings, VisualEffects } from './types';

export const [visualDraft, setVisualDraft] = createSignal<{ projectId: string; clip: TimelineClip }>();
export const [canvasDraft, setCanvasDraft] = createSignal<{ projectId: string; settings: TimelineSettings }>();
export const [effectsDraft, setEffectsDraft] = createSignal<{ projectId: string; clipId: string; effects: VisualEffects }>();
export const previewClip = (clip: TimelineClip) => {
  const base = visualDraft()?.projectId === currentProject()?.id && visualDraft()?.clip.id === clip.id ? visualDraft()!.clip : clip;
  const e = effectsDraft();
  return e && e.projectId === currentProject()?.id && e.clipId === clip.id ? { ...base, effects: e.effects } : base;
};
export const previewSettings = () => canvasDraft()?.projectId === currentProject()?.id ? canvasDraft()!.settings : currentProject()?.timeline.settings;
