import { mediaService, type MediaCache } from '../../app/services/media-service';

interface Entry { projectId: string; promise: Promise<MediaCache>; references: number; started: boolean; settled: boolean; run: () => void; cancel: () => void }
// Visible clips share requests. Cancel queued decodes once all their clips leave view.
const cache = new Map<string, Entry>();
const queue: Entry[] = [];
let running = 0;
function trim() { if (cache.size > 64) for (const [key, value] of cache) { if (!value.references && value.settled) { cache.delete(key); if (cache.size <= 64) break; } } }
function drain() { while (running < 2 && queue.length) { const entry = queue.shift()!; entry.started = true; running++; entry.run(); } }
/** Running backend reads finish under the two-worker limit; their results are no longer retained. */
export function clearTimelineCache(projectId: string) {
  for (const [key, entry] of cache) if (entry.projectId === projectId) {
    cache.delete(key);
    if (!entry.started) { const index = queue.indexOf(entry); if (index >= 0) queue.splice(index, 1); entry.cancel(); }
  }
}
export function timelineCache(projectId: string, assetId: string, checksum: string | undefined | null, kind: 'thumbnail' | 'waveform'): { promise: Promise<MediaCache>; release: () => void } {
  const key = `${projectId}:${assetId}:${checksum}:${kind}`;
  let entry = cache.get(key);
  if (!entry) {
    let run!: () => void, cancel!: () => void;
    const promise = new Promise<MediaCache>((resolve, reject) => {
      run = () => { void mediaService.cache(projectId, assetId, kind).then(resolve, reject).finally(() => { entry!.settled = true; running--; drain(); trim(); }); };
      cancel = () => reject(new Error('Thumbnail đã ra ngoài vùng xem.'));
    });
    entry = { projectId, promise, references: 0, started: false, settled: false, run, cancel };
    cache.set(key, entry);
    const captured = entry;
    void promise.catch(() => { if (cache.get(key) === captured) cache.delete(key); });
    queue.push(entry);
  }
  entry.references++;
  drain();
  trim();
  const captured = entry;
  let released = false;
  return { promise: entry.promise, release() {
    if (released) return; released = true; captured.references--;
    if (!captured.references && !captured.started) {
      const index = queue.indexOf(captured); if (index >= 0) queue.splice(index, 1);
      if (cache.get(key) === captured) cache.delete(key);
      captured.cancel();
    }
    trim();
  } };
}
