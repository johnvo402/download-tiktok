import { createEffect, createMemo, createSignal, onCleanup, onMount, untrack, on } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { currentProject, currentTranscript, selectedOverlay, ocrRegionEditing, aiRemovalPreview } from '../../../app/project-state';
import type { Rect } from '../../overlays/overlay-utils';
interface Status { sessionId:string; state:string; position:number; playing:boolean; framePts:number; renderMs:number; error?:string }
interface Bounds { x:number;y:number;width:number;height:number;dpr:number;visible:boolean }
export interface Draft {kind:'blur'|'logo'|'text'|'subtitle'|'ocr';id:string;rect:Rect}
export function createNativePreview(viewport:()=>HTMLDivElement|undefined, media:()=>HTMLMediaElement|undefined, position:(time:number,playing:boolean)=>void) {
 const engine = import.meta.env.VITE_PREVIEW_ENGINE ?? 'auto';
 const [preferred,setPreferred]=createSignal(engine!=='compatibility');
 const [ready,setReady]=createSignal(false);
 const [warning,setWarning]=createSignal('');
 const [mounted,setMounted]=createSignal(false);
 const [reload,setReload]=createSignal(0);
 const [session,setSession]=createSignal<string>();
 const [draft,setDraft]=createSignal<Draft>();
 const [pick,setPick]=createSignal<Rect>();
 let alive=true, generation=0, version=0, lastAudio=0, scheduled=0, lastBounds='', lastVisual='', lastTranslations='';
 let listeners:UnlistenFn[]=[];let observer:ResizeObserver|undefined;let mutations:MutationObserver|undefined;
 createEffect(() => {
  mounted(); const id=session(), isReady=ready(), isNative=preferred(); const frame=viewport();
  if(frame){ frame.dataset.previewSession=id??''; frame.dataset.previewEngine=isNative?'native':'compatibility'; frame.dataset.previewReady=String(isReady); }
 });
 const bounds=():Bounds|undefined=>{
  const frame=viewport();if(!frame)return;const rect=frame.getBoundingClientRect();
  // WebView menus cannot overlap a native HWND. Suspend its visibility during dialogs.
  const occluded=!!document.querySelector('[role="dialog"],[role="menu"],.modal-backdrop,.jobs-dock-panel,[data-kb-popper-content-wrapper],.media-error-overlay,.media-status-overlay');
  return {x:rect.left,y:rect.top,width:Math.max(1,rect.width),height:Math.max(1,rect.height),dpr:window.devicePixelRatio,visible:!document.hidden&&!occluded&&rect.width>0&&rect.height>0};
 };
 const resize=()=>{
  const id=session(),b=bounds();if(!id||!b)return;const key=JSON.stringify(b);if(key===lastBounds)return;lastBounds=key;
  void invoke('preview_resize',{sessionId:id,bounds:b}).catch(()=>undefined);
 };
 const control=(action:string,time?:number)=>{const id=session();if(id)void invoke('preview_control',{sessionId:id,action,time:time??null}).catch(()=>undefined);};
 const scene=()=>{
  const id=session(),p=currentProject();if(!id||!p)return;
  const d=draft();
  const visual={subtitle:p.subtitle,blurRegions:p.blurRegions,logos:p.logos,textOverlays:p.textOverlays};
  const translations=(currentTranscript()?.segments??[]).map(s=>({id:s.id,start:s.start,end:s.end,translated:s.translated}));
  const visualKey=JSON.stringify(visual),translationKey=JSON.stringify(translations);
  const selected=selectedOverlay();
  let rect:Rect|undefined=pick();
  if(!rect&&d)rect=d.rect;
  if(!rect&&selected?.type==='blur')rect=p.blurRegions.find(r=>r.id===selected.id);
  if(!rect&&selected?.type==='logo')rect=p.logos.find(r=>r.id===selected.id);
  if(!rect&&selected?.type==='text')rect=p.textOverlays.find(r=>r.id===selected.id);
  if(!rect&&selected?.type==='subtitle')rect=p.blurRegions.find(r=>r.role==='subtitle')??p.subtitle.region??undefined;
  if(!rect&&ocrRegionEditing())rect=p.transcriptSettings?.ocrRegion??undefined;
  const w=p.source.width??1,h=p.source.height??1;
  const guide=rect&&rect.width>0&&rect.height>0?{x:Math.max(0,Math.round(rect.x*w)),y:Math.max(0,Math.round(rect.y*h)),width:Math.max(1,Math.floor(rect.width*w)),height:Math.max(1,Math.floor(rect.height*h))}:null;
  const sendVisual=visualKey!==lastVisual?visual:null;
  const old:typeof translations=lastTranslations?JSON.parse(lastTranslations):[];
  const patches=translationKey!==lastTranslations?[...translations.filter(s=>JSON.stringify(s)!==JSON.stringify(old.find(o=>o.id===s.id))),...old.filter(o=>!translations.some(s=>s.id===o.id)).map(o=>({...o,deleted:true}))]:null;
  lastVisual=visualKey;lastTranslations=translationKey;
  void invoke('preview_update_scene',{sessionId:id,version:++version,visual:sendVisual,translations:patches,guide,transform:d&&d.kind!=='ocr'?{kind:d.kind,id:d.id,...d.rect}:null}).catch(()=>undefined);
 };
 const scheduleScene=()=>{if(scheduled)return;scheduled=requestAnimationFrame(()=>{scheduled=0;untrack(scene);});};
 createEffect(()=>{currentProject()?.subtitle;currentProject()?.blurRegions;currentProject()?.logos;currentProject()?.textOverlays;currentTranscript()?.segments;selectedOverlay();ocrRegionEditing();draft();pick();session();scheduleScene();});
 const sourceKey=()=>JSON.stringify([currentProject()?.id,currentProject()?.source.videoPath,currentProject()?.source.checksumSha256,aiRemovalPreview()?.path,aiRemovalPreview()?.startSeconds]);
 const sourceIdentity=createMemo(()=>JSON.stringify([mounted(),sourceKey(),reload()]));
 createEffect(on(sourceIdentity, () => {
  if(!mounted()||engine==='compatibility')return;
  const key=sourceKey();const p=currentProject();const b=bounds();
  if(!p?.source.videoPath||!b){++generation;const old=untrack(session);setSession(undefined);setReady(false);if(old)void invoke('preview_close',{sessionId:old}).catch(()=>undefined);return;}
  setPreferred(engine!=='compatibility');setWarning('');
  const token=++generation;const old=untrack(session);setSession(undefined);setReady(false);lastVisual='';lastTranslations='';lastBounds='';
  const preview=aiRemovalPreview()?.projectId===p.id && !aiRemovalPreview()?.nativeCache?aiRemovalPreview():undefined;
  void (async()=>{
   if(old)await invoke('preview_close',{sessionId:old});
   const id=await invoke<string>('preview_open',{projectId:p.id,bounds:b,clean:preview?{path:preview.path,startSeconds:preview.startSeconds,endSeconds:preview.endSeconds}:null});
   if(!alive||token!==generation||key!==sourceKey()){await invoke('preview_close',{sessionId:id});return;}
   setSession(id);resize();scene();
   const audio=media(); if(audio&&Number.isFinite(audio.currentTime)&&audio.currentTime>0)control('seek',audio.currentTime+(preview?.startSeconds??0));
   if(audio&&!audio.paused)control('play');
   const status=await invoke<Status>('preview_status',{sessionId:id});if(status.state==='error')throw new Error(status.error);
  })().catch((error:unknown)=>{
   if(alive&&token===generation){setWarning(error instanceof Error?error.message:String(error));if(engine==='auto')setPreferred(false);}
  });
 }));
 onMount(()=>{
  void Promise.all([
   listen<Status>('preview-status',({payload:st})=>{
    if(st.sessionId!==session())return;
    if(st.state==='closed'){setSession(undefined);setReady(false);return;}
    if(st.state==='error'){setReady(false);setWarning(st.error??'Preview GPU không khả dụng.');if(engine==='auto')setPreferred(false);const id=session();setSession(undefined);if(id)void invoke('preview_close',{sessionId:id});return;}
    setReady(true);
    if(st.state==='suspended'){media()?.pause();position(st.position,false);return;}
    const audio=media();if(!audio||audio.paused||!Number.isFinite(audio.duration))position(st.position,st.playing);
   }),
   listen<{sessionId:string;paths:string[]}>('preview-drop',({payload:p})=>{if(p.sessionId===session())window.dispatchEvent(new CustomEvent('native-preview-drop',{detail:p.paths}));}),
   listen<{sessionId:string;kind:string;x:number;y:number;buttons:number}>('preview-pointer',({payload:p})=>{
    if(p.sessionId!==session())return;const rect=viewport()?.getBoundingClientRect();if(!rect)return;
    window.dispatchEvent(new CustomEvent('native-preview-pointer',{detail:{...p,clientX:rect.left+p.x,clientY:rect.top+p.y}}));
   })
  ]).then(ls=>{if(!alive)ls.forEach(l=>l());else{listeners=ls;setMounted(true);}}).catch((error:unknown)=>{if(alive){setWarning(error instanceof Error?error.message:String(error));setPreferred(false);}});
  observer=new ResizeObserver(resize);if(viewport())observer.observe(viewport()!);
  mutations=new MutationObserver(resize);mutations.observe(document.body,{childList:true,subtree:true,attributes:true,attributeFilter:['hidden','open','aria-hidden']});
  window.addEventListener('resize',resize);document.addEventListener('fullscreenchange',resize);document.addEventListener('visibilitychange',resize);
  const tick=(now:number)=>{if(alive){resize();const audio=media();if(session()&&audio&&!audio.paused&&Number.isFinite(audio.currentTime)&&now-lastAudio>=100){lastAudio=now;control('audio',audio.currentTime+(aiRemovalPreview()?.nativeCache?0:aiRemovalPreview()?.startSeconds??0));}requestAnimationFrame(tick);}};requestAnimationFrame(tick);
 });
 onCleanup(()=>{alive=false;++generation;cancelAnimationFrame(scheduled);observer?.disconnect();mutations?.disconnect();listeners.forEach(l=>l());window.removeEventListener('resize',resize);document.removeEventListener('fullscreenchange',resize);document.removeEventListener('visibilitychange',resize);const id=session();if(id)void invoke('preview_close',{sessionId:id}).catch(()=>undefined);});
 return {preferred,ready,warning,control,setDraft,setPick,reload:()=>setReload(n=>n+1)};
}
