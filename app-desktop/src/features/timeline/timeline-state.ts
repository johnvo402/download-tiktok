import { createSignal } from 'solid-js';
import { currentProject, playbackTime, setNotice, setCurrentProject } from '../../app/project-state';
import { flushPendingSaves } from '../../app/save-guard';
import { flushAllVisualSettings } from '../overlays/overlay-state';
import { mediaService } from '../../app/services/media-service';
import { selectVideoClip, selectedClipId, selectedVideoId } from './video-context-state';
import { clipEndUs, sourceToTimelineUs, timelineDurationUs } from './time';

const [transport, setTransport] = createSignal({ projectId: '', timeUs: 0, active: false, playing: false });
const [selection, setSelection] = createSignal<{ projectId: string; clipId: string; editingClipId?: string }>();
const [multiSelection, setMultiSelection] = createSignal<{ projectId: string; ids: string[] }>();
export const selectedTimelineIds = () => {
  const project = currentProject(); if (!project) return [];
  const ids = multiSelection()?.projectId === project.id && selection()?.editingClipId === selectedClipId() ? multiSelection()!.ids : [selectedClipId() ?? project.timeline.legacyBinding?.clipId].filter(Boolean) as string[];
  return ids.filter(id => project.timeline.clips.some(c => c.id === id));
};
export function setTimelineSelection(ids: string[]) {
  const project = currentProject(); if (!project) return;
  setMultiSelection({ projectId: project.id, ids });
  setSelection({ projectId: project.id, clipId: ids[0] ?? '', editingClipId: selectedClipId() });
}
export const timelineTimeUs = () => transport().projectId === currentProject()?.id ? transport().timeUs : 0;
export const timelinePreviewActive = () => transport().projectId === currentProject()?.id && transport().active;
export const timelinePlaying = () => timelinePreviewActive() && transport().playing;
export function timelineCursorUs(): number {
  if (timelinePreviewActive()) return timelineTimeUs();
  const doc = currentProject()?.timeline;
  const clip = doc?.clips.find(c => c.id === selectedClipId()) ?? doc?.clips.find(c => c.content.kind === 'video' && c.content.videoContentId === selectedVideoId());
  const sourceTime = Math.round(Math.max(0, playbackTime()) * 1e6);
  return clip ? sourceTime >= clip.sourceOutUs ? clipEndUs(clip) : sourceToTimelineUs(clip, Math.max(clip.sourceInUs, sourceTime)) ?? clip.startUs : timelineTimeUs();
}
export const timelineSelectedClipId = () => selection()?.projectId === currentProject()?.id && selection()?.editingClipId === selectedClipId() ? selection()?.clipId : selectedClipId();
export function seekTimeline(timeUs: number) {
  if (!Number.isFinite(timeUs)) return;
  const project = currentProject(); if (!project) return;
  setTransport({ projectId: project.id, timeUs: Math.round(Math.max(0, Math.min(timelineDurationUs(project.timeline.clips), timeUs))), active: true, playing: timelinePlaying() });
}
export function setTimelinePlaying(playing: boolean) {
  if (playing && !timelinePreviewActive()) seekTimeline(timelineTimeUs());
  setTransport(value => ({ ...value, playing }));
}
export function reviewSelectedSource() { setTransport(value => ({ ...value, active: false, playing: false })); }
export async function selectTimelineClip(clipId: string, additive = false) {
  const project = currentProject(); const clip = project?.timeline.clips.find(c => c.id === clipId);
  if (!project || !clip) return;
  const previous = selectedTimelineIds();
  if (clip.content.kind === 'video') {
    await selectVideoClip(clipId);
    // A failed save must not change the visible selection.
    if (selectedClipId() !== clipId) return;
  }
  if (currentProject()?.id === project.id) setSelection({ projectId: project.id, clipId, editingClipId: selectedClipId() });
  if (currentProject()?.id === project.id) setTimelineSelection(additive ? previous.includes(clipId) ? previous.filter(id => id !== clipId) : [...previous, clipId] : [clipId]);
}
export const [trackSaving, setTrackSaving] = createSignal(false);
export async function toggleTrack(trackId: string, control: 'locked' | 'enabled' | 'muted' | 'solo') {
  if (trackSaving()) return;
  const project = currentProject(); const track = project?.timeline.tracks.find(t => t.id === trackId);
  if (!project || !track) return;
  setTrackSaving(true);
  try {
    await flushPendingSaves(); await flushAllVisualSettings();
    const updated = await mediaService.trackControl(project.id, trackId, control, !track[control]);
    if (currentProject()?.id === project.id && updated.updatedAt >= currentProject()!.updatedAt) setCurrentProject(updated);
  } catch (error) { setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không lưu được track.' }); }
  finally { setTrackSaving(false); }
}
