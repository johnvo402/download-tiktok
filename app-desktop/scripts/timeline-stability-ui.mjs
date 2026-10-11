// npm run test:timeline-ui; optional PLAYWRIGHT_MODULE selects the desktop bundled runtime.
import { spawn, execFileSync } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { mkdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
const root = fileURLToPath(new URL('..', import.meta.url));
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(process.env.PLAYWRIGHT_MODULE).href : 'playwright');
const output = path.join(root, '.release-cache/timeline-stability-ui');
mkdirSync(output, { recursive: true });
const ffmpeg = process.env.FFMPEG_PATH || path.join(root, 'src-tauri/resources/ffmpeg/ffmpeg.exe');
for (const [id, color, hz] of [['a', 'red', 440], ['b', 'blue', 880]]) execFileSync(ffmpeg, ['-v','error','-y','-f','lavfi','-i',`color=${color}:s=320x180:r=25:d=4`,'-f','lavfi','-i',`sine=frequency=${hz}:duration=4`,'-c:v','libx264','-pix_fmt','yuv420p','-c:a','aac','-shortest',path.join(output, `${id}.mp4`)], { windowsHide: true });
execFileSync(ffmpeg, ['-v','error','-y','-i',path.join(output, 'a.mp4'),'-i',path.join(output, 'b.mp4'),'-map','1:v','-map','0:a','-map','1:a','-c','copy','-disposition:a:0','0','-disposition:a:1','default',path.join(output, 'b-default.mp4')], { windowsHide: true });
const port = Number(process.env.TIMELINE_TEST_PORT || 1422), url = `http://127.0.0.1:${port}`;
const server = spawn(process.execPath, [path.join(root, 'node_modules/vite/bin/vite.js'), '--host','127.0.0.1','--port',String(port),'--strictPort'], { cwd: root, windowsHide: true, stdio: ['ignore','pipe','pipe'] });
let serverLog = ''; server.stdout.on('data', chunk => { serverLog += chunk; }); server.stderr.on('data', chunk => { serverLog += chunk; });
let browser, page;
const report = { checks: [], measurements: [] };
try {
  for (let i = 0; i < 100; i++) {
    if (server.exitCode !== null) throw new Error(serverLog);
    try { if ((await fetch(url)).ok) break; } catch {}
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHANNEL ? { channel: process.env.PLAYWRIGHT_CHANNEL } : {}) });
  page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('HeapProfiler.enable');
  const heap = async () => { await cdp.send('HeapProfiler.collectGarbage'); return (await cdp.send('Runtime.getHeapUsage')).usedSize; };
  await page.goto(`${url}/scripts/tests/timeline-ui/index.html`);
  await page.waitForSelector('.timeline-clip');
  const closed = [];
  for (const minutes of [30, 60, 90]) {
    const started = performance.now(); await page.evaluate(m => window.open(m), minutes);
    await page.waitForFunction(() => !window.frame().buffering);
    const openMs = performance.now() - started;
    const measurement = await page.evaluate(() => window.measure());
    assert.equal(measurement.clips, minutes * 30); assert.ok(measurement.thousandScenesMs < 5000);
    const viewport = page.locator('.timeline-viewport'); await viewport.focus();
    for (let i = 0; i < 10; i++) await page.keyboard.press('+');
    await page.screenshot({ path: path.join(output, 'zoom.png') });
    const visible = await page.locator('.timeline-clip').count();
    assert.ok(visible < 100, `Zoomed viewport must virtualize the long project: ${visible} visible`);
    for (const at of [0, 6e6, minutes * 60e6 - 2e6, 0]) {
      const begin = performance.now(); await page.evaluate(t => window.seek(t), at);
      await page.waitForFunction(() => !window.frame().buffering);
      const video = page.locator('[data-prefetch="false"] video');
      assert.equal(await video.count(), 1); assert.ok(await page.locator('video').count() <= 7); assert.ok(await page.locator('audio').count() <= 9);
      assert.equal(await page.locator('[data-prefetch="false"].timeline-preview-layer').count(), 2);
      const source = await video.evaluate(v => v.currentSrc);
      const index = Math.floor(at / 6e6); assert.ok(source.endsWith(index % 2 ? '/b-default.mp4' : '/a.mp4'));
      const color = await video.evaluate(v => { const canvas = document.createElement('canvas'); canvas.width = 1; canvas.height = 1; const ctx = canvas.getContext('2d'); ctx.drawImage(v, 0, 0, 1, 1); return [...ctx.getImageData(0, 0, 1, 1).data]; });
      assert.ok(index % 2 ? color[2] > 220 && color[0] < 25 : color[0] > 220 && color[2] < 25, 'Decoded pixels belong to the latest clip');
      assert.match(await page.locator('.timeline-preview-typography').innerText(), index % 2 ? /Caption B/ : /Caption A/);
      report.measurements.push({ minutes, seekUs: at, readyMs: performance.now() - begin });
    }
    if (minutes === 90) {
      await page.evaluate(() => window.split('clip-0', 3e6)); await page.evaluate(() => window.undo()); await page.evaluate(() => window.redo());
      assert.equal(await page.evaluate(() => window.measure().clips), minutes * 30 + 3);
      report.checks.push('split with linked caption/audio, undo and redo');
    }
    const openHeap = await heap();
    await page.evaluate(() => window.close()); await page.waitForTimeout(150);
    assert.equal(await page.locator('video,audio,.timeline-editor,.timeline-preview').count(), 0);
    const closedHeap = await heap(); closed.push(closedHeap);
    report.measurements.push({ minutes, openMs, ...measurement, openHeapMiB: openHeap / 1048576, closedHeapMiB: closedHeap / 1048576 });
  }
  assert.ok(closed[2] - closed[0] < 16 * 1048576, 'Repeated project close must not retain large DOM/media heaps');
  await page.evaluate(() => { window.missing(true); window.open(30); });
  assert.equal(await page.evaluate(() => window.canUndo()), false, 'History is released when the project closes');
  await page.getByRole('alert').filter({ hasText: 'Nguồn bị thiếu' }).waitFor();
  await page.evaluate(() => window.missing(false)); await page.getByRole('button', { name: 'Thử lại preview' }).click();
  await page.waitForFunction(() => !window.frame().buffering); report.checks.push('missing source retry');
  await page.evaluate(() => window.hold()); await page.waitForTimeout(100);
  await page.evaluate(() => { window.close(); window.resumeRequests(); window.open(30); });
  await page.waitForFunction(() => window.frame().source === 'Caption A');
  await page.evaluate(() => window.release()); await page.waitForTimeout(100);
  assert.equal(await page.evaluate(() => window.frame().source), 'Caption A'); report.checks.push('late manifest from closed project cannot replace reopened source');
  await page.evaluate(() => { for (let i = 0; i < 100; i++) window.seek(i % 2 ? 7e6 : 1e6); window.seek(7e6); });
  await page.waitForFunction(() => !window.frame().buffering);
  assert.match(await page.locator('[data-prefetch="false"] video').evaluate(v => v.currentSrc), /b-default\.mp4$/);
  await page.evaluate(() => { window.seek(5.8e6); window.play(true); });
  await page.waitForFunction(() => window.frame().time > 6.5e6);
  const tones = await page.evaluate(() => window.audioProbe());
  assert.ok(tones.hz880 > .06 && tones.hz440 < .01, 'Preview uses the default second stream, matching the export PCM test');
  report.measurements.push({ defaultAudioStream: 2, ...tones });
  await page.evaluate(() => window.play(false));
  assert.match(await page.locator('[data-prefetch="false"] video').evaluate(v => v.currentSrc), /b-default\.mp4$/);
  assert.match(await page.locator('[data-prefetch="false"] .timeline-preview-typography').innerText(), /Caption B/);
  report.checks.push('100 rapid seeks and continuous playback across A/B, matching decoded pixels/captions and the default audio stream');
  await page.setViewportSize({ width: 800, height: 900 });
  assert.ok(await page.locator('.timeline-editor').evaluate(e => e.scrollWidth <= e.clientWidth + 1));
  await page.screenshot({ path: path.join(output, 'timeline.png') });
  assert.deepEqual(errors, []); report.checks.push('30/60/90 minute real media, seek, virtualization, close cleanup, narrow window and no page errors');
  writeFileSync(path.join(output, 'report.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));
} catch (error) {
  report.failure = String(error?.stack || error);
  report.serverLog = serverLog;
  await page?.screenshot({ path: path.join(output, 'failure.png') }).catch(() => {});
  writeFileSync(path.join(output, 'report.json'), JSON.stringify(report, null, 2));
  throw error;
} finally { await browser?.close(); server.kill(); }
