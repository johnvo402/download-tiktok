import { describe, expect, it } from 'vitest';
import { audioOffsetUs, audioRegions, captionAt, clipAudio, clipWarnings, regionGain, sourceWithDraft, type SourceMedia } from './audio-layout';
import { applyEdit } from './edit-commands';
import type { TimelineClip, TimelineDocument, TimelineTrack } from './types';
import type { Transcript } from '../transcript/types';
const million = 1e6;
function fixture() {
  const track = (id: string, kind: TimelineTrack['kind'], order: number): TimelineTrack => ({ id:`track-${id}`,kind,order,name:id,locked:false,enabled:true,muted:false,solo:false });
  const clip = (id: string, start: number, content: TimelineClip['content'], trackId = 'visual'): TimelineClip => ({ id,startUs:start*million,sourceInUs:0,sourceOutUs:10*million,rate:{numerator:1,denominator:1},content:'assetId' in content ? {...content,assetId:`asset-${content.assetId}`} : content,trackId:`track-${trackId}`,linkedClipId:null,groupId:null,
    transform:{x:.5,y:.5,scaleX:1,scaleY:1,rotation:0,opacity:1,flipX:false,flipY:false,fit:'contain',crop:{x:0,y:0,width:1,height:1}} });
  const doc: TimelineDocument = { schemaVersion:1,settings:{width:640,height:360,frameRate:{numerator:25,denominator:1},backgroundColor:'#000000'},tracks:[track('visual','visual',0),track('music','audio',1),track('captions','subtitle',2)],legacyBinding:null,
    assets:['a','b','music'].map(id=>({id:`asset-${id}`,kind:id==='music'?'audio':'video',relativePath:`media/${id}`,durationUs:10*million,frameRate:null,variableFrameRate:null,metadata:{videoPath:`media/${id}`,originalName:id,audioStreams:[{streamIndex:1,codec:'aac',language:null,title:null,channels:1,sampleRate:48000}]}})),
    videoContents:['a','b'].map(id=>({id,assetId:`asset-${id}`,revision:3,settings:{dubbing:{enabled:true,originalVolume:1,dubbedVolume:.8,duckingEnabled:true,duckingAmount:.75},translation:{targetLanguage:id}} as TimelineDocument['videoContents'][number]['settings']})),
    clips:[clip('a1',0,{kind:'video',assetId:'a',videoContentId:'a'}),clip('b1',10,{kind:'video',assetId:'b',videoContentId:'b'}),clip('a2',20,{kind:'video',assetId:'a',videoContentId:'a'})] };
  const sources = new Map<string, SourceMedia>(['a','b'].map(id=>[id,{videoContentId:id,contentRevision:3,warnings:[],voices:[{segmentId:id,path:`${id}.wav`,startUs:2*million,endUs:6*million,speechStartUs:2.2*million,speechEndUs:5.8*million}],
    transcript:{segments:[{id,start:2.2,end:5.8,source:`Source ${id}`,translated:`Translated ${id}`}]} as Transcript} satisfies SourceMedia]));
  return { doc, sources, clip };
}
let serial = 0;
function edit(doc: TimelineDocument, command: Parameters<typeof applyEdit>[1]): TimelineDocument {
  const next = applyEdit(doc, command, () => `new-${++serial}`);
  return {...doc,...next,tracks:next.tracks.map(t=>({...doc.tracks.find(old=>old.id===t.id),enabled:true,locked:false,solo:false,muted:false,...t}))};
}
describe('audio/subtitle mapping and non-destructive edits', () => {
  it('stops outdated/deleted/different-file voice immediately in a draft and shifts unchanged speech with source timestamps', () => {
    const {sources}=fixture(), source=sources.get('a')!;
    source.transcript!.videoContentId='a'; source.transcript!.segments[0].tts={audioPath:'a.wav',duration:4,status:'ready'};
    const draft=structuredClone(source.transcript!);
    draft.segments[0].start=3.2;draft.segments[0].end=6.8;
    let updated=sourceWithDraft(source,draft);
    expect(updated.voices[0].startUs).toBe(3e6);expect(updated.voices[0].speechStartUs).toBe(3.2e6);
    expect(source.voices[0].startUs).toBe(2e6);
    draft.segments[0].translated='Unsaved edit';draft.segments[0].tts!.status='outdated';
    updated=sourceWithDraft(source,draft);expect(updated.voices).toEqual([]);expect(updated.warnings[0].segmentId).toBe('a');
    draft.segments[0].translated='Translated a';draft.segments[0].tts!.status='ready';draft.segments[0].tts!.audioPath='replacement.wav';
    expect(sourceWithDraft(source,draft).voices).toEqual([]);
    draft.segments=[];expect(sourceWithDraft(source,draft).voices).toEqual([]);
    draft.videoContentId='b';expect(sourceWithDraft(source,draft)).toBe(source);
  });
  it('maps B/A order, repeated A and gaps without modifying source transcript or file paths', () => {
    let {doc,sources} = fixture(); const before = JSON.stringify([...sources]);
    doc = edit(doc,{kind:'reorder',ids:['a1'],direction:1});
    const voice = audioRegions(doc,sources).filter(r=>r.kind==='voice');
    expect(voice.map(r=>[r.clip.id,r.path,r.startUs])).toEqual([['a1','a.wav',12e6],['b1','b.wav',2e6],['a2','a.wav',22e6]]);
    expect(captionAt(doc.clips[0],sources.get('a')!.transcript,13e6)).toBe('Translated a');
    expect(captionAt(doc.clips[0],sources.get('a')!.transcript,10e6)).toBe('');
    expect(JSON.stringify([...sources])).toBe(before);
  });
  it('trims and splits a WAV by source offset, keeping clipped captions half-open', () => {
    let {doc,sources} = fixture(); doc = edit(doc,{kind:'trim',id:'a1',edge:'start',atUs:3e6});
    let r = audioRegions(doc,sources).find(r=>r.clip.id==='a1'&&r.kind==='voice')!;
    expect([r.startUs,r.endUs,audioOffsetUs(r,3e6)]).toEqual([3e6,6e6,1e6]);
    expect(audioOffsetUs(r,6e6)).toBeNull(); expect(audioOffsetUs(r,2e6)).toBeNull();
    doc = edit(doc,{kind:'split',ids:['a1'],atUs:4e6});
    const left = doc.clips.find(c=>c.id==='a1')!, right = doc.clips.find(c=>c.content.kind==='video'&&c.startUs===4e6)!;
    const voices = audioRegions(doc,sources).filter(r=>[left.id,right.id].includes(r.clip.id)&&r.kind==='voice');
    expect(voices.map(r=>[r.startUs,r.endUs])).toEqual([[3e6,4e6],[4e6,6e6]]);
    r = voices.find(r=>r.clip.id===right.id)!;
    expect(audioOffsetUs(r,4e6)).toBe(2e6);
    expect(captionAt(left,sources.get('a')!.transcript,4e6)).toBe('');
    expect(captionAt(right,sources.get('a')!.transcript,4e6)).toBe('Translated a');
    expect(captionAt(right,sources.get('a')!.transcript,5.8e6)).toBe('');
  });
  it('retimes dependent audio/subtitles together, and voice speed consumes the existing WAV', () => {
    let {doc,sources,clip} = fixture();
    doc.clips.push({...clip('caption',0,{kind:'subtitles',videoContentId:'a'},'captions'),linkedClipId:'a1'});
    doc = edit(doc,{kind:'rate',id:'a1',rate:{numerator:2,denominator:1}});
    const a=doc.clips[0], caption=doc.clips.at(-1)!;
    expect(a.sourceOutUs).toBe(10e6); expect(caption.rate).toEqual(a.rate);
    let r=audioRegions(doc,sources).find(r=>r.clip.id==='a1'&&r.kind==='voice')!;
    expect([r.startUs,r.endUs,audioOffsetUs(r,1.5e6)]).toEqual([1e6,3e6,1e6]);
    expect(captionAt(caption,sources.get('a')!.transcript,1.5e6)).toBe('Translated a');
    doc=edit(doc,{kind:'audio',id:'a1',audio:{...clipAudio(a,doc),voiceSpeed:2}});
    r=audioRegions(doc,sources).find(r=>r.clip.id==='a1'&&r.kind==='voice')!;
    expect([r.startUs,r.endUs,audioOffsetUs(r,1.5e6)]).toEqual([1e6,2e6,2e6]);
    expect(sources.get('a')!.voices[0].path).toBe('a.wav');
  });
  it('extracts original audio and voice once, follows splits/copies, and explicitly detaches just that audio', () => {
    let {doc,sources} = fixture(); const content = JSON.stringify(doc.videoContents);
    doc=edit(doc,{kind:'extractAudio',videoClipId:'a1'});
    let original=doc.clips.at(-1)!; expect(original.content.kind).toBe('sourceAudio'); expect(original.linkedClipId).toBe('a1');
    expect(doc.clips[0].audio?.muted).toBe(true);
    expect(()=>edit(doc,{kind:'extractAudio',videoClipId:'a1'})).toThrow(/đã có/);
    doc=edit(doc,{kind:'voice',videoClipId:'a1'});
    const voice=doc.clips.at(-1)!;
    expect(doc.clips[0].audio?.voiceEnabled).toBe(false);
    expect(audioRegions(doc,sources).filter(r=>r.kind==='voice'&&['a1',voice.id].includes(r.clip.id))).toHaveLength(1);
    doc=edit(doc,{kind:'split',ids:['a1'],atUs:4e6});
    const right=doc.clips.find(c=>c.content.kind==='video'&&c.startUs===4e6)!;
    expect(doc.clips.find(c=>c.content.kind==='sourceAudio'&&c.startUs===4e6)?.linkedClipId).toBe(right.id);
    doc=edit(doc,{kind:'detach',id:original.id});
    original=doc.clips.find(c=>c.id===original.id)!; expect(original.linkedClipId).toBe(null);
    doc=edit(doc,{kind:'move',ids:['a1'],deltaUs:30e6});
    expect(doc.clips.find(c=>c.id===original.id)!.startUs).toBe(0);
    expect(JSON.stringify(doc.videoContents)).toBe(content);
  });
  it('mixes source/music fades and ducks against audible speech only, obeying track mute/solo', () => {
    const {doc,sources,clip}=fixture(); const music=clip('music',0,{kind:'audio',assetId:'music'},'music');
    music.audio={...clipAudio(music,doc),volume:1.5,fadeInUs:1e6,fadeOutUs:2e6,duckingEnabled:true,duckAttackUs:100000,duckReleaseUs:200000}; doc.clips.push(music);
    const regions=audioRegions(doc,sources), r=regions.find(r=>r.clip.id==='music')!;
    expect(regionGain(r,.5e6,regions,doc)).toBeCloseTo(.75);
    expect(regionGain(r,2.15e6,regions,doc)).toBeCloseTo(.9375);
    expect(regionGain(r,3e6,regions,doc)).toBeCloseTo(.375);
    expect(regionGain(r,5.9e6,regions,doc)).toBeCloseTo(.9375);
    expect(regionGain(r,9e6,regions,doc)).toBeCloseTo(.75);
    doc.tracks[0].muted=true; expect(regionGain(r,3e6,regions,doc)).toBeCloseTo(1.5);
    doc.tracks[0].muted=false; doc.tracks[0].solo=true; expect(regionGain(r,3e6,regions,doc)).toBe(0);
    doc.tracks[0].solo=false; doc.tracks[1].solo=true; expect(regionGain(r,3e6,regions,doc)).toBe(1.5);
    expect(regionGain(r,10e6,regions,doc)).toBe(0);
  });
  it('warns about mid-word cuts/overflow and excludes unrelated missing voices without changing timestamps', () => {
    const {doc,sources}=fixture(), source=sources.get('a')!; const c=doc.clips[0];
    c.sourceInUs=3e6;c.sourceOutUs=4e6;
    source.warnings.push({segmentId:'unrelated',message:'Missing'}, {segmentId:'a',message:'Outdated voice'});
    const before=JSON.stringify(source), warnings=clipWarnings(c,source);
    expect(warnings.map(w=>w.message)).toEqual([expect.stringMatching(/giữa lời/),'Outdated voice',expect.stringMatching(/vượt mép/)]);
    expect(warnings.some(w=>w.message==='Missing')).toBe(false); expect(JSON.stringify(source)).toBe(before);
    c.sourceInUs=0;c.sourceOutUs=4e6;
    expect(clipWarnings(c,source).find(w=>w.requiredSpeed)?.requiredSpeed).toBeCloseTo(1.9);
    c.audio={...clipAudio(c,doc),voiceSpeed:2};
    expect(clipWarnings(c,source).some(w=>w.message.includes('Giọng đọc vượt'))).toBe(false);
  });
  it('rejects invalid audio/retime/locked edits atomically and preserves default old clips', () => {
    const {doc}=fixture(), before=JSON.stringify(doc);
    expect(doc.clips[0].audio).toBeUndefined();
    expect(()=>edit(doc,{kind:'audio',id:'a1',audio:{...clipAudio(doc.clips[0],doc),volume:NaN}})).toThrow();
    expect(()=>edit(doc,{kind:'audio',id:'a1',audio:{...clipAudio(doc.clips[0],doc),duckAttackUs:500001}})).toThrow();
    expect(()=>edit(doc,{kind:'rate',id:'a1',rate:{numerator:0,denominator:1}})).toThrow();
    expect(()=>edit(doc,{kind:'rate',id:'a1',rate:{numerator:1,denominator:2}})).toThrow(/chồng lấn/);
    expect(JSON.stringify(doc)).toBe(before);
    doc.tracks[0].locked=true;
    expect(()=>edit(doc,{kind:'audio',id:'a1',audio:{...clipAudio(doc.clips[0],doc),volume:.5}})).toThrow(/khóa/);
  });
});
