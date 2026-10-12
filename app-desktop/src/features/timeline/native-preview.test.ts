import { afterEach, describe, expect, it, vi } from 'vitest';
// The native controller runs in the browser; exercise Solid's client scheduler
// while keeping IPC/events deterministic and independent of a physical GPU.
vi.mock('solid-js', async () => vi.importActual('solid-js/dist/solid.js'));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: () => true }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => vi.fn()) }));
vi.mock('./timeline-state', () => ({ timelineSeekRevision: () => 1, timelineTimeUs: () => 1e6 }));
import { createRoot, createSignal } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { setCurrentProject } from '../../app/project-state';
import type { Project } from '../projects/types';
import type { TimelineClip } from './types';
import { createTimelineNativePreview } from './native-preview';

const flush = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); };
afterEach(() => { setCurrentProject(undefined); vi.unstubAllGlobals(); vi.clearAllMocks(); });
function fixture() {
  vi.stubGlobal('document', { hidden: false, querySelector: () => null });
  vi.stubGlobal('window', { devicePixelRatio: 1 });
  vi.stubGlobal('requestAnimationFrame', vi.fn(() => 1));
  vi.stubGlobal('cancelAnimationFrame', vi.fn());
  const clip = (id: string): TimelineClip => ({ id, trackId: 'visual', startUs: 0, sourceInUs: 0, sourceOutUs: 4e6,
    rate: { numerator: 1, denominator: 1 }, content: { kind: 'video', assetId: id, videoContentId: id } } as TimelineClip);
  setCurrentProject({ id: 'project', timeline: { assets: ['a', 'b'].map(id => ({ id, relativePath: `${id}.mp4`, metadata: {} })) } } as Project);
  const [selected, select] = createSignal<TimelineClip | undefined>(clip('a'));
  let dispose!: () => void;
  const controller = createRoot(stop => {
    dispose = stop;
    return createTimelineNativePreview(() => ({ getBoundingClientRect: () => ({ left: 0, top: 0, width: 320, height: 180 }) } as HTMLElement), selected, () => false, () => {});
  });
  return { controller, select: (id: string) => select(clip(id)), dispose };
}

describe('native preview request ownership', () => {
  it('ignores an obsolete failure and drains the new session seek', async () => {
    let rejectOld!: (error: Error) => void;
    const old = new Promise<void>((_, reject) => { rejectOld = reject; });
    vi.mocked(invoke).mockImplementation(async (command, args) => {
      if (command === 'preview_open') return `session-${(args as { timelineClipId: string }).timelineClipId}`;
      if (command === 'preview_control' && (args as { sessionId: string }).sessionId === 'session-a') return old;
    });
    const f = fixture();
    try {
      await flush();
      expect(invoke).toHaveBeenCalledWith('preview_control', expect.objectContaining({ sessionId: 'session-a' }));
      f.select('b'); await flush();
      expect(f.controller.active()).toBe(true);
      rejectOld(new Error('old decoder closed')); await flush();
      expect(invoke).toHaveBeenCalledWith('preview_control', expect.objectContaining({ sessionId: 'session-b', time: 1 }));
      expect(invoke).not.toHaveBeenCalledWith('preview_close', { sessionId: 'session-b' });
      expect(f.controller.active()).toBe(true);
      expect(f.controller.warning()).toBe('');
    } finally { f.dispose(); await flush(); }
  });
  it('falls back on a current session failure and closes its decoder', async () => {
    vi.mocked(invoke).mockImplementation(async command => {
      if (command === 'preview_open') return 'session-a';
      if (command === 'preview_control') throw new Error('current decoder failed');
    });
    const f = fixture();
    try {
      await flush();
      expect(f.controller.active()).toBe(false);
      expect(f.controller.warning()).toContain('current decoder failed');
      expect(invoke).toHaveBeenCalledWith('preview_close', { sessionId: 'session-a' });
    } finally { f.dispose(); await flush(); }
  });
});
