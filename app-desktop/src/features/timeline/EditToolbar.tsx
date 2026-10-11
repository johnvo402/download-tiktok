import { createSignal, For, Show } from 'solid-js';
import { Undo2, Redo2, Scissors, Copy, ClipboardPaste, CopyPlus, Trash2, ListMinus, Group, Ungroup, Link, Unlink, Magnet, Settings2, ChevronLeft, ChevronRight } from 'lucide-solid';
import Modal from '../../shared/components/Modal';
import { currentProject, setActiveWorkspacePanelTab } from '../../app/project-state';
import { clipEndUs, secondsToTimeUs, timelineDurationUs } from './time';
import { seekTimeline, selectedTimelineIds, setTimelinePlaying } from './timeline-state';
import { canPaste, canRedo, canUndo, commitEdit, copyTimelineClips, editBusy, editNotice, pasteTimelineClips, redoLabel, redoTimeline, setSnapEnabled, snapEnabled, undoLabel, undoTimeline } from './edit-state';
import { clipTrackKind } from './edit-commands';
import { openEffectsProperties } from './ClipContextMenu';

export default function EditToolbar(props: { cursorUs: number }) {
  const [editing, setEditing] = createSignal<'move' | 'start' | 'end'>();
  const [at, setAt] = createSignal('0');
  const [target, setTarget] = createSignal('');
  const clip = () => currentProject()?.timeline.clips.find(c => c.id === selectedTimelineIds()[0]);
  const disabled = () => editBusy() || !selectedTimelineIds().length;
  function open(mode: 'move' | 'start' | 'end') { const c = clip(); if (!c) return; setAt(String((mode === 'end' ? clipEndUs(c) : c.startUs) / 1e6)); setTarget(c.trackId); setEditing(mode); }
  async function submit() {
    const c = clip(), mode = editing(); if (!c || !mode) return;
    try {
      const atUs = secondsToTimeUs(Number(at()));
      const done = mode === 'move' ? await commitEdit({ kind: 'move', ids: selectedTimelineIds(), anchorId: c.id, deltaUs: atUs - c.startUs, trackId: target() }, 'Di chuyển clip')
        : await commitEdit({ kind: 'trim', id: c.id, edge: mode, atUs }, mode === 'start' ? 'Trim đầu clip' : 'Trim cuối clip');
      if (done) setEditing(undefined);
    } catch (error) { editNotice(error); }
  }
  return <>
    <div class="timeline-edit-toolbar" role="group" aria-label="Thao tác dựng clip">
      <button type="button" disabled={!canUndo()} aria-label="Hoàn tác timeline" title={`Hoàn tác ${undoLabel() ?? ''} (Ctrl+Z)`} onClick={() => void undoTimeline()}><Undo2 size={15} /></button>
      <button type="button" disabled={!canRedo()} aria-label="Làm lại timeline" title={`Làm lại ${redoLabel() ?? ''} (Ctrl+Shift+Z)`} onClick={() => void redoTimeline()}><Redo2 size={15} /></button>
      <span class="timeline-tool-divider" />
      <button type="button" disabled={disabled()} aria-label="Cắt tại playhead" title="Cắt tại playhead (S)" onClick={() => void commitEdit({ kind: 'split', ids: selectedTimelineIds(), atUs: props.cursorUs }, 'Cắt clip')}><Scissors size={15} /></button>
      <button type="button" disabled={disabled()} aria-label="Sao chép clip" title="Sao chép (Ctrl+C)" onClick={copyTimelineClips}><Copy size={15} /></button>
      <button type="button" disabled={editBusy() || !canPaste()} aria-label="Dán tại playhead" title="Dán tại playhead (Ctrl+V)" onClick={() => void pasteTimelineClips(props.cursorUs)}><ClipboardPaste size={15} /></button>
      <button type="button" disabled={disabled()} aria-label="Nhân bản clip" title="Nhân bản vào cuối dự án (Ctrl+D)" onClick={() => void commitEdit({ kind: 'duplicate', ids: selectedTimelineIds(), atUs: timelineDurationUs(currentProject()!.timeline.clips) }, 'Nhân bản clip')}><CopyPlus size={15} /></button>
      <button type="button" disabled={disabled()} aria-label="Xóa clip" title="Xóa clip (Delete)" onClick={() => void commitEdit({ kind: 'delete', ids: selectedTimelineIds(), ripple: false }, 'Xóa clip')}><Trash2 size={15} /></button>
      <button type="button" disabled={disabled()} aria-label="Xóa và dồn khoảng trống" title="Xóa khoảng thời gian trên toàn bộ track và dồn phần sau (Shift+Delete)" onClick={() => void commitEdit({ kind: 'delete', ids: selectedTimelineIds(), ripple: true }, 'Xóa ripple')}><ListMinus size={15} /></button>
      <span class="timeline-tool-divider" />
      <button type="button" disabled={disabled()} aria-label="Đưa clip ra trước" title="Đổi thứ tự với clip trước trên cùng track" onClick={() => void commitEdit({ kind: 'reorder', ids: selectedTimelineIds(), direction: -1 }, 'Đổi thứ tự clip')}><ChevronLeft size={15} /></button>
      <button type="button" disabled={disabled()} aria-label="Đưa clip ra sau" title="Đổi thứ tự với clip sau trên cùng track" onClick={() => void commitEdit({ kind: 'reorder', ids: selectedTimelineIds(), direction: 1 }, 'Đổi thứ tự clip')}><ChevronRight size={15} /></button>
      <button type="button" disabled={disabled() || selectedTimelineIds().length < 2} aria-label="Nhóm clip" title="Nhóm clip (Ctrl+G)" onClick={() => void commitEdit({ kind: 'group', ids: selectedTimelineIds() }, 'Nhóm clip')}><Group size={15} /></button>
      <button type="button" disabled={disabled()} aria-label="Tách nhóm clip" title="Tách nhóm (Ctrl+Shift+G)" onClick={() => void commitEdit({ kind: 'ungroup', ids: selectedTimelineIds() }, 'Tách nhóm clip')}><Ungroup size={15} /></button>
      <button type="button" disabled={disabled() || selectedTimelineIds().length < 2} aria-label="Liên kết lớp với video" title="Chọn một video và các lớp để liên kết" onClick={() => void commitEdit({ kind: 'link', ids: selectedTimelineIds() }, 'Liên kết clip')}><Link size={15} /></button>
      <button type="button" disabled={disabled()} aria-label="Tách liên kết clip" title="Tách lớp phụ thuộc khỏi video" onClick={() => void commitEdit({ kind: 'unlink', ids: selectedTimelineIds() }, 'Tách liên kết clip')}><Unlink size={15} /></button>
      <span class="timeline-tool-divider" />
      <button type="button" aria-label="Hít mép clip" aria-pressed={snapEnabled()} title="Hít mép clip/playhead (N); giữ Alt trong lúc kéo để bỏ hít" onClick={() => setSnapEnabled(v => !v)}><Magnet size={15} /></button>
      <button type="button" disabled={disabled()} aria-label="Di chuyển clip bằng thời gian" title="Nhập thời điểm và chọn track" onClick={() => open('move')}><Settings2 size={15} /></button>
      <button type="button" disabled={disabled()} aria-label="Trim đầu bằng thời gian" onClick={() => open('start')}>Trim đầu</button>
      <button type="button" disabled={disabled()} aria-label="Trim cuối bằng thời gian" onClick={() => open('end')}>Trim cuối</button>
      <label class="timeline-add-track"><span>Track</span><select aria-label="Thêm track" value="" disabled={editBusy()} onChange={event => { const kind = event.currentTarget.value as 'visual' | 'audio' | 'subtitle'; if (kind) void commitEdit({ kind: 'track', action: 'add', trackKind: kind }, 'Thêm track'); event.currentTarget.value = ''; }}><option value="">+ Thêm</option><option value="visual">Hình ảnh</option><option value="audio">Âm thanh</option><option value="subtitle">Phụ đề</option></select></label>
      <span class="timeline-selection-count">{selectedTimelineIds().length} chọn{editBusy() ? ' · Đang lưu…' : ''}</span>
      <button type="button" disabled={editBusy()} onClick={() => { setTimelinePlaying(false); seekTimeline(props.cursorUs); setActiveWorkspacePanelTab('properties'); }}>Hình ảnh / khung hình</button>
      <button type="button" disabled={disabled()} title="Fade hình, chuyển cảnh và keyframe (K)" onClick={openEffectsProperties}>Hiệu ứng / keyframe</button>
      <button type="button" disabled={editBusy()} onClick={() => void commitEdit({ kind: 'text', text: 'Nội dung chữ', atUs: props.cursorUs }, 'Thêm lớp chữ')}>+ Chữ</button>
      <button type="button" disabled={disabled() || clip()?.content.kind !== 'video'} onClick={() => void commitEdit({ kind: 'caption', videoClipId: clip()!.id }, 'Thêm phụ đề liên kết')}>+ Phụ đề</button>
      <button type="button" disabled={disabled() || clip()?.content.kind !== 'video'} onClick={() => void commitEdit({ kind: 'voice', videoClipId: clip()!.id }, 'Thêm lớp giọng liên kết')}>+ Giọng</button>
      <button type="button" disabled={disabled()} onClick={() => { setTimelinePlaying(false); setActiveWorkspacePanelTab('properties'); }}>Âm thanh</button>
    </div>
    <Show when={editing()}><Modal title={editing() === 'move' ? 'Di chuyển clip' : editing() === 'start' ? 'Trim đầu clip' : 'Trim cuối clip'} onClose={() => { if (!editBusy()) setEditing(undefined); }}>
      <form class="timeline-edit-form" onSubmit={event => { event.preventDefault(); void submit(); }}>
        <p>Thời gian theo dự án. Clip liên kết đi theo video; nhóm được di chuyển cùng nhau.</p>
        <label>Thời điểm (giây)<input aria-label="Thời điểm dựng (giây)" type="number" min="0" step="0.001" required value={at()} disabled={editBusy()} onInput={event => setAt(event.currentTarget.value)} /></label>
        <Show when={editing() === 'move'}><label>Track đích<select aria-label="Track đích" value={target()} disabled={editBusy()} onChange={event => setTarget(event.currentTarget.value)}><For each={currentProject()?.timeline.tracks.filter(t => clip() && t.kind === clipTrackKind(clip()!) && !t.locked)}>{track => <option value={track.id}>{track.name}</option>}</For></select></label></Show>
        <div class="dialog-actions"><button type="button" class="button button-quiet" disabled={editBusy()} onClick={() => setEditing(undefined)}>Hủy</button><button type="submit" class="button button-primary" disabled={editBusy()}>Áp dụng</button></div>
      </form>
    </Modal></Show>
  </>;
}
