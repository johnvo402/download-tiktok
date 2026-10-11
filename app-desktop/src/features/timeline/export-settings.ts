import type { TimelineExportOptions } from '../../app/services/render-service';
import type { TimelineDocument } from './types';
import { clipEndUs, timelineDurationUs } from './time';

export function defaultTimelineExport(doc: TimelineDocument): TimelineExportOptions {
  const even = (v: number) => Math.max(2, Math.min(8192, Math.round(v / 2) * 2));
  return { width: even(doc.settings.width), height: even(doc.settings.height),
    frameRate: { ...doc.settings.frameRate }, startUs: null, endUs: null };
}
export function selectedExportRange(doc: TimelineDocument, ids: string[]) {
  const clips = doc.clips.filter(c => ids.includes(c.id));
  if (!clips.length) return null;
  return { startUs: Math.min(...clips.map(c => c.startUs)), endUs: Math.max(...clips.map(clipEndUs)) };
}
export function exportDurationUs(doc: TimelineDocument, config?: TimelineExportOptions) {
  return config?.startUs != null && config.endUs != null ? Math.max(0, config.endUs - config.startUs) : timelineDurationUs(doc.clips);
}
