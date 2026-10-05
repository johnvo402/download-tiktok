import { expect, test } from 'vitest';
import { flushPendingSaves, registerSaveTask } from './save-guard';

test('close waits for all saves and keeps rejected tasks for retry', async () => {
  let attempts = 0;
  const unregister = registerSaveTask('test', async () => { attempts++; if (attempts === 1) throw new Error('disk full'); });
  await expect(flushPendingSaves()).rejects.toThrow('disk full');
  await flushPendingSaves();
  expect(attempts).toBe(2);
  unregister();
  await flushPendingSaves();
  expect(attempts).toBe(2);
});

test('concurrent close requests share one flush', async () => {
  let complete: (() => void) | undefined;
  let calls = 0;
  const unregister = registerSaveTask('inflight', () => { calls++; return new Promise<void>(resolve => { complete = resolve; }); });
  const first = flushPendingSaves();
  const second = flushPendingSaves();
  expect(calls).toBe(1);
  complete!();
  await Promise.all([first, second]);
  unregister();
});
