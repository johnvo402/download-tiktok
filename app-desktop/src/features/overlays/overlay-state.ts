import { currentProject, setCurrentProject, setNotice, setProjects } from '../../app/project-state';
import { projectService } from '../../app/services/project-service';
import type { Project } from '../projects/types';

type VisualSettings = Pick<Project, 'subtitle' | 'blurRegions' | 'logos' | 'textOverlays'>;
type VisualChanges = Partial<VisualSettings>;

const revisions = new Map<string, number>();
const pending = new Map<string, { revision: number; settings: VisualSettings }>();
const timers = new Map<string, number>();
const inFlight = new Map<string, Promise<void>>();

export function updateVisualSettings(changes: VisualChanges, flush = false) {
  const project = currentProject();
  if (!project) return;
  setCurrentProject({ ...project, ...changes });
  const revision = (revisions.get(project.id) ?? 0) + 1;
  revisions.set(project.id, revision);
  const current = currentProject()!;
  pending.set(project.id, {
    revision,
    settings: { subtitle: current.subtitle, blurRegions: current.blurRegions, logos: current.logos, textOverlays: current.textOverlays ?? [] },
  });
  window.clearTimeout(timers.get(project.id));
  timers.set(project.id, window.setTimeout(() => void persistVisualSettings(project.id).catch(() => undefined), flush ? 0 : 350));
}

export async function persistVisualSettings(projectId: string) {
  const active = inFlight.get(projectId);
  if (active) {
    await active;
    if (pending.has(projectId)) await persistVisualSettings(projectId);
    return;
  }
  const task = pending.get(projectId);
  if (!task) return;
  const write = projectService.saveVisualSettings(projectId, task.settings).then((saved) => {
    if (pending.get(projectId)?.revision === task.revision) pending.delete(projectId);
    setProjects((items) => items.map((item) => item.id === projectId ? { ...item, updatedAt: saved.updatedAt } : item));
    setCurrentProject((current) => current?.id === projectId ? { ...current, updatedAt: saved.updatedAt } : current);
  }).catch((error: unknown) => {
    setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể lưu overlay và kiểu phụ đề.' });
    throw error;
  });
  inFlight.set(projectId, write);
  try {
    await write;
  } finally {
    if (inFlight.get(projectId) === write) inFlight.delete(projectId);
  }
  if (pending.has(projectId)) await persistVisualSettings(projectId);
}

/** Wait until the latest visual settings are on disk before export reads project.json. */
export async function flushVisualSettings(projectId: string) {
  window.clearTimeout(timers.get(projectId));
  await persistVisualSettings(projectId);
  if (pending.has(projectId)) throw new Error('Không thể lưu hết các thay đổi về blur và phụ đề. Hãy thử lại trước khi export.');
}

export function mergeVisualProject(updated: Project) {
  const localChanges = pending.get(updated.id);
  let merged = updated;
  if (localChanges) {
    const localLogos = new Map(localChanges.settings.logos.map((logo) => [logo.id, logo]));
    const logos = updated.logos.map((logo) => {
      const local = localLogos.get(logo.id);
      return local ? { ...logo, x: local.x, y: local.y, width: local.width, height: local.height, enabled: local.enabled, opacity: local.opacity, maintainAspectRatio: local.maintainAspectRatio } : logo;
    });
    const settings = { ...localChanges.settings, logos };
    pending.set(updated.id, { ...localChanges, settings });
    merged = { ...updated, ...settings };
  }
  setCurrentProject((current) => current?.id === updated.id ? merged : current);
  setProjects((items) => items.map((item) => item.id === updated.id ? merged : item));
}

export async function flushAllVisualSettings() {
  for (const projectId of [...pending.keys()]) await flushVisualSettings(projectId);
}
