// Opt-in integration. Requires the isolated QA app (never run against user projects).
import assert from 'node:assert/strict';
const pages = await (await fetch(`http://127.0.0.1:${process.env.JC_PREVIEW_CDP_PORT ?? '9224'}/json/list`)).json();
const page = pages.find(p => p.url.startsWith('http://127.0.0.1:1420/'));
assert.ok(page, 'Start the isolated QA desktop first');
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise(resolve => ws.addEventListener('open', resolve, {once:true}));
let sequence=0;const pending=new Map();
ws.addEventListener('message',event=>{const m=JSON.parse(event.data);if(m.id&&pending.has(m.id)){pending.get(m.id)(m);pending.delete(m.id);}});
async function evaluate(expression) {
 const id=++sequence;
 const result=await new Promise((resolve,reject)=>{
  const timer=setTimeout(()=>{pending.delete(id);reject(new Error('CDP timeout'));},30000);
  pending.set(id,m=>{clearTimeout(timer);m.error?reject(new Error(m.error.message)):resolve(m.result);});
  ws.send(JSON.stringify({id,method:'Runtime.evaluate',params:{expression,awaitPromise:true,returnByValue:true,userGesture:true}}));
 });
 if(result.exceptionDetails)throw new Error(JSON.stringify(result.exceptionDetails));
 return result.result.value;
}
const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const project='9bab3f9b-4811-4e19-938a-0cc4ec9d300d';
const invoke=(command,args)=>evaluate(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)},${JSON.stringify(args)})`);
const status=()=>invoke('audio_preview_status',{projectId:project});
let original;
try {
 const list=await invoke('list_projects',{});
 assert.ok(list.projects.length && list.projects.every(p=>p.name==='Native Preview QA'),'Refuse real user data');
 await evaluate(`(async()=>{const router=await import('/src/app/project-state.ts'); if(router.currentProject()?.id!==${JSON.stringify(project)})location.href='/project/${project}/translation';})()`);
 // Route names are discovered from the running app if a different QA project is open.
 let current;
 for(let i=0;i<50;i++) {
  current=await evaluate(`(async()=>{const s=await import('/src/app/project-state.ts');return s.currentProject()?.id;})()`);
  if(current===project)break;
  await delay(100);
 }
 assert.equal(current,project,'Open the QA project containing the audio fixtures first');
 for(let i=0;i<30;i++){try{await status();break;}catch{await delay(200);}}
 const initial=await status();
 original=await evaluate(`(async()=>{const s=await import('/src/app/project-state.ts');return s.currentProject().dubbing;})()`);
 for(const changes of [{originalVolume:0.25},{muteOriginal:true},{muteOriginal:false,originalVolume:0.75},{enabled:false},{enabled:true}]){
  await evaluate(`(async()=>{const s=await import('/src/app/project-state.ts');const p=s.currentProject();s.setCurrentProject({...p,dubbing:{...p.dubbing,...${JSON.stringify(changes)}}});})()`);
  await delay(250);assert.equal((await status()).sessionId,initial.sessionId,'Gain/mute edits must not rebuild the device/decoder');
 }
 await invoke('audio_preview_control',{projectId:project,sessionId:initial.sessionId,action:'seek',time:0.4});
 await invoke('audio_preview_control',{projectId:project,sessionId:initial.sessionId,action:'play'});
 // Cold FFmpeg/device startup under a busy build can take over one second.
 for(let i=0;i<40;i++){if((await status()).time>0.9)break;await delay(100);}
 const played=await status();
 assert.ok(played.time>0.9,JSON.stringify(played));
 await invoke('audio_preview_control',{projectId:project,sessionId:initial.sessionId,action:'pause'});
 const paused=await status();await delay(200);assert.equal((await status()).time,paused.time);
 await invoke('audio_preview_close',{projectId:project,sessionId:'stale-id'});
 assert.equal((await status()).sessionId,initial.sessionId);
 const duration=await evaluate(`(async()=>{const s=await import('/src/app/project-state.ts');return s.currentProject().source.duration;})()`);
 await invoke('audio_preview_control',{projectId:project,sessionId:initial.sessionId,action:'seek',time:duration-0.4});
 await invoke('audio_preview_control',{projectId:project,sessionId:initial.sessionId,action:'play'});
 let ended;
 for(let i=0;i<50;i++){ended=await status();if(!ended.playing && Math.abs(ended.time-duration)<0.001)break;await delay(100);}
 assert.ok(!ended.playing && Math.abs(ended.time-duration)<0.001,`Drain final device buffer: ${JSON.stringify(ended)}`);
 await evaluate(`(async()=>{const r=await import('/src/app/services/render-service.ts');window.__qaExports={};window.__qaUnlisten=await r.renderService.onProgress(e=>{window.__qaExports[e.jobId]=e;});})()`);
 const results=[];
 for(const dubbing of [false,true]){
  const options={quality:'fast',encoder:'cpu',videoCodec:'h264',applyBlur:false,applyLogos:false,burnSubtitles:false,
   includeOriginalAudio:true,includeDubbing:dubbing,originalVolume:0.5,dubbedVolume:0.8,ducking:dubbing,duckingAmount:0.5,
   dialogueReduction:false,outputDirectory:null,outputFileName:`audio-engine-qa-${dubbing?'dub':'original'}`};
  const validation=await invoke('validate_export',{projectId:project,options});assert.ok(validation.valid);
  const job=await invoke('start_export',{projectId:project,options});let event;
  for(let i=0;i<120;i++){event=await evaluate(`window.__qaExports[${JSON.stringify(job)}]`);if(event && ['completed','failed','cancelled'].includes(event.status))break;await delay(500);}
  assert.equal(event?.status,'completed',JSON.stringify(event));results.push({dubbing,output:event.output,elapsedSeconds:event.elapsedSeconds});
 }
 console.log(JSON.stringify({passed:true,sessionStable:true,staleCloseSafe:true,pauseClockStable:true,endOfFileDrained:true,exports:results},null,2));
} finally {
 const active=await status().catch(()=>undefined);
 if(active)await invoke('audio_preview_control',{projectId:project,sessionId:active.sessionId,action:'pause'}).catch(()=>{});
 if(original) await evaluate(`(async()=>{const s=await import('/src/app/project-state.ts');const p=s.currentProject();if(p?.id===${JSON.stringify(project)})s.setCurrentProject({...p,dubbing:${JSON.stringify(original)}});})()`).catch(()=>{});
 await evaluate(`window.__qaUnlisten?.();delete window.__qaExports;delete window.__qaUnlisten;`).catch(()=>{});
 ws.close();
}
