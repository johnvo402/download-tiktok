import { describe, expect, it, vi } from 'vitest';
import type { MediaCache } from '../../app/services/media-service';
const mock = vi.hoisted(() => ({ requests: [] as Array<{ assetId: string; resolve: (cache: MediaCache) => void }> }));
vi.mock('../../app/services/media-service', () => ({ mediaService: { cache: (_project: string, assetId: string) => new Promise<MediaCache>(resolve => mock.requests.push({ assetId, resolve })) } }));
import { timelineCache } from './timeline-cache';
const empty: MediaCache = { thumbnailPath: null, waveform: null };
describe('visible timeline cache requests', () => {
  it('deduplicates sources, limits decoding to two and cancels offscreen queued work', async () => {
    const a = timelineCache('cache-test', 'a', 'hash', 'thumbnail');
    const duplicate = timelineCache('cache-test', 'a', 'hash', 'thumbnail');
    const b = timelineCache('cache-test', 'b', 'hash', 'waveform');
    const removed = timelineCache('cache-test', 'removed', 'hash', 'thumbnail');
    const removedResult = removed.promise.catch(() => 'cancelled');
    const next = timelineCache('cache-test', 'next', 'hash', 'thumbnail');
    expect(mock.requests.map(r => r.assetId)).toEqual(['a', 'b']);
    expect(duplicate.promise).toBe(a.promise);
    removed.release();
    expect(await removedResult).toBe('cancelled');
    mock.requests[0].resolve(empty); await a.promise;
    await vi.waitFor(() => expect(mock.requests.map(r => r.assetId)).toEqual(['a', 'b', 'next']));
    mock.requests[1].resolve(empty); mock.requests[2].resolve(empty);
    await Promise.all([b.promise, next.promise]);
    a.release(); duplicate.release(); b.release(); next.release();
    const reused = timelineCache('cache-test', 'a', 'hash', 'thumbnail');
    expect(await reused.promise).toEqual(empty);
    expect(mock.requests).toHaveLength(3); reused.release();
  });
});
