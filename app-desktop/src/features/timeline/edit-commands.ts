import type { ClipAudio, ClipStyle, ClipTransform, TimelineClip, TimelineDocument, TimelineSettings, TimelineTrack } from './types';
import { clipAudio, validateAudio } from './audio-layout';
import { validateCanvas, validateStyle, validateTransform } from './visual-layout';
import { clipDurationUs, clipEndUs, MAX_TIME_US, rescaleTime, timelineToSourceUs } from './time';

export interface EditLayout {
  settings: TimelineSettings;
  clips: TimelineClip[];
  tracks: Array<Pick<TimelineTrack, 'id' | 'kind' | 'name' | 'order'>>;
  legacyBinding: TimelineDocument['legacyBinding'];
}
export type EditCommand =
  | { kind: 'audio'; id: string; audio: ClipAudio }
  | { kind: 'voice'; videoClipId: string }
  | { kind: 'extractAudio'; videoClipId: string }
  | { kind: 'detach'; id: string }
  | { kind: 'rate'; id: string; rate: TimelineClip['rate'] }
  | { kind: 'visual'; id: string; transform: ClipTransform; style?: ClipStyle; text?: string }
  | { kind: 'canvas'; settings: TimelineSettings }
  | { kind: 'text'; text: string; atUs: number }
  | { kind: 'caption'; videoClipId: string }
  | { kind: 'move'; ids: string[]; deltaUs: number; anchorId?: string; trackId?: string }
  | { kind: 'trim'; id: string; edge: 'start' | 'end'; atUs: number }
  | { kind: 'split'; ids: string[]; atUs: number }
  | { kind: 'delete'; ids: string[]; ripple: boolean }
  | { kind: 'duplicate'; ids: string[]; atUs: number }
  | { kind: 'reorder'; ids: string[]; direction: -1 | 1 }
  | { kind: 'paste'; clips: TimelineClip[]; atUs: number }
  | { kind: 'group' | 'ungroup' | 'link' | 'unlink'; ids: string[] }
  | { kind: 'add'; assetId: string; atUs: number | null }
  | { kind: 'track'; action: 'add'; trackKind: TimelineTrack['kind'] }
  | { kind: 'track'; action: 'reorder'; id: string; direction: -1 | 1 };
