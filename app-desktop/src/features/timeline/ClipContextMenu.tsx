import { onCleanup, onMount } from 'solid-js';
import { currentProject, setActiveWorkspacePanelTab } from '../../app/project-state';
import { canPaste, commitEdit, copyTimelineClips, editBusy, pasteTimelineClips } from './edit-state';
import { selectedTimelineIds, setTimelinePlaying, timelineCursorUs } from './timeline-state';
import { timelineDurationUs } from './time';

export function openEffectsProperties() {
  setTimelinePlaying(false); setActiveWorkspacePanelTab('properties');
  requestAnimationFrame(() => document.getElementById('timeline-effects-properties')?.scrollIntoView({ block: 'nearest' }));
}
export default function ClipContextMenu(props: { x: number; y: number; restore?: HTMLElement; close(): void }) {
  let menu!: HTMLDivElement;
  const locked = () => !!currentProject()?.timeline.clips.some(c => selectedTimelineIds().includes(c.id) && currentProject()!.timeline.tracks.find(t => t.id === c.trackId)?.locked);
  const disabled = () => editBusy() || locked();
  const run = (action: () => unknown) => { props.close(); action(); props.restore?.focus(); };
  onMount(() => {
    const rect = menu.getBoundingClientRect();
    menu.style.left = `${Math.max(8, Math.min(props.x, window.innerWidth - rect.width - 8))}px`;
    menu.style.top = `${Math.max(8, Math.min(props.y, window.innerHeight - rect.height - 8))}px`;
    menu.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus();
    const outside = (e: PointerEvent) => { if (!menu.contains(e.target as Node)) props.close(); };
    const dismiss = () => props.close();
    document.addEventListener('pointerdown', outside, true); window.addEventListener('resize', dismiss);
    onCleanup(() => { document.removeEventListener('pointerdown', outside, true); window.removeEventListener('resize', dismiss); });
  });
  function keys(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === 'Escape') { e.preventDefault(); props.close(); props.restore?.focus(); return; }
    if (e.key === 'Tab') { props.close(); return; }
    const nodes = [...menu.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')], index = nodes.indexOf(document.activeElement as HTMLButtonElement);
    if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(e.key)) {
      e.preventDefault(); const next = e.key === 'Home' ? 0 : e.key === 'End' ? nodes.length - 1 : (index + (e.key === 'ArrowDown' ? 1 : -1) + nodes.length) % nodes.length;
      nodes[next]?.focus();
    }
  }
  return <div ref={menu} role="menu" aria-label="Thao tác clip" class="timeline-context-menu" onKeyDown={keys} onContextMenu={e => e.preventDefault()}>
    <button type="button" role="menuitem" disabled={editBusy()} onClick={() => run(openEffectsProperties)}>Hiệu ứng / keyframe <kbd>K</kbd></button>
    <button type="button" role="menuitem" disabled={disabled()} onClick={() => run(() => commitEdit({ kind: 'split', ids: selectedTimelineIds(), atUs: timelineCursorUs() }, 'Cắt clip'))}>Cắt tại playhead <kbd>S</kbd></button>
    <button type="button" role="menuitem" disabled={editBusy()} onClick={() => run(copyTimelineClips)}>Sao chép <kbd>Ctrl+C</kbd></button>
    <button type="button" role="menuitem" disabled={editBusy() || !canPaste()} onClick={() => run(() => pasteTimelineClips(timelineCursorUs()))}>Dán tại playhead <kbd>Ctrl+V</kbd></button>
    <button type="button" role="menuitem" disabled={disabled()} onClick={() => run(() => commitEdit({ kind: 'duplicate', ids: selectedTimelineIds(), atUs: timelineDurationUs(currentProject()!.timeline.clips) }, 'Nhân bản clip'))}>Nhân bản <kbd>Ctrl+D</kbd></button>
    <button type="button" role="menuitem" disabled={disabled()} onClick={() => run(() => commitEdit({ kind: 'delete', ids: selectedTimelineIds(), ripple: false }, 'Xóa clip'))}>Xóa <kbd>Delete</kbd></button>
    <button type="button" role="menuitem" disabled={disabled()} onClick={() => run(() => commitEdit({ kind: 'delete', ids: selectedTimelineIds(), ripple: true }, 'Xóa ripple'))}>Xóa và dồn khoảng trống <kbd>Shift+Delete</kbd></button>
  </div>;
}
