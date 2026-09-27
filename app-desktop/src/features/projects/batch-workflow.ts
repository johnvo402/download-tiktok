import { listen } from '@tauri-apps/api/event';
import { projectService } from '../../app/services/project-service';
import { translationService } from '../../app/services/translation-service';
import { ttsService } from '../../app/services/tts-service';
import { renderService, type ExportOptions } from '../../app/services/render-service';
import type { Project } from './types';
import type { TranscriptionEvent, TranslationJobEvent, TtsJobEvent } from '../transcript/types';

export interface BatchSteps {
  transcriptIfMissing: boolean;
  translateMissing: boolean;
  generateMissingVoices: boolean;
  export: boolean;
}

export interface BatchProjectCheck {
  project: Project;
  valid: boolean;
  reasons: string[];
}

export interface BatchProgress {
  projectId: string;
  projectName: string;
  projectIndex: number;
  projectCount: number;
  step: string;
  message: string;
  status: 'running' | 'completed' | 'failed' | 'cancelled';
}

type NativeProgress = TranscriptionEvent | TranslationJobEvent | TtsJobEvent | {
  projectId: string; jobId: string; status: string; progress?: number; message: string;
};

function isTranslationTerminal(status: string) { return ['completed', 'partial', 'failed', 'cancelled'].includes(status); }
function isAsrTerminal(status: string) { return ['completed', 'failed', 'canceled', 'cancelled'].includes(status); }
function isExportTerminal(status: string) { return ['completed', 'failed', 'cancelled'].includes(status); }

export async function validateBatchProjects(projectIds: string[], steps: BatchSteps): Promise<BatchProjectCheck[]> {
  let appSettings: Awaited<ReturnType<typeof translationService.getSettings>> | undefined;
  let asrEnvironment: Awaited<ReturnType<typeof projectService.getAsrEnvironment>> | undefined;
  let ttsEnvironment: Awaited<ReturnType<typeof ttsService.getEnvironment>> | undefined;
  try {
    if (steps.transcriptIfMissing || steps.translateMissing) appSettings = await translationService.getSettings();
  } catch { /* The per-project check below explains the unavailable settings. */ }
  try {
    if (steps.transcriptIfMissing && appSettings) asrEnvironment = await projectService.getAsrEnvironment(appSettings.settings.asr.model, appSettings.settings.asr.language);
  } catch { /* The per-project check below explains the unavailable worker. */ }
  try {
    if (steps.generateMissingVoices) ttsEnvironment = await ttsService.getEnvironment();
  } catch { /* The per-project check below explains the unavailable worker. */ }
  return Promise.all(projectIds.map(async (id) => {
    try {
      const project = await projectService.get(id);
      const reasons: string[] = [];
      if (!project.source.videoPath) reasons.push('Chưa có video nguồn.');
      const transcript = await projectService.getTranscript(id);
      if (!transcript && !steps.transcriptIfMissing && (steps.translateMissing || steps.generateMissingVoices || steps.export)) {
        reasons.push('Chưa có transcript và workflow không tạo transcript.');
      }
      if (!transcript && steps.transcriptIfMissing && !project.source.audioStreams?.length
        && !project.source.subtitleTracks?.some((track) => track.textSupported)) {
        reasons.push('Video không có audio hoặc subtitle text để tạo transcript.');
      }
      if (!transcript && steps.transcriptIfMissing
        && !((project.source.subtitleTracks ?? []).filter((track) => track.textSupported).length === 1)
        && project.source.audioStreams?.length) {
        if (!asrEnvironment?.workerInstalled) reasons.push('AI Worker chưa sẵn sàng; hãy sửa chữa hoặc cài lại ứng dụng.');
        else if (!asrEnvironment.modelCached) reasons.push(`Model faster-whisper ${appSettings?.settings.asr.model ?? ''} chưa được tải.`);
        else if (appSettings?.settings.asr.engine === 'whisperX' && appSettings.settings.asr.language === 'auto') reasons.push('WhisperX cần ngôn ngữ nguồn cụ thể để chọn aligner.');
        else if (appSettings?.settings.asr.engine === 'whisperX' && !asrEnvironment.whisperXInstalled) reasons.push('Chưa cài WhisperX.');
        else if (appSettings?.settings.asr.engine === 'whisperX' && !asrEnvironment.alignmentModelCached) reasons.push(`WhisperX chưa cài aligner ${appSettings.settings.asr.language}.`);
      }
      const needsTranslation = transcript
        ? transcript.segments.some((segment) => !segment.translated.trim())
        : steps.transcriptIfMissing;
      if (steps.translateMissing && needsTranslation && appSettings) {
        if (appSettings.settings.translation.provider === 'gemini' && !appSettings.geminiCredentialConfigured) reasons.push('Chưa cấu hình Gemini API key.');
        if (appSettings.settings.translation.provider === 'googleCloud' && !appSettings.googleCloudCredentialConfigured) reasons.push('Chưa cấu hình Google Cloud Translation credential.');
      } else if (steps.translateMissing && needsTranslation && !appSettings) {
        reasons.push('Không thể đọc cấu hình Translation.');
      }
      const needsVoice = transcript
        ? transcript.segments.some((segment) => segment.translated.trim() && (!segment.tts?.audioPath || segment.tts.status !== 'ready'))
        : steps.transcriptIfMissing && steps.translateMissing;
      if (steps.generateMissingVoices && needsVoice) {
        if (!ttsEnvironment?.workerInstalled) reasons.push('VieNeu-TTS SDK chưa sẵn sàng; mở Cài đặt → Voice / TTS để xem cách khắc phục.');
        else if (!ttsEnvironment.modelCached) reasons.push('Model VieNeu-TTS chưa được tải.');
      }
      if (project.subtitle.enabled && steps.export && !project.blurRegions.some((region) => region.role === 'subtitle')) {
        reasons.push('Export subtitle cần một vùng blur có vai trò Subtitle Blur.');
      }
      if (project.dubbing.enabled && steps.export && !steps.generateMissingVoices && transcript) {
        const incomplete = transcript.segments.some((segment) => segment.translated.trim()
          && (!segment.tts?.audioPath || segment.tts.status !== 'ready'));
        if (incomplete) reasons.push('Đang bật lồng tiếng nhưng còn thiếu audio TTS.');
      }
      if (steps.export && !project.source.videoPath) reasons.push('Không thể export khi chưa có video nguồn.');
      return { project, valid: reasons.length === 0, reasons };
    } catch (error) {
      return { project: { id, name: 'Không thể đọc project' } as Project, valid: false, reasons: [error instanceof Error ? error.message : String(error)] };
    }
  }));
}

