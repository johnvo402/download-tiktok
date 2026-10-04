// Opt-in native desktop regression, only for an isolated Native Preview QA project.
import assert from 'node:assert/strict';
const projectId=process.env.JC_TEXT_REMOVAL_PROJECT;
assert.ok(projectId,'Set JC_TEXT_REMOVAL_PROJECT to an isolated QA project with completed STTN patches');
const pages=await (await fetch('http://127.0.0.1:9224/json/list')).json();
const page=pages.find(p=>p.url.startsWith('http://127.0.0.1:1420/'));
assert.ok(page,'Start an isolated debug desktop on port 9224');
const ws=new WebSocket(page.webSocketDebuggerUrl);
await new Promise(r=>ws.addEventListener('open',r,{once:true}));
let serial=0;const pending=new Map();
ws.addEventListener('message',e=>{const m=JSON.parse(e.data);if(pending.has(m.id)){pending.get(m.id)(m);pending.delete(m.id);}});
const evalJs=expression=>new Promise((resolve,reject)=>{
 const id=++serial,timer=setTimeout(()=>{pending.delete(id);reject(Error('CDP timeout'));},60000);
 pending.set(id,m=>{clearTimeout(timer);if(m.error||m.result?.exceptionDetails)reject(Error(JSON.stringify(m)));else resolve(m.result.result.value);});
 ws.send(JSON.stringify({id,method:'Runtime.evaluate',params:{expression,awaitPromise:true,returnByValue:true}}));
});
const invoke=(name,args={})=>evalJs(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(name)},${JSON.stringify(args)})`);
const delay=ms=>new Promise(r=>setTimeout(r,ms));
async function audioStatus(){
 for(let i=0;i<100;i++){
  try{return await invoke('audio_preview_status',{projectId});}catch{await delay(100);}
 }
 throw Error('Audio clock did not become ready');
}
async function session(){
 for(let i=0;i<100;i++){
  const data=await evalJs(`(()=>{const el=document.querySelector('[data-preview-session]');return el?{...el.dataset}:null;})()`);
  if(data?.previewSession&&data.previewReady==='true')return data.previewSession;
  await delay(200);
 }
 throw Error('Native preview did not become ready');
}
async function seek(id,time){
 let last;
 const audio=await audioStatus();
 await invoke('audio_preview_control',{projectId,sessionId:audio.sessionId,action:'pause',time:null,volume:null,dubbing:null});
 await invoke('audio_preview_control',{projectId,sessionId:audio.sessionId,action:'seek',time,volume:null,dubbing:null});
 await invoke('preview_control',{sessionId:id,action:'pause',time:null});
 await invoke('preview_control',{sessionId:id,action:'seek',time});
 for(let i=0;i<100;i++){
  const status=await invoke('preview_status',{sessionId:id});
  last=status;
  assert.notEqual(status.state,'error',JSON.stringify(status));
  if(Math.abs(status.framePts-time)<.18&&status.frames>0)return status;
  await delay(100);
 }
 throw Error(`Seek did not present a source frame: ${JSON.stringify(last)}`);
}
try{
 const listing=await invoke('list_projects');
 assert.ok(listing.projects.every(p=>p.name.startsWith('Native Preview QA')),'Refusing a non-QA data directory');
 const project=await invoke('get_project',{id:projectId});
 assert.ok(project.name.startsWith('Native Preview QA'));
 const region=project.blurRegions.find(r=>r.effect==='sttn');assert.ok(region);
 const cache=await invoke('get_text_removal_cache_status',{projectId,regionId:region.id});
 assert.equal(cache.state,'removed');
 await evalJs(`location.href='/project/${projectId}/translate';true`);
 const id=await session();
 const statuses=[];
 for(const time of [.2,.8,1.6]){
  const s=await seek(id,time);assert.ok(s.cleanPatches>0,JSON.stringify(s));statuses.push(s);
 }
 await seek(id,.2);
 const audio=await audioStatus();
 await invoke('audio_preview_control',{projectId,sessionId:audio.sessionId,action:'play',time:null,volume:null,dubbing:null});
 let playing;
 for(let i=0;i<50;i++){
  await delay(50);playing=await invoke('preview_status',{sessionId:id});
  if(playing.playing&&playing.frames>=statuses.at(-1).frames)break;
 }
 assert.ok(playing.playing,JSON.stringify({video:playing,audio:await audioStatus()}));
 await invoke('audio_preview_control',{projectId,sessionId:audio.sessionId,action:'pause',time:null,volume:null,dubbing:null});
 // Reopening the route must resolve persisted patches with no in-memory AI clip state.
 await evalJs('location.reload();true');const reopened=await session();
 const restored=await seek(reopened,.8);assert.ok(restored.cleanPatches>0,JSON.stringify(restored));
 const after=await invoke('get_text_removal_cache_status',{projectId,regionId:region.id});
 assert.equal(after.cachedBytes,cache.cachedBytes);
 console.log(JSON.stringify({passed:true,projectId,seekStatuses:statuses,restored,cache:after},null,2));
}finally{ws.close();}
