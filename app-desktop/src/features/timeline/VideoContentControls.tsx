import { createMemo, createSignal, For, Show } from 'solid-js';
import { currentProject, setNotice } from '../../app/project-state';
import { projectService } from '../../app/services/project-service';
import { translationService } from '../../app/services/translation-service';
import { ttsService } from '../../app/services/tts-service';
import { flushPendingSaves } from '../../app/save-guard';
import { clipVideoId, projectForVideo, videoIdsForScope } from './video-context';
import { isRunningVideoJob, rememberVideoJob, selectedClipId, selectedVideoId, selectingVideo, selectVideoClip, videoJobEvents } from './video-context-state';
import './video-content.css';

export default function VideoContentControls() {
  const [scope, setScope] = createSignal<'current' | 'selected' | 'all'>('current');
  const [checked, setChecked] = createSignal<Set<string>>(new Set());
  const [starting, setStarting] = createSignal(false);
  const [batchErrors, setBatchErrors] = createSignal<string[]>([]);
  const clips = createMemo(() => currentProject()?.timeline.clips.filter((clip) => clip.content.kind === 'video') ?? []);
  const ids = createMemo(() => [...new Set(clips().map(clipVideoId).filter((id): id is string => !!id))]);
  const name = (id: string) => {
    const project = currentProject();
    return project ? projectForVideo(project, id).source.originalName ?? 'Video' : 'Video';
  };
  const events = () => Object.entries(videoJobEvents()).filter(([, event]) => event.projectId === currentProject()?.id);

  async function start(kind: 'asr' | 'translation' | 'tts') {
    if (starting()) return;
    const project = currentProject();
    if (!project) return;
    const targets = videoIdsForScope(project, scope(), selectedVideoId(), checked());
    if (!targets.length) { setNotice({ kind: 'info', message: 'Chọn ít nhất một video.' }); return; }
    setStarting(true); setBatchErrors([]);
    try {
      await flushPendingSaves();
      const latest = await projectService.get(project.id);
      const settings = kind === 'asr' ? (await translationService.getSettings()).settings.asr : undefined;
      // A failure affects one video; continue launching the remaining targets.
      for (const id of targets) {
        try {
          const view = projectForVideo(latest, id);
          if (kind === 'asr' && view.transcript) continue;
          let jobId: string;
          if (kind === 'asr') {
            jobId = await projectService.generateTranscript(project.id, { ...settings!, videoContentId: id,
              mode: view.transcriptSettings.mode, ocrFps: view.transcriptSettings.ocrFps, ocrRegion: view.transcriptSettings.ocrRegion });
          } else if (kind === 'translation') {
            jobId = await translationService.start(project.id, { videoContentId: id, scope: 'missing', segmentIds: [], overwriteEdited: false });
          } else {
            jobId = await ttsService.start(project.id, { videoContentId: id, scope: 'missing', segmentIds: [] });
          }
          // Desktop progress can arrive before the invoke response. Never replace
          // a newer/terminal event with this local queued placeholder.
          const key = `${kind}:${project.id}:${id}`;
          if (videoJobEvents()[key]?.jobId !== jobId) rememberVideoJob(kind, {
            projectId: project.id, videoContentId: id, jobId, status: 'queued', message: 'Đã xếp hàng',
          });
        } catch (error) {
          setBatchErrors((errors) => [...errors, `${name(id)}: ${error instanceof Error ? error.message : 'Không khởi chạy được tác vụ.'}`]);
        }
      }
    } catch (error) {
      setBatchErrors([error instanceof Error ? error.message : 'Không lưu được nội dung trước khi chạy.']);
    } finally { setStarting(false); }
  }

  async function cancel(key: string, jobId: string) {
    try {
      if (key.startsWith('asr:')) await projectService.cancelTranscription(jobId);
      else if (key.startsWith('tts:')) await ttsService.cancel(jobId);
      else await translationService.cancel(jobId);
    } catch (error) { setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không hủy được tác vụ.' }); }
  }

  return <Show when={clips().length}><div class="video-content-controls">
    <label>Video đang chỉnh sửa<select aria-label="Chọn clip video để dịch" disabled={selectingVideo()} value={selectedClipId() ?? clips().find((clip) => clipVideoId(clip) === selectedVideoId())?.id ?? ''}
      onChange={(event) => void selectVideoClip(event.currentTarget.value)}>
      <For each={clips().map((clip) => clip.id)}>{(id, index) => {
        const clip = () => clips().find((clip) => clip.id === id)!;
        return <option value={id}>{index() + 1}. {name(clipVideoId(clip())!)} · {Math.round(clip().startUs / 1e6)}s</option>;
      }}</For>
    </select></label>
    <div class="video-scope-actions"><select aria-label="Phạm vi xử lý video" value={scope()} onChange={(event) => setScope(event.currentTarget.value as ReturnType<typeof scope>)}>
      <option value="current">Video hiện tại</option><option value="selected">Các video đã chọn</option><option value="all">Tất cả video</option>
    </select><button type="button" disabled={starting()} onClick={() => void start('asr')} title="Chỉ nhận dạng video chưa có transcript">Nhận dạng phần thiếu</button>
      <button type="button" disabled={starting()} onClick={() => void start('translation')}>Dịch phần thiếu</button>
      <button type="button" disabled={starting()} onClick={() => void start('tts')}>Tạo giọng phần thiếu</button></div>
    <Show when={scope() === 'selected'}><div class="video-scope-selection"><For each={ids()}>{(id) => <label><input type="checkbox" checked={checked().has(id)} onChange={(event) => {
      const next = new Set(checked()); event.currentTarget.checked ? next.add(id) : next.delete(id); setChecked(next);
    }} />{name(id)}</label>}</For></div></Show>
    <Show when={events().length}><details class="video-jobs"><summary>Tiến độ từng video · {events().filter(([, event]) => isRunningVideoJob(event)).length} đang chạy</summary>
      <For each={events()}>{([key, event]) => <div class="video-job"><span>{event.videoContentId ? name(event.videoContentId) : 'Video chính'} · {key.startsWith('asr:') ? 'Nhận dạng' : key.startsWith('tts:') ? 'Giọng đọc' : 'Dịch'}<small role="status">{event.message}</small></span>
        <Show when={isRunningVideoJob(event)}><button type="button" aria-label={`Hủy tác vụ ${event.videoContentId ? name(event.videoContentId) : 'video chính'}`} onClick={() => void cancel(key, event.jobId)}>Hủy</button></Show></div>}</For>
    </details></Show>
    <For each={batchErrors()}>{(error) => <p class="video-job-error" role="alert">{error}</p>}</For>
  </div></Show>;
}
