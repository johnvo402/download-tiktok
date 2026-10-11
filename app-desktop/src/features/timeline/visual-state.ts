import { createSignal } from 'solid-js';
import { currentProject } from '../../app/project-state';
import type { TimelineClip, TimelineSettings } from './types';

export const [visualDraft, setVisualDraft] = createSignal<{ projectId: string; clip: TimelineClip }>();
export const [canvasDraft, setCanvasDraft] = createSignal<{ projectId: string; settings: TimelineSettings }>();
export const previewClip = (clip: TimelineClip) => visualDraft()?.projectId === currentProject()?.id && visualDraft()?.clip.id === clip.id ? visualDraft()!.clip : clip;
export const previewSettings = () => canvasDraft()?.projectId === currentProject()?.id ? canvasDraft()!.settings : currentProject()?.timeline.settings;
