import { createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { AlertTriangle, Check, Clapperboard, Download, FolderOpen, LoaderCircle, X } from 'lucide-solid';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { currentProject, setActiveWorkspacePanelTab, setNotice } from '../../app/project-state';
import { renderService } from '../../app/services/render-service';
import { dialogueService, type DialogueEnvironment } from '../../app/services/dialogue-service';
import type { ExportHistoryEntry, ExportJobEvent, ExportOptions, ExportValidation } from '../../app/services/render-service';
import VideoEncoderSelect from '../../shared/components/VideoEncoderSelect';
import Modal from '../../shared/components/Modal';
import { flushVisualSettings } from '../overlays/overlay-state';
import { flushPendingSaves } from '../../app/save-guard';
import TimelineExportSettings from '../timeline/TimelineExportSettings';
import { defaultTimelineExport, exportDurationUs } from '../timeline/export-settings';

const terminal = (status?: string) => status === 'completed' || status === 'partial' || status === 'failed' || status === 'cancelled';
const formatTime = (seconds: number) => {
  if (!Number.isFinite(seconds) || seconds < 0) return '00:00';
  const total = Math.floor(seconds); return `${String(Math.floor(total / 60)).padStart(2, '0')}:${String(total % 60).padStart(2, '0')}`;
};

const defaultExportName = () => {
  const now = new Date();
  const pad = (value: number) => String(value).padStart(2, '0');
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}_${pad(now.getHours())}-${pad(now.getMinutes())}-${pad(now.getSeconds())}`;
};

export default function ExportDialog(props: { onClose: () => void }) {
  const project = () => currentProject();
  const initialExportName = defaultExportName();
  const sourceBitrateKbps = () => {
    const source = project()?.source;
    if (!source?.fileSize || !source.duration || source.duration <= 0) return null;
    const audioReserve = (source.audioStreams?.length ?? 0) > 0 ? 192 : 0;
    return Math.max(500, Math.min(50_000, Math.round(((source.fileSize * 8 / source.duration / 1000 - audioReserve) / 100)) * 100));
  };
  const [outputFileName, setOutputFileName] = createSignal(initialExportName);
  const [options, setRawOptions] = createSignal<ExportOptions>({ quality: 'balanced', videoCodec: 'h264', encoder: 'auto', videoBitrateKbps: sourceBitrateKbps(), applyBlur: true, applyLogos: true, burnSubtitles: project()?.subtitle.enabled ?? true, enableSubtitleBlurOverride: false, includeOriginalAudio: !(project()?.dubbing.muteOriginal ?? false), includeDubbing: project()?.dubbing.enabled ?? false, ducking: project()?.dubbing.duckingEnabled ?? false, outputDirectory: null, outputFileName: initialExportName, originalVolume: project()?.dubbing.originalVolume ?? 1, dubbedVolume: project()?.dubbing.dubbedVolume ?? 1, duckingAmount: project()?.dubbing.duckingAmount ?? 0.75, dialogueReduction: project()?.dubbing.dialogueReductionEnabled ?? false, dialogueReductionAmount: project()?.dubbing.dialogueReductionAmount ?? 0.65 });
  let validationReady = false;
  const setOptions = (value: ExportOptions | ((previous: ExportOptions) => ExportOptions)) => {
    const next = setRawOptions(value);
    if (validationReady) queueValidation();
    return next;
  };
  if (project()?.timeline) setOptions(value => ({ ...value, timeline: defaultTimelineExport(project()!.timeline) }));
  const [dialogueEnvironment, setDialogueEnvironment] = createSignal<DialogueEnvironment>();
  const [validation, setValidation] = createSignal<ExportValidation>();
  const [validationError, setValidationError] = createSignal('');
  const [checking, setChecking] = createSignal(false);
  const [jobId, setJobId] = createSignal<string>();
  const [job, setJob] = createSignal<ExportJobEvent>();
  const [history, setHistory] = createSignal<ExportHistoryEntry[]>([]);
  const [exportDirectory, setExportDirectory] = createSignal('');
  const [canceling, setCanceling] = createSignal(false);
  const [starting, setStarting] = createSignal(false);
  const busy = () => starting() || !!jobId() && !terminal(job()?.status);
  const effectiveExportDirectory = () => options().outputDirectory || exportDirectory();
  const dialogueReductionReady = () => !!dialogueEnvironment()?.runtimeInstalled && !!dialogueEnvironment()?.modelCached;
  const estimatedOutputMiB = () => {
    const bitrate = options().videoBitrateKbps;
    const seconds = options().timeline && project()?.timeline ? exportDurationUs(project()!.timeline, options().timeline) / 1e6 : project()?.source.duration;
    if (!bitrate || !seconds || seconds <= 0) return undefined;
    const audioKbps = validation()?.expectedAudio ? 192 : 0;
    return ((bitrate + audioKbps) * seconds / 8192) * 1.04;
  };
  let validationRevision = 0;
  let validationTimer: ReturnType<typeof setTimeout> | undefined;
  let unlisten: (() => void) | undefined;

  const subtitleBlur = () => project()?.blurRegions.find((region) => region.role === 'subtitle');
  const subtitleRegion = () => subtitleBlur() ?? project()?.subtitle.region ?? null;
  const displayExportPath = (relative?: string | null) => {
    if (relative && /^(?:[a-zA-Z]:[\\/]|\\\\|\/)/.test(relative)) return relative;
    const filename = relative?.split(/[\\/]/).pop();
    if (!filename) return exportDirectory();
    const folder = effectiveExportDirectory();
    return folder ? `${folder.replace(/[\\/]$/, '')}${folder.includes('\\') ? '\\' : '/'}${filename}` : relative;
  };
  const close = () => { if (busy()) return; props.onClose(); };
  const refreshHistory = async () => {
    const id = project()?.id; if (!id) return;
    try { setHistory(await renderService.history(id)); } catch { /* The export itself remains usable if history is unavailable. */ }
  };
  const openExportFolder = async () => {
    const id = project()?.id;
    if (!id) return;
    try { setExportDirectory(await renderService.openExportFolder(id, options().outputDirectory)); }
    catch (error) { setValidationError(error instanceof Error ? error.message : String(error)); }
  };
  const chooseExportDirectory = async () => {
    try {
      const selected = await openDialog({ title: 'Chọn thư mục lưu video', directory: true, multiple: false });
      if (typeof selected === 'string') {
        setOptions((value) => ({ ...value, outputDirectory: selected }));
        setExportDirectory(selected);
        setValidationError('');
      }
    } catch (error) {
      setValidationError(error instanceof Error ? error.message : String(error));
    }
  };

  onMount(() => {
    void dialogueService.environment().then((environment) => {
      setDialogueEnvironment(environment);
      setOptions((value) => ({ ...value, dialogueReduction: environment.runtimeInstalled && environment.modelCached && !!project()?.dubbing.dialogueReductionEnabled }));
    }).catch(() => setDialogueEnvironment(undefined));
    void refreshHistory();
    const id = project()?.id;
    if (id) void renderService.exportDirectory(id).then(setExportDirectory).catch((error: unknown) => setValidationError(error instanceof Error ? error.message : String(error)));
    void renderService.onProgress((event) => {
      if (event.projectId !== project()?.id || event.jobId !== jobId()) return;
      setJob(event);
      if (terminal(event.status)) {
        setCanceling(false);
        queueValidation();
        void refreshHistory();
        if (event.status === 'completed') setNotice({ kind: 'success', message: `Đã xuất video: ${displayExportPath(event.output) ?? 'exports'}` });
        else if (event.status === 'partial') setNotice({ kind: 'info', message: event.message });
        else if (event.status === 'failed') setNotice({ kind: 'error', message: event.message });
      }
    }).then((dispose) => { unlisten = dispose; });
  });
  onMount(() => { validationReady = true; queueValidation(); });
  onCleanup(() => { validationReady = false; ++validationRevision; clearTimeout(validationTimer); unlisten?.(); });

  // Debounce edits and explicitly revalidate after cancellation/completion.
  function queueValidation() {
    if (!validationReady || busy()) return;
    clearTimeout(validationTimer);
    const revision = ++validationRevision;
    setChecking(true); setValidationError(''); setValidation(undefined);
    validationTimer = setTimeout(() => void validateSettings(revision), 150);
  }
  async function validateSettings(revision: number) {
    const id = project()?.id;
    if (!id) { setChecking(false); return; }
    const value = options();
    void flushVisualSettings(id).then(() => renderService.validate(id, value)).then((value) => {
      if (revision === validationRevision) setValidation(value);
    }).catch((error: unknown) => {
      if (revision === validationRevision) setValidationError(error instanceof Error ? error.message : String(error));
    }).finally(() => revision === validationRevision && setChecking(false));
  }

  const start = async () => {
    if (busy()) return;
    const id = project()?.id; if (!id) return;
    if (outputNameError()) { setValidationError(outputNameError()); return; }
    const exportOptions = { ...options(), outputFileName: outputFileName().trim() };
    clearTimeout(validationTimer); ++validationRevision; setChecking(false); setStarting(true);
    try {
      await flushPendingSaves();
      await flushVisualSettings(id);
      const latestValidation = await renderService.validate(id, exportOptions);
      setValidation(latestValidation);
      if (!latestValidation.valid || latestValidation.warnings.length) {
        setValidationError(latestValidation.warnings.length ? 'Có phụ đề không vừa vùng subtitle ở cỡ chữ tối thiểu. Hãy kiểm tra layout trước khi export.' : 'Thiết lập hiện tại chưa sẵn sàng để export.');
        return;
      }
      setValidationError('');
      const next = await renderService.start(id, exportOptions);
      setJobId(next); setJob({ projectId: id, jobId: next, status: 'queued', progress: 0, elapsedSeconds: 0, totalSeconds: validation()?.duration ?? 0, speed: null, output: null, message: 'Đang chuẩn bị FFmpeg…', layoutWarnings: [] });
      setNotice({ kind: 'info', message: 'Đã bắt đầu export. Bạn có thể theo dõi tiến độ tại đây.' });
    } catch (error) { setValidationError(error instanceof Error ? error.message : String(error)); }
    finally { setStarting(false); }
  };

  const cancel = async () => {
    if (!jobId()) return;
    setCanceling(true);
    try { await renderService.cancel(jobId()!); } catch (error) { setCanceling(false); setValidationError(error instanceof Error ? error.message : String(error)); }
  };

  const needsSubtitleRegion = () => !options().timeline && options().burnSubtitles && !subtitleRegion();
  const blockedByWarning = () => (validation()?.warnings.length ?? 0) > 0;
  const outputNameError = () => {
    const value = outputFileName().trim().replace(/\.mp4$/i, '');
    if (!value) return 'Nhập tên file export.';
    if (value.length > 180 || /[<>:"/\\|?*\u0000-\u001f]/.test(value)) return 'Tên file tối đa 180 ký tự và không được chứa \\/:*?"<>|.';
    if (/^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])$/i.test(value.split('.')[0])) return 'Tên này được Windows dành riêng.';
    return '';
  };

  return <Modal title="Export video" description="Chọn nơi lưu video hoàn chỉnh. Video nguồn và dữ liệu project không bị thay đổi." onClose={close}>
    <div class="export-dialog-content">
      <fieldset disabled={busy()} style={{ border: '0', padding: '0', margin: '0', 'min-width': '0' }}>
      <label class="export-quality"><span>Nội dung xuất</span><select class="settings-select" value={options().timeline ? 'timeline' : 'source'} onChange={e => setOptions(value => ({ ...value, timeline: e.currentTarget.value === 'timeline' ? defaultTimelineExport(project()!.timeline) : undefined }))}><option value="timeline">Timeline đã dựng</option><option value="source">Video nguồn · Blur/logo/phụ đề nguồn</option></select></label>
      <Show when={options().timeline}>{value => <TimelineExportSettings document={project()!.timeline} value={value()} onChange={timeline => setOptions(previous => ({ ...previous, timeline }))} />}</Show>
      <div class="export-file-row"><span class="export-file-icon"><Clapperboard size={17} /></span><div class="export-file-details"><label for="export-file-name">Tên file xuất</label><div class="export-file-name-control"><input id="export-file-name" class="text-input" maxlength="180" value={outputFileName()} onInput={(event) => { setOutputFileName(event.currentTarget.value); setValidationError(''); }} /><Show when={!outputFileName().trim().toLowerCase().endsWith('.mp4')}><span>.mp4</span></Show></div><Show when={outputNameError()}><small class="export-name-error" role="alert">{outputNameError()}</small></Show><small>{options().timeline?.width ?? project()?.source.width ?? '—'} × {options().timeline?.height ?? project()?.source.height ?? '—'} · {formatTime(options().timeline ? exportDurationUs(project()!.timeline, options().timeline) / 1e6 : project()?.source.duration ?? 0)}</small></div></div>
      <div class="export-location"><div><strong>Nơi lưu video {options().outputDirectory ? '· đã chọn' : '· mặc định'}</strong><code>{effectiveExportDirectory() || 'Đang lấy đường dẫn…'}</code></div><div class="export-location-actions"><button class="button button-quiet" type="button" onClick={() => void chooseExportDirectory()}><FolderOpen size={14} /> Chọn thư mục</button><Show when={options().outputDirectory}><button class="button button-quiet" type="button" onClick={() => setOptions((value) => ({ ...value, outputDirectory: null }))}>Mặc định</button></Show><button class="button button-quiet" type="button" onClick={() => void openExportFolder()}><FolderOpen size={14} /> Mở</button></div></div>

      <Show when={!options().timeline}><div class="export-section"><h3>VIDEO NGUỒN</h3>
        <label class="export-option"><span>Áp dụng các vùng blur</span><input type="checkbox" checked={options().applyBlur} onChange={(event) => setOptions((value) => ({ ...value, applyBlur: event.currentTarget.checked }))} /></label>
        <label class="export-option"><span>Chèn logo</span><input type="checkbox" checked={options().applyLogos} onChange={(event) => setOptions((value) => ({ ...value, applyLogos: event.currentTarget.checked }))} /></label>
        <label class="export-option"><span>Chèn phụ đề dịch trong vùng subtitle</span><input type="checkbox" checked={options().burnSubtitles} onChange={(event) => setOptions((value) => ({ ...value, burnSubtitles: event.currentTarget.checked }))} /></label>
        <Show when={options().burnSubtitles && subtitleBlur() && !subtitleBlur()?.enabled}><label class="export-override"><input type="checkbox" checked={options().enableSubtitleBlurOverride} onChange={(event) => setOptions((value) => ({ ...value, enableSubtitleBlurOverride: event.currentTarget.checked }))} /><span>Bật Subtitle Blur cho lần export này, không đổi cấu hình project</span></label></Show>
        <div class="export-option"><span>Vùng phụ đề</span><strong>{subtitleBlur() ? 'Subtitle Region · Blur/STTN' : project()?.subtitle.region ? 'Vùng subtitle riêng' : 'Chưa chọn'}</strong></div>
      </div>

      </Show>
      <div class="export-section"><h3>AUDIO & QUALITY</h3>
        <Show when={!options().timeline}>
        <label class="export-option"><span>Âm thanh gốc</span><input type="checkbox" checked={options().includeOriginalAudio} onChange={(event) => setOptions((value) => ({ ...value, includeOriginalAudio: event.currentTarget.checked }))} /></label>
        <label class="export-option"><span>Lồng tiếng đã tạo</span><input type="checkbox" checked={options().includeDubbing} onChange={(event) => setOptions((value) => ({ ...value, includeDubbing: event.currentTarget.checked }))} /></label>
        <Show when={options().includeOriginalAudio}><label class="export-option"><span>Âm lượng gốc <small>{Math.round(options().originalVolume * 100)}%</small></span><input type="range" min="0" max="1" step="0.01" value={options().originalVolume} onInput={(event) => setOptions((value) => ({ ...value, originalVolume: Number(event.currentTarget.value) }))} /></label></Show>
        <Show when={options().includeDubbing}><label class="export-option"><span>Âm lượng lồng tiếng <small>{Math.round(options().dubbedVolume * 100)}%</small></span><input type="range" min="0" max="1" step="0.01" value={options().dubbedVolume} onInput={(event) => setOptions((value) => ({ ...value, dubbedVolume: Number(event.currentTarget.value) }))} /></label></Show>
        <Show when={options().includeDubbing && options().includeOriginalAudio}><label class="export-option"><span>Duck original audio</span><input type="checkbox" checked={options().ducking} onChange={(event) => setOptions((value) => ({ ...value, ducking: event.currentTarget.checked }))} /></label></Show>
        <Show when={options().ducking && options().includeOriginalAudio && options().includeDubbing}><label class="export-option"><span>Mức giảm khi voice hoạt động <small>{Math.round(options().duckingAmount * 100)}%</small></span><input type="range" min="0" max="1" step="0.01" value={options().duckingAmount} onInput={(event) => setOptions((value) => ({ ...value, duckingAmount: Number(event.currentTarget.value) }))} /></label></Show>
        <label class="export-option" title={dialogueReductionReady() ? 'Tách và giảm riêng giọng nói của nhân vật.' : 'Cài bộ xử lý và mô hình BandIt v2 trong Cài đặt → Nhận dạng lời thoại.'} style={{ opacity: dialogueReductionReady() ? 1 : 0.48 }}><span>Giảm thoại gốc <small>{dialogueReductionReady() ? 'BandIt v2' : 'Chưa cài mô hình'}</small></span><input type="checkbox" disabled={!dialogueReductionReady() || !options().includeOriginalAudio} checked={dialogueReductionReady() && options().dialogueReduction} onChange={(event) => setOptions((value) => ({ ...value, dialogueReduction: event.currentTarget.checked }))} /></label>
        <Show when={dialogueReductionReady() && options().dialogueReduction}><label class="export-option"><span>Mức giảm thoại <small>{Math.round(options().dialogueReductionAmount * 100)}%</small></span><input type="range" min="0" max="1" step="0.01" value={options().dialogueReductionAmount} onInput={(event) => setOptions((value) => ({ ...value, dialogueReductionAmount: Number(event.currentTarget.value) }))} /></label></Show>
        </Show>
        <label class="export-quality"><span>Codec video</span><select class="settings-select" value={options().videoCodec} onChange={(event) => { const videoCodec = event.currentTarget.value as ExportOptions['videoCodec']; setOptions((value) => ({ ...value, videoCodec })); }}><option value="h264">H.264 · tương thích rộng</option><option value="hevc">HEVC / H.265 · file gọn hơn</option></select></label>
        <VideoEncoderSelect codec={options().videoCodec} value={options().encoder ?? 'auto'} resolvedLabel={validation()?.resolvedEncoderLabel}
          disabled={busy()} onChange={(encoder) => setOptions((value) => ({ ...value, encoder }))}
          onRefreshed={queueValidation} />
        <label class="export-quality"><span>Chất lượng encode</span><select class="settings-select" value={options().quality} onChange={(event) => setOptions((value) => ({ ...value, quality: event.currentTarget.value as ExportOptions['quality'] }))}><option value="fast">Nhanh · ít nén hơn</option><option value="balanced">Cân bằng</option><option value="high">Chất lượng cao</option></select></label>
        <div class="export-bitrate-row"><label for="export-video-bitrate">Bitrate video</label><div><input id="export-video-bitrate" type="number" min="500" max="50000" step="100" value={options().videoBitrateKbps ?? ''} aria-describedby="export-bitrate-help" onInput={(event) => { const raw = event.currentTarget.value; const value = raw === '' ? null : Number(raw); setOptions((previous) => ({ ...previous, videoBitrateKbps: value !== null && Number.isFinite(value) ? value : null })); }} /><span>kbps</span></div></div>
        <small id="export-bitrate-help" class="export-encoder-note">Giới hạn bitrate giúp giữ dung lượng gần video nguồn. Để dùng chế độ chất lượng tự động, xóa giá trị.</small>
        <div class="export-bitrate-actions"><Show when={sourceBitrateKbps()}><button class="button button-quiet" type="button" onClick={() => setOptions((value) => ({ ...value, videoBitrateKbps: sourceBitrateKbps() }))}>Gần dung lượng nguồn</button></Show><button class="button button-quiet" type="button" onClick={() => setOptions((value) => ({ ...value, videoBitrateKbps: null }))}>Chất lượng tự động</button></div>
        <Show when={estimatedOutputMiB() !== undefined}><small class="export-size-estimate">Dung lượng ước tính khoảng {estimatedOutputMiB()!.toFixed(1)} MiB · thực tế thay đổi theo nội dung và âm thanh.</small></Show>
      </div>

      <Show when={needsSubtitleRegion()}><div class="export-alert"><AlertTriangle size={15} /><span>Phụ đề dịch đang bật nhưng chưa có vùng subtitle. Hãy chọn vùng riêng trên preview hoặc đặt một vùng blur làm Subtitle Region; app không tự đặt phụ đề ở vị trí global.</span><button class="button button-quiet" type="button" onClick={() => { close(); setActiveWorkspacePanelTab('properties'); setNotice({ kind: 'info', message: 'Trong Properties, chọn vùng subtitle trên preview hoặc đặt vùng blur thành Subtitle Region.' }); }}>Properties</button></div></Show>
      </fieldset>
      <Show when={validationError()}><div class="export-alert export-alert-error" role="alert"><AlertTriangle size={15} /><span>{validationError()}</span></div></Show>

      <Show when={checking() || starting()}><div class="export-validation" role="status"><LoaderCircle size={14} class="export-spinner" /> {starting() ? 'Đang kiểm tra và chụp snapshot media…' : 'Đang kiểm tra layout và media…'}</div></Show>
      <Show when={validation()}>{(result) => <div class="export-validation" classList={{ 'export-validation-error': !result().valid }}>
        <div class="export-validation-title"><Check size={14} /> {options().timeline ? 'Snapshot timeline' : 'Layout phụ đề'} <span>{result().translatedSegments} đoạn phụ đề</span></div>
        <Show when={options().timeline} fallback={<div class="export-validation-row">{result().fitsNormally} vừa cỡ chữ đặt</div>}><div class="export-validation-row">{result().width} × {result().height} · {formatTime(result().duration)} · {result().expectedAudio ? 'Có âm thanh' : 'Không có âm thanh'}</div></Show>
        <Show when={result().fitWithSmallerFont}><div class="export-validation-row">{result().fitWithSmallerFont} segment auto-fit; nhỏ nhất {result().smallestFontSize}px (cỡ đã đặt: {project()?.subtitle.fontSize ?? '—'}px)</div></Show>
      <For each={result().audioWarnings ?? []}>{message => <div class="export-alert" role="status"><AlertTriangle size={14} /><span>{message}</span></div>}</For><Show when={blockedByWarning()}><div class="export-alert export-alert-error"><AlertTriangle size={14} /><span>Không thể xuất an toàn: các segment sau không vừa vùng subtitle ở cỡ chữ tối thiểu. Mở rộng vùng subtitle, giảm cỡ chữ tối thiểu hoặc rút gọn bản dịch; nội dung sẽ không bị cắt.</span></div><For each={result().warnings}>{(warning, index) => <div class="export-warning-segment">#{index() + 1} · {formatTime(warning.start)} — {warning.segmentId}</div>}</For></Show>
        <Show when={result().dubbingSegments}><div class="export-validation-row">{result().dubbingSegments} voice clips sẽ được đặt theo timestamp</div></Show>
      </div>}</Show>

      <Show when={job()}>{(state) => <div class="export-job-progress" role="status"><div class="export-job-header"><strong>{state().status === 'completed' ? 'Export hoàn tất' : state().status === 'failed' ? 'Export thất bại' : state().status === 'cancelled' ? 'Đã hủy export' : 'Đang export'}</strong><span>{Math.round(state().progress * 100)}%</span></div><div class="export-progress-track"><span style={{ width: `${state().progress * 100}%` }} /></div><p>{state().message}</p><small>{formatTime(state().elapsedSeconds)} / {formatTime(state().totalSeconds)}{state().speed ? ` · ${state().speed?.toFixed(1)}×` : ''}</small><Show when={state().output}><code class="export-output-path">{displayExportPath(state().output)}</code></Show><Show when={state().status === 'completed'}><button class="button button-quiet export-reveal-button" type="button" onClick={() => void openExportFolder()}><FolderOpen size={14} /> Mở video trong thư mục</button></Show></div>}</Show>

      <Show when={history().length}><div class="export-history"><h3>LẦN EXPORT GẦN ĐÂY</h3><For each={history().slice(0, 3)}>{(item) => <div class="export-history-item"><span classList={{ 'history-failed': item.status !== 'completed' }}>{item.status === 'completed' ? 'Hoàn tất' : item.status === 'cancelled' ? 'Đã hủy' : 'Lỗi'}</span><strong>{item.file ? displayExportPath(item.file) : item.message ?? 'Export'}</strong><small>{new Date(item.createdAt).toLocaleString('vi-VN')} · {item.sizeBytes ? `${(item.sizeBytes / 1024 / 1024).toFixed(1)} MB` : '—'}</small></div>}</For></div></Show>
    </div>
    <footer class="modal-actions export-actions">
      <button class="button button-quiet" type="button" disabled={busy()} onClick={close}><X size={14} />{busy() ? 'Đang chạy…' : 'Đóng'}</button>
      <Show when={jobId() && !terminal(job()?.status)}><button class="button button-danger" type="button" disabled={canceling()} onClick={() => void cancel()}>{canceling() ? 'Đang hủy…' : 'Hủy export'}</button></Show>
      <Show when={!jobId() || terminal(job()?.status)}><button class="button button-primary" type="button" disabled={busy() || checking() || !validation()?.valid || blockedByWarning() || needsSubtitleRegion() || !!outputNameError()} onClick={() => void start()}><Download size={14} /> Export MP4</button></Show>
    </footer>
  </Modal>;
}
