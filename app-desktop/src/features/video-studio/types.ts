export type ContentLanguage = 'vi' | 'en';
export interface VideoIdea { id: string; title: string; description: string; hook: string; recommended: boolean; reason: string }
export interface OutlineChapter { id: string; title: string; summary: string; targetSeconds: number }
export interface SubtitleCue { start: number; end: number; text: string }
export interface SceneAudio { path: string; duration: number; narration: string; language: ContentLanguage; voice: string; cues: SubtitleCue[] }
export interface VideoScene {
  id: string; chapterId: string; title: string; narration: string; imagePrompt: string;
  image: string | null; imageHistory: string[]; audio: SceneAudio | null; audioHistory: SceneAudio[];
}
export interface StudioOverlay { id: string; kind: 'text' | 'logo'; text: string; path: string | null; x: number; y: number; width: number; fontSize: number; color: string; opacity: number; start: number; end: number }
export interface StudioDraft {
  schemaVersion: 1; projectId: string; screen: 'ideas' | 'editor'; topic: string; language: ContentLanguage;
  targetMinutes: number; aspectRatio: '16:9' | '9:16'; contentMode: 'factual' | 'fiction';
  ideas: VideoIdea[]; selectedIdeaId: string | null; title: string; description: string; stylePrompt: string;
  voice: string; outline: OutlineChapter[]; outlineApproved: boolean; scenes: VideoScene[];
  sampleApproved: boolean; motion: boolean; subtitle: { enabled: boolean; y: number; fontSize: number; color: string; background: boolean };
  overlays: StudioOverlay[]; updatedAt: string;
}
export interface StudioProgress { projectId: string; jobId: string; status: string; message: string; progress: number }
export type AiAction = 'ideas' | 'outline' | 'chapter' | 'rewrite';
