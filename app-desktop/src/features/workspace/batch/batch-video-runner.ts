import { listen } from '@tauri-apps/api/event';
import { setCurrentProject, setCurrentTranscript } from '../../../app/project-state';
import { projectService } from '../../../app/services/project-service';
import { translationService } from '../../../app/services/translation-service';
import { ttsService } from '../../../app/services/tts-service';
import { renderService, type ExportOptions } from '../../../app/services/render-service';
import { batchVideoService } from '../../../app/services/batch-video-service';
import type { Transcript, TranscriptionEvent, TranslationJobEvent, TtsJobEvent } from '../../transcript/types';
import type { Project } from '../../projects/types';
import type { BatchVideoItem, BatchVideoQueue, BatchVideoRunUpdate } from './types';
import { orderBatchVideos, videoOutputName } from './types';

type ExportEvent = {
  projectId: string;
  jobId: string;
  status: string;
  progress: number;
  output: string | null;
  message: string;
};
type NativeProgress = TranscriptionEvent | TranslationJobEvent | TtsJobEvent | ExportEvent;

const isAsrTerminal = (status: string) => ['completed', 'failed', 'canceled', 'cancelled'].includes(status);
const isTranslationTerminal = (status: string) => ['completed', 'partial', 'failed', 'cancelled'].includes(status);
const isExportTerminal = (status: string) => ['completed', 'failed', 'cancelled'].includes(status);

export class BatchVideoRunner {
  private cancelled = false;
  private cancelCurrent?: () => Promise<unknown>;
  private rejectWait?: (error: Error) => void;
  private cancelFallback?: ReturnType<typeof setTimeout>;
  private queue?: BatchVideoQueue;
  private onQueue?: (queue: BatchVideoQueue) => void;
  private saveChain: Promise<unknown> = Promise.resolve();

  async cancel() {
    this.cancelled = true;
    if (!this.cancelCurrent) return;
    try {
      const accepted = await this.cancelCurrent();
      if (accepted === false) this.rejectWait?.(new Error('Job hiện tại đã kết thúc trước khi nhận yêu cầu hủy.'));
      else this.armCancelFallback();
    } catch (error) {
      this.rejectWait?.(error instanceof Error ? error : new Error('Không thể gửi yêu cầu hủy job hiện tại.'));
    }
  }

