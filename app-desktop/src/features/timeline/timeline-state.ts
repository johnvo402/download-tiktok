import { createSignal } from 'solid-js';
import { currentProject, setNotice, setCurrentProject } from '../../app/project-state';
import { flushPendingSaves } from '../../app/save-guard';
import { flushAllVisualSettings } from '../overlays/overlay-state';
import { mediaService } from '../../app/services/media-service';
import { selectVideoClip, selectedClipId } from './video-context-state';
import { timelineDurationUs } from './time';

const [transport, setTransport] = createSignal({ projectId: '', timeUs: 0, active: false, playing: false });
const [selection, setSelection] = createSignal<{ projectId: string; clipId: string; editingClipId?: string }>();
export const timelineTimeUs = () => transport().projectId === currentProject()?.id ? transport().timeUs : 0;
export const timelinePreviewActive = () => transport().projectId === currentProject()?.id && transport().active;
export const timelinePlaying = () => timelinePreviewActive() && transport().playing;
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
export async function selectTimelineClip(clipId: string) {
  const project = currentProject(); const clip = project?.timeline.clips.find(c => c.id === clipId);
  if (!project || !clip) return;
  if (clip.content.kind === 'video') {
    await selectVideoClip(clipId);
    // A failed save must not change the visible selection.
    if (selectedClipId() !== clipId) return;
  }
  if (currentProject()?.id === project.id) setSelection({ projectId: project.id, clipId, editingClipId: selectedClipId() });
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