export class BatchWorkflowRunner {
  private cancelled = false;
  private cancelCurrent?: () => Promise<unknown>;
  private rejectWait?: (error: Error) => void;

  async cancel() {
    this.cancelled = true;
    this.rejectWait?.(new Error('Đã hủy batch workflow.'));
    try { await this.cancelCurrent?.(); } catch { /* Native job will report cancellation in the job queue. */ }
  }

  async run(
    projects: Project[],
    steps: BatchSteps,
    onProgress: (progress: BatchProgress) => void,
  ): Promise<BatchProgress[]> {
    this.cancelled = false;
    const outcomes: BatchProgress[] = [];
    for (let index = 0; index < projects.length; index += 1) {
      if (this.cancelled) break;
      const seed = projects[index];
      const send = (step: string, message: string, status: BatchProgress['status'] = 'running') => {
        const value = { projectId: seed.id, projectName: seed.name, projectIndex: index + 1, projectCount: projects.length, step, message, status };
        onProgress(value);
        return value;
      };
      try {
        const project = await projectService.get(seed.id);
        let transcript = await projectService.getTranscript(project.id);

        if (!transcript && steps.transcriptIfMissing) {
          const textTracks = (project.source.subtitleTracks ?? []).filter((track) => track.textSupported);
          const transcriptSettings = project.transcriptSettings ?? { mode: 'speechRecognition' as const, ocrFps: 2 as const, ocrRegion: null };
          if (transcriptSettings.mode !== 'ocr' && textTracks.length === 1) {
            send('transcript', 'Đang trích subtitle text…');
            await projectService.extractSubtitle(project.id, textTracks[0].streamIndex);
            transcript = await projectService.getTranscript(project.id);
          } else {
            const settings = await translationService.getSettings();
            if (transcriptSettings.mode === 'ocr' && !transcriptSettings.ocrRegion) throw new Error('Hãy chọn vùng subtitle trong khung video trước khi chạy batch OCR.');
            if (transcriptSettings.mode !== 'ocr' && !project.source.audioStreams?.length) throw new Error(textTracks.length > 1
              ? 'Có nhiều subtitle track; hãy chọn một track trong project trước khi chạy batch.'
              : 'Video không có audio để tạo transcript.');
            send('transcript', 'Đang tạo transcript bằng Speech Recognition…');
            await this.waitForJob<TranscriptionEvent>(
              'transcription-progress',
              isAsrTerminal,
              () => projectService.generateTranscript(project.id, {
                engine: settings.settings.asr.engine,
                model: settings.settings.asr.model,
                device: settings.settings.asr.device,
                language: settings.settings.asr.language === 'auto' ? 'auto' : settings.settings.asr.language,
                mode: transcriptSettings.mode,
                ocrFps: transcriptSettings.ocrFps,
                ocrRegion: transcriptSettings.ocrRegion,
                ocrModel: settings.settings.asr.ocrModel,
                ocrDevice: settings.settings.asr.ocrDevice,
              }),
              (jobId) => projectService.cancelTranscription(jobId),
            );
            transcript = await projectService.getTranscript(project.id);
          }
          if (!transcript) throw new Error('Tác vụ transcript hoàn tất nhưng không tìm thấy dữ liệu đã lưu.');
        }

        if ((steps.translateMissing || steps.generateMissingVoices || steps.export) && !transcript) {
          throw new Error('Project chưa có transcript.');
        }
        if (transcript && steps.translateMissing && transcript.segments.some((segment) => !segment.translated.trim())) {
          send('translate', 'Đang dịch các segment còn thiếu…');
          const result = await this.waitForJob<TranslationJobEvent>(
            'translation-progress',
            isTranslationTerminal,
            () => translationService.start(project.id, { scope: 'missing', segmentIds: [], overwriteEdited: false }),
            (jobId) => translationService.cancel(jobId),
          );
          if (result.status !== 'completed') throw new Error(result.message || 'Translation chưa hoàn tất toàn bộ segment.');
          transcript = await projectService.getTranscript(project.id);
        }

        if (transcript && steps.generateMissingVoices && transcript.segments.some((segment) => segment.translated.trim()
          && (!segment.tts?.audioPath || segment.tts.status !== 'ready'))) {
          send('tts', 'Đang tạo audio cho các segment còn thiếu…');
          const result = await this.waitForJob<TtsJobEvent>(
            'tts-progress',
            isTranslationTerminal,
            () => ttsService.start(project.id, { scope: 'missing', segmentIds: [] }),
            (jobId) => ttsService.cancel(jobId),
          );
          if (result.status !== 'completed') throw new Error(result.message || 'Tạo giọng đọc chưa hoàn tất.');
          transcript = await projectService.getTranscript(project.id);
        }

        if (steps.export) {
          const latest = await projectService.get(project.id);
          if (!transcript) transcript = await projectService.getTranscript(project.id);
          const options: ExportOptions = {
            quality: 'balanced', applyBlur: true, applyLogos: true,
            burnSubtitles: latest.subtitle.enabled, enableSubtitleBlurOverride: false,
            includeOriginalAudio: !latest.dubbing.muteOriginal,
            includeDubbing: latest.dubbing.enabled, ducking: latest.dubbing.duckingEnabled,
            outputDirectory: null, outputFileName: `${new Date().toISOString().slice(0, 19).replace(/[:T]/g, '-')}`,
            originalVolume: latest.dubbing.originalVolume,
            dubbedVolume: latest.dubbing.dubbedVolume, duckingAmount: latest.dubbing.duckingAmount,
            dialogueReduction: latest.dubbing.dialogueReductionEnabled,
            dialogueReductionAmount: latest.dubbing.dialogueReductionAmount,
          };
          const validation = await renderService.validate(project.id, options);
          if (!validation.valid || validation.warnings.length) {
            throw new Error(validation.warnings[0]?.message ?? 'Thiết lập export chưa sẵn sàng.');
          }
          send('export', 'Đang export video…');
          const result = await this.waitForJob('export-progress', isExportTerminal,
            () => renderService.start(project.id, options), (jobId) => renderService.cancel(jobId));
          if (result.status !== 'completed') throw new Error(result.message || 'Export thất bại.');
        }

        const done = send('complete', 'Workflow đã hoàn tất cho project.', 'completed');
        outcomes.push(done);
      } catch (error) {
        const cancelled = this.cancelled;
        const failed = send(cancelled ? 'cancelled' : 'failed', error instanceof Error ? error.message : String(error), cancelled ? 'cancelled' : 'failed');
        outcomes.push(failed);
        if (cancelled) break;
        // A failed project is isolated; the remaining selected projects keep running.
      }
    }
    return outcomes;
  }

  private async waitForJob<T extends NativeProgress>(
    eventName: string,
    isTerminal: (status: string) => boolean,
    start: () => Promise<string>,
    cancel: (jobId: string) => Promise<unknown>,
  ): Promise<T> {
    let jobId: string | undefined;
    const buffered: T[] = [];
    let resolveResult!: (event: T) => void;
    let rejectResult!: (error: Error) => void;
    const completion = new Promise<T>((resolve, reject) => { resolveResult = resolve; rejectResult = reject; });
    this.rejectWait = rejectResult;
    const unlisten = await listen<T>(eventName, (event) => {
      const payload = event.payload;
      if (!jobId) buffered.push(payload);
      else if (payload.jobId === jobId && isTerminal(payload.status)) resolveResult(payload);
    });
    try {
      jobId = await start();
      this.cancelCurrent = () => cancel(jobId!);
      const terminal = buffered.find((event) => event.jobId === jobId && isTerminal(event.status));
      if (terminal) resolveResult(terminal);
      const result = await completion;
      if (result.status !== 'completed') throw new Error(result.message || `Job ${result.status}.`);
      return result;
    } finally {
      unlisten();
      this.rejectWait = undefined;
      this.cancelCurrent = undefined;
    }
  }
}
