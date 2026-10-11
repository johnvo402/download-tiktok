import { createSignal } from 'solid-js';
import { currentProject, setCurrentProject, setNotice, setProjectSaveState } from '../../app/project-state';
import { flushOtherSaves, registerSaveTask } from '../../app/save-guard';
import { flushAllVisualSettings } from '../overlays/overlay-state';
import { mediaService } from '../../app/services/media-service';
import { projectService } from '../../app/services/project-service';
import { selectedClipId, selectedVideoId, selectVideoClip } from './video-context-state';
import { applyEdit, editClosure, editLayout, sameLayout, validateEdit, type EditCommand, type EditLayout } from './edit-commands';
import { selectedTimelineIds, setTimelineSelection, setTimelinePlaying } from './timeline-state';
import type { TimelineClip } from './types';

interface Step { before: EditLayout; after: EditLayout; beforeSelection: string[]; afterSelection: string[]; label: string }
interface History { undo: Step[]; redo: Step[] }
const histories = new Map<string, History>();
const [version, setVersion] = createSignal(0);
export const [editBusy, setEditBusy] = createSignal(false);
export const [snapEnabled, setSnapEnabled] = createSignal(true);
const history = () => { version(); const id = currentProject()?.id ?? ''; if (!histories.has(id)) { if (histories.size >= 5) histories.delete(histories.keys().next().value!); histories.set(id, { undo: [], redo: [] }); } return histories.get(id)!; };
export const canUndo = () => history().undo.length > 0 && !editBusy();
export const canRedo = () => history().redo.length > 0 && !editBusy();
export const undoLabel = () => history().undo.at(-1)?.label;
export const redoLabel = () => history().redo.at(-1)?.label;
const [clipboard, setClipboard] = createSignal<{ projectId: string; clips: TimelineClip[] }>();
export const canPaste = () => clipboard()?.projectId === currentProject()?.id && !!clipboard()?.clips.length;
export function copyTimelineClips() {
  const project = currentProject(); if (!project) return;
  const ids = editClosure(project.timeline, selectedTimelineIds()); if (!ids.size) return;
  setClipboard({ projectId: project.id, clips: structuredClone(project.timeline.clips.filter(c => ids.has(c.id))) });
  setNotice({ kind: 'info', message: `Đã sao chép ${ids.size} clip.` });
}
export async function pasteTimelineClips(atUs: number) {
  if (canPaste()) return commitEdit({ kind: 'paste', clips: clipboard()!.clips, atUs }, 'Dán clip');
  return false;
}
export function editNotice(error: unknown) { setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thực hiện được thao tác dựng.' }); }

async function transaction(build: (doc: NonNullable<ReturnType<typeof currentProject>>['timeline'], h: History) => { next: EditLayout; selection: string[]; applyHistory: (after: EditLayout) => void } | undefined): Promise<boolean> {
  if (editBusy() || !currentProject()) return false;
  const projectId = currentProject()!.id;
  setEditBusy(true); setTimelinePlaying(false);
  let writing = false;
  const pending = Promise.resolve().then(async () => {
    await flushOtherSaves('timeline-edit'); await flushAllVisualSettings();
    const project = currentProject(); if (!project || project.id !== projectId) throw new Error('Dự án đã đóng.');
    const h = history(), expected = editLayout(project.timeline), action = build(project.timeline, h);
    if (!action || sameLayout(expected, action.next)) return false;
    validateEdit(project.timeline, action.next); writing = true; setProjectSaveState('saving');
    const updated = await mediaService.edit(projectId, expected, action.next);
    if (currentProject()?.id !== projectId) return false;
    if (updated.updatedAt >= currentProject()!.updatedAt) setCurrentProject(updated);
    action.applyHistory(editLayout(updated.timeline)); setVersion(v => v + 1);
    setTimelineSelection(action.selection.filter(id => updated.timeline.clips.some(c => c.id === id)));
    setProjectSaveState('saved');
    return true;
  });
  const unregister = registerSaveTask('timeline-edit', async () => { await pending; });
  try {
    const done = await pending;
    // Keep the transcript editor on a surviving instance of the same content when possible.
    const project = currentProject();
    if (done && project?.id === projectId && !project.timeline.clips.some(c => c.id === selectedClipId())) {
      const next = project.timeline.clips.find(c => c.content.kind === 'video' && c.content.videoContentId === selectedVideoId()) ?? project.timeline.clips.find(c => c.content.kind === 'video');
      if (next) { const ids = selectedTimelineIds(); await selectVideoClip(next.id); setTimelineSelection(ids); }
    }
    return done;
  } catch (error) {
    if (writing) setProjectSaveState('error'); editNotice(error);
    // Re-sync layout after a CAS rejection; content remains owned by its source.
    if (writing) try { const latest = await projectService.get(projectId); if (currentProject()?.id === projectId && latest.updatedAt >= currentProject()!.updatedAt) { setCurrentProject(latest); setProjectSaveState('saved'); } } catch { /* The original save error stays visible. */ }
    return false;
  } finally { unregister(); setEditBusy(false); }
}
export async function commitEdit(command: EditCommand, label: string, expected?: EditLayout): Promise<boolean> {
  return transaction((doc, h) => {
    const before = editLayout(doc);
    if (expected && !sameLayout(expected, before)) throw new Error('Timeline đã thay đổi trong khi kéo; thử lại thao tác.');
    if (h.undo.length && !sameLayout(h.undo.at(-1)!.after, before)) { h.undo = []; h.redo = []; setVersion(v => v + 1); }
    const next = applyEdit(doc, command), beforeSelection = selectedTimelineIds();
    const additions = next.clips.filter(c => !doc.clips.some(old => old.id === c.id)).map(c => c.id);
    const afterSelection = additions.length ? additions : beforeSelection.filter(id => next.clips.some(c => c.id === id));
    return { next, selection: afterSelection, applyHistory(after) { h.undo.push({ before, after, beforeSelection, afterSelection, label }); if (h.undo.length > 50) h.undo.shift(); h.redo = []; } };
  });
}
export async function undoTimeline() { return transaction((doc, h) => {
  const step = h.undo.at(-1); if (!step) return;
  if (!sameLayout(editLayout(doc), step.after)) throw new Error('Timeline đã thay đổi ngoài lịch sử; không thể undo đè dữ liệu mới.');
  return { next: step.before, selection: step.beforeSelection, applyHistory() { h.undo.pop(); h.redo.push(step); } };
}); }
export async function redoTimeline() { return transaction((doc, h) => {
  const step = h.redo.at(-1); if (!step) return;
  if (!sameLayout(editLayout(doc), step.before)) throw new Error('Timeline đã thay đổi ngoài lịch sử; không thể redo đè dữ liệu mới.');
  return { next: step.after, selection: step.afterSelection, applyHistory() { h.redo.pop(); h.undo.push(step); } };
}); }