export const editLayout = (doc: TimelineDocument): EditLayout => structuredClone({ settings: doc.settings, clips: doc.clips, tracks: doc.tracks.map(({ id, kind, name, order }) => ({ id, kind, name, order })), legacyBinding: doc.legacyBinding });
export const sameLayout = (a: EditLayout, b: EditLayout) => JSON.stringify(a) === JSON.stringify(b);
function fail(message: string): never { throw new Error(message); }
const time = (value: number) => { if (!Number.isSafeInteger(value) || value < 0 || value > MAX_TIME_US) fail('Thời gian dựng không hợp lệ.'); return value; };
export function clipTrackKind(clip: TimelineClip): TimelineTrack['kind'] {
  return clip.content.kind === 'subtitles' ? 'subtitle' : ['audio', 'voice', 'sourceAudio'].includes(clip.content.kind) ? 'audio' : 'visual';
}
/** Groups and parent/dependent links form one move/delete/copy unit. */
export function editClosure(doc: Pick<TimelineDocument, 'clips'>, ids: string[]): Set<string> {
  const result = new Set(ids.filter(id => doc.clips.some(c => c.id === id)));
  let changed = true;
  while (changed) {
    changed = false;
    const groups = new Set(doc.clips.filter(c => result.has(c.id) && c.groupId).map(c => c.groupId));
    for (const c of doc.clips) {
      if (result.has(c.id) && c.linkedClipId && !result.has(c.linkedClipId)) { result.add(c.linkedClipId); changed = true; }
      if (!result.has(c.id) && (c.groupId && groups.has(c.groupId) || c.linkedClipId && result.has(c.linkedClipId))) { result.add(c.id); changed = true; }
    }
  }
  return result;
}
export function validateEdit(doc: TimelineDocument, next: EditLayout) {
  validateCanvas(next.settings);
  const ids = new Set<string>([...doc.assets, ...doc.videoContents].map(value => value.id)); const tracks = new Map(next.tracks.map(t => [t.id, t]));
  const orders = new Set<number>();
  for (const t of next.tracks) {
    if (ids.has(t.id) || orders.has(t.order) || !t.name.trim()) fail('Track hoặc thứ tự track không hợp lệ.');
    ids.add(t.id); orders.add(t.order);
  }
  for (const c of next.clips) {
    if (c.audio) { if (!['video', 'audio', 'voice', 'sourceAudio'].includes(c.content.kind)) fail('Clip không có âm thanh.'); validateAudio(c.audio); }
    validateTransform(c.transform); if (c.style) { if (!['text', 'subtitles'].includes(c.content.kind)) fail('Kiểu chữ chỉ thuộc lớp chữ/phụ đề.'); validateStyle(c.style); }
    if (c.content.kind === 'text' && (!c.content.text.trim() || c.content.text.length > 10000)) fail('Nội dung chữ phải từ 1 đến 10.000 ký tự.');
    if (ids.has(c.id)) fail('ID clip bị trùng.'); ids.add(c.id);
    time(c.startUs); time(c.sourceInUs); time(c.sourceOutUs); time(clipEndUs(c));
    const track = tracks.get(c.trackId); if (!track || track.kind !== clipTrackKind(c)) fail('Loại clip không phù hợp track đích.');
    const content = c.content;
    const assetId = 'assetId' in content ? content.assetId : 'videoContentId' in content ? doc.videoContents.find(v => v.id === content.videoContentId)?.assetId : undefined;
    const asset = doc.assets.find(a => a.id === assetId);
    if ('videoContentId' in content && !doc.videoContents.some(v => v.id === content.videoContentId && (!('assetId' in content) || v.assetId === content.assetId))) fail('Nội dung dịch không khớp nguồn video.');
    if (assetId && !asset) fail('Nguồn của clip không còn tồn tại.');
    if (asset && asset.kind !== 'image' && (asset.durationUs === null || c.sourceOutUs > asset.durationUs)) fail('Điểm trim vượt thời lượng nguồn.');
    if (c.linkedClipId) {
      const parent = next.clips.find(p => p.id === c.linkedClipId);
      if (!parent || parent.content.kind !== 'video' || content.kind === 'video') fail('Liên kết clip không hợp lệ.');
      if ('videoContentId' in content && content.videoContentId !== parent.content.videoContentId) fail('Phụ đề/giọng thuộc video khác.');
    }
  }
  for (const track of doc.tracks) {
    const incoming = tracks.get(track.id);
    if (incoming && incoming.kind !== track.kind) fail('Không thể đổi loại track.');
    if (track.locked && (JSON.stringify(incoming) !== JSON.stringify({ id: track.id, kind: track.kind, name: track.name, order: track.order })
      || JSON.stringify(doc.clips.filter(c => c.trackId === track.id)) !== JSON.stringify(next.clips.filter(c => c.trackId === track.id)))) fail(`Track “${track.name}” đang khóa.`);
  }
  for (const t of next.tracks) {
    const clips = next.clips.filter(c => c.trackId === t.id).sort((a, b) => a.startUs - b.startUs);
    if (clips.some((c, i) => i > 0 && clipEndUs(clips[i - 1]) > c.startUs)) fail('Clip chồng lấn trên cùng track. Chọn vị trí trống hoặc track khác.');
  }
}
function sourceAtBoundary(c: TimelineClip, at: number): number {
  return at === clipEndUs(c) ? c.sourceOutUs : timelineToSourceUs(c, at) ?? fail('Điểm cắt nằm ngoài clip.');
}
function alignedBoundary(c: TimelineClip, at: number): number {
  if (at === c.startUs || at === clipEndUs(c)) return at;
  let a = c.rate.numerator, b = c.rate.denominator;
  while (b) [a, b] = [b, a % b];
  const quantum = c.rate.denominator / a;
  return c.startUs + Math.round((at - c.startUs) / quantum) * quantum;
}
function slice(c: TimelineClip, start: number, end: number): TimelineClip {
  const result = { ...c, startUs: start, sourceInUs: sourceAtBoundary(c, start), sourceOutUs: sourceAtBoundary(c, end) };
  if (result.sourceOutUs <= result.sourceInUs) fail('Phần clip quá ngắn để cắt ở tốc độ hiện tại.');
  return result;
}
function freeTrack(doc: TimelineDocument, next: EditLayout, kind: TimelineTrack['kind'], start: number, duration: number, uuid: () => string, preferred?: string): string {
  const choices = [...next.tracks].sort((a, b) => Number(b.id === preferred) - Number(a.id === preferred));
  const available = choices.find(t => t.kind === kind && !doc.tracks.find(old => old.id === t.id)?.locked
    && !next.clips.some(c => c.trackId === t.id && c.startUs < start + duration && clipEndUs(c) > start));
  if (available) return available.id;
  const id = uuid(), order = Math.max(-1, ...next.tracks.map(t => t.order)) + 1;
  next.tracks.push({ id, order, kind, name: kind === 'visual' ? 'Lớp hình ảnh' : kind === 'audio' ? 'Âm thanh' : 'Phụ đề' });
  return id;
}
function repairBinding(next: EditLayout) {
  const binding = next.legacyBinding; if (!binding) return;
  if (binding.clipId && next.clips.some(c => c.id === binding.clipId)) return;
  binding.clipId = next.clips.find(c => c.content.kind === 'video' && c.content.videoContentId === binding.videoContentId)?.id ?? null;
}
function insertCopies(doc: TimelineDocument, next: EditLayout, originals: TimelineClip[], atUs: number, uuid: () => string) {
  if (!originals.length) fail('Chưa có clip để dán.');
  time(atUs);
  const origin = Math.min(...originals.map(c => c.startUs));
  const idMap = new Map(originals.map(c => [c.id, uuid()]));
  const groups = new Map<string, string>(); const destinations = new Map<string, string>();
  for (const c of originals) {
    const copy = structuredClone(c); copy.id = idMap.get(c.id)!; copy.startUs = time(atUs + c.startUs - origin);
    if (copy.groupId) { if (!groups.has(copy.groupId)) groups.set(copy.groupId, uuid()); copy.groupId = groups.get(copy.groupId)!; }
    copy.linkedClipId = copy.linkedClipId ? idMap.get(copy.linkedClipId) ?? null : null;
    // Keep all clips from a copied track together, including its gaps.
    if (!destinations.has(c.trackId)) {
      const sameTrack = originals.filter(other => other.trackId === c.trackId);
      const min = Math.min(...sameTrack.map(other => other.startUs));
      const max = Math.max(...sameTrack.map(clipEndUs));
      destinations.set(c.trackId, freeTrack(doc, next, clipTrackKind(c), atUs + min - origin, max - min, uuid, c.trackId));
    }
    copy.trackId = destinations.get(c.trackId)!; next.clips.push(copy);
  }
}
export function applyEdit(doc: TimelineDocument, command: EditCommand, uuid: () => string = () => crypto.randomUUID()): EditLayout {
  const next = editLayout(doc);
  if (command.kind === 'canvas') next.settings = structuredClone(command.settings);
  else if (command.kind === 'audio') {
    const c = next.clips.find(c => c.id === command.id) ?? fail('Clip không tồn tại.'); c.audio = structuredClone(command.audio);
  } else if (command.kind === 'detach') {
    const c = next.clips.find(c => c.id === command.id) ?? fail('Clip không tồn tại.');
    if (!['voice', 'sourceAudio', 'audio'].includes(c.content.kind)) fail('Chọn lớp âm thanh để tách liên kết.');
    c.linkedClipId = null; c.groupId = null;
  } else if (command.kind === 'rate') {
    const anchor = next.clips.find(c => c.id === command.id) ?? fail('Clip không tồn tại.');
    const ids = editClosure(doc, [anchor.id]);
    const origin = Math.min(...next.clips.filter(c => ids.has(c.id)).map(c => c.startUs));
    const old = anchor.rate, ratio = { numerator: old.numerator * command.rate.denominator, denominator: old.denominator * command.rate.numerator };
    for (const c of next.clips.filter(c => ids.has(c.id))) {
      c.startUs = origin + rescaleTime(c.startUs - origin, ratio);
      const n = c.rate.numerator * command.rate.numerator * old.denominator, d = c.rate.denominator * command.rate.denominator * old.numerator;
      if (!Number.isSafeInteger(n) || !Number.isSafeInteger(d)) fail('Tốc độ không hợp lệ.');
      let a = n, b = d; while (b) [a, b] = [b, a % b]; c.rate = { numerator: n / a, denominator: d / a };
    }
  } else if (command.kind === 'voice' || command.kind === 'extractAudio') {
    const parent = next.clips.find(c => c.id === command.videoClipId && c.content.kind === 'video') ?? fail('Chọn video trước.');
    if (parent.content.kind !== 'video') fail('Clip không phải video.');
    const video = parent.content;
    const audio = clipAudio(parent, doc), voice = command.kind === 'voice';
    if (voice && next.clips.some(c => c.content.kind === 'voice' && c.linkedClipId === parent.id)) fail('Video đã có lớp giọng liên kết.');
    if (!voice && next.clips.some(c => c.content.kind === 'sourceAudio' && c.linkedClipId === parent.id)) fail('Video đã có lớp audio gốc liên kết.');
    if (!voice && !doc.assets.find(a => a.id === video.assetId)?.metadata.audioStreams?.length) fail('Video này không có audio gốc.');
    parent.audio = { ...audio, ...(voice ? { voiceEnabled: false } : { muted: true }) };
    next.clips.push({ ...structuredClone(parent), id: uuid(), trackId: freeTrack(doc, next, 'audio', parent.startUs, clipDurationUs(parent), uuid),
      content: voice ? { kind: 'voice', videoContentId: video.videoContentId } : { kind: 'sourceAudio', assetId: video.assetId, videoContentId: video.videoContentId },
      linkedClipId: parent.id, groupId: null, audio: { ...audio, muted: false, volume: voice ? doc.videoContents.find(v => v.id === video.videoContentId)?.settings.dubbing.dubbedVolume ?? 1 : audio.volume } });
  }
  else if (command.kind === 'visual') {
    const c = next.clips.find(c => c.id === command.id) ?? fail('Clip không tồn tại.');
    if (['audio', 'voice', 'sourceAudio'].includes(c.content.kind)) fail('Clip âm thanh không có thuộc tính hình ảnh.');
    c.transform = structuredClone(command.transform);
    if (command.style) c.style = structuredClone(command.style);
    if (command.text !== undefined) { if (c.content.kind !== 'text') fail('Chỉ sửa chữ trên lớp chữ.'); c.content.text = command.text; }
  } else if (command.kind === 'text' || command.kind === 'caption') {
    const parent = command.kind === 'caption' ? doc.clips.find(c => c.id === command.videoClipId && c.content.kind === 'video') ?? fail('Chọn video để thêm lớp phụ đề.') : undefined;
    const start = parent?.startUs ?? (command.kind === 'text' ? time(command.atUs) : 0);
    const duration = parent ? clipDurationUs(parent) : 5e6;
    const content: TimelineClip['content'] = parent && parent.content.kind === 'video' ? { kind: 'subtitles', videoContentId: parent.content.videoContentId } : { kind: 'text', text: command.kind === 'text' ? command.text : '' };
    next.clips.push({ id: uuid(), trackId: freeTrack(doc, next, parent ? 'subtitle' : 'visual', start, duration, uuid), startUs: start, sourceInUs: parent?.sourceInUs ?? 0, sourceOutUs: parent?.sourceOutUs ?? duration, rate: parent?.rate ?? { numerator: 1, denominator: 1 }, content, linkedClipId: parent?.id ?? null, groupId: null,
      transform: { x: .5, y: parent ? .88 : .5, scaleX: 1, scaleY: 1, rotation: 0, opacity: 1, flipX: false, flipY: false, fit: 'contain', crop: { x: 0, y: 0, width: 1, height: 1 } } });
  } else if (command.kind === 'track') {
    if (command.action === 'add') next.tracks.push({ id: uuid(), kind: command.trackKind, name: command.trackKind === 'visual' ? 'Lớp hình ảnh' : command.trackKind === 'audio' ? 'Âm thanh' : 'Phụ đề', order: Math.max(-1, ...next.tracks.map(t => t.order)) + 1 });
    else {
      const ordered = [...next.tracks].sort((a, b) => a.order - b.order);
      const index = ordered.findIndex(t => t.id === command.id), other = ordered[index + command.direction];
      if (index < 0 || !other) fail('Track đã ở đầu/cuối danh sách.');
      [ordered[index].order, other.order] = [other.order, ordered[index].order];
    }
  } else if (command.kind === 'add') {
    const asset = doc.assets.find(a => a.id === command.assetId) ?? fail('Nguồn không tồn tại.');
    const kind = asset.kind === 'audio' ? 'audio' : 'visual';
    const duration = asset.kind === 'image' ? 5e6 : asset.durationUs ?? fail('Chưa có thời lượng nguồn.');
    const start = command.atUs ?? Math.max(0, ...next.clips.filter(c => c.content.kind === 'video').map(clipEndUs));
    const video = doc.videoContents.find(v => v.assetId === asset.id);
    const c: TimelineClip = { id: uuid(), trackId: freeTrack(doc, next, kind, start, duration, uuid), startUs: time(start), sourceInUs: 0, sourceOutUs: duration,
      rate: { numerator: 1, denominator: 1 }, content: asset.kind === 'video' ? { kind: 'video', assetId: asset.id, videoContentId: video?.id ?? fail('Thiếu nội dung video.') } : { kind: asset.kind, assetId: asset.id },
      linkedClipId: null, groupId: null, transform: { x: .5, y: .5, scaleX: 1, scaleY: 1, rotation: 0, opacity: 1, flipX: false, flipY: false, fit: 'contain', crop: { x: 0, y: 0, width: 1, height: 1 } } };
    next.clips.push(c);
    if (!next.legacyBinding && c.content.kind === 'video') next.legacyBinding = { assetId: asset.id, videoContentId: c.content.videoContentId, clipId: c.id };
  } else if (command.kind === 'paste') insertCopies(doc, next, command.clips, command.atUs, uuid);
  else if (command.kind === 'trim') {
    const requested = next.clips.find(c => c.id === command.id) ?? fail('Clip không tồn tại.');
    const dependentSpeech = requested.content.kind === 'voice' || requested.content.kind === 'subtitles';
    const c = requested.linkedClipId && dependentSpeech ? next.clips.find(p => p.id === requested.linkedClipId)! : requested;
    const requestedEdge = command.edge === 'start' ? requested.startUs : clipEndUs(requested);
    const parentEdge = command.edge === 'start' ? c.startUs : clipEndUs(c);
    const oldStart = c.startUs, oldEnd = clipEndUs(c), at = time(alignedBoundary(c, time(parentEdge + time(command.atUs) - requestedEdge)));
    const delta = at - (command.edge === 'start' ? oldStart : oldEnd);
    if (command.edge === 'start') { c.sourceInUs += Math.sign(delta) * rescaleTime(Math.abs(delta), c.rate); c.startUs = at; }
    else c.sourceOutUs += Math.sign(delta) * rescaleTime(Math.abs(delta), c.rate);
    time(c.sourceInUs); time(c.sourceOutUs); clipDurationUs(c);
    const end = clipEndUs(c);
    const parent = c.linkedClipId ? next.clips.find(p => p.id === c.linkedClipId) : undefined;
    if (parent && (c.startUs < parent.startUs || end > clipEndUs(parent))) fail('Lớp liên kết phải nằm trong thời lượng video.');
    for (const child of next.clips.filter(child => child.linkedClipId === c.id)) {
      const start = child.startUs, out = clipEndUs(child);
      const from = start === oldStart ? c.startUs : Math.max(start, c.startUs);
      const to = out === oldEnd ? end : Math.min(out, end);
      if (from >= to) fail('Trim loại bỏ toàn bộ lớp liên kết. Tách liên kết trước khi trim.');
      child.sourceInUs += Math.sign(from - start) * rescaleTime(Math.abs(from - start), child.rate);
      child.sourceOutUs += Math.sign(to - out) * rescaleTime(Math.abs(to - out), child.rate);
      child.startUs = from;
    }
  } else {
    const selected = editClosure(doc, command.ids);
    if (!selected.size) fail('Chọn clip trước khi dựng.');
    const clips = next.clips.filter(c => selected.has(c.id));
    if (command.kind === 'move') {
      if (!Number.isSafeInteger(command.deltaUs)) fail('Khoảng di chuyển không hợp lệ.');
      const anchor = clips.find(c => c.id === command.anchorId) ?? clips[0];
      const sourceTrack = anchor.trackId;
      for (const c of clips) { c.startUs = time(c.startUs + command.deltaUs); if (command.trackId && c.trackId === sourceTrack) c.trackId = command.trackId; }
    } else if (command.kind === 'reorder') {
      const anchor = next.clips.find(c => c.id === command.ids[0])!;
      const ordered = next.clips.filter(c => c.trackId === anchor.trackId).sort((a, b) => a.startUs - b.startUs);
      const index = ordered.findIndex(c => c.id === anchor.id), other = ordered[index + command.direction];
      if (!other) fail('Clip đã ở đầu/cuối track.');
      const neighbors = editClosure(doc, [other.id]);
      if ([...neighbors].some(id => selected.has(id))) fail('Các clip cùng nhóm không thể đổi thứ tự riêng. Tách nhóm trước.');
      const first = command.direction === 1 ? anchor : other, second = command.direction === 1 ? other : anchor;
      const gap = second.startUs - clipEndUs(first);
      const firstDelta = clipDurationUs(second) + gap, secondDelta = first.startUs - second.startUs;
      const firstIds = first.id === anchor.id ? selected : neighbors, secondIds = second.id === anchor.id ? selected : neighbors;
      next.clips.forEach(c => { if (firstIds.has(c.id)) c.startUs = time(c.startUs + firstDelta); else if (secondIds.has(c.id)) c.startUs = time(c.startUs + secondDelta); });
    } else if (command.kind === 'duplicate') insertCopies(doc, next, clips, command.atUs, uuid);
    else if (command.kind === 'group') { if (clips.length < 2) fail('Chọn ít nhất hai clip để nhóm.'); const group = uuid(); clips.forEach(c => { c.groupId = group; }); }
    else if (command.kind === 'ungroup') clips.forEach(c => { c.groupId = null; });
    else if (command.kind === 'link') {
      const parents = clips.filter(c => c.content.kind === 'video');
      if (parents.length !== 1 || clips.length < 2) fail('Chọn một video và lớp phụ thuộc để liên kết; nhiều video hãy dùng Nhóm.');
      const parent = parents[0];
      for (const c of clips.filter(c => c.id !== parent.id)) {
        if (c.startUs < parent.startUs || clipEndUs(c) > clipEndUs(parent)) fail('Lớp liên kết phải nằm trong khoảng thời gian video.');
        c.linkedClipId = parent.id;
      }
    } else if (command.kind === 'unlink') clips.forEach(c => { c.linkedClipId = null; });
    else if (command.kind === 'split') {
      const at = time(command.atUs), parents = new Map<string, string>(), groups = new Map<string, string>();
      const right: TimelineClip[] = [];
      for (const c of clips) if (c.startUs < at && at < clipEndUs(c)) {
        const cut = alignedBoundary(c, at);
        if (cut <= c.startUs || cut >= clipEndUs(c)) fail('Điểm cắt quá gần mép clip ở tốc độ hiện tại.');
        const copy = slice(c, cut, clipEndUs(c)); copy.id = uuid();
        c.sourceOutUs = sourceAtBoundary(c, cut); if (c.sourceOutUs <= c.sourceInUs) fail('Điểm cắt quá gần mép clip.');
        if (c.content.kind === 'video') parents.set(c.id, copy.id);
        if (c.groupId) { if (!groups.has(c.groupId)) groups.set(c.groupId, uuid()); copy.groupId = groups.get(c.groupId)!; }
        right.push(copy);
      }
      if (!right.length) fail('Playhead phải nằm bên trong clip cần cắt.');
      for (const c of [...right, ...clips.filter(c => c.startUs >= at)]) {
        if (c.linkedClipId && parents.has(c.linkedClipId)) c.linkedClipId = parents.get(c.linkedClipId)!;
        if (c.groupId && groups.has(c.groupId)) c.groupId = groups.get(c.groupId)!;
      }
      next.clips.push(...right);
    } else if (command.kind === 'delete') {
      next.clips = next.clips.filter(c => !selected.has(c.id));
      if (command.ripple) {
        const windows: Array<[number, number]> = [];
        for (const c of [...clips].sort((a, b) => a.startUs - b.startUs)) {
          const last = windows.at(-1); if (last && c.startUs <= last[1]) last[1] = Math.max(last[1], clipEndUs(c)); else windows.push([c.startUs, clipEndUs(c)]);
        }
        const removedBefore = (at: number) => windows.reduce((sum, [a, b]) => sum + Math.max(0, Math.min(b, at) - a), 0);
        const pieces = new Map<string, Array<{ clip: TimelineClip; start: number; end: number }>>();
        for (const c of next.clips) {
          let ranges: Array<[number, number]> = [[c.startUs, clipEndUs(c)]];
          for (const [a, b] of windows) ranges = ranges.flatMap(([start, end]) => a >= end || b <= start ? [[start, end] as [number, number]] : [[start, Math.min(end, a)], [Math.max(start, b), end]].filter(([s, e]) => s < e) as Array<[number, number]>);
          pieces.set(c.id, ranges.map(([start, end], i) => ({ start, end, clip: { ...slice(c, start, end), id: i ? uuid() : c.id, startUs: time(start - removedBefore(start)) } })));
        }
        for (const list of pieces.values()) for (const piece of list) if (piece.clip.linkedClipId) {
          const parent = pieces.get(piece.clip.linkedClipId)?.find(p => p.start <= piece.start && p.end >= piece.end);
          if (!parent) fail('Ripple không giữ được lớp liên kết. Tách liên kết hoặc xóa thường.');
          piece.clip.linkedClipId = parent.clip.id;
        }
        next.clips = [...pieces.values()].flatMap(list => list.map(p => p.clip));
      }
    }
  }
  repairBinding(next); validateEdit(doc, next); return next;
}
/** Snap any moving edge to non-moving edges, zero or playhead within a pixel tolerance. */
export function snapMove(doc: TimelineDocument, ids: string[], deltaUs: number, playheadUs: number, toleranceUs: number): number {
  const selected = editClosure(doc, ids), moving = doc.clips.filter(c => selected.has(c.id));
  const targets = [0, playheadUs, ...doc.clips.filter(c => !selected.has(c.id)).flatMap(c => [c.startUs, clipEndUs(c)])];
  let adjustment = toleranceUs + 1, result = deltaUs;
  for (const c of moving) for (const edge of [c.startUs, clipEndUs(c)]) for (const target of targets) {
    const diff = target - edge - deltaUs;
    if (Math.abs(diff) < adjustment && Math.abs(diff) <= toleranceUs) { adjustment = Math.abs(diff); result = deltaUs + diff; }
  }
  return result;
}
export function snapTime(doc: TimelineDocument, ignoredIds: string[], atUs: number, playheadUs: number, toleranceUs: number): number {
  const ignored = editClosure(doc, ignoredIds);
  const targets = [0, playheadUs, ...doc.clips.filter(c => !ignored.has(c.id)).flatMap(c => [c.startUs, clipEndUs(c)])];
  return targets.filter(t => Math.abs(t - atUs) <= toleranceUs).sort((a, b) => Math.abs(a - atUs) - Math.abs(b - atUs))[0] ?? atUs;
}
