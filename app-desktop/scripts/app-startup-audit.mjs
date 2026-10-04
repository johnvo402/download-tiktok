// Isolated debug app timing. No model/network jobs or access to user projects.
import {spawn, execFileSync} from 'node:child_process';
import path from 'node:path';
import fs from 'node:fs';
import assert from 'node:assert/strict';
const root=process.cwd(), output=process.argv[2];
assert.ok(output, 'Pass output JSON path');
const began=performance.now();
const processHandle=spawn(path.resolve('src-tauri/target/debug/ai-video-localization.exe'),[],{windowsHide:true,stdio:'ignore',env:{...process.env,
 JC_QA_DATA_DIR:process.env.JC_TASK09_DATA_ROOT || path.join(root,'.release-cache/preview-validation/data/com.ai.video-localization'),
 WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-port=9224',
 WEBVIEW2_USER_DATA_FOLDER:path.join(root,'.release-cache/task09-startup-webview'),
 LOCALIZATION_PYTHON:path.join(root,'python/.release-ai-venv/Scripts/python.exe')}});
const delay=ms=>new Promise(r=>setTimeout(r,ms));
let ws;
try {
 let page;
 for(let i=0;i<1200;i++){
  try{const pages=await(await fetch('http://127.0.0.1:9224/json/list')).json();page=pages.find(p=>p.url.startsWith('http://127.0.0.1:1420'));}catch{}
  if(page)break;await delay(100);
 }
 assert.ok(page, 'Debug app did not start');
 ws=new WebSocket(page.webSocketDebuggerUrl);await new Promise(r=>ws.addEventListener('open',r,{once:true}));
 let serial=0;const pending=new Map();
 ws.addEventListener('message',e=>{const m=JSON.parse(e.data);pending.get(m.id)?.(m);pending.delete(m.id);});
 const evaluate=(expression,timeout=10000)=>new Promise((resolve,reject)=>{
  const id=++serial;const timer=setTimeout(()=>reject(Error('CDP timeout')),timeout);
  pending.set(id,m=>{clearTimeout(timer);if(m.error||m.result?.exceptionDetails)reject(Error(JSON.stringify(m)));else resolve(m.result.result.value);});
  ws.send(JSON.stringify({id,method:'Runtime.evaluate',params:{expression,awaitPromise:true,returnByValue:true}}));
 });
 let loaded=false;
 for(let i=0;i<1200;i++){
  if(await evaluate(`(async()=>{const s=await import('http://127.0.0.1:1420/src/app/project-state.ts');return s.appStartupReady()&&!s.projectsLoading();})()`).catch(()=>false)){loaded=true;break;}
  await delay(100);
 }
 if(!loaded) console.log(await evaluate('({url:location.href,ready:document.readyState,body:document.body?.innerText.slice(0,1000)})'));
 assert.ok(loaded,'Project Manager did not become ready');
 const ready=performance.now()-began;
 const projects=await evaluate(`window.__TAURI_INTERNALS__.invoke('list_projects')`);
 assert.ok(projects.projects.every(p=>p.name.startsWith('Native Preview QA')),'Unexpected non-QA project');
 let projectOpenMs=null;
 if(projects.projects.length){
  const start=performance.now();
  await evaluate(`window.__TAURI_INTERNALS__.invoke('get_project',{id:${JSON.stringify(projects.projects[0].id)}})`);
  projectOpenMs=performance.now()-start;
 }
 const memory=JSON.parse(execFileSync(path.resolve('.release-cache/sttn-qa/venv/Scripts/python.exe'),['-c',
  `import psutil,json;p=psutil.Process(${processHandle.pid});items=[p,*p.children(recursive=True)];print(json.dumps([{'name':c.name(),'rssBytes':c.memory_info().rss} for c in items if c.is_running()]))`],{encoding:'utf-8',windowsHide:true}));
 const report={measuredAt:new Date().toISOString(),projectListReadyMs:ready,projectReadMs:projectOpenMs,
  treeRssBytes:memory.reduce((n,c)=>n+c.rssBytes,0),processes:memory,
  definition:'Debug app + running Vite; disk cache not flushed. Project read measures command, not completed native video load.'};
 fs.mkdirSync(path.dirname(output),{recursive:true});fs.writeFileSync(output,JSON.stringify(report,null,2));console.log(report);
 if(process.env.JC_TASK09_RUNTIME_SMOKE==='1'){
  const invoke=(name,args={},timeout=300000)=>evaluate(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(name)},${JSON.stringify(args)})`,timeout);
  const before=await invoke('get_optional_runtime_status');
  const plan=await invoke('get_optional_runtime_plan',{feature:'ocr_openvino',profile:'openvino'});
  const installed=await invoke('install_optional_runtime',{feature:'ocr_openvino',profile:'openvino'},900000);
  assert.ok(installed.installedFeatures.includes('ocr_openvino'));
  const checked=await invoke('check_optional_runtime_feature',{feature:'ocr_openvino'});
  assert.ok(checked.verified,JSON.stringify(checked));
  const duplicate=await invoke('get_optional_runtime_plan',{feature:'ocr_openvino',profile:'openvino'});
  assert.equal(duplicate.estimatedDownloadBytes,0);
  const removed=await invoke('uninstall_optional_runtime',{feature:'ocr_openvino'});
  assert.ok(!removed.installedFeatures.includes('ocr_openvino'));
  for(const feature of before.installedFeatures)assert.ok(removed.installedFeatures.includes(feature));
  const runtimeReport={before,plan,installed,checked,duplicate,removed};
  fs.writeFileSync(path.resolve('docs/benchmarks/task09-runtime-install.json'),JSON.stringify(runtimeReport,null,2));
  console.log('Isolated Intel GPU install/check/reuse/uninstall passed.');
 }
 if(process.env.JC_TASK09_SHARED_RUNTIME_SMOKE==='1') {
  assert.ok(process.env.JC_TASK09_DATA_ROOT?.includes('.release-cache'), 'Use an isolated runtime root');
  const invoke=(name,args={},timeout=600000)=>evaluate(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(name)},${JSON.stringify(args)})`,timeout);
  await invoke('uninstall_optional_runtime',{feature:'bandit'});
  await invoke('install_optional_runtime',{feature:'sttn',profile:'cpu'});
  const plan=await invoke('get_optional_runtime_plan',{feature:'bandit',profile:'cpu'});
  assert.equal(plan.alreadyInstalled,false);
  assert.ok(plan.missingComponents.length>0);
  assert.ok(plan.reusedComponents.some(label=>label.includes('CPU')));
  const installed=await invoke('install_optional_runtime',{feature:'bandit',profile:'cpu'});
  assert.ok(installed.installedFeatures.includes('sttn') && installed.installedFeatures.includes('bandit'));
  const checked=await invoke('check_optional_runtime_feature',{feature:'bandit'});
  assert.ok(checked.verified,JSON.stringify(checked));
  const removed=await invoke('uninstall_optional_runtime',{feature:'bandit'});
  assert.ok(removed.installedFeatures.includes('sttn') && !removed.installedFeatures.includes('bandit'));
  const survivor=await invoke('check_optional_runtime_feature',{feature:'sttn'});
  assert.ok(survivor.verified,JSON.stringify(survivor));
  fs.writeFileSync(path.resolve('docs/benchmarks/task09-shared-runtime.json'),JSON.stringify({plan,installed,checked,removed,survivor},null,2));
  console.log('Real CPU shared Torch staging/add/remove regression passed.');
 }
 if(process.env.JC_TASK09_STTN_SMOKE==='1') {
  const log=execFileSync(process.execPath,['scripts/text-removal-smoke.mjs'],{cwd:root,encoding:'utf-8',windowsHide:true,timeout:300000,
    env:{...process.env,JC_TEXT_REMOVAL_SOURCE:path.join(root,'.release-cache/sttn-qa/source.mp4')}});
  fs.writeFileSync(path.resolve('.release-cache/task09-desktop-sttn.log'),log);
  assert.ok(log.includes('"passed": true'),log);
  console.log('Desktop sparse detection/STTN/cancel/cache/export regression passed.');
 }
 await evaluate(`(async()=>{const {getCurrentWindow}=await import('/node_modules/@tauri-apps/api/window.js');await getCurrentWindow().close();})()`).catch(()=>{});
}finally{ws?.close();processHandle.kill();}
