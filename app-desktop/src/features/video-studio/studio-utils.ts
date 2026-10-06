import type { SceneAudio, StudioDraft, VideoScene } from './types';

export function newDraft(projectId: string): StudioDraft {
  return { schemaVersion: 1, projectId, screen: 'ideas', topic: '', language: 'vi', targetMinutes: 20, aspectRatio: '16:9', contentMode: 'factual', ideas: [], selectedIdeaId: null, title: '', description: '', stylePrompt: 'Cinematic documentary, realistic lighting, consistent visual style, no text or logos.', voice: '', outline: [], outlineApproved: false, scenes: [], sampleApproved: false, motion: true, subtitle: { enabled: true, y: .86, fontSize: 42, color: '#ffffff', background: true }, overlays: [], updatedAt: new Date().toISOString() };
}
export const audioCurrent = (scene: VideoScene, draft: StudioDraft) => !!scene.audio && scene.audio.narration === scene.narration && scene.audio.language === draft.language && scene.audio.voice === draft.voice;
export function removeChapter(draft: StudioDraft, chapterId: string): StudioDraft {
  if (!draft.outline.some(chapter => chapter.id === chapterId)) return draft;
  return {
    ...draft,
    outline: draft.outline.filter(chapter => chapter.id !== chapterId),
    scenes: draft.scenes.filter(scene => scene.chapterId !== chapterId),
    outlineApproved: false,
    sampleApproved: false,
  };
}
export function timeline(draft: StudioDraft) {
  let time = 0;
  return draft.scenes.map(scene => {
    const duration = audioCurrent(scene, draft) ? scene.audio!.duration : 0;
    const entry = { scene, start: time, end: time + duration, duration };
    time += duration;
    return entry;
  });
}
export function exportProblems(draft: StudioDraft): string[] {
  const problems: string[] = [];
  if (!draft.scenes.length) problems.push('Chưa có cảnh trong kịch bản.');
  const incomplete = draft.outline.filter(chapter => !draft.scenes.some(scene => scene.chapterId === chapter.id));
  if (incomplete.length) problems.push(`Chưa viết ${incomplete.length} chương.`);
  const noImage = draft.scenes.filter(scene => !scene.image).length;
  const noAudio = draft.scenes.filter(scene => !audioCurrent(scene, draft)).length;
  if (noImage) problems.push(`${noImage} cảnh chưa có ảnh.`);
  if (noAudio) problems.push(`${noAudio} cảnh cần tạo hoặc cập nhật giọng đọc.`);
  const duration = timeline(draft).at(-1)?.end ?? 0;
  if (duration < draft.targetMinutes * 60) problems.push(`Lời kể thực tế ${formatTime(duration)} / mục tiêu ${formatTime(draft.targetMinutes * 60)}. Bổ sung nội dung hoặc chương rồi tạo giọng đọc.`);
  return problems;
}
export function formatTime(seconds: number): string {
  const n = Math.max(0, Math.floor(seconds));
  return `${Math.floor(n / 60).toString().padStart(2, '0')}:${(n % 60).toString().padStart(2, '0')}`;
}
export function scenePrompt(draft: StudioDraft, scene: VideoScene): string {
  return `Create one image for a ${draft.aspectRatio} narrated video.\nGlobal visual style: ${draft.stylePrompt}\nScene: ${scene.imagePrompt}\nMaintain continuity of characters, clothing, period and palette. No captions, text, watermark or logos. Return the generated image.`;
}
export function restoreAudio(scene: VideoScene, audio: SceneAudio): VideoScene {
  return { ...scene, audio, audioHistory: [scene.audio, ...scene.audioHistory].filter((a): a is SceneAudio => !!a && a.path !== audio.path).slice(0, 5) };
}
