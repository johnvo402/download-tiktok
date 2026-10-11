import type { Project } from '../projects/types';

/** Integer microseconds, restricted to JavaScript's safe integer range. */
export type TimeUs = number;
export interface Rational { numerator: number; denominator: number }
export interface MediaAsset {
  id: string;
  kind: 'video' | 'image' | 'audio';
  relativePath: string;
  durationUs: TimeUs | null;
  frameRate: Rational | null;
  /** null means unknown; decoding uses source timestamps, never inferred frame numbers. */
  variableFrameRate: boolean | null;
  metadata: Project['source'];
}
export interface VideoSettings {
  transcript: Project['transcript'];
  translation: Project['translation'];
  subtitle: Project['subtitle'];
  dubbing: Project['dubbing'];
  transcriptSettings: Project['transcriptSettings'];
  blurRegions: Project['blurRegions'];
  logos: Project['logos'];
  textOverlays: Project['textOverlays'];
}
export interface VideoContent { id: string; assetId: string; revision: number; settings: VideoSettings }
export interface TimelineTrack {
  id: string; kind: 'visual' | 'audio' | 'subtitle'; name: string;
  /** Ascending order composites from back to front. */
  order: number; locked: boolean; enabled: boolean; muted: boolean; solo: boolean;
}
export interface ClipTransform {
  x: number; y: number; scaleX: number; scaleY: number; rotation: number;
  opacity: number; flipX: boolean; flipY: boolean; fit: 'contain' | 'cover' | 'stretch';
  crop: { x: number; y: number; width: number; height: number };
}
export type ClipContent =
  | { kind: 'video'; assetId: string; videoContentId: string }
  | { kind: 'image' | 'audio'; assetId: string }
  | { kind: 'text'; text: string }
  | { kind: 'subtitles' | 'voice'; videoContentId: string };
export interface TimelineClip {
  id: string; trackId: string; startUs: TimeUs;
  /** Source interval [sourceInUs, sourceOutUs), independent of timeline placement. */
  sourceInUs: TimeUs; sourceOutUs: TimeUs; rate: Rational;
  content: ClipContent; transform: ClipTransform;
  linkedClipId: string | null; groupId: string | null;
}
export interface TimelineSettings {
  width: number; height: number; frameRate: Rational; backgroundColor: string;
}
export interface TimelineDocument {
  schemaVersion: 1;
  settings: TimelineSettings;
  assets: MediaAsset[]; videoContents: VideoContent[]; tracks: TimelineTrack[]; clips: TimelineClip[];
  /** Default for commands without a content ID; legacy preview/export until T08–T09. */
  legacyBinding: { assetId: string; videoContentId: string; clipId: string | null } | null;
}
