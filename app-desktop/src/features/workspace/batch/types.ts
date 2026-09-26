export type BatchVideoStatus = 'queued' | 'running' | 'completed' | 'failed' | 'cancelled';

export interface BatchVideoItem {
  id: string;
  sourcePath: string;
  fileName: string;
  status: BatchVideoStatus;
  stage: string | null;
  /** Percent through the current video; kept as a fraction from 0 to 1. */
  progress: number;
  message: string;
  outputPath: string | null;
}

export interface BatchVideoQueue {
  schemaVersion: number;
  projectId: string;
  sampleId: string;
  sampleLoaded: boolean;
  sampleChecksum: string | null;
  outputDirectory: string | null;
  updatedAt: string;
  items: BatchVideoItem[];
}

export interface BatchVideoRunUpdate {
  itemId: string;
  status: BatchVideoStatus;
  stage: string | null;
  progress: number;
  message: string;
  outputPath?: string | null;
}

export function createEmptyBatchVideoQueue(projectId: string): BatchVideoQueue {
  return {
    schemaVersion: 1,
    projectId,
    sampleId: '',
    sampleLoaded: false,
    sampleChecksum: null,
    outputDirectory: null,
    updatedAt: new Date().toISOString(),
    items: [],
  };
}

export function orderBatchVideos(queue: BatchVideoQueue): BatchVideoItem[] {
  const sample = queue.items.find((item) => item.id === queue.sampleId);
  return sample ? [sample, ...queue.items.filter((item) => item.id !== queue.sampleId)] : queue.items;
}

export function videoOutputName(fileName: string): string {
  const base = fileName.replace(/\.[^.]+$/, '').trim();
  const safe = base.replace(/[<>:"/\\|?*\u0000-\u001f]/g, '_').replace(/[. ]+$/g, '').slice(0, 150);
  return `${safe || 'video'}_localized`;
}
