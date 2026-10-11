// Browser integration fixture: real media/DOM/Web Audio, deterministic Tauri boundary.
import { render } from 'solid-js/web';
import { Show } from 'solid-js';
import '../../../src/styles.css';
import TimelinePreview from '../../../src/features/timeline/TimelinePreview';
import TimelineEditor from '../../../src/features/timeline/TimelineEditor';
import { currentProject, setCurrentProject, setCurrentTranscript } from '../../../src/app/project-state';
import { seekTimeline, timelineBuffering, timelineTimeUs, setTimelinePlaying } from '../../../src/features/timeline/timeline-state';
import { canUndo, commitEdit, undoTimeline, redoTimeline } from '../../../src/features/timeline/edit-state';
import { editLayout, sameLayout, validateEdit } from '../../../src/features/timeline/edit-commands';
import { refreshSourceMedia, sourceMedia } from '../../../src/features/timeline/source-media';
import { playbackIndex, timelineScene } from '../../../src/features/timeline/scene-plan';
import type { Project } from '../../../src/features/projects/types';
import type { TimelineClip } from '../../../src/features/timeline/types';

const qa = window as any;
const createSource = AudioContext.prototype.createMediaElementSource;
AudioContext.prototype.createMediaElementSource = function (media) {
  const source = createSource.call(this, media); (media as any).__qaAudio = { source, context: this }; return source;
};
const transform = { x: .5, y: .5, scaleX: 1, scaleY: 1, rotation: 0, opacity: 1, flipX: false, flipY: false, fit: 'contain' as const, crop: { x: 0, y: 0, width: 1, height: 1 } };
const settings = { transcript: { status: 'completed', segmentCount: 1 }, translation: { sourceLanguage: 'auto', targetLanguage: 'vi', customPrompt: '', context: '', segments: [] },
  dubbing: { enabled: false, originalVolume: 1, dubbedVolume: 1 }, subtitle: { enabled: true, fontFamily: 'Arial', fontSize: 30, backgroundColor: '#000000', textColor: '#ffffff' },
  transcriptSettings: { mode: 'speechRecognition', ocrRegion: null }, blurRegions: [], logos: [], textOverlays: [] };
function project(minutes: number): Project {
  const clips: TimelineClip[] = [];
  for (let i = 0; i < minutes * 10; i++) {
    const content = i % 2 ? 'b' : 'a';
    const video: TimelineClip = { id: `clip-${i}`, trackId: 'video', startUs: i * 6e6, sourceInUs: 0, sourceOutUs: 4e6, rate: { numerator: 2, denominator: 3 }, content: { kind: 'video', assetId: `asset-${content}`, videoContentId: content }, transform, linkedClipId: null, groupId: null };
    clips.push(video, { ...video, id: `caption-${i}`, trackId: 'caption', content: { kind: 'subtitles', videoContentId: content }, linkedClipId: video.id },
      { ...video, id: `audio-${i}`, trackId: 'audio', content: { kind: 'sourceAudio', assetId: `asset-${content}`, videoContentId: content }, linkedClipId: video.id, audio: { volume: 0, muted: true, fadeInUs: 0, fadeOutUs: 0, voiceEnabled: false, voiceSpeed: 1, duckingEnabled: false, duckingAmount: .75, duckAttackUs: 60000, duckReleaseUs: 180000 } });
  }
  return { id: 'stability-project', name: 'Dự án 90 phút', schemaVersion: 12, updatedAt: new Date().toISOString(), ...settings,
    source: { videoPath: 'media/source/a.mp4', width: 320, height: 180, duration: 4 },
    timeline: { schemaVersion: 1, settings: { width: 320, height: 180, frameRate: { numerator: 30000, denominator: 1001 }, backgroundColor: '#000000' },
      legacyBinding: { assetId: 'asset-a', videoContentId: 'a', clipId: 'clip-0' },
      assets: ['a','b'].map(id => ({ id: `asset-${id}`, kind: 'video', relativePath: `media/source/${id}.mp4`, durationUs: 4e6, frameRate: null, variableFrameRate: null, metadata: { videoPath: `media/source/${id}.mp4`, originalName: `Video ${id.toUpperCase()}`, width: 320, height: 180, duration: 4, fps: 25, audioStreams: [{ streamIndex: 1, codec: 'aac', language: null, title: null, channels: 1, sampleRate: 48000 }] } })),
      videoContents: ['a','b'].map(id => ({ id, assetId: `asset-${id}`, revision: 0, settings })),
      tracks: ['video','caption','audio'].map((id, order) => ({ id, name: id, kind: id === 'video' ? 'visual' : id === 'caption' ? 'subtitle' : 'audio', order, enabled: true, locked: false, muted: false, solo: false })), clips } } as unknown as Project;
}
let stored = project(30), missing = false, callback = 0, held: Array<() => void> = [], hold = false;
const transcript = (id: string, text = `Caption ${id.toUpperCase()}`) => ({ schemaVersion: 1, projectId: stored.id, videoContentId: id, segments: [{ id: `segment-${id}`, start: 0, end: 4, source: text, translated: text, tts: null }] });
const manifest = (id: string, text?: string) => ({ videoContentId: id, contentRevision: 0, transcript: transcript(id, text), voices: [], warnings: [] });
qa.calls = [];
qa.isTauri = true;
qa.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
qa.__TAURI_INTERNALS__ = { convertFileSrc: (p: string) => p, transformCallback: () => ++callback,
  invoke: async (command: string, args: any) => {
    qa.calls.push({ command, args });
    if (command === 'get_timeline_source_media') {
      if (hold) return new Promise(resolve => held.push(() => resolve(manifest(args.videoContentId, 'Obsolete response'))));
      return manifest(args.videoContentId);
    }
    if (command === 'get_media_asset_path') { if (missing) throw new Error('Nguồn bị thiếu'); return `/.release-cache/timeline-stability-ui/${args.assetId === 'asset-b' ? 'b-default' : 'a'}.mp4`; }
    if (command === 'get_media_cache') return { thumbnailPath: null, waveform: null };
    if (command === 'get_project') return structuredClone(stored);
    if (command === 'apply_timeline_edit') {
      if (!sameLayout(editLayout(stored.timeline), args.expected)) throw new Error('CAS mismatch');
      validateEdit(stored.timeline, args.next);
      stored.timeline = { ...stored.timeline, ...structuredClone(args.next), tracks: args.next.tracks.map((t: any) => ({ ...stored.timeline.tracks.find(old => old.id === t.id), ...t })) };
      stored.updatedAt = new Date(Date.parse(stored.updatedAt) + 1).toISOString();
      return structuredClone(stored);
    }
    if (command.startsWith('plugin:event|')) return ++callback;
    if (command === 'get_recovery_notices') return [];
    if (command.startsWith('native_preview')) throw new Error('Native preview unavailable in browser fixture');
    return null;
  } };
