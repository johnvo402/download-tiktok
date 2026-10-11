import { describe, expect, it } from 'vitest';
import { clipVideoId, eventForVideo, mergeVideoView, projectForVideo, videoIdsForScope } from './video-context';
import type { Project } from '../projects/types';

function fixture() {
  const settings = (label: string) => ({ transcript: null, translation: { sourceLanguage: 'auto', targetLanguage: label, segments: [] },
    dubbing: { voice: label }, subtitle: {}, transcriptSettings: {}, blurRegions: [], logos: [], textOverlays: [] });
  return { id: 'project', source: { originalName: 'A.mp4', videoPath: 'a.mp4' }, ...settings('vi'),
    timeline: { assets: [{ id: 'asset-a', metadata: { originalName: 'A.mp4', videoPath: 'a.mp4' } },
      { id: 'asset-b', metadata: { originalName: 'B.mp4', videoPath: 'b.mp4' } }],
    videoContents: [{ id: 'a', assetId: 'asset-a', revision: 2, settings: settings('vi') },
      { id: 'b', assetId: 'asset-b', revision: 7, settings: settings('en') }],
    clips: [{ id: 'clip-a', startUs: 0, content: { kind: 'video', assetId: 'asset-a', videoContentId: 'a' } },
      { id: 'clip-b', startUs: 10000000, content: { kind: 'video', assetId: 'asset-b', videoContentId: 'b' } },
      { id: 'clone-a', startUs: 20000000, content: { kind: 'video', assetId: 'asset-a', videoContentId: 'a' } }],
    legacyBinding: { videoContentId: 'a', assetId: 'asset-a', clipId: 'clip-a' } } } as unknown as Project;
}

describe('per-video editing context', () => {
  it('selects source settings through clip content identity and retains them after reorder/trim', () => {
    const project = fixture();
    const id = clipVideoId(project.timeline.clips[1]);
    project.timeline.clips[1].startUs = 0;
    project.timeline.clips[1].sourceInUs = 5000000;
    project.timeline.clips.reverse();
    expect(projectForVideo(project, id).source.originalName).toBe('B.mp4');
    expect(projectForVideo(project, id).translation.targetLanguage).toBe('en');
    expect(projectForVideo(project, id).dubbing.voice).toBe('en');
    expect(project.timeline.videoContents[1].revision).toBe(7);
    expect(project.source.originalName).toBe('A.mp4');
  });
  it('merges a B editing projection without replacing the legacy A preview or A settings', () => {
    const project = fixture();
    const view = projectForVideo(project, 'b');
    view.translation = { ...view.translation, targetLanguage: 'ja' };
    const merged = mergeVideoView(project, view, 'b');
    expect(merged.source.originalName).toBe('A.mp4');
    expect(merged.translation.targetLanguage).toBe('vi');
    expect(projectForVideo(merged, 'b').translation.targetLanguage).toBe('ja');
    expect(project.timeline.videoContents[1].settings.translation.targetLanguage).toBe('en');
  });
  it('accepts only progress events belonging to the selected video and project', () => {
    const project = fixture();
    const event = { projectId: project.id, videoContentId: 'a', jobId: 'job-a', status: 'completed', message: 'A' };
    expect(eventForVideo(event, project, 'a')).toBe(event);
    expect(eventForVideo(event, project, 'b')).toBeUndefined();
    expect(eventForVideo({ ...event, projectId: 'other' }, project, 'a')).toBeUndefined();
    expect(eventForVideo({ ...event, videoContentId: null }, project, 'a')).toBeDefined();
    expect(eventForVideo({ ...event, videoContentId: null }, project, 'b')).toBeUndefined();
  });
  it('deduplicates repeated clips and filters selected/all scopes by current timeline membership', () => {
    const project = fixture();
    expect(videoIdsForScope(project, 'all', 'a', new Set())).toEqual(['a', 'b']);
    expect(videoIdsForScope(project, 'selected', 'a', new Set(['b', 'deleted']))).toEqual(['b']);
    expect(videoIdsForScope(project, 'current', 'b', new Set())).toEqual(['b']);
    expect(videoIdsForScope(project, 'current', 'deleted', new Set())).toEqual([]);
    expect(() => projectForVideo(project, 'foreign')).toThrow();
  });
});
