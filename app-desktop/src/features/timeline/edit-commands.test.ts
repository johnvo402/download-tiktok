import { describe, expect, it } from 'vitest';
import { applyEdit, editClosure, editLayout, snapMove, snapTime } from './edit-commands';
import { clipEndUs, timelineToSourceUs } from './time';
import type { TimelineClip, TimelineDocument, TimelineTrack } from './types';
const transform: TimelineClip['transform'] = { x:.5,y:.5,scaleX:1,scaleY:1,rotation:0,opacity:1,flipX:false,flipY:false,fit:'contain',crop:{x:0,y:0,width:1,height:1} };
function fixture(): TimelineDocument {
  const track = (id: string, kind: TimelineTrack['kind'], order: number): TimelineTrack => ({ id:`track-${id}`, kind, order, name:id,locked:false,enabled:true,muted:false,solo:false });
  const clip = (id: string, trackId: string, startUs: number, sourceInUs: number, sourceOutUs: number, content: TimelineClip['content'], linkedClipId: string|null = null): TimelineClip => ({ id,trackId:`track-${trackId}`,startUs,sourceInUs,sourceOutUs,content:'assetId' in content ? {...content,assetId:`asset-${content.assetId}`} : content,linkedClipId,groupId:null,transform,rate:{numerator:1,denominator:1} });
  return { schemaVersion:1,settings:{width:640,height:360,backgroundColor:'#000000',frameRate:{numerator:25,denominator:1}},
    assets:['a','b','music','image'].map(id=>({id:`asset-${id}`,kind:id==='image'?'image':id==='music'?'audio':'video',durationUs:id==='image'?null:30e6,relativePath:`media/${id}`,frameRate:null,variableFrameRate:null,metadata:{originalName:id,videoPath:`media/${id}`}})),
    videoContents:['a','b'].map(id=>({id:`content-${id}`,assetId:`asset-${id}`,revision:7,settings:{translation:{targetLanguage:id},dubbing:{voice:id}} as unknown as TimelineDocument['videoContents'][number]['settings']})),
    tracks:[track('video','visual',0),track('image','visual',1),track('music','audio',2),track('voice','audio',3),track('subtitles','subtitle',4)],
    clips:[clip('a','video',0,2e6,12e6,{kind:'video',assetId:'a',videoContentId:'content-a'}),clip('b','video',10e6,0,10e6,{kind:'video',assetId:'b',videoContentId:'content-b'}),
      clip('image','image',4e6,0,3e6,{kind:'image',assetId:'image'},'a'),clip('music','music',0,0,20e6,{kind:'audio',assetId:'music'}),
      clip('voice','voice',0,2e6,12e6,{kind:'voice',videoContentId:'content-a'},'a'),clip('subtitles','subtitles',0,2e6,12e6,{kind:'subtitles',videoContentId:'content-a'},'a')],
    legacyBinding:{assetId:'asset-a',videoContentId:'content-a',clipId:'a'} };
}
let serial = 0; const uuid = () => `new-${++serial}`;
const apply = (doc: TimelineDocument, command: Parameters<typeof applyEdit>[1]) => applyEdit(doc, command, uuid);
const find = (layout: ReturnType<typeof applyEdit>, id: string) => layout.clips.find(c => c.id === id)!;
const updated = (doc: TimelineDocument, layout: ReturnType<typeof applyEdit>): TimelineDocument => ({ ...doc, ...layout, tracks:layout.tracks.map(t=>({...doc.tracks.find(old=>old.id===t.id),locked:false,enabled:true,muted:false,solo:false,...t})) });
describe('transactional timeline editing', () => {
  it('edits one visual instance and the canvas without changing source coordinates, shared content or timing', () => {
    const doc = fixture(), before = JSON.stringify(doc);
    const visual = apply(doc, { kind: 'visual', id: 'a', transform: { ...transform, x: .75, scaleX: .25, scaleY: .25, rotation: 30, crop: { x: .2, y: .1, width: .6, height: .8 } } });
    expect(find(visual, 'a').sourceInUs).toBe(2e6); expect(find(visual, 'voice').transform).toEqual(transform);
    expect(doc.videoContents[0].revision).toBe(7); expect(JSON.stringify(doc)).toBe(before);
    const canvas = apply(doc, { kind: 'canvas', settings: { ...doc.settings, width: 1080, height: 1920 } });
    expect(canvas.clips).toEqual(doc.clips); expect(canvas.settings.width).toBe(1080);
    doc.tracks[0].locked = true; expect(() => apply(doc, { kind: 'visual', id: 'a', transform: { ...transform, opacity: .5 } })).toThrow(/khóa/);
  });
  it('adds editable text and a linked caption with the trimmed source window/rate', () => {
    const doc = fixture(); const text = apply(doc, { kind: 'text', text: 'Logo chữ', atUs: 4e6 });
    expect(text.clips.at(-1)?.content).toEqual({ kind: 'text', text: 'Logo chữ' }); expect(text.clips.at(-1)?.startUs).toBe(4e6);
    const caption = apply(doc, { kind: 'caption', videoClipId: 'a' }); const c = caption.clips.at(-1)!;
    expect(c.content).toEqual({ kind: 'subtitles', videoContentId: 'content-a' }); expect(c.linkedClipId).toBe('a'); expect(c.sourceInUs).toBe(2e6);
    expect(timelineToSourceUs(c, 3e6)).toBe(5e6); expect(doc.videoContents.length).toBe(2);
  });
  it('moves linked clips and groups without rewriting source windows or content/revisions', () => {
    const doc=fixture(), bytes=JSON.stringify(doc);
    const grouped=updated(doc,apply(doc,{kind:'group',ids:['a','b']}));
    expect(editClosure(grouped,['image']).size).toBe(5);
    const moved=apply(grouped,{kind:'move',ids:['a'],deltaUs:10e6});
    expect(find(moved,'a').startUs).toBe(10e6);expect(find(moved,'b').startUs).toBe(20e6);expect(find(moved,'image').startUs).toBe(14e6);
    expect(timelineToSourceUs(find(moved,'voice'),12e6)).toBe(4e6);
    expect(find(moved,'a').sourceInUs).toBe(2e6);expect(JSON.stringify(doc)).toBe(bytes);
  });
  it('trims the video and linked subtitle/voice, intersects a linked image, and rejects source overflow', () => {
    const doc=fixture(), trimmed=apply(doc,{kind:'trim',id:'a',edge:'start',atUs:5e6});
    expect(find(trimmed,'a').sourceInUs).toBe(7e6);expect(clipEndUs(find(trimmed,'a'))).toBe(10e6);
    expect(find(trimmed,'voice').sourceInUs).toBe(7e6);expect(find(trimmed,'image').sourceInUs).toBe(1e6);
    expect(()=>apply(doc,{kind:'trim',id:'a',edge:'start',atUs:8e6})).toThrow(/toàn bộ lớp/);
    const alone={...doc,clips:doc.clips.filter(c=>c.id==='a')};
    expect(()=>apply(alone,{kind:'trim',id:'a',edge:'end',atUs:40e6})).toThrow(/vượt/);
    expect(()=>apply(alone,{kind:'trim',id:'a',edge:'start',atUs:10e6})).toThrow();
    const imageTrim=apply(doc,{kind:'trim',id:'image',edge:'start',atUs:5e6});
    expect(find(imageTrim,'a').startUs).toBe(0);expect(find(imageTrim,'image').sourceInUs).toBe(1e6);
    expect(()=>apply(doc,{kind:'trim',id:'image',edge:'end',atUs:11e6})).toThrow(/thời lượng video/);
  });
  it('splits linked video/subtitle/voice/image and rebinds right halves to the new video instance', () => {
    const doc=fixture(), cut=apply(doc,{kind:'split',ids:['a'],atUs:5e6});
    const right=cut.clips.find(c=>c.content.kind==='video' && c.startUs===5e6)!;
    expect(find(cut,'a').sourceOutUs).toBe(7e6);expect(right.sourceInUs).toBe(7e6);
    for(const kind of ['voice','subtitles','image'])expect(cut.clips.find(c=>c.content.kind===kind && c.startUs===5e6)?.linkedClipId).toBe(right.id);
    expect(timelineToSourceUs(right,8e6)).toBe(timelineToSourceUs(doc.clips[0],8e6));
    expect(()=>apply(doc,{kind:'split',ids:['a'],atUs:0})).toThrow(/bên trong/);
  });
  it('quantizes fractional-rate cuts to representable microsecond boundaries and preserves mapping', () => {
    const doc=fixture();doc.clips=[{...doc.clips[0],sourceInUs:0,sourceOutUs:10,rate:{numerator:1,denominator:2}}];
    const cut=apply(doc,{kind:'split',ids:['a'],atUs:3});const right=cut.clips[1];
    expect(clipEndUs(cut.clips[0])).toBe(right.startUs);expect(right.startUs).toBe(4);
    for(let t=4;t<20;t++)expect(timelineToSourceUs(right,t)).toBe(timelineToSourceUs(doc.clips[0],t));
  });
  it('swaps A/B order including their dependent layers and keeps source timestamps intact', () => {
    const doc=fixture(), reordered=apply(doc,{kind:'reorder',ids:['a'],direction:1});
    expect(find(reordered,'b').startUs).toBe(0);expect(find(reordered,'a').startUs).toBe(10e6);
    expect(find(reordered,'image').startUs).toBe(14e6);expect(find(reordered,'subtitles').startUs).toBe(10e6);
    expect(find(reordered,'a').sourceInUs).toBe(2e6);
  });
  it('duplicates and pastes new instances, keeps shared content and maps internal links/groups', () => {
    const doc=fixture(), grouped=updated(doc,apply(doc,{kind:'group',ids:['a','image']}));
    const copies=apply(grouped,{kind:'duplicate',ids:['a'],atUs:20e6});
    const video=copies.clips.find(c=>c.id!=='a'&&c.content.kind==='video'&&c.content.videoContentId==='content-a')!;
    expect(video.startUs).toBe(20e6);expect(video.groupId).not.toBe(find(copies,'a').groupId);
    expect(copies.clips.find(c=>c.content.kind==='voice'&&c.id!=='voice')?.linkedClipId).toBe(video.id);
    const selected=grouped.clips.filter(c=>editClosure(grouped,['a']).has(c.id));
    const pasted=apply(grouped,{kind:'paste',clips:selected,atUs:0});
    expect(pasted.tracks.length).toBeGreaterThan(grouped.tracks.length);
    expect(pasted.clips.filter(c=>c.content.kind==='video'&&c.startUs===0)).toHaveLength(2);
  });
  it('deletes normally without shifting, then ripple trims music and shifts B by the same global interval', () => {
    const doc=fixture(), deleted=apply(doc,{kind:'delete',ids:['a'],ripple:false});
    expect(find(deleted,'b').startUs).toBe(10e6);expect(deleted.legacyBinding?.clipId).toBeNull();expect(deleted.clips).toHaveLength(2);
    const ripple=apply(doc,{kind:'delete',ids:['a'],ripple:true});
    expect(find(ripple,'b').startUs).toBe(0);expect(find(ripple,'music').sourceInUs).toBe(10e6);expect(clipEndUs(find(ripple,'music'))).toBe(10e6);
    expect(timelineToSourceUs(find(ripple,'music'),2e6)).toBe(12e6);
  });
  it('handles disjoint ripple windows and keeps both surviving music portions with different source offsets', () => {
    const doc=fixture();doc.clips.find(c=>c.id==='music')!.sourceOutUs=30e6;
    doc.clips.push({...doc.clips[2],id:'later-image',startUs:25e6,sourceOutUs:2e6,linkedClipId:null});
    const ripple=apply(doc,{kind:'delete',ids:['a','later-image'],ripple:true});
    const music=ripple.clips.filter(c=>c.content.kind==='audio');
    expect(music.map(c=>[c.startUs,c.sourceInUs,c.sourceOutUs])).toEqual([[0,10e6,25e6],[15e6,27e6,30e6]]);
  });
  it('rejects overlap, incompatible track and any change to a locked track atomically', () => {
    const doc=fixture(), original=editLayout(doc);
    expect(()=>apply(doc,{kind:'move',ids:['b'],deltaUs:-5e6})).toThrow(/chồng lấn/);
    expect(()=>apply(doc,{kind:'move',ids:['b'],deltaUs:0,trackId:'music'})).toThrow(/không phù hợp/);
    doc.tracks.find(t=>t.id==='track-music')!.locked=true;
    expect(()=>apply(doc,{kind:'delete',ids:['a'],ripple:true})).toThrow(/khóa/);
    expect(editLayout(doc)).toEqual(original);
  });
  it('links/unlinks and groups/ungroups without changing content and inserts owned assets into free tracks', () => {
    const doc=fixture();doc.clips[2].linkedClipId=null;
    const linked=updated(doc,apply(doc,{kind:'link',ids:['a','image']}));expect(linked.clips[2].linkedClipId).toBe('a');
    const unlinked=apply(linked,{kind:'unlink',ids:['a']});expect(find(unlinked,'image').linkedClipId).toBeNull();
    const added=apply(doc,{kind:'add',assetId:'asset-image',atUs:5e6});expect(added.clips.at(-1)!.trackId).not.toBe('track-video');
    const empty={...doc,clips:[],legacyBinding:null};const first=apply(empty,{kind:'add',assetId:'asset-b',atUs:null});expect(first.legacyBinding?.clipId).toBe(first.clips[0].id);
  });
  it('snaps moving edges or trim time within tolerance, excludes moving links and preserves unsnapped values', () => {
    const doc=fixture();expect(snapMove(doc,['a'],10e6+2000,30e6,8000)).toBe(10e6);
    expect(snapMove(doc,['a'],11e6,30e6,8000)).toBe(11e6);
    expect(snapTime(doc,['a'],10e6+2000,30e6,8000)).toBe(10e6);
    expect(snapTime(doc,['a'],11e6,30e6,8000)).toBe(11e6);
  });
});
