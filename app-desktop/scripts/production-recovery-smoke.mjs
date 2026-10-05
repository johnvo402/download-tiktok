// Opt-in Windows desktop fault injection. All projects/media stay in .release-cache.
// Requires running Vite, debug exe, bundled FFmpeg and the existing QA psutil env.
import { spawn, execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import crypto from 'node:crypto';

const root = process.cwd();
const qa = path.resolve('.release-cache/task10-desktop', crypto.randomUUID());
assert.ok(qa.startsWith(path.join(root, '.release-cache') + path.sep));
const data = path.join(qa, 'dữ liệu');
fs.mkdirSync(data, { recursive: true });
const ffmpeg = path.resolve('src-tauri/resources/ffmpeg/ffmpeg.exe');
const ffprobe = path.resolve('src-tauri/resources/ffmpeg/ffprobe.exe');
const python = path.resolve('.release-cache/sttn-qa/venv/Scripts/python.exe');
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const source = path.join(qa, 'video tiếng Việt.mp4');
execFileSync(ffmpeg, ['-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=160x120:rate=12', '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000', '-t', '3', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-c:a', 'aac', source], { windowsHide: true });
const checksum = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const sourceHash = checksum(source);
const report = { measuredAt: new Date().toISOString(), isolatedRoot: qa, channel: 'beta', checks: [] };
let app, ws, serial = 0, pending = new Map();
const check = (name, details = {}) => { report.checks.push({ name, passed: true, ...details }); console.log('PASS:', name); };
async function evaluate(expression, timeout = 30000) {
  return new Promise((resolve, reject) => {
    const id = ++serial;
    const timer = setTimeout(() => { pending.delete(id); reject(Error('CDP timeout')); }, timeout);
    pending.set(id, msg => { clearTimeout(timer); if (msg.error || msg.result?.exceptionDetails) reject(Error(JSON.stringify(msg))); else resolve(msg.result.result.value); });
    ws.send(JSON.stringify({ id, method: 'Runtime.evaluate', params: { expression, awaitPromise: true, returnByValue: true } }));
  });
}
const invoke = (name, args = {}, timeout = 120000) => evaluate(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(name)},${JSON.stringify(args)})`, timeout);
async function launch() {
  const began = performance.now();
  app = spawn(path.resolve('src-tauri/target/debug/ai-video-localization.exe'), [], {
    windowsHide: true, stdio: 'ignore', env: { ...process.env, JC_QA_DATA_DIR: data, JC_RENDER_BACKEND: 'legacy',
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: '--remote-debugging-port=9225', WEBVIEW2_USER_DATA_FOLDER: path.join(qa, 'webview'), LOCALIZATION_PYTHON: python }
  });
  let page;
  for (let i = 0; i < 600; i++) {
    try { page = (await (await fetch('http://127.0.0.1:9225/json/list')).json()).find(p => p.url.startsWith('http://127.0.0.1:1420')); } catch {}
    if (page) break; await sleep(100);
  }
  assert.ok(page, 'Desktop did not start');
  ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise(resolve => ws.addEventListener('open', resolve, { once: true }));
  pending = new Map();
  ws.addEventListener('message', event => { const msg = JSON.parse(event.data); pending.get(msg.id)?.(msg); pending.delete(msg.id); });
  for (let i = 0; i < 600; i++) {
    if (await evaluate(`(async()=>{const s=await import('/src/app/project-state.ts');return s.appStartupReady()&&!s.projectsLoading();})()`).catch(() => false)) {
      report.startupDebugWarmMs ??= performance.now() - began; return;
    }
    await sleep(100);
  }
  throw Error('Project Manager not ready');
}
async function closed(timeout = 20000) {
  for (let i = 0; i < timeout / 100; i++) { if (app.exitCode !== null || app.signalCode !== null) { ws?.close(); await sleep(1000); return; } await sleep(100); }
  throw Error('App did not close');
}
async function close() {
  await evaluate(`(async()=>{const {getCurrentWindow}=await import('/node_modules/@tauri-apps/api/window.js');await getCurrentWindow().close();})()`).catch(() => {});
  await closed();
}
async function editTranslation(text) {
  for (let i = 0; i < 200; i++) {
    const found = await evaluate(`(()=>{const t=document.querySelector('textarea.segment-translation-input');if(!t)return false;t.focus();t.value=${JSON.stringify(text)};t.dispatchEvent(new Event('input',{bubbles:true}));return true;})()`);
    if (found) return; await sleep(100);
  }
  throw Error('Transcript editor missing');
}
async function openProject(id) {
  await evaluate(`location.href='/project/${id}/translate'`);
  await sleep(2000);
}
async function jobTerminal(id) {
  for (let i = 0; i < 1800; i++) { const item = (await invoke('list_jobs')).find(j => j.id === id); if (item && ['completed', 'partial', 'failed', 'cancelled', 'interrupted'].includes(item.status)) return item; await sleep(100); }
  throw Error('Job timeout');
}
const options = { quality: 'balanced', encoder: 'cpu', videoCodec: 'h264', applyBlur: false, applyLogos: false, burnSubtitles: false,
  enableSubtitleBlurOverride: false, includeOriginalAudio: true, includeDubbing: false, ducking: false, outputDirectory: null,
  outputFileName: 'Kiểm tra', originalVolume: 1, dubbedVolume: 1, duckingAmount: 0.75, dialogueReduction: false, dialogueReductionAmount: 0.65 };
try {
  await launch();
  assert.equal((await invoke('list_projects')).projects.length, 0);
  const secondApp = spawn(path.resolve('src-tauri/target/debug/ai-video-localization.exe'), [], { windowsHide: true, stdio: 'ignore', env: { ...process.env, JC_QA_DATA_DIR: data } });
  const secondExit = await new Promise((resolve, reject) => { const timer=setTimeout(()=>{secondApp.kill();reject(Error('Second instance did not exit'));},15000); secondApp.once('exit',code=>{clearTimeout(timer);resolve(code);}); });
  assert.equal(secondExit,0); check('A second desktop instance cannot open the same data root');
  report.initialProcessMemory = JSON.parse(execFileSync(python, ['-c', `import psutil,json;p=psutil.Process(${app.pid});print(json.dumps([{'name':c.name(),'rssBytes':c.memory_info().rss} for c in [p,*p.children(recursive=True)] if c.is_running()]))`],{encoding:'utf-8',windowsHide:true}));
  const project = await invoke('create_project' , { name: 'Production QA tiếng Việt', storageDirectory: null });
  const id = project.id, projectRoot = path.join(data, 'projects', id);
  const imported = await invoke('import_video', { projectId: id, sourcePath: source, replace: false });
  const transcript = { schemaVersion: 1, projectId: id, sourceType: 'asr', sourceTrack: null, language: 'vi', videoChecksum: imported.project.source.checksumSha256,
    segments: [{ id: crypto.randomUUID(), start: 0.2, end: 2.5, source: 'Xin chào.', translated: 'Chưa chỉnh sửa', speakerId: null, sourceType: 'asr', translationStatus: 'translated', tts: null }] };
  await invoke('save_transcript', { projectId: id, transcript });
  await openProject(id); await editTranslation('Bản dịch được lưu khi đóng ngay.'); await close();
  assert.equal(JSON.parse(fs.readFileSync(path.join(projectRoot, 'subtitles/transcript.json'))).segments[0].translated, 'Bản dịch được lưu khi đóng ngay.');
  assert.equal(JSON.parse(fs.readFileSync(path.join(data, 'session.json'))).cleanShutdown, true);
  check('Immediate desktop close flushes edited transcript and marks clean shutdown');
  await launch(); assert.equal((await invoke('get_transcript', { projectId: id })).segments[0].translated, 'Bản dịch được lưu khi đóng ngay.');
  await openProject(id);
  // A directory where the next backup belongs forces a real write failure.
  const backup = path.join(projectRoot, 'project.json.bak1'), retained = path.join(qa, 'project-backup-retained');
  if (fs.existsSync(backup)) fs.renameSync(backup, retained);
  fs.mkdirSync(backup);
  await editTranslation('Giữ dữ liệu chưa lưu khi lỗi ổ đĩa.');
  await evaluate(`(async()=>{const {getCurrentWindow}=await import('/node_modules/@tauri-apps/api/window.js');await getCurrentWindow().close();})()`);
  await sleep(1800); assert.equal(app.exitCode, null);
  assert.equal(await evaluate(`document.querySelector('textarea.segment-translation-input')?.value`), 'Giữ dữ liệu chưa lưu khi lỗi ổ đĩa.');
  fs.rmdirSync(backup); if (fs.existsSync(retained)) fs.renameSync(retained, backup);
  await close(); check('Failed close save keeps app open and draft; retry saves after storage repaired');
  await launch();
  const transcriptPath = path.join(projectRoot, 'subtitles/transcript.json');
  fs.writeFileSync(transcriptPath, '{broken');
  assert.ok((await invoke('get_transcript', { projectId: id })).segments.length === 1);
  assert.ok((await invoke('get_recovery_notices')).length > 0); check('Corrupt transcript recovered from valid rotating backup with notice');
  const before = await invoke('get_transcript', { projectId: id });
  const current = await invoke('get_project', { id }); fs.unlinkSync(path.join(projectRoot, current.source.videoPath));
  await invoke('import_video', { projectId: id, sourcePath: source, replace: true, restoreSource: true });
  assert.deepEqual((await invoke('get_transcript', { projectId: id })).segments, before.segments); check('Relink identical missing source preserves transcript');
  const start = performance.now();
  const exportJob = await invoke('start_export', { projectId: id, options });
  assert.equal((await jobTerminal(exportJob)).status, 'completed');
  const history = await invoke('get_export_history', { projectId: id });
  const final = path.isAbsolute(history[0].file) ? history[0].file : path.join(projectRoot, history[0].file);
  const probe = JSON.parse(execFileSync(ffprobe, ['-v', 'error', '-show_streams', '-show_format', '-of', 'json', final], { encoding: 'utf-8', windowsHide: true }));
  assert.equal(probe.streams.find(s => s.codec_type === 'video').codec_name, 'h264');
  assert.ok(probe.streams.some(s => s.codec_type === 'audio')); assert.ok(Math.abs(Number(probe.format.duration) - 3) < .15);
  assert.equal(checksum(source), sourceHash);
  check('Real CPU H264/AAC export validates duration and preserves source', { wallMs: performance.now() - start, sizeBytes: fs.statSync(final).size });
  const second = await invoke('start_export', { projectId: id, options }); assert.equal((await jobTerminal(second)).status, 'completed');
  assert.notEqual((await invoke('get_export_history', { projectId: id }))[0].file, history[0].file); check('Existing final export is not overwritten');
  const long = path.join(qa, 'video dài.mp4');
  execFileSync(ffmpeg, ['-v', 'error', '-f', 'lavfi', '-i', 'color=size=160x120:rate=1', '-t', '7200', '-c:v', 'libx264', '-preset', 'ultrafast', '-pix_fmt', 'yuv420p', long], { windowsHide: true, timeout: 120000 });
  await invoke('import_video', { projectId: id, sourcePath: long, replace: true });
  const crashedJob = await invoke('start_export', { projectId: id, options: { ...options, includeOriginalAudio: false, outputFileName: 'Không hoàn chỉnh' } });
  let children = [];
  for (let i = 0; i < 300; i++) {
    children = JSON.parse(execFileSync(python, ['-c', `import psutil,json;print(json.dumps([p.pid for p in psutil.Process(${app.pid}).children(recursive=True) if p.name().lower()=='ffmpeg.exe']))`], { encoding: 'utf-8', windowsHide: true }));
    if (children.length) break; await sleep(20);
  }
  assert.ok(children.length, 'No active FFmpeg to fault-inject'); app.kill(); await closed();
  const alive = JSON.parse(execFileSync(python, ['-c', `import psutil,json;print(json.dumps([p for p in ${JSON.stringify(children)} if psutil.pid_exists(p)]))`], { encoding: 'utf-8', windowsHide: true }));
  assert.deepEqual(alive, []); check('Killing desktop terminates its owned FFmpeg children');
  await launch(); assert.equal((await invoke('get_session_status')).previousSessionInterrupted, true);
  assert.equal((await jobTerminal(crashedJob)).status, 'interrupted'); await sleep(1000);
  assert.ok(!fs.existsSync(path.join(projectRoot, 'cache/render', crashedJob)));
  assert.ok(!fs.readdirSync(path.join(projectRoot, 'exports')).some(file => file.startsWith(`.avl-render-${crashedJob}-`)));
  assert.ok(fs.existsSync(final)); check('Restart marks interrupted export, cleans owned partials, preserves completed output');
  const retry = await invoke('retry_unified_job', { jobId: crashedJob });
  await invoke('cancel_unified_job', { jobId: retry }); assert.equal((await jobTerminal(retry)).status, 'cancelled');
  check('Retry requires explicit action and cancellation leaves no final output');
  const diagnostics = await invoke('export_diagnostics', { destination: qa });
  const files = fs.readdirSync(diagnostics, { recursive: true, withFileTypes: true }).filter(entry=>entry.isFile()).map(entry=>path.join(entry.parentPath,entry.name));
  for (const file of files) { const text=fs.readFileSync(file,'utf-8'); for(const secret of ['Production QA tiếng Việt','Giữ dữ liệu chưa lưu khi lỗi ổ đĩa.',qa]) assert.ok(!text.includes(secret),'Private fixture payload entered diagnostics'); }
  check('Local shareable diagnostics excludes project names, transcript and user paths', { file: diagnostics });
  const longProject = await invoke('get_project', { id });
  const largeTranscript = { ...transcript, videoChecksum: longProject.source.checksumSha256,
    segments: Array.from({length:1000},(_,i)=>({ ...transcript.segments[0],id:crypto.randomUUID(),start:i*2,end:i*2+1,source:`Câu thử ${i}`,translated:`Bản dịch ${i}` })) };
  await invoke('save_transcript',{projectId:id,transcript:largeTranscript});
  const opened=performance.now(); await openProject(id);
  let rows=0;
  for(let i=0;i<150;i++){rows=await evaluate(`document.querySelectorAll('textarea.segment-translation-input').length`);if(rows===1000)break;await sleep(100);}
  assert.equal(rows,1000);
  const largeMemory=JSON.parse(execFileSync(python,['-c',`import psutil,json;p=psutil.Process(${app.pid});print(json.dumps(sum(c.memory_info().rss for c in [p,*p.children(recursive=True)] if c.is_running())))`],{encoding:'utf-8',windowsHide:true}));
  check('Desktop opens 1000-segment editor on 2-hour source', {editorReadyMs:performance.now()-opened,treeRssBytes:largeMemory,definition:'Warm debug/Vite, all 1000 editors present; not sustained playback FPS or 2-hour export.'});
  await close(); report.passed = true;
} catch (error) { report.passed = false; report.error = String(error); throw error; }
finally {
  ws?.close(); if (app?.exitCode === null) app.kill();
  fs.mkdirSync('docs/benchmarks', { recursive: true });
  fs.writeFileSync('docs/benchmarks/task10-desktop-recovery.json', JSON.stringify(report, null, 2));
}
