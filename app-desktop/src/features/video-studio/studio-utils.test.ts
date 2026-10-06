import { describe, expect, it } from 'vitest';
import { audioCurrent, exportProblems, newDraft, removeChapter, restoreAudio, timeline } from './studio-utils';
import type { SceneAudio, VideoScene } from './types';

const audio: SceneAudio = { path: 'studio/assets/a.wav', duration: 600, narration: 'Lời kể', language: 'vi', voice: 'Voice', cues: [{ start: 0, end: 600, text: 'Lời kể' }] };
const scene = (id: string): VideoScene => ({ id, chapterId: 'chapter', title: id, narration: audio.narration, imagePrompt: 'Prompt', image: 'studio/assets/a.png', imageHistory: [], audio: { ...audio }, audioHistory: [] });
describe('video readiness', () => {
  it('requires the selected duration of current audio, with all images and chapters ready', () => {
    const d = newDraft('project'); d.voice = 'Voice'; d.outline = [{ id: 'chapter', title: 'Chapter', summary: '', targetSeconds: 1200 }];
    d.scenes = [scene('a')];
    expect(exportProblems(d).join(' ')).toContain('10:00');
    d.scenes.push(scene('b')); expect(exportProblems(d)).toEqual([]);
    d.scenes[1].image = null; expect(exportProblems(d).join(' ')).toContain('chưa có ảnh');
    d.outline.push({ id: 'missing', title: 'Missing', summary: '', targetSeconds: 120 });
    expect(exportProblems(d).join(' ')).toContain('Chưa viết 1 chương');
  });
  it('allows a two-minute video and still enforces a longer selected target', () => {
    const d = newDraft('project'); d.targetMinutes = 2; d.voice = 'Voice';
    d.outline = [{ id: 'chapter', title: 'Chapter', summary: '', targetSeconds: 120 }];
    const short = scene('short');
    short.audio = { ...audio, duration: 120, cues: [{ start: 0, end: 120, text: audio.narration }] };
    d.scenes = [short];
    expect(exportProblems(d)).toEqual([]);
    d.targetMinutes = 3;
    expect(exportProblems(d).join(' ')).toContain('02:00 / mục tiêu 03:00');
  });
  it('invalidates audio on text, language or voice changes and excludes stale duration', () => {
    const d = newDraft('project'); d.voice = 'Voice'; d.scenes = [scene('a'), scene('b')];
    expect(timeline(d)[1].start).toBe(600);
    d.scenes[0].narration = 'Đã sửa'; expect(audioCurrent(d.scenes[0], d)).toBe(false);
    expect(timeline(d).at(-1)?.end).toBe(600);
    d.language = 'en'; expect(timeline(d).at(-1)?.end).toBe(0);
    d.language = 'vi'; d.voice = 'Another'; expect(timeline(d).at(-1)?.end).toBe(0);
  });
  it('restores previous audio without losing the current version', () => {
    const s = scene('a'); const previous = { ...audio, path: 'studio/assets/old.wav' };
    s.audioHistory = [previous]; const restored = restoreAudio(s, previous);
    expect(restored.audio?.path).toBe(previous.path);
    expect(restored.audioHistory.map(a => a.path)).toEqual([audio.path]);
  });
  it('removes only the selected chapter and its scenes, recalculating actual duration', () => {
    const d = newDraft('project'); d.voice = 'Voice'; d.outlineApproved = d.sampleApproved = true;
    d.outline = ['chapter', 'other'].map(id => ({ id, title: id, summary: '', targetSeconds: 600 }));
    const kept = { ...scene('b'), chapterId: 'other' };
    d.scenes = [scene('a'), kept];
    expect(exportProblems(d)).toEqual([]);
    const result = removeChapter(d, 'chapter');
    expect(result.outline.map(c => c.id)).toEqual(['other']);
    expect(result.scenes).toEqual([kept]);
    expect(result.scenes[0]).toBe(kept);
    expect(result.outlineApproved).toBe(false);
    expect(result.sampleApproved).toBe(false);
    expect(timeline(result).at(-1)?.end).toBe(600);
    expect(exportProblems(result).join(' ')).toContain('10:00');
    expect(d.scenes).toHaveLength(2);
    expect(removeChapter(d, 'missing')).toBe(d);
    const empty = removeChapter(result, 'other');
    expect(empty.outline).toEqual([]); expect(empty.scenes).toEqual([]);
    expect(exportProblems(empty).join(' ')).toContain('Chưa có cảnh');
  });
});
