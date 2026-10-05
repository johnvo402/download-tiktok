/** A close/navigation barrier. Rejected saves keep the app and drafts open. */
const tasks = new Map<string, () => Promise<void>>();
let flushing: Promise<void> | undefined;

export function registerSaveTask(id: string, flush: () => Promise<void>): () => void {
  tasks.set(id, flush);
  return () => { if (tasks.get(id) === flush) tasks.delete(id); };
}

export function flushPendingSaves(): Promise<void> {
  if (flushing) return flushing;
  const work = (async () => {
    for (const flush of [...tasks.values()]) await flush();
  })();
  flushing = work.finally(() => { flushing = undefined; });
  return flushing;
}