  async run(
    initialQueue: BatchVideoQueue,
    updateQueue: (queue: BatchVideoQueue) => void,
    updateProgress: (update: BatchVideoRunUpdate) => void,
  ): Promise<void> {
    if (!initialQueue.sampleLoaded) throw new Error('Hãy nạp video mẫu và cấu hình project trước khi chạy hàng đợi.');
    if (initialQueue.items.length < 2) throw new Error('Hãy thêm ít nhất hai video để chạy chế độ hàng loạt.');
    if (initialQueue.items.some((item) => item.status === 'running')) throw new Error('Hàng đợi đang có một video chạy dở. Hãy tải lại danh sách trước khi tiếp tục.');
    this.cancelled = false;
    this.queue = structuredClone(initialQueue);
    this.onQueue = updateQueue;
    const pending = orderBatchVideos(this.queue).filter((item) => item.status !== 'completed');

    for (const queued of pending) {
      if (this.cancelled) break;
      const item = queued;
      try {
        if (this.cancelled) break;
        await this.setItem(item.id, { status: 'running', stage: 'import', progress: 0, message: 'Đang xác minh và nhập video vào project.' }, updateProgress, true);
        const prior = await projectService.get(this.queue.projectId);
        // Avoid hashing the sample file twice when setup already imported the same checksum.
        // If the active source changed since setup, restore the sample from its selected path.
        const sampleAlreadyLoaded = item.id === this.queue.sampleId
          && this.queue.sampleLoaded
          && Boolean(this.queue.sampleChecksum)
          && prior.source.checksumSha256 === this.queue.sampleChecksum;
        const importResult = sampleAlreadyLoaded
          ? { project: prior, duplicate: true, replaced: false }
          : item.id === this.queue.sampleId
            ? await projectService.importVideo(this.queue.projectId, item.sourcePath, true)
            : await projectService.importVideo(this.queue.projectId, item.sourcePath, Boolean(prior.source.videoPath));
        setCurrentProject(importResult.project);
        if (importResult.replaced || importResult.project.source.checksumSha256 !== prior.source.checksumSha256) {
          setCurrentTranscript(undefined);
        }

        let project = importResult.project;
        if (this.cancelled) throw new Error('Đã hủy hàng đợi xử lý video.');
        let transcript = await projectService.getTranscript(project.id);
        if (!transcript) {
          transcript = await this.createTranscript(project, item, updateProgress);
          project = await projectService.get(project.id);
          setCurrentProject(project);
          setCurrentTranscript(transcript);
        }

        if (transcript.segments.some((segment) => !segment.translated.trim())) {
          if (this.cancelled) throw new Error('Đã hủy hàng đợi xử lý video.');
          await this.waitForJob<TranslationJobEvent>(
            project.id,
            'translation-progress',
            isTranslationTerminal,
            (event) => updateProgress({ itemId: item.id, status: 'running', stage: 'translate', progress: 0.25 + nativeProgress(event) * 0.25, message: event.message }),
            () => translationService.start(project.id, { scope: 'missing', segmentIds: [], overwriteEdited: false }),
            (jobId) => translationService.cancel(jobId),
          );
          transcript = await projectService.getTranscript(project.id);
          if (!transcript) throw new Error('Không tìm thấy transcript sau khi dịch.');
          setCurrentTranscript(transcript);
        }

        project = await projectService.get(project.id);
        const needsVoice = project.dubbing.enabled && transcript.segments.some((segment) => segment.translated.trim()
          && (!segment.tts?.audioPath || segment.tts.status !== 'ready'));
        if (needsVoice) {
          if (this.cancelled) throw new Error('Đã hủy hàng đợi xử lý video.');
          await this.waitForJob<TtsJobEvent>(
            project.id,
            'tts-progress',
            isTranslationTerminal,
            (event) => updateProgress({ itemId: item.id, status: 'running', stage: 'tts', progress: 0.5 + nativeProgress(event) * 0.2, message: event.message }),
            () => ttsService.start(project.id, { scope: 'missing', segmentIds: [] }),
            (jobId) => ttsService.cancel(jobId),
          );
          transcript = await projectService.getTranscript(project.id);
          if (!transcript) throw new Error('Không tìm thấy transcript sau khi tạo giọng.');
          setCurrentTranscript(transcript);
        }

        project = await projectService.get(project.id);
        const options: ExportOptions = {
          quality: 'balanced',
          videoCodec: 'h264',
          encoder: 'auto',
          applyBlur: true,
          applyLogos: true,
          burnSubtitles: project.subtitle.enabled,
          enableSubtitleBlurOverride: false,
          includeOriginalAudio: !project.dubbing.muteOriginal && (project.source.audioStreams?.length ?? 0) > 0,
          includeDubbing: project.dubbing.enabled,
          ducking: project.dubbing.duckingEnabled,
          outputDirectory: this.queue.outputDirectory,
          outputFileName: videoOutputName(item.fileName),
          originalVolume: project.dubbing.originalVolume,
          dubbedVolume: project.dubbing.dubbedVolume,
          duckingAmount: project.dubbing.duckingAmount,
          dialogueReduction: project.dubbing.dialogueReductionEnabled,
          dialogueReductionAmount: project.dubbing.dialogueReductionAmount,
        };
        const validation = await renderService.validate(project.id, options);
        if (!validation.valid) throw new Error('Thiết lập export chưa sẵn sàng.');
        if (validation.warnings.length) {
          const first = validation.warnings[0];
          throw new Error(`${first.message} Segment ${first.segmentId} (${first.start.toFixed(2)}s).`);
        }
        if (this.cancelled) throw new Error('Đã hủy hàng đợi xử lý video.');
        const exportEvent = await this.waitForJob<ExportEvent>(
          project.id,
          'export-progress',
          isExportTerminal,
          (event) => updateProgress({ itemId: item.id, status: 'running', stage: 'export', progress: 0.7 + nativeProgress(event) * 0.3, message: event.message }),
          () => renderService.start(project.id, options),
          (jobId) => renderService.cancel(jobId),
        );
        await this.setItem(item.id, {
          status: 'completed', stage: 'complete', progress: 1,
          message: 'Đã dịch và export xong.', outputPath: exportEvent.output,
        }, updateProgress, true);
      } catch (error) {
        const cancelled = this.cancelled;
        await this.setItem(item.id, {
          status: cancelled ? 'cancelled' : 'failed',
          stage: cancelled ? 'cancelled' : 'failed',
          progress: 0,
          message: error instanceof Error ? error.message : String(error),
        }, updateProgress, true);
        if (cancelled) break;
      }
    }
    await this.flushSave();
  }

