import { projectService } from '../../app/services/project-service';
import { translationService } from '../../app/services/translation-service';
import { ttsService } from '../../app/services/tts-service';
import { projectForVideo } from './video-context';
import type { Project } from '../projects/types';

/** This ID is captured when an editor mounts, never looked up after an await. */
export function videoServices(id?: string) {
  const view = (project: Project) => projectForVideo(project, id);
  return {
    project: {
      ...projectService,
      get: (projectId: string) => projectService.get(projectId).then(view),
      getTranscript: (projectId: string) => projectService.getTranscript(projectId, id),
      saveTranscript: (projectId: string, transcript: Parameters<typeof projectService.saveTranscript>[1]) => projectService.saveTranscript(projectId, transcript, id).then(view),
      extractSubtitle: (projectId: string, trackIndex: number) => projectService.extractSubtitle(projectId, trackIndex, id),
      updateTranslationLanguages: (projectId: string, source: string, target: string) => projectService.updateTranslationLanguages(projectId, source, target, id).then(view),
      updateDubbingSettings: (projectId: string, dubbing: Project['dubbing']) => projectService.updateDubbingSettings(projectId, dubbing, id).then(view),
      updateTranscriptSettings: (projectId: string, settings: Project['transcriptSettings']) => projectService.updateTranscriptSettings(projectId, settings, id).then(view),
      generateTranscript: (projectId: string, options: Parameters<typeof projectService.generateTranscript>[1]) => projectService.generateTranscript(projectId, { ...options, videoContentId: id }),
    },
    translation: { ...translationService, start: (projectId: string, request: Parameters<typeof translationService.start>[1]) => translationService.start(projectId, { ...request, videoContentId: id }) },
    tts: {
      ...ttsService,
      start: (projectId: string, request: Parameters<typeof ttsService.start>[1]) => ttsService.start(projectId, { ...request, videoContentId: id }),
      getAudioPath: (projectId: string, segmentId: string) => ttsService.getAudioPath(projectId, segmentId, id),
    },
  };
}
