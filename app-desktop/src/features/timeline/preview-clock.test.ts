import { describe, expect, it } from 'vitest';
import { PreviewClock, PreviewPlayers, type PreviewParticipant } from './preview-clock';
import { frameStepUs } from './time';

describe('shared timeline transport', () => {
  it('keeps absolute elapsed time over 90 minutes, without RAF rounding drift', () => {
    const clock = new PreviewClock(); let time = 0;
    for (let i = 0; i <= 90 * 60 * 60; i++) time = clock.advance(i * 1000 / 60, time, 1, true, true, 10e9);
    expect(time).toBe(90 * 60 * 1e6);
  });
  it('gates a stalled source and reanchors at seek/pause instead of catching up through it', () => {
    const c = new PreviewClock();
    expect(c.advance(0, 2e6, 1, true, true, 3e6)).toBe(2e6);
    expect(c.advance(2000, 2e6, 1, true, true, 3e6)).toBe(3e6); // Exact clip boundary.
    expect(c.advance(3000, 3e6, 1, true, false, 4e6)).toBe(3e6);
    expect(c.advance(5000, 3e6, 1, true, true, 4e6)).toBe(3e6);
    expect(c.advance(5100, 3e6, 1, true, true, 4e6)).toBe(3.1e6);
    expect(c.advance(5200, 20e6, 2, true, true, 30e6)).toBe(20e6);
    expect(c.advance(5400, 20e6, 2, true, true, 30e6)).toBe(20.2e6);
    expect(c.advance(6000, 20.2e6, 2, false, true, 30e6)).toBe(20.2e6);
    expect(c.advance(9000, 20.2e6, 2, true, true, 30e6)).toBe(20.2e6);
    expect(c.advance(9100, 20.2e6, 2, false, true, 10e6)).toBe(10e6); // Deleted tail while paused.
  });
  it('starts/stops all streams together, keeps prefetch silent and releases registration safely', () => {
    const pool = new PreviewPlayers();
    function player() {
      let ready = false, playing = false;
      const p: PreviewParticipant = { prepare: () => ready, play: p => { playing = p; }, error: () => undefined };
      return { p, ready: () => { ready = true; }, playing: () => playing };
    }
    const video = player(), voice = player(), next = player();
    const release = pool.register('video', video.p); pool.register('voice', voice.p); pool.register('next', next.p);
    video.ready(); next.ready();
    expect(pool.sync(['video', 'voice'], 0, 1, true).ready).toBe(false);
    expect(video.playing()).toBe(false); expect(next.playing()).toBe(false);
    voice.ready(); expect(pool.sync(['video', 'voice'], 0, 1, true).ready).toBe(true);
    expect(video.playing()).toBe(true); expect(voice.playing()).toBe(true);
    expect(pool.sync(['next'], 10e6, 2, true).ready).toBe(true);
    expect(video.playing()).toBe(false); expect(voice.playing()).toBe(false); expect(next.playing()).toBe(true);
    expect(pool.sync(['missing'], 0, 3, true).ready).toBe(false); expect(next.playing()).toBe(false);
    pool.register('video', next.p); release(); // An old cleanup cannot remove a newer owner.
    expect(pool.sync(['video'], 0, 4, true).ready).toBe(true);
    pool.pause(); expect(next.playing()).toBe(false);
  });
  it('steps on the rational output frame grid without decimal accumulation', () => {
    const fps = { numerator: 30000, denominator: 1001 };
    let time = 0; for (let i = 0; i < 30000; i++) time = frameStepUs(time, fps, 1);
    expect(time).toBe(1001e6);
    expect(frameStepUs(time, fps, -1)).toBe(1000966633);
    expect(frameStepUs(0, fps, -1)).toBe(0);
    expect(() => frameStepUs(0, { numerator: 0, denominator: 1 }, 1)).toThrow();
  });
});