  private async createTranscript(project: Project, item: BatchVideoItem, update: (value: BatchVideoRunUpdate) => void): Promise<Transcript> {
    const transcriptSettings = project.transcriptSettings ?? { mode: 'speechRecognition' as const, ocrFps: 2 as const, ocrRegion: null };
    const textTracks = (project.source.subtitleTracks ?? []).filter((track) => track.textSupported);
    if (transcriptSettings.mode !== 'ocr' && textTracks.length === 1) {
      update({ itemId: item.id, status: 'running', stage: 'transcript', progress: 0.05, message: 'Đang trích subtitle text.' });
      const transcript = await projectService.extractSubtitle(project.id, textTracks[0].streamIndex);
      if (!transcript.segments.length) throw new Error('Subtitle track không có segment văn bản.');
      return transcript;
    }
    if (transcriptSettings.mode === 'ocr' && !transcriptSettings.ocrRegion) {
      throw new Error('Project đang dùng OCR nhưng video mẫu chưa có vùng OCR. Hãy chọn vùng trong preview trước khi chạy batch.');
    }
    if (transcriptSettings.mode !== 'ocr' && !project.source.audioStreams?.length) {
      throw new Error(textTracks.length > 1
        ? 'Video có nhiều subtitle track. Hãy trích track mong muốn ở video mẫu trước khi chạy batch.'
        : 'Video không có audio hoặc subtitle text để tạo transcript.');
    }
    const settings = await translationService.getSettings();
    update({ itemId: item.id, status: 'running', stage: 'transcript', progress: 0.05, message: transcriptSettings.mode === 'ocr' ? 'Đang nhận dạng subtitle trên hình.' : 'Đang tạo transcript bằng Speech Recognition.' });
    const event = await this.waitForJob<TranscriptionEvent>(
      project.id,
      'transcription-progress',
      isAsrTerminal,
      (progress) => update({ itemId: item.id, status: 'running', stage: 'transcript', progress: 0.05 + nativeProgress(progress) * 0.2, message: progress.message }),
      () => projectService.generateTranscript(project.id, {
        engine: settings.settings.asr.engine,
        model: settings.settings.asr.model,
        device: settings.settings.asr.device,
        language: settings.settings.asr.language,
        mode: transcriptSettings.mode,
        ocrFps: transcriptSettings.ocrFps,
        ocrRegion: transcriptSettings.ocrRegion,
        ocrModel: settings.settings.asr.ocrModel,
        ocrDevice: settings.settings.asr.ocrDevice,
      }),
      (jobId) => projectService.cancelTranscription(jobId),
    );
    if (event.status !== 'completed') throw new Error(event.message || 'Không thể tạo transcript cho video.');
    const transcript = await projectService.getTranscript(project.id);
    if (!transcript?.segments.length) throw new Error('Transcript không có segment nào.');
    return transcript;
  }

  private async waitForJob<T extends NativeProgress>(
    projectId: string,
    eventName: string,
    isTerminal: (status: string) => boolean,
    onProgress: (event: T) => void,
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
      if (payload.projectId !== projectId) return;
      if (!jobId) buffered.push(payload);
      else if (payload.jobId === jobId) {
        onProgress(payload);
        if (isTerminal(payload.status)) resolveResult(payload);
      }
    });
    try {
      jobId = await start();
      this.cancelCurrent = () => cancel(jobId!);
      if (this.cancelled) {
        const accepted = await cancel(jobId);
        if (accepted === false) rejectResult(new Error('Job hiện tại đã kết thúc trước khi nhận yêu cầu hủy.'));
        else this.armCancelFallback();
      }
      for (const event of buffered) {
        if (event.jobId !== jobId) continue;
        onProgress(event);
        if (isTerminal(event.status)) resolveResult(event);
      }
      const result = await completion;
      if (result.status !== 'completed') throw new Error(result.message || `Tác vụ kết thúc ở trạng thái ${result.status}.`);
      return result;
    } finally {
      unlisten();
      if (this.cancelFallback) clearTimeout(this.cancelFallback);
      this.cancelFallback = undefined;
      this.rejectWait = undefined;
      this.cancelCurrent = undefined;
    }
  }

  private async setItem(
    itemId: string,
    patch: Partial<BatchVideoItem>,
    update: (value: BatchVideoRunUpdate) => void,
    persist: boolean,
  ) {
    if (!this.queue) return;
    this.queue = {
      ...this.queue,
      items: this.queue.items.map((item) => item.id === itemId ? { ...item, ...patch } : item),
    };
    this.onQueue?.(structuredClone(this.queue));
    const value = this.queue.items.find((item) => item.id === itemId)!;
    update({ itemId, status: value.status, stage: value.stage, progress: value.progress, message: value.message, outputPath: value.outputPath });
    if (persist) {
      const snapshot = structuredClone(this.queue);
      const save = this.saveChain.catch(() => undefined).then(() => batchVideoService.save(snapshot.projectId, snapshot));
      this.saveChain = save;
      const saved = await save;
      this.queue = saved;
      this.onQueue?.(structuredClone(saved));
    }
  }

  private async flushSave() {
    await this.saveChain;
  }

  private armCancelFallback() {
    if (this.cancelFallback) clearTimeout(this.cancelFallback);
    this.cancelFallback = setTimeout(() => {
      this.rejectWait?.(new Error('Không nhận được xác nhận dừng từ job sau 30 giây. Hãy kiểm tra mục Job và thử lại nếu cần.'));
    }, 30_000);
  }
}

function nativeProgress(event: NativeProgress): number {
  if ('currentBatch' in event) return event.totalBatches > 0 ? event.completedBatches / event.totalBatches : 0;
  if ('completedSegments' in event) return event.totalSegments > 0 ? event.completedSegments / event.totalSegments : event.progress;
  return Math.max(0, Math.min(1, event.progress ?? 0));
}
