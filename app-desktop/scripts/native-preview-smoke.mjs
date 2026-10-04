// Opt-in integration against the development WebView2 endpoint. Uses isolated QA projects only.
import assert from 'node:assert/strict';
const port = process.env.JC_PREVIEW_CDP_PORT ?? '9224';
const pages = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const page = pages.find(p => p.url.startsWith('http://127.0.0.1:1420/'));
assert.ok(page, 'Start the isolated desktop app with the documented CDP flag first');
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise(resolve => ws.addEventListener('open', resolve, { once: true }));
const pending = new Map(); let nextId = 0; const errors = [];
ws.addEventListener('message', event => {
 const message = JSON.parse(event.data);
 if (message.id && pending.has(message.id)) { pending.get(message.id)(message); pending.delete(message.id); }
 else if (message.method === 'Runtime.exceptionThrown') errors.push(message.params.exceptionDetails.text);
});
function send(method, params = {}) {
 return new Promise((resolve,reject) => {
  const id = ++nextId;const timer=setTimeout(()=>{pending.delete(id);reject(new Error(`CDP timeout: ${method}`));},20000);
  pending.set(id, message=>{clearTimeout(timer);message.error?reject(new Error(message.error.message)):resolve(message.result);});
  ws.send(JSON.stringify({id,method,params}));
 });
}
async function evaluate(expression) {
 const result = await send('Runtime.evaluate', {expression, awaitPromise:true, returnByValue:true, userGesture:true});
 if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
 return result.result.value;
}
const delay = ms => new Promise(resolve => setTimeout(resolve,ms));
async function status() {
 return evaluate(`window.__TAURI_INTERNALS__.invoke('preview_status',{sessionId:document.querySelector('.preview-frame').dataset.previewSession})`);
}
try {
 await send('Runtime.enable');
 const data = await evaluate(`window.__TAURI_INTERNALS__.invoke('list_projects')`);
 assert.ok(data.projects.every(p => p.name==='Native Preview QA'), 'Refuse to run against real user projects');
 if (!await evaluate(`!!document.querySelector('.preview-frame')`)) {
  for(let tries=0;tries<50;tries++){if(await evaluate(`!!document.querySelector('.project-open')`))break;await delay(100);}
  assert.equal(await evaluate(`!!document.querySelector('.project-open')`),true,'Wait for the project manager to finish loading');
  await evaluate(`document.querySelector('.project-open').click()`);await delay(3000);
 }
 for (let tries=0;tries<30;tries++){if(await evaluate(`document.querySelector('.preview-frame')?.dataset.previewReady==='true'`))break;await delay(200);}
 const initial = await status(); assert.equal(initial.decode,'hardware');assert.equal(initial.state,'paused');
 assert.equal(await evaluate(`!!document.querySelector('audio.workspace-video') && document.querySelector('audio.workspace-video').muted && !document.querySelector('video.workspace-video')`),true,'Native playback must not decode a second browser video/audio');
 await evaluate(`document.querySelector('.play-button').click()`);await delay(1300);
 const play=await status(); const audio=await evaluate(`(async()=>{const s=await import('/src/app/project-state.ts');return (await window.__TAURI_INTERNALS__.invoke('audio_preview_status',{projectId:s.currentProject().id})).time;})()`);
 assert.ok(play.playing);assert.ok(play.framePts>0.5);assert.ok(Math.abs(play.position-audio)<0.25,`audio drift=${play.position-audio}`);
 await evaluate(`document.querySelector('.play-button').click()`);await delay(250);
 const paused=await status();await delay(350);const pausedAgain=await status();assert.equal(pausedAgain.framePts,paused.framePts);assert.equal(pausedAgain.frames,paused.frames,'Pause must not keep decoding/rendering');
 await evaluate(`(()=>{const slider=document.querySelector('.playback-seek');for(const t of [1,8,2,9,4.7]){slider.value=t;slider.dispatchEvent(new Event('input',{bubbles:true}));}})()`);
 await delay(700);const seek=await status();assert.ok(Math.abs(seek.framePts-4.7)<0.04,`latest seek pts=${seek.framePts}`);
 await evaluate(`(async()=>{const s=await import('/src/app/project-state.ts');const p=s.currentProject();const tr=s.currentTranscript();s.setCurrentProject({...p,blurRegions:p.blurRegions.map(r=>({...r,strength:72}))});s.setCurrentTranscript({...tr,segments:tr.segments.map(x=>({...x,translated:x.translated+' · thử'}))});})()`);
 await delay(700);const edited=await status();assert.equal(edited.sessionId,initial.sessionId,'Scene edits must not recreate the decoder/device');assert.ok(edited.frames>seek.frames);
 const viewportBefore=await evaluate(`document.querySelector('.preview-frame').getBoundingClientRect().width`);
 await evaluate(`(()=>{const frame=document.querySelector('.preview-frame');frame.style.width='450px';frame.style.minWidth='0';frame.style.maxWidth='450px';})()`);
 await delay(400);
 const viewportAfter=await evaluate(`document.querySelector('.preview-frame').getBoundingClientRect().width`);
 assert.ok(viewportAfter<=450 && Math.abs(viewportBefore-viewportAfter)>1,'Resize must change the actual viewport');
 assert.equal((await status()).sessionId,initial.sessionId);
 await evaluate(`(()=>{const frame=document.querySelector('.preview-frame');frame.style.removeProperty('width');frame.style.removeProperty('min-width');frame.style.removeProperty('max-width');})()`);await delay(300);
 await evaluate(`(()=>{const modal=document.createElement('div');modal.role='dialog';modal.id='qa-modal';document.body.append(modal);})()`);await delay(250);await evaluate(`document.getElementById('qa-modal').remove()`);await delay(350);assert.equal((await status()).sessionId,initial.sessionId);
 assert.deepEqual(errors,[]);
 console.log(JSON.stringify({passed:true,adapter:initial.renderer,decode:initial.decode,renderMs:edited.renderMs,seekMs:seek.seekMs,audioDriftSeconds:Math.abs(play.position-audio),pauseFrameCount:paused.frames,sessionStable:true},null,2));
} finally {
 await evaluate(`(()=>{document.getElementById('qa-modal')?.remove();const frame=document.querySelector('.preview-frame');if(frame){frame.style.removeProperty('width');frame.style.removeProperty('min-width');frame.style.removeProperty('max-width');}})()`).catch(()=>{});
 ws.close();
}