qa.open = (minutes: number) => { stored = project(minutes); setCurrentTranscript(undefined); setCurrentProject(structuredClone(stored)); seekTimeline(0); };
qa.close = () => { setTimelinePlaying(false); setCurrentProject(undefined); setCurrentTranscript(undefined); };
qa.seek = seekTimeline;
qa.play = setTimelinePlaying;
qa.audioProbe = async () => {
  const media = document.querySelector<HTMLAudioElement>('[data-prefetch="false"].timeline-audio-region audio')!;
  const { source, context } = (media as any).__qaAudio;
  const capture = context.createScriptProcessor(4096, 1, 1), mute = context.createGain(); mute.gain.value = 0;
  const samples: number[] = []; capture.onaudioprocess = (e: AudioProcessingEvent) => samples.push(...e.inputBuffer.getChannelData(0));
  source.connect(capture); capture.connect(mute); mute.connect(context.destination);
  await new Promise(resolve => setTimeout(resolve, 800)); source.disconnect(capture); capture.disconnect(); mute.disconnect();
  const part = samples.slice(Math.round(context.sampleRate * .1));
  const tone = (hz: number) => { let re = 0, im = 0; part.forEach((v, i) => { const p = i * hz * 2 * Math.PI / context.sampleRate; re += v * Math.cos(p); im += v * Math.sin(p); }); return 2 * Math.hypot(re, im) / part.length; };
  return { hz440: tone(440), hz880: tone(880) };
};
qa.split = (id: string, atUs: number) => commitEdit({ kind: 'split', ids: [id], atUs }, 'Cắt clip');
qa.undo = undoTimeline; qa.redo = redoTimeline; qa.canUndo = canUndo;
qa.frame = () => ({ time: timelineTimeUs(), buffering: timelineBuffering(), source: sourceMedia('a')?.data?.transcript?.segments[0]?.translated });
qa.missing = (value: boolean) => { missing = value; };
qa.hold = () => { hold = true; refreshSourceMedia(); };
qa.release = () => { hold = false; const old = held; held = []; old.forEach(resolve => resolve()); };
qa.resumeRequests = () => { hold = false; };
qa.measure = () => {
  const doc = currentProject()!.timeline;
  const began = performance.now(), index = playbackIndex(doc), built = performance.now() - began;
  const start = performance.now();
  for (let i = 0; i < 1000; i++) timelineScene(doc, new Map(), (i * 7919999) % index.endUs, [], index);
  return { clips: doc.clips.length, buildMs: built, thousandScenesMs: performance.now() - start };
};
qa.open(30);
render(() => <Show when={currentProject()}><div class="translation-workspace" style={{ height: '100vh' }}><TimelinePreview /><TimelineEditor /></div></Show>, document.getElementById('root')!);
