import { expect, test } from 'vitest';
import { flushOtherSaves, flushPendingSaves, registerSaveTask } from './save-guard';

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

test('a navigation save barrier waits for a transaction which first flushes other drafts without waiting on itself', async () => {
  let resolveDraft!: () => void, resolveWrite!: () => void;
  const draft = new Promise<void>(resolve => { resolveDraft = resolve; });
  const write = new Promise<void>(resolve => { resolveWrite = resolve; });
  const removeDraft = registerSaveTask('source-test', () => draft);
  let writing = false, closed = false;
  const transaction = Promise.resolve().then(async () => { await flushOtherSaves('timeline-test'); writing = true; await write; });
  const removeTransaction = registerSaveTask('timeline-test', () => transaction);
  const closing = flushPendingSaves().then(() => { closed = true; });
  await Promise.resolve(); expect(writing).toBe(false); expect(closed).toBe(false);
  resolveDraft(); await draft; await Promise.resolve(); await Promise.resolve();
  expect(writing).toBe(true); expect(closed).toBe(false);
  resolveWrite(); await closing; expect(closed).toBe(true);
  removeTransaction(); removeDraft();
});
