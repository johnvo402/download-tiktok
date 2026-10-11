import { createSignal } from 'solid-js';
import { currentProject, setCurrentProject, setCurrentTranscript, setNotice, setOcrRegionPicking, setOcrRegionEditing, setSelectedOverlay, setSeekRequest } from '../../app/project-state';
import { flushPendingSaves } from '../../app/save-guard';
import { flushAllVisualSettings } from '../overlays/overlay-state';
import type { Project } from '../projects/types';
import type { TranscriptionEvent, TranslationJobEvent, TtsJobEvent } from '../transcript/types';
import { clipVideoId, eventForVideo, mergeVideoView, projectForVideo, type VideoJobEvent } from './video-context';

const [selection, setSelection] = createSignal<{ projectId: string; clipId: string }>();
export const [selectingVideo, setSelectingVideo] = createSignal(false);
export const selectedClipId = () => selection()?.projectId === currentProject()?.id ? selection()?.clipId : undefined;
export const selectedVideoId = () => {
  const project = currentProject();
  return clipVideoId(project?.timeline.clips.find((clip) => clip.id === selectedClipId())) ?? project?.timeline.legacyBinding?.videoContentId;
};
export const editingProject = () => {
  const project = currentProject();
  return project ? projectForVideo(project, selectedVideoId()) : undefined;
};
export function setEditingProject(value: Project | undefined | ((project: Project | undefined) => Project | undefined)) {
  const base = currentProject();
  const next = typeof value === 'function' ? value(editingProject()) : value;
  if (!base || !next || base.id !== next.id) return;
  setCurrentProject(mergeVideoView(base, next, selectedVideoId()));
}

export async function selectVideoClip(clipId: string) {
  const project = currentProject();
  if (!project || selectingVideo() || !clipVideoId(project.timeline.clips.find((clip) => clip.id === clipId))) return;
  setSelectingVideo(true);
  try {
    await flushPendingSaves();
    await flushAllVisualSettings();
    if (currentProject()?.id !== project.id) return;
    const nextId = clipVideoId(project.timeline.clips.find((clip) => clip.id === clipId));
    if (nextId === selectedVideoId()) { setSelection({ projectId: project.id, clipId }); return; }
    setCurrentTranscript(undefined);
    setOcrRegionPicking(false); setOcrRegionEditing(false); setSelectedOverlay(null); setSeekRequest(null);
    setSelection({ projectId: project.id, clipId });
  } catch (error) {
    setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không lưu được nội dung trước khi đổi video.' });
  } finally { setSelectingVideo(false); }
}

type EventMap = Record<string, VideoJobEvent>;
export const [videoJobEvents, setVideoJobEvents] = createSignal<EventMap>({});
export function rememberVideoJob(kind: 'asr' | 'translation' | 'tts', event: VideoJobEvent) {
  setVideoJobEvents((events) => ({ ...events, [`${kind}:${event.projectId}:${event.videoContentId ?? 'legacy'}`]: event }));
}
function selectedEvent<T extends VideoJobEvent>(kind: string) {
  const project = currentProject();
  if (!project) return;
  return eventForVideo(videoJobEvents()[`${kind}:${project.id}:${selectedVideoId() ?? 'legacy'}`] as T | undefined, project, selectedVideoId())
    ?? eventForVideo(videoJobEvents()[`${kind}:${project.id}:legacy`] as T | undefined, project, selectedVideoId());
}
export const selectedAsrEvent = () => selectedEvent<TranscriptionEvent>('asr');
export const selectedTranslationEvent = () => selectedEvent<TranslationJobEvent>('translation');
export const selectedTtsEvent = () => selectedEvent<TtsJobEvent>('tts');
export const isRunningVideoJob = (event: VideoJobEvent) => ['queued', 'running', 'progress'].includes(event.status);
