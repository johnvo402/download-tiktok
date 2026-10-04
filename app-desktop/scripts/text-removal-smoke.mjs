// Opt-in actual desktop/worker/FFmpeg integration. Requires isolated QA data and installed STTN.
import assert from 'node:assert/strict';
import path from 'node:path';
const source=process.env.JC_TEXT_REMOVAL_SOURCE;
assert.ok(source,'Set JC_TEXT_REMOVAL_SOURCE to a short real video containing hard subtitles');
const pages=await (await fetch('http://127.0.0.1:9224/json/list')).json();
const page=pages.find(p=>p.url.startsWith('http://127.0.0.1:1420/'));
assert.ok(page,'Start the isolated debug desktop with CDP port 9224');
const ws=new WebSocket(page.webSocketDebuggerUrl);await new Promise(r=>ws.addEventListener('open',r,{once:true}));
let serial=0;const pending=new Map();
ws.addEventListener('message',event=>{const m=JSON.parse(event.data);if(pending.has(m.id)){pending.get(m.id)(m);pending.delete(m.id);}});
const evalJs=expression=>new Promise((resolve,reject)=>{
 const id=++serial,timer=setTimeout(()=>{pending.delete(id);reject(Error('CDP timeout'));},60000);
 pending.set(id,m=>{clearTimeout(timer);const r=m.result;if(m.error||r?.exceptionDetails)reject(Error(JSON.stringify(m)));else resolve(r.result.value);});
 ws.send(JSON.stringify({id,method:'Runtime.evaluate',params:{expression,awaitPromise:true,returnByValue:true,userGesture:true}}));
});
const invoke=(name,args={})=>evalJs(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(name)},${JSON.stringify(args)})`);
const delay=ms=>new Promise(r=>setTimeout(r,ms));
async function terminal(id){
 let last='';
 for(let i=0;i<600;i++){
  const jobs=await invoke('list_jobs');const job=jobs.find(j=>j.id===id);
  if(job && ['completed','failed','cancelled'].includes(job.status))return job;
  if(job && job.message!==last){last=job.message;console.log(job.message);}
  await delay(500);
 }
 throw Error('Job timed out');
}
try {
 const listing=await invoke('list_projects');assert.ok(listing.projects.every(p=>p.name.startsWith('Native Preview QA')),'Refusing non-QA projects');
 let project=await invoke('create_project',{name:'Native Preview QA STTN v2',storageDirectory:null});
 project=(await invoke('import_video',{projectId:project.id,sourcePath:path.resolve(source),replace:false})).project;
 const regionId=crypto.randomUUID();
 const region={id:regionId,x:.20,y:.68,width:.55,height:.23,strength:100,enabled:true,role:'generic',effect:'sttn',
   startMs:0,endMs:null,edgeBlend:0,pixelNoise:0,textRemovalScanFps:2,textRemovalMode:'fullRegion'};
 project=await invoke('save_visual_settings',{projectId:project.id,settings:{subtitle:{...project.subtitle,enabled:false},blurRegions:[region],logos:[],textOverlays:[]}});
 const env=await invoke('get_text_removal_environment');assert.ok(env.ocrReady&&env.sttnModelInstalled&&env.sttnRuntimeInstalled,JSON.stringify(env));
 const detected=await terminal(await invoke('start_text_removal_preview',{projectId:project.id,regionId,currentTime:.5,detectOnly:true}));
 assert.equal(detected.status,'completed',JSON.stringify(detected));
 const detectedStatus=await invoke('get_text_removal_cache_status',{projectId:project.id,regionId});
 assert.equal(detectedStatus.state,'detected');assert.ok(detectedStatus.intervals>0);
 const cancelId=await invoke('start_text_removal_preview',{projectId:project.id,regionId,currentTime:.5,detectOnly:false});
 await delay(800);await invoke('cancel_export',{jobId:cancelId});assert.equal((await terminal(cancelId)).status,'cancelled');
 const result=await terminal(await invoke('start_text_removal_preview',{projectId:project.id,regionId,currentTime:.5,detectOnly:false}));
 assert.equal(result.status,'completed',JSON.stringify(result));
 const cached=await invoke('get_text_removal_cache_status',{projectId:project.id,regionId});
 assert.equal(cached.state,'removed');assert.ok(cached.cachedFrames>0);
 const before=Date.now();
 const repeated=await terminal(await invoke('start_text_removal_preview',{projectId:project.id,regionId,currentTime:.8,detectOnly:false}));
 assert.equal(repeated.status,'completed');assert.equal((await invoke('get_text_removal_cache_status',{projectId:project.id,regionId})).cachedFrames,cached.cachedFrames);
 const cachedPreviewSeconds=(Date.now()-before)/1000;
 const exportId=await invoke('start_export',{projectId:project.id,options:{quality:'balanced',encoder:'cpu',videoCodec:'h264',
   applyBlur:true,applyLogos:false,burnSubtitles:false,includeOriginalAudio:false,includeDubbing:false,outputFileName:'STTN-v2-QA'}});
 const exported=await terminal(exportId);assert.equal(exported.status,'completed',JSON.stringify(exported));
 const history=await invoke('get_export_history',{projectId:project.id});assert.ok(history.some(h=>h.status==='completed'));
 await evalJs(`(async()=>{const s=await import('/src/app/project-state.ts');s.setCurrentProject(${JSON.stringify(project)});s.setAiRemovalPreview(undefined);location.href='/project/${project.id}/translate';})()`);
 console.log(JSON.stringify({passed:true,projectId:project.id,source:project.source,detected:detectedStatus,removed:cached,
   cachedPreviewSeconds,exportHistory:history},null,2));
}finally{ws.close();}
