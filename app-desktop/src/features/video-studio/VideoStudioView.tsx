import { createEffect, createMemo, createSignal, For, Index, onCleanup, Show } from 'solid-js';
import { isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { ArrowLeft, ArrowDown, ArrowUp, Check, Copy, Download, Film, ImagePlus, LoaderCircle, Plus, RefreshCw, Save, Sparkles, Star, Trash2, Volume2, X } from 'lucide-solid';
import { currentProject, setNotice } from '../../app/project-state';
import { registerSaveTask } from '../../app/save-guard';
import { studioService } from '../../app/services/studio-service';
import { ttsService } from '../../app/services/tts-service';
import { renderService } from '../../app/services/render-service';
import { audioCurrent, exportProblems, formatTime, newDraft, removeChapter, restoreAudio, scenePrompt, timeline } from './studio-utils';
import Modal from '../../shared/components/Modal';
import type { AiAction, OutlineChapter, StudioDraft, StudioOverlay, StudioProgress, VideoIdea, VideoScene } from './types';
import StudioPreview from './StudioPreview';
import './video-studio.css';

type GeneratedScene = Pick<VideoScene, 'title' | 'narration' | 'imagePrompt'>;
export default function VideoStudioView() {
  const [draft, setDraft] = createSignal<StudioDraft>(newDraft(''));
  const [loading, setLoading] = createSignal(true);
  const [loadError, setLoadError] = createSignal('');
  const [error, setError] = createSignal('');
  const [busy, setBusy] = createSignal('');
  const [jobId, setJobId] = createSignal('');
  const [progress, setProgress] = createSignal<StudioProgress>();
  const [saveState, setSaveState] = createSignal('Đã lưu');
  const [chapterId, setChapterId] = createSignal('');
  const [chapterToDelete, setChapterToDelete] = createSignal('');
  const [sceneId, setSceneId] = createSignal('');
  const [voices, setVoices] = createSignal<string[]>([]);
  const [ttsReady, setTtsReady] = createSignal(false);
  const [ttsMessage, setTtsMessage] = createSignal('Đang kiểm tra VieNeu-TTS…');
  const [instruction, setInstruction] = createSignal('');
  const [overlayId, setOverlayId] = createSignal('');
  const [lastExport, setLastExport] = createSignal('');
  let revision = 0; let savedRevision = 0;
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  let saving: Promise<void> | undefined;
  let alive = true; let stopped = false;
  let unlisten: (() => void) | undefined;
  const chosenChapter = createMemo(() => draft().outline.find(c => c.id === chapterId()));
  const pendingChapter = createMemo(() => draft().outline.find(c => c.id === chapterToDelete()));
  const chapterScenes = createMemo(() => draft().scenes.filter(s => s.chapterId === chapterId()));
  const scene = createMemo(() => draft().scenes.find(s => s.id === sceneId()));
  const overlay = createMemo(() => draft().overlays.find(o => o.id === overlayId()));
  const duration = createMemo(() => timeline(draft()).at(-1)?.end ?? 0);
  const problems = createMemo(() => exportProblems(draft()));
  const locked = () => !!busy() || loading() || !!loadError();
  let deleteChapterButton: HTMLButtonElement | undefined;
  const closeChapterDeletion = () => { setChapterToDelete(''); deleteChapterButton?.focus(); };
  const deleteChapter = () => {
    if (locked()) return;
    const current = pendingChapter(); if (!current) return;
    const index = draft().outline.findIndex(c => c.id === current.id);
    const next = draft().outline[index + 1]?.id ?? draft().outline[index - 1]?.id ?? '';
    update(d => removeChapter(d, current.id));
    chooseChapter(next); setInstruction(''); closeChapterDeletion();
    setNotice({ kind: 'success', message: `Đã xóa chương “${current.title}” khỏi kịch bản.` });
  };

  const flush = async (): Promise<void> => {
    if (saveTimer) { clearTimeout(saveTimer); saveTimer = undefined; }
    if (saving) { await saving; if (revision !== savedRevision) await flush(); return; }
    if (revision === savedRevision || loading() || !draft().projectId) return;
    const snapshot = draft(); const version = revision;
    setSaveState('Đang lưu…');
    const task = studioService.save(snapshot).then(() => {
      savedRevision = version; if (alive) setSaveState(version === revision ? 'Đã lưu' : 'Đang lưu…');
    }).catch((reason: unknown) => { if (alive) setSaveState('Lưu thất bại'); throw reason; });
    saving = task;
    try { await task; } finally { saving = undefined; }
    if (revision !== savedRevision) await flush();
  };
  const unregister = registerSaveTask('video-studio', async () => {
    if (busy()) throw new Error('Tác vụ tạo video đang chạy. Hãy hoàn tất hoặc bấm Dừng trước khi rời màn hình.');
    if (loadError()) return;
    await flush();
  });
  const update = (change: (d: StudioDraft) => StudioDraft) => {
    setDraft(d => ({ ...change(d), updatedAt: new Date().toISOString() })); revision++;
    setSaveState('Đang lưu…');
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => { void flush().catch(reason => { if (alive) setError(reason instanceof Error ? reason.message : 'Không lưu được bản nháp.'); }); }, 600);
  };
  const field = <K extends keyof StudioDraft>(key: K, value: StudioDraft[K]) => update(d => ({ ...d, [key]: value }));
  const editScene = (id: string, patch: Partial<VideoScene>) => update(d => ({ ...d, sampleApproved: false, scenes: d.scenes.map(s => s.id === id ? { ...s, ...patch } : s) }));
  const editOverlay = (id: string, patch: Partial<StudioOverlay>) => update(d => ({ ...d, overlays: d.overlays.map(o => o.id === id ? { ...o, ...patch } : o) }));
  const chooseScene = (id: string) => { setSceneId(id); const next = draft().scenes.find(s => s.id === id); if (next) setChapterId(next.chapterId); };
  const chooseChapter = (id: string) => { setChapterId(id); setSceneId(draft().scenes.find(s => s.chapterId === id)?.id ?? ''); };
  const load = async (id: string) => {
    setLoading(true); setLoadError('');
    try {
      const loaded = await studioService.load(id);
      if (!alive || currentProject()?.id !== id) return;
      setDraft(loaded ?? newDraft(id)); revision = savedRevision = 0;
      chooseChapter(loaded?.outline[0]?.id ?? '');
    } catch (reason) { if (alive) setLoadError(reason instanceof Error ? reason.message : 'Không đọc được bản nháp.'); }
    finally { if (alive) setLoading(false); }
  };
  const refreshTts = async () => {
    try {
      const env = await ttsService.getEnvironment();
      if (!alive) return;
      setVoices(env.voices); setTtsReady(env.workerInstalled && env.modelCached);
      setTtsMessage(env.workerInstalled && env.modelCached ? 'VieNeu-TTS sẵn sàng · Việt / Anh' : 'Cài VieNeu-TTS và tải model trong Cài đặt → Voice / TTS.');
      if (!draft().voice && !loading() && !loadError() && env.voices.length) field('voice', env.voices.includes('Ngọc Huyền') ? 'Ngọc Huyền' : env.voices[0]);
    } catch (reason) { if (alive) { setTtsReady(false); setTtsMessage(reason instanceof Error ? reason.message : 'Chưa kiểm tra được VieNeu-TTS.'); } }
  };
  createEffect(() => {
    const id = currentProject()?.id;
    if (id) void load(id).then(refreshTts);
  });
  if (isTauri()) void listen<StudioProgress>('studio-progress', event => {
    if (alive && event.payload.jobId === jobId()) setProgress(event.payload);
  }).then(dispose => { if (alive) unlisten = dispose; else dispose(); });
  onCleanup(() => { alive = false; unlisten?.(); unregister(); if (saveTimer) clearTimeout(saveTimer); if (jobId()) void studioService.cancel(jobId()); });

  const run = async (label: string, action: () => Promise<void>) => {
    if (locked()) return;
    setBusy(label); setError(''); stopped = false; setProgress(undefined);
    try { await flush(); await action(); await flush(); }
    catch (reason) { if (alive) setError(reason instanceof Error ? reason.message : 'Không thể hoàn tất tác vụ.'); }
    finally { if (alive) { setBusy(''); setJobId(''); } }
  };
  const ai = async <T,>(action: AiAction, chapter: string | null = null, selectedScene: string | null = null) => {
    if (stopped) throw new Error('Đã dừng. Nội dung đã hoàn thành được giữ lại.');
    const id = crypto.randomUUID(); setJobId(id);
    return studioService.ai<T>(draft(), action, id, chapter, selectedScene, instruction());
  };
  const stop = async () => { stopped = true; if (jobId()) await studioService.cancel(jobId()).catch(() => undefined); };
  const ideas = () => run('Đang tìm 5 ý tưởng…', async () => {
    if (!draft().topic.trim()) throw new Error('Nhập một chủ đề hoặc ngách nội dung trước.');
    const result = await ai<{ ideas: VideoIdea[] }>('ideas');
    field('ideas', result.ideas.map(idea => ({ ...idea, id: crypto.randomUUID() })));
  });
  const selectIdea = (idea: VideoIdea) => update(d => d.selectedIdeaId === idea.id
    ? { ...d, screen: 'editor' }
    : { ...d, screen: 'editor', selectedIdeaId: idea.id, title: idea.title, description: idea.description, outline: [], outlineApproved: false, scenes: [], sampleApproved: false });
  const outline = () => run('Đang lập dàn ý…', async () => {
    if (!draft().title.trim()) throw new Error('Nhập tiêu đề trước khi lập dàn ý.');
    const result = await ai<{ chapters: OutlineChapter[] }>('outline');
    update(d => ({ ...d, outline: result.chapters, outlineApproved: false, sampleApproved: false }));
    chooseChapter(result.chapters[0]?.id ?? '');
  });
  const writeChapter = async (id: string) => {
    const result = await ai<{ scenes: GeneratedScene[] }>('chapter', id);
    const added = result.scenes.map(s => ({ ...s, id: crypto.randomUUID(), chapterId: id, image: null, imageHistory: [], audio: null, audioHistory: [] }));
    update(d => {
      const all = [...d.scenes, ...added];
      return { ...d, sampleApproved: false, scenes: d.outline.flatMap(c => all.filter(s => s.chapterId === c.id)) };
    });
    if (chapterId() === id || !sceneId()) chooseScene(added[0]?.id ?? '');
    await flush();
  };
  const writeMissing = () => run('Đang viết kịch bản theo chương…', async () => {
    if (!draft().outlineApproved) throw new Error('Duyệt dàn ý trước khi viết kịch bản.');
    for (const chapter of draft().outline) {
      if (stopped) break;
      if (!draft().scenes.some(s => s.chapterId === chapter.id)) await writeChapter(chapter.id);
    }
  });
  const rewrite = () => run('Đang chỉnh sửa cảnh…', async () => {
    const current = scene(); if (!current) return;
    if (!instruction().trim()) throw new Error('Nhập yêu cầu chỉnh sửa cho AI.');
    const result = await ai<{ scenes: GeneratedScene[] }>('rewrite', null, current.id); editScene(current.id, result.scenes[0]);
  });
  const importImage = (logo = false) => run('Đang nhập ảnh…', async () => {
    const current = scene(); if (!logo && !current) return;
    const path = await open({ title: logo ? 'Chọn ảnh logo' : 'Nhập ảnh đã tạo trong Codex', multiple: false, filters: [{ name: 'Ảnh', extensions: ['png', 'jpg', 'jpeg', 'webp'] }] });
    if (typeof path !== 'string') return;
    const relative = await studioService.importImage(draft().projectId, path);
    if (logo) {
      const item: StudioOverlay = { id: crypto.randomUUID(), kind: 'logo', path: relative, text: '', x: .82, y: .04, width: .12, fontSize: 42, color: '#ffffff', opacity: 1, start: 0, end: 10800 };
      update(d => ({ ...d, overlays: [...d.overlays, item] })); setOverlayId(item.id);
    } else if (current) editScene(current.id, { image: relative, imageHistory: [current.image, ...current.imageHistory].filter((v): v is string => !!v).slice(0, 5) });
  });
  const copyPrompt = async () => {
    const current = scene(); if (!current) return;
    try { await navigator.clipboard.writeText(scenePrompt(draft(), current)); setNotice({ kind: 'success', message: 'Đã sao chép prompt. Dán vào Codex desktop để tạo ảnh.' }); }
    catch { setError('Không sao chép được. Chọn và sao chép prompt trong ô bên dưới.'); }
  };
  const makeVoice = async (id: string, preserveApproval = false) => {
    if (stopped) return;
    const nextJob = crypto.randomUUID(); setJobId(nextJob);
    const audio = await studioService.voice(draft(), id, nextJob);
    const current = draft().scenes.find(s => s.id === id); if (!current) return;
    const patch = restoreAudio(current, audio);
    update(d => ({ ...d, sampleApproved: preserveApproval && d.sampleApproved, scenes: d.scenes.map(s => s.id === id ? { ...s, ...patch } : s) }));
    await flush();
  };
  const voicesMissing = () => run('Đang tạo giọng đọc từng cảnh…', async () => {
    if (!draft().sampleApproved) throw new Error('Nghe và duyệt một cảnh mẫu trước khi tạo toàn bộ giọng đọc.');
    for (const item of draft().scenes) { if (stopped) break; if (!audioCurrent(item, draft())) await makeVoice(item.id, true); }
  });
  const addChapter = () => {
    const next: OutlineChapter = { id: crypto.randomUUID(), title: `Chương ${draft().outline.length + 1}`, summary: 'Bổ sung nội dung để đạt thời lượng mục tiêu.', targetSeconds: 180 };
    update(d => ({ ...d, outline: [...d.outline, next] })); chooseChapter(next.id);
  };
  const addScene = () => {
    if (!chapterId()) return;
    const next: VideoScene = { id: crypto.randomUUID(), chapterId: chapterId(), title: 'Cảnh mới', narration: '', imagePrompt: '', image: null, imageHistory: [], audio: null, audioHistory: [] };
    update(d => ({ ...d, scenes: d.outline.flatMap(c => [...d.scenes, next].filter(s => s.chapterId === c.id)) })); chooseScene(next.id);
  };
  const moveScene = (direction: number) => {
    const current = scene(); if (!current) return;
    const list = chapterScenes(); const index = list.findIndex(s => s.id === current.id); const other = list[index + direction]; if (!other) return;
    update(d => { const all = [...d.scenes]; const a = all.findIndex(s => s.id === current.id); const b = all.findIndex(s => s.id === other.id); [all[a], all[b]] = [all[b], all[a]]; return { ...d, scenes: all }; });
  };
  const addText = () => {
    const item: StudioOverlay = { id: crypto.randomUUID(), kind: 'text', text: 'Nội dung text', path: null, x: .05, y: .05, width: .5, fontSize: 48, color: '#ffffff', opacity: 1, start: 0, end: 10800 };
    update(d => ({ ...d, overlays: [...d.overlays, item] })); setOverlayId(item.id);
  };
  const exportVideo = () => run('Đang xuất video MP4…', async () => {
    if (problems().length) throw new Error(problems().join('\n'));
    const id = crypto.randomUUID(); setJobId(id);
    const path = await studioService.export(draft(), id); setLastExport(path);
    setNotice({ kind: 'success', message: 'Đã lưu video MP4 trong thư mục exports của dự án.' });
  });
  const subtitleField = <K extends keyof StudioDraft['subtitle']>(key: K, value: StudioDraft['subtitle'][K]) => update(d => ({ ...d, subtitle: { ...d.subtitle, [key]: value } }));

  return <main class="studio-view">
    <Show when={!loading()} fallback={<div class="studio-loading"><LoaderCircle class="studio-spin" size={25} /> Đang mở bản nháp…</div>}>
      <Show when={!loadError()} fallback={<div class="studio-loading"><p class="studio-error" role="alert">{loadError()}</p><button class="button" onClick={() => void load(currentProject()!.id)}>Thử mở lại</button></div>}>
        <header class="studio-header"><div><span class="studio-eyebrow"><Film size={14} /> TẠO VIDEO AI</span><h1>{draft().screen === 'ideas' ? 'Từ ý tưởng đến câu chuyện' : 'Thực hiện ý tưởng'}</h1><p>{draft().screen === 'ideas' ? 'Chọn một trong 5 hướng nội dung để bắt đầu video của bạn.' : `${draft().title || 'Video mới'} · ${formatTime(duration())} lời kể / ${draft().targetMinutes} phút mục tiêu`}</p></div><div class="studio-header-actions"><span class="studio-muted" role="status">{saveState()}</span><button class="button button-quiet" disabled={locked()} onClick={() => void run('Đang lưu bản nháp…', async () => { await flush(); })}><Save size={14} /> Lưu nháp</button><Show when={draft().screen === 'editor'}><button class="button button-primary" disabled={locked() || !!problems().length} title={problems().join('\n')} onClick={() => void exportVideo()}><Download size={15} /> Lưu video</button></Show></div></header>
        <Show when={busy()}><div class="studio-progress" role="status"><LoaderCircle class="studio-spin" size={17} /><div><strong>{busy()}</strong><span>{progress()?.message ?? 'Dữ liệu đã hoàn thành sẽ được lưu để tiếp tục sau.'}</span></div><progress max="1" value={progress()?.progress ?? 0} /><button class="button button-quiet" onClick={() => void stop()}><X size={14} /> Dừng</button></div></Show>
        <Show when={error()}><div class="studio-error" role="alert">{error()}<button class="studio-icon-button" aria-label="Đóng thông báo" onClick={() => setError('')}><X size={14} /></button></div></Show>
        <Show when={draft().screen === 'ideas'} fallback={<>
          <div class="studio-editor-toolbar"><button class="button button-quiet" disabled={locked()} onClick={() => field('screen', 'ideas')}><ArrowLeft size={14} /> Ý tưởng</button><span class="studio-step">1. Dàn ý</span><span class="studio-step">2. Kịch bản & ảnh</span><span class="studio-step">3. Xem trước</span><span class="studio-step">4. Lưu video</span></div>
          <div class="studio-editor-grid">
            <aside class="studio-chapters"><div class="studio-panel-heading"><span>Chương & cảnh</span><button class="studio-icon-button" title="Thêm chương" aria-label="Thêm chương" disabled={locked() || !draft().outline.length} onClick={addChapter}><Plus size={15} /></button></div>
              <Show when={draft().outline.length} fallback={<div class="studio-empty-small">Lập và duyệt dàn ý để bắt đầu.</div>}>
                <For each={draft().outline}>{chapter => <button class="studio-chapter" classList={{ active: chapterId() === chapter.id }} onClick={() => chooseChapter(chapter.id)}><strong>{chapter.title}</strong><span>{formatTime(chapter.targetSeconds)} dự kiến · {draft().scenes.filter(s => s.chapterId === chapter.id).length} cảnh</span></button>}</For>
                <div class="studio-panel-heading"><span>Cảnh trong chương</span><button class="studio-icon-button" aria-label="Thêm cảnh" disabled={locked()} onClick={addScene}><Plus size={15} /></button></div>
                <For each={chapterScenes()}>{(item, index) => <button class="studio-scene-row" classList={{ active: sceneId() === item.id }} onClick={() => chooseScene(item.id)}><span class="studio-scene-number">{String(index() + 1).padStart(2, '0')}</span><span><strong>{item.title}</strong><small>{item.image ? 'Có ảnh' : 'Chưa có ảnh'} · {audioCurrent(item, draft()) ? formatTime(item.audio!.duration) : 'Cần giọng đọc'}</small></span></button>}</For>
              </Show>
            </aside>
            <div class="studio-center"><StudioPreview draft={draft()} sceneId={sceneId()} disabled={locked()} onScene={chooseScene} onSubtitlePosition={y => subtitleField('y', y)} onOverlayPosition={(id, x, y) => editOverlay(id, { x, y })} />
              <div class="studio-readiness"><strong><Check size={15} /> Chuẩn bị xuất video</strong><Show when={problems().length} fallback={<p>Ảnh và giọng đọc đầy đủ. Video đạt thời lượng mục tiêu.</p>}><ul><For each={problems()}>{problem => <li>{problem}</li>}</For></ul></Show><Show when={lastExport()}><p class="studio-export-success">Đã lưu: {lastExport()} <button class="button button-quiet" onClick={() => void renderService.openExportFolder(draft().projectId, null).catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)))}>Mở thư mục</button></p></Show></div>
              <div class="studio-center-actions"><button class="button" disabled={locked() || !ttsReady() || !draft().sampleApproved || !draft().scenes.length} onClick={() => void voicesMissing()}><Volume2 size={15} /> Tạo / tiếp tục giọng đọc còn thiếu</button><Show when={scene()?.image && scene() && audioCurrent(scene()!, draft())}><button class="button" disabled={locked()} onClick={() => field('sampleApproved', true)}><Check size={15} /> {draft().sampleApproved ? 'Đã duyệt cảnh mẫu' : 'Duyệt cảnh mẫu'}</button></Show></div>
            </div>
            <aside class="studio-inspector"><fieldset disabled={locked()}>
              <details open class="studio-section"><summary>Thông tin video</summary><label>Tiêu đề<input value={draft().title} onInput={e => field('title', e.currentTarget.value)} /></label><label>Mô tả<textarea rows="3" value={draft().description} onInput={e => field('description', e.currentTarget.value)} /></label><div class="studio-form-row"><label>Ngôn ngữ<select value={draft().language} onChange={e => { update(d => ({ ...d, language: e.currentTarget.value as 'vi' | 'en', sampleApproved: false })); }}><option value="vi">Tiếng Việt</option><option value="en">English</option></select></label><label>Khung hình<select value={draft().aspectRatio} onChange={e => field('aspectRatio', e.currentTarget.value as '16:9' | '9:16')}><option>16:9</option><option>9:16</option></select></label></div><label>Thời lượng mục tiêu (phút)<input type="number" min="2" max="180" value={draft().targetMinutes} onChange={e => field('targetMinutes', Math.min(180, Math.max(2, Number(e.currentTarget.value) || 2)))} /></label><label>Phong cách ảnh chung<textarea rows="3" value={draft().stylePrompt} onInput={e => field('stylePrompt', e.currentTarget.value)} /></label><label>Giọng đọc<select value={voices().includes(draft().voice) ? draft().voice : ''} onChange={e => update(d => ({ ...d, voice: e.currentTarget.value, sampleApproved: false }))}><option value="">Chọn giọng đọc</option><For each={voices()}>{voice => <option value={voice}>{voice}</option>}</For></select></label><p class="studio-muted">{ttsMessage()}</p><button class="button button-quiet" onClick={() => void refreshTts()}><RefreshCw size={13} /> Kiểm tra lại giọng</button><label class="studio-check"><input type="checkbox" checked={draft().motion} onChange={e => field('motion', e.currentTarget.checked)} /> Zoom nhẹ trên ảnh</label></details>
              <details open class="studio-section"><summary>Dàn ý & kịch bản</summary><Show when={!draft().outline.length}><button class="button button-primary" onClick={() => void outline()}><Sparkles size={14} /> Lập dàn ý</button></Show><Show when={chosenChapter()}>{chapter => <><label>Tên chương<input value={chapter().title} onInput={e => update(d => ({ ...d, outline: d.outline.map(c => c.id === chapter().id ? { ...c, title: e.currentTarget.value } : c) }))} /></label><label>Nội dung chương<textarea rows="3" value={chapter().summary} onInput={e => update(d => ({ ...d, outline: d.outline.map(c => c.id === chapter().id ? { ...c, summary: e.currentTarget.value } : c) }))} /></label><button class="button button-quiet" disabled={locked()} onClick={e => { deleteChapterButton = e.currentTarget; setChapterToDelete(chapter().id); }}><Trash2 size={13} aria-hidden="true" /> Xóa chương</button></>}</Show><Show when={draft().outline.length && !draft().outlineApproved}><button class="button button-primary" onClick={() => field('outlineApproved', true)}><Check size={14} /> Duyệt dàn ý</button></Show><Show when={draft().outlineApproved}><div class="studio-button-row"><button class="button" onClick={() => void writeMissing()} disabled={draft().outline.every(c => draft().scenes.some(s => s.chapterId === c.id))}><Sparkles size={14} /> Viết các chương còn thiếu</button><Show when={chosenChapter() && !chapterScenes().length}><button class="button" onClick={() => void run('Đang viết chương…', () => writeChapter(chapterId()))}>Viết chương này</button></Show></div></Show></details>
              <Show when={scene()}>{current => <details open class="studio-section"><summary>Chỉnh sửa cảnh</summary><label>Tên cảnh<input value={current().title} onInput={e => editScene(current().id, { title: e.currentTarget.value })} /></label><label>Lời kể<textarea rows="7" value={current().narration} onInput={e => editScene(current().id, { narration: e.currentTarget.value })} /></label><label>Prompt ảnh của cảnh<textarea rows="4" value={current().imagePrompt} onInput={e => editScene(current().id, { imagePrompt: e.currentTarget.value })} /></label><div class="studio-button-row"><button class="button" onClick={() => void copyPrompt()}><Copy size={14} /> Sao chép prompt</button><button class="button" onClick={() => void importImage()}><ImagePlus size={14} /> Nhập ảnh từ Codex</button></div><details><summary class="studio-muted">Prompt đầy đủ để dán vào Codex</summary><textarea rows="5" readonly value={scenePrompt(draft(), current())} /></details><Show when={current().imageHistory.length}><label>Khôi phục ảnh trước<select value="" onChange={e => { const value = e.currentTarget.value; if (value) editScene(current().id, { image: value, imageHistory: [current().image, ...current().imageHistory].filter((p): p is string => !!p && p !== value).slice(0, 5) }); }}><option value="">Chọn bản ảnh…</option><For each={current().imageHistory}>{(path, i) => <option value={path}>Bản trước {i() + 1}</option>}</For></select></label></Show><button class="button button-primary" disabled={!ttsReady() || !current().narration.trim() || !draft().voice} onClick={() => void run('Đang tạo bản xem trước…', () => makeVoice(current().id))}><Volume2 size={14} /> {current().audio ? 'Tạo lại giọng cảnh này' : 'Tạo giọng & bản xem trước'}</button><Show when={current().audioHistory.length}><label>Khôi phục giọng trước<select value="" onChange={e => { const audio = current().audioHistory.find(a => a.path === e.currentTarget.value); if (audio) editScene(current().id, restoreAudio(current(), audio)); }}><option value="">Chọn bản giọng…</option><For each={current().audioHistory}>{(audio, i) => <option value={audio.path}>Bản {i() + 1} · {audio.voice}</option>}</For></select></label></Show><label>Yêu cầu AI chỉnh sửa<textarea rows="2" placeholder="Ví dụ: mở đầu hấp dẫn hơn, giữ nguyên dữ kiện…" value={instruction()} onInput={e => setInstruction(e.currentTarget.value)} /></label><button class="button" disabled={!instruction().trim()} onClick={() => void rewrite()}><Sparkles size={14} /> Chỉnh sửa bằng Codex</button><div class="studio-button-row"><button class="studio-icon-button" aria-label="Đưa cảnh lên" onClick={() => moveScene(-1)}><ArrowUp size={15} /></button><button class="studio-icon-button" aria-label="Đưa cảnh xuống" onClick={() => moveScene(1)}><ArrowDown size={15} /></button><button class="button button-quiet" onClick={() => { update(d => ({ ...d, scenes: d.scenes.filter(s => s.id !== current().id) })); setSceneId(chapterScenes()[0]?.id ?? ''); }}><Trash2 size={13} /> Xóa cảnh</button></div></details>}</Show>
              <details class="studio-section"><summary>Phụ đề</summary><label class="studio-check"><input type="checkbox" checked={draft().subtitle.enabled} onChange={e => subtitleField('enabled', e.currentTarget.checked)} /> Hiển thị phụ đề</label><label>Vị trí dọc<input type="range" min=".02" max=".96" step=".01" value={draft().subtitle.y} onInput={e => subtitleField('y', Number(e.currentTarget.value))} /></label><div class="studio-form-row"><label>Cỡ chữ<input type="number" min="12" max="120" value={draft().subtitle.fontSize} onChange={e => subtitleField('fontSize', Math.max(12, Math.min(120, Number(e.currentTarget.value) || 42)))} /></label><label>Màu chữ<input type="color" value={draft().subtitle.color} onInput={e => subtitleField('color', e.currentTarget.value)} /></label></div><label class="studio-check"><input type="checkbox" checked={draft().subtitle.background} onChange={e => subtitleField('background', e.currentTarget.checked)} /> Nền tối sau chữ</label><Show when={scene()?.audio}><Index each={scene()!.audio!.cues}>{(cue, i) => <div class="studio-cue"><div class="studio-form-row"><label>Bắt đầu (s)<input type="number" min="0" step=".01" value={cue().start} onChange={e => { const a = scene()!.audio!; editScene(sceneId(), { audio: { ...a, cues: a.cues.map((c, n) => n === i ? { ...c, start: Number(e.currentTarget.value) } : c) } }); }} /></label><label>Kết thúc (s)<input type="number" min="0" step=".01" value={cue().end} onChange={e => { const a = scene()!.audio!; editScene(sceneId(), { audio: { ...a, cues: a.cues.map((c, n) => n === i ? { ...c, end: Number(e.currentTarget.value) } : c) } }); }} /></label></div><textarea rows="2" aria-label={`Phụ đề đoạn ${i + 1}`} value={cue().text} onInput={e => { const a = scene()!.audio!; editScene(sceneId(), { audio: { ...a, cues: a.cues.map((c, n) => n === i ? { ...c, text: e.currentTarget.value } : c) } }); }} /></div>}</Index></Show></details>
              <details class="studio-section"><summary>Logo & text</summary><div class="studio-button-row"><button class="button" disabled={draft().overlays.length >= 20} onClick={() => void importImage(true)}><ImagePlus size={13} /> Thêm logo</button><button class="button" disabled={draft().overlays.length >= 20} onClick={addText}><Plus size={13} /> Thêm text</button></div><label>Đối tượng<select value={overlayId()} onChange={e => setOverlayId(e.currentTarget.value)}><option value="">Chọn đối tượng</option><For each={draft().overlays}>{(o, i) => <option value={o.id}>{o.kind === 'logo' ? 'Logo' : o.text.slice(0, 30)} · {i() + 1}</option>}</For></select></label><Show when={overlay()}>{item => <><Show when={item().kind === 'text'}><label>Nội dung<textarea rows="2" value={item().text} onInput={e => editOverlay(item().id, { text: e.currentTarget.value })} /></label><div class="studio-form-row"><label>Cỡ chữ<input type="number" min="12" max="160" value={item().fontSize} onChange={e => editOverlay(item().id, { fontSize: Math.min(160, Math.max(12, Number(e.currentTarget.value) || 48)) })} /></label><label>Màu<input type="color" value={item().color} onInput={e => editOverlay(item().id, { color: e.currentTarget.value })} /></label></div></Show><label>Vị trí ngang<input type="range" min="0" max=".98" step=".01" value={item().x} onInput={e => editOverlay(item().id, { x: Number(e.currentTarget.value) })} /></label><label>Vị trí dọc<input type="range" min="0" max=".96" step=".01" value={item().y} onInput={e => editOverlay(item().id, { y: Number(e.currentTarget.value) })} /></label><label>Độ rộng<input type="range" min=".02" max="1" step=".01" value={item().width} onInput={e => editOverlay(item().id, { width: Number(e.currentTarget.value) })} /></label><label>Độ trong suốt<input type="range" min="0" max="1" step=".01" value={item().opacity} onInput={e => editOverlay(item().id, { opacity: Number(e.currentTarget.value) })} /></label><div class="studio-form-row"><label>Hiện từ giây<input type="number" min="0" max="10799" value={item().start} onChange={e => editOverlay(item().id, { start: Number(e.currentTarget.value) })} /></label><label>Đến giây<input type="number" min="1" max="10800" value={item().end} onChange={e => editOverlay(item().id, { end: Number(e.currentTarget.value) })} /></label></div><button class="button button-quiet" onClick={() => { update(d => ({ ...d, overlays: d.overlays.filter(o => o.id !== item().id) })); setOverlayId(''); }}><Trash2 size={13} /> Xóa đối tượng</button></>}</Show></details>
            </fieldset></aside>
          </div>
        </>}>
          <section class="studio-discovery"><fieldset disabled={locked()} class="studio-topic-form"><label>Chủ đề hoặc ngách nội dung<textarea rows="3" placeholder="Ví dụ: Chiến tranh thế giới, những bí ẩn dưới đại dương…" value={draft().topic} onInput={e => field('topic', e.currentTarget.value)} /></label><div class="studio-form-row"><label>Ngôn ngữ nội dung<select value={draft().language} onChange={e => field('language', e.currentTarget.value as 'vi' | 'en')}><option value="vi">Tiếng Việt</option><option value="en">English</option></select></label><label>Thời lượng (phút)<input type="number" min="2" max="180" value={draft().targetMinutes} onChange={e => field('targetMinutes', Math.max(2, Math.min(180, Number(e.currentTarget.value) || 2)))} /></label></div><label>Hướng nội dung<select value={draft().contentMode} onChange={e => field('contentMode', e.currentTarget.value as 'factual' | 'fiction')}><option value="factual">Dữ kiện / lịch sử / giải thích</option><option value="fiction">Giả tưởng / giả định</option></select></label><button class="button button-primary" disabled={!draft().topic.trim()} onClick={() => void ideas()}><Sparkles size={16} /> {draft().ideas.length ? 'Tạo 5 ý tưởng khác' : 'Khám phá 5 ý tưởng'}</button><p class="studio-muted">Video từ 2 phút · Codex tạo nội dung · Bạn duyệt trước khi thực hiện.</p><Show when={draft().selectedIdeaId}><button class="button button-quiet" onClick={() => field('screen', 'editor')}>Tiếp tục ý tưởng đang làm</button></Show></fieldset>
            <div class="studio-ideas"><Show when={draft().ideas.length} fallback={<div class="studio-ideas-empty"><Sparkles size={38} /><h2>Một chủ đề, năm cách kể</h2><p>AI đề xuất 5 ý tưởng có tiêu đề, mô tả và câu mở đầu. Một ý tưởng được đánh sao kèm lý do đề xuất.</p></div>}><For each={draft().ideas}>{(idea, index) => <article class="studio-idea-card" classList={{ 'studio-recommended': idea.recommended }}><div class="studio-idea-meta"><span>Ý TƯỞNG {String(index() + 1).padStart(2, '0')}</span><Show when={idea.recommended}><span class="studio-recommendation"><Star size={13} fill="currentColor" /> AI đề xuất</span></Show></div><h2>{idea.title}</h2><p>{idea.description}</p><blockquote>{idea.hook}</blockquote><Show when={idea.recommended}><p class="studio-idea-reason">{idea.reason}</p></Show><button class="button" disabled={locked() || (!!draft().scenes.length && draft().selectedIdeaId !== idea.id)} title={draft().scenes.length ? 'Dự án đã có kịch bản. Tạo dự án mới để thực hiện ý tưởng khác.' : undefined} onClick={() => selectIdea(idea)}>Thực hiện ý tưởng <ArrowDown size={14} /></button></article>}</For></Show></div>
          </section>
        </Show>
      </Show>
    </Show>
    <Show when={pendingChapter()}>{chapter => <Modal
      title="Xóa chương?"
      description={`Bỏ chương “${chapter().title}” và ${draft().scenes.filter(s => s.chapterId === chapter().id).length} cảnh trong chương khỏi kịch bản. Bạn cần duyệt lại dàn ý sau khi xóa.`}
      danger onClose={closeChapterDeletion}
    ><footer class="modal-actions modal-actions-delete">
      <button class="button button-quiet" type="button" onClick={closeChapterDeletion}>Hủy</button>
      <button class="button button-danger" type="button" disabled={locked()} onClick={deleteChapter}>Xóa chương và các cảnh</button>
    </footer></Modal>}</Show>
  </main>;
}
