import { afterEach, describe, expect, it, vi } from 'vitest';
import { mediaPlayer } from './media-player';

class Video extends EventTarget {
  readyState = 2; seeking = false; paused = true; playbackRate = 1; error = null; currentTime = 0;
  frames = new Map<number, VideoFrameRequestCallback>(); next = 0; removed = false; loaded = false;
  playResult: Promise<void> = Promise.resolve();
  play() { this.paused = false; return this.playResult; }
  pause() { this.paused = true; }
  removeAttribute() { this.removed = true; }
  load() { this.loaded = true; }
  requestVideoFrameCallback(cb: VideoFrameRequestCallback) { const id = ++this.next; this.frames.set(id, cb); return id; }
  cancelVideoFrameCallback(id: number) { this.frames.delete(id); }
  seeked() { this.seeking = false; this.dispatchEvent(new Event('seeked')); }
  render(time: number) { const callbacks = [...this.frames.values()]; this.frames.clear(); callbacks.forEach(cb => cb(0, { mediaTime: time } as VideoFrameCallbackMetadata)); }
}
afterEach(() => vi.unstubAllGlobals());
function fixture() {
  vi.stubGlobal('HTMLVideoElement', Video);
  const media = new Video(), fail = vi.fn(), visibility = vi.fn(), gain = vi.fn();
  const player = mediaPlayer(media as unknown as HTMLVideoElement, { position: t => t, rate: () => 2, frameUs: () => 40000, fail, visibility, gain });
  return { media, player, fail, visibility, gain };
}
describe('latest seek wins and resource lifetime', () => {
  it('rejects obsolete decoded frames and never exposes them at the new playhead', () => {
    const { media, player, visibility } = fixture();
    expect(player.prepare(1e6, 1, false)).toBe(false); media.seeked();
    const obsolete = [...media.frames.values()][0]!;
    expect(player.prepare(8e6, 2, false)).toBe(false);
    obsolete(0, { mediaTime: 1 } as VideoFrameCallbackMetadata);
    expect(visibility).not.toHaveBeenCalledWith(true);
    media.seeked(); media.render(8);
    expect(player.prepare(8e6, 2, false)).toBe(true); expect(visibility).toHaveBeenLastCalledWith(true);
    player.dispose(); expect(media.removed && media.loaded && media.paused).toBe(true);
    expect(media.frames.size).toBe(0);
  });
  it('ignores stale play rejection after a seek and mutes immediately', async () => {
    const { media, player, fail, gain } = fixture();
    player.prepare(1e6, 1, true); media.seeked(); media.render(1);
    let reject!: (e: Error) => void; media.playResult = new Promise((_resolve, r) => { reject = r; });
    player.play(true); expect(media.paused).toBe(false);
    player.prepare(8e6, 2, true); player.play(false);
    expect(media.paused).toBe(true); expect(gain).toHaveBeenLastCalledWith(false);
    reject(new Error('old request')); await Promise.resolve(); await Promise.resolve();
    expect(fail).not.toHaveBeenCalled(); player.dispose();
  });
  it('reuses the exact prefetched frame, blocks waiting/seeking and releases old callbacks', () => {
    const { media, player } = fixture();
    player.prefetch(2e6); media.seeked(); media.render(2);
    expect(player.prepare(2e6, 50, true)).toBe(true);
    media.readyState = 1; expect(player.prepare(2e6, 50, true)).toBe(false);
    media.readyState = 2; media.seeking = true; expect(player.prepare(2e6, 50, true)).toBe(false);
    player.dispose(); media.seeked(); media.render(2); expect(player.prepare(2e6, 51, true)).toBe(false);
  });
});
