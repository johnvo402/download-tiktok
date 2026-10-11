import type { Project } from '../projects/types';
import type { TimelineClip, VideoSettings } from './types';

export function clipVideoId(clip?: TimelineClip) {
  return clip?.content.kind === 'video' ? clip.content.videoContentId : undefined;
}

export function projectForVideo(project: Project, id?: string): Project {
  if (!id) return project;
  const content = project.timeline.videoContents.find((content) => content.id === id);
  const asset = project.timeline.assets.find((asset) => asset.id === content?.assetId);
  if (!content || !asset) throw new Error('Nội dung video không còn tồn tại.');
  return { ...project, source: asset.metadata, ...content.settings };
}

export function videoSettings(project: Project): VideoSettings {
  const { transcript, translation, subtitle, dubbing, transcriptSettings, blurRegions, logos, textOverlays } = project;
  return { transcript, translation, subtitle, dubbing, transcriptSettings, blurRegions, logos, textOverlays };
}

/** Convert a local editing projection back to the canonical project document. */
export function mergeVideoView(base: Project, view: Project, id?: string): Project {
  if (!id) return view;
  const document = { ...view.timeline, videoContents: view.timeline.videoContents.map((content) =>
    content.id === id ? { ...content, settings: videoSettings(view) } : content) };
  const merged = { ...view, timeline: document };
  const bound = document.legacyBinding?.videoContentId;
  return bound ? projectForVideo(merged, bound) : { ...merged, source: base.source, ...videoSettings(base) };
}

export type VideoJobEvent = { projectId: string; videoContentId?: string | null; jobId: string; status: string; message: string; contentRevision?: number | null };
export function eventForVideo<T extends VideoJobEvent>(event: T | undefined, project: Project, id?: string): T | undefined {
  if (!event || event.projectId !== project.id) return;
  const target = event.videoContentId ?? project.timeline.legacyBinding?.videoContentId;
  return target === id ? event : undefined;
}

export function videoIdsForScope(project: Project, scope: 'current' | 'selected' | 'all', current: string | undefined, selected: ReadonlySet<string>) {
  const ids = new Set(project.timeline.clips.map(clipVideoId).filter((id): id is string => !!id));
  if (scope === 'current') return current && ids.has(current) ? [current] : [];
  return [...ids].filter((id) => scope === 'all' || selected.has(id));
}
