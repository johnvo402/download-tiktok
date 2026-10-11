import { createSignal } from 'solid-js';
import { open } from '@tauri-apps/plugin-dialog';
import { currentProject, setActiveWorkspacePanelTab, setNotice } from '../../app/project-state';
import { flushPendingSaves } from '../../app/save-guard';
import { mediaService, type MediaImportItem } from '../../app/services/media-service';
import { mergeVisualProject } from '../overlays/overlay-state';
import { MEDIA_EXTENSIONS } from './media-utils';

export const [mediaBusy, setMediaBusy] = createSignal(false);
export const [mediaProgress, setMediaProgress] = createSignal('');
export const [mediaResults, setMediaResults] = createSignal<{ projectId: string; items: MediaImportItem[] }>();

export async function importMediaPaths(paths: string[]) {
  const projectId = currentProject()?.id;
  if (!projectId || mediaBusy() || !paths.length) return;
  setMediaBusy(true); setMediaProgress('Đang chuẩn bị nhập media…');
  setActiveWorkspacePanelTab('media'); setMediaResults(undefined);
  try {
    await flushPendingSaves();
    const result = await mediaService.import(projectId, paths);
    if (currentProject()?.id !== projectId) return;
    // Read the newest snapshot, including preferences saved while the import was running.
    const { projectService } = await import('../../app/services/project-service');
    const latest = await projectService.get(projectId);
    if (currentProject()?.id !== projectId) return;
    mergeVisualProject(latest);
    setMediaResults({ projectId, items: result.items });
    const failed = result.items.filter(item => item.status === 'failed').length;
    setNotice({ kind: failed ? 'info' : 'success', message: `Đã xử lý ${result.items.length - failed}/${result.items.length} tệp. Chọn “Thêm vào timeline” để sử dụng nguồn.` });
  } catch (error) {
    if (currentProject()?.id === projectId) setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không nhập được media.' });
  } finally { setMediaBusy(false); setMediaProgress(''); }
}
export async function browseMedia() {
  if (mediaBusy()) return;
  try {
    const paths = await open({ multiple: true, directory: false, title: 'Thêm video, ảnh và audio', filters: [{ name: 'Media', extensions: MEDIA_EXTENSIONS }] });
    if (paths) await importMediaPaths(Array.isArray(paths) ? paths : [paths]);
  } catch (error) { setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không mở được hộp chọn tệp.' }); }
}
