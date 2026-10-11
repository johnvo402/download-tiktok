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
  | { kind: 'sourceAudio'; assetId: string; videoContentId: string }
  | { kind: 'text'; text: string }
  | { kind: 'subtitles' | 'voice'; videoContentId: string };
export interface TimelineClip {
  id: string; trackId: string; startUs: TimeUs;
  /** Source interval [sourceInUs, sourceOutUs), independent of timeline placement. */
  sourceInUs: TimeUs; sourceOutUs: TimeUs; rate: Rational;
  content: ClipContent; transform: ClipTransform;
  linkedClipId: string | null; groupId: string | null;
  /** Per-instance typography, independent of translated source content. */
  style?: ClipStyle;
  audio?: ClipAudio;
  effects?: VisualEffects;
  transitionIn?: ClipTransition;
}
export interface MotionValues { x: number; y: number; scaleX: number; scaleY: number; opacity: number }
/** Absolute source timestamps keep animation stable through trim, split and fixed retime. */
export interface VisualKeyframe extends MotionValues { sourceUs: TimeUs }
export interface VisualEffects { fadeInUs: TimeUs; fadeOutUs: TimeUs; keyframes: VisualKeyframe[] }
export interface ClipTransition { kind: 'dissolve' | 'wipeLeft'; fromClipId: string; durationUs: TimeUs }
export interface ClipAudio {
  volume: number; muted: boolean; fadeInUs: number; fadeOutUs: number;
  voiceEnabled: boolean; voiceSpeed: number;
  duckingEnabled: boolean; duckingAmount: number; duckAttackUs: number; duckReleaseUs: number;
}
export interface ClipStyle {
  fontFamily: string; fontSize: number; fontWeight: number;
  color: string; backgroundColor: string; backgroundOpacity: number;
  alignment: 'left' | 'center' | 'right';
}
export interface TimelineSettings {
  width: number; height: number; frameRate: Rational; backgroundColor: string;
}
export interface TimelineDocument {
  schemaVersion: 1;
  settings: TimelineSettings;
  assets: MediaAsset[]; videoContents: VideoContent[]; tracks: TimelineTrack[]; clips: TimelineClip[];
  /** Default for commands without a content ID; preserves legacy source editor/export. */
  legacyBinding: { assetId: string; videoContentId: string; clipId: string | null } | null;
}
