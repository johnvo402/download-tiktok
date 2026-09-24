import { createSignal, Show } from 'solid-js';
import type { Project } from './types';
import { projectService } from '../../app/services/project-service';
import { validateProjectName } from '../../app/lib/project-name';
import { setNotice } from '../../app/project-state';
import Modal from '../../shared/components/Modal';
import { FolderOpen, LoaderCircle, Trash2, X } from 'lucide-solid';
import { open as openDialog } from '@tauri-apps/plugin-dialog';

export type ProjectDialogMode = 'create' | 'rename' | 'delete';

export default function ProjectDialog(props: {
  mode: ProjectDialogMode;
  project?: Project;
  onClose: () => void;
  onCreated: (project: Project) => void;
  onRenamed: (project: Project) => void;
  onDeleted: (id: string) => void;
}) {
  const [name, setName] = createSignal(props.project?.name ?? '');
  const [error, setError] = createSignal('');
  const [pending, setPending] = createSignal(false);
  const [storageDirectory, setStorageDirectory] = createSignal<string | null>(null);

  const chooseStorageDirectory = async () => {
    try {
      const selected = await openDialog({ title: 'Chọn thư mục lưu dự án', directory: true, multiple: false });
      if (typeof selected === 'string') {
        setStorageDirectory(selected);
        setError('');
      }
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Không thể chọn thư mục lưu dự án.');
    }
  };

  const submit = async (event: SubmitEvent) => {
    event.preventDefault();
    if (props.mode === 'delete' || pending()) return;
    const validationMessage = validateProjectName(name());
    if (validationMessage) {
      setError(validationMessage);
      return;
    }
    setError('');
    setPending(true);
    try {
      if (props.mode === 'create') {
        const project = await projectService.create(name(), storageDirectory());
        props.onCreated(project);
      } else if (props.project) {
        const project = await projectService.rename(props.project.id, name());
        props.onRenamed(project);
      }
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : 'Không thể lưu dự án.');
    } finally {
      setPending(false);
    }
  };

  const remove = async () => {
    if (!props.project || pending()) return;
    setPending(true);
    try {
      await projectService.delete(props.project.id);
      props.onDeleted(props.project.id);
    } catch (reason) {
      setNotice({ kind: 'error', message: reason instanceof Error ? reason.message : 'Không thể xóa dự án.' });
      props.onClose();
    } finally {
      setPending(false);
    }
  };

  const isDelete = () => props.mode === 'delete';
  const title = () => props.mode === 'create' ? 'Tạo dự án mới' : props.mode === 'rename' ? 'Đổi tên dự án' : 'Xóa dự án';
  const description = () => props.mode === 'create'
    ? 'Chọn thư mục lưu nếu muốn. Ứng dụng tạo thư mục project riêng bên trong vị trí đó.'
    : props.mode === 'rename'
      ? 'Tên hiển thị có thể thay đổi mà không ảnh hưởng dữ liệu dự án.'
      : undefined;

  return (
    <Modal title={title()} description={description()} onClose={props.onClose} danger={isDelete()}>
      <Show when={!isDelete()} fallback={
        <div class="delete-confirmation">
          <div class="delete-warning-icon"><Trash2 size={21} /></div>
          <div class="delete-copy">
            <p class="delete-project-name">{props.project?.name}</p>
            <p>Thao tác này sẽ xóa vĩnh viễn toàn bộ dữ liệu của dự án, gồm video, phụ đề, bản dịch, âm thanh và các tệp đã xuất.</p>
            <p class="delete-irreversible">Không thể hoàn tác sau khi xác nhận.</p>
          </div>
        </div>
      }>
        <form class="modal-form" onSubmit={submit}>
          <label for="project-name">Tên dự án</label>
          <input
            id="project-name"
            class="text-input"
            type="text"
            maxlength="200"
            autocomplete="off"
            autofocus
            placeholder="Ví dụ: Video giới thiệu sản phẩm"
            value={name()}
            aria-invalid={!!error()}
            aria-describedby={error() ? 'project-name-error' : 'project-name-hint'}
            onInput={(event) => { setName(event.currentTarget.value); setError(''); }}
          />
          <div class="field-helper-row">
            <Show when={error()} fallback={<span id="project-name-hint">Tối đa 100 ký tự</span>}>
              <span id="project-name-error" class="field-error" role="alert">{error()}</span>
            </Show>
            <span class="field-count">{Array.from(name()).length}/100</span>
          </div>
          <Show when={props.mode === 'create'}>
            <div class="project-storage-picker">
              <div class="project-storage-heading"><span>Thư mục lưu trữ</span><small>{storageDirectory() ? 'TÙY CHỌN' : 'MẶC ĐỊNH ỨNG DỤNG'}</small></div>
              <div class="project-storage-location">
                <span class="project-storage-path" title={storageDirectory() ?? 'Dữ liệu ứng dụng / projects'}>{storageDirectory() ?? 'Dữ liệu ứng dụng / projects'}</span>
                <button class="button button-quiet" type="button" disabled={pending()} onClick={() => void chooseStorageDirectory()}><FolderOpen size={14} /> Chọn folder</button>
                <Show when={storageDirectory()}><button class="icon-button project-storage-reset" type="button" aria-label="Dùng thư mục mặc định" title="Dùng thư mục mặc định" onClick={() => setStorageDirectory(null)}><X size={14} /></button></Show>
              </div>
              <small class="project-storage-help">Dự án sẽ được tạo trong thư mục con có mã ID riêng.</small>
            </div>
          </Show>
          <footer class="modal-actions">
            <button class="button button-quiet" type="button" disabled={pending()} onClick={props.onClose}>Hủy</button>
            <button class="button button-primary" type="submit" disabled={pending()}>
              <Show when={pending()}><LoaderCircle class="spin" size={16} /></Show>
              {props.mode === 'create' ? 'Tạo dự án' : 'Lưu thay đổi'}
            </button>
          </footer>
        </form>
      </Show>
      <Show when={isDelete()}>
        <footer class="modal-actions modal-actions-delete">
          <button class="button button-quiet" type="button" disabled={pending()} onClick={props.onClose}>Hủy</button>
          <button class="button button-danger" type="button" disabled={pending()} onClick={remove}>
            <Show when={pending()}><LoaderCircle class="spin" size={16} /></Show>
            Xóa vĩnh viễn
          </button>
        </footer>
      </Show>
    </Modal>
  );
}
