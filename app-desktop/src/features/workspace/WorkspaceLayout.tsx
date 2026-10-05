import { flushPendingSaves } from '../../app/save-guard';
import { A, useLocation, useNavigate, useParams } from '@solidjs/router';
import { createEffect, createSignal, Show, onCleanup } from 'solid-js';
import {
  ArrowLeft, AudioLines, ChevronRight, CircleHelp, PanelLeftClose,
  PanelLeftOpen, Settings2, Merge, SlidersHorizontal,
  Download, Layers3,
} from 'lucide-solid';
import { currentProject, projectSaveState, setAppStartupReady, setCurrentProject, setNotice } from '../../app/project-state';
import { projectService } from '../../app/services/project-service';
import TranslationView from './translation/TranslationView';
import TranslationPanel from './translation/TranslationPanel';
import VideoToolsView from './VideoToolsView';
import ExportDialog from './ExportDialog';
import ProjectSettingsDialog from './ProjectSettingsDialog';
import BatchVideoDialog from './batch/BatchVideoDialog';
import jcAiLogo from '../../shared/assets/jc-ai-logo.png';

type Section = 'translate' | 'download' | 'merge';

export default function WorkspaceLayout() {
  const params = useParams<{ projectId: string; section: string }>();
  const location = useLocation();
  const navigate = useNavigate();
  const [collapsed, setCollapsed] = createSignal(false);
  const [previewRatio, setPreviewRatio] = createSignal(58);
  const [resizing, setResizing] = createSignal(false);
  const [loading, setLoading] = createSignal(false);
  const [exportOpen, setExportOpen] = createSignal(false);
  const [projectSettingsOpen, setProjectSettingsOpen] = createSignal(false);
  const [batchOpen, setBatchOpen] = createSignal(false);

  createEffect(() => {
    const id = params.projectId;
    if (!id || currentProject()?.id === id) {
      setAppStartupReady(true);
      return;
    }
    let active = true;
    setLoading(true);
    void projectService.get(id).then((project) => {
      if (active) setCurrentProject(project);
    }).catch((error: unknown) => {
      if (!active) return;
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể mở dự án.' });
      navigate('/');
    }).finally(() => {
      if (!active) return;
      setLoading(false);
      setAppStartupReady(true);
    });
    onCleanup(() => { active = false; });
  });

  const section = (): Section => {
    if (params.section === 'download' || params.section === 'import') return 'download';
    return params.section === 'merge' ? 'merge' : 'translate';
  };
  const leave = async (path: string) => {
    try { await flushPendingSaves(); navigate(path); }
    catch (error) { setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Chưa lưu được thay đổi.' }); }
  };
  const setSection = (next: Section) => {
    if (!params.projectId) return;
    void leave(`/project/${params.projectId}/${next}`);
  };

  const startResize = (event: PointerEvent) => {
    event.preventDefault();
    const container = (event.currentTarget as HTMLElement).parentElement;
    if (!container) return;
    const bounds = container.getBoundingClientRect();
    setResizing(true);
    document.body.classList.add('resizing-columns');
    const move = (moveEvent: PointerEvent) => {
      const percentage = ((moveEvent.clientX - bounds.left) / bounds.width) * 100;
      setPreviewRatio(Math.min(70, Math.max(38, percentage)));
    };
    const finish = () => {
      setResizing(false);
      document.body.classList.remove('resizing-columns');
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', finish);
      window.removeEventListener('pointercancel', finish);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', finish);
    window.addEventListener('pointercancel', finish);
  };

  const adjustResize = (event: KeyboardEvent) => {
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
    event.preventDefault();
    setPreviewRatio((value) => Math.min(70, Math.max(38, value + (event.key === 'ArrowRight' ? 2 : -2))));
  };

  return (
    <div class="workspace-shell" classList={{ 'workspace-sidebar-collapsed': collapsed(), 'workspace-is-resizing': resizing() }}>
      <aside class="workspace-sidebar">
        <div class="workspace-brand-row">
          <div class="workspace-brand-icon" aria-hidden="true"><img src={jcAiLogo} alt="" /></div>
          <Show when={!collapsed()}><span>JC AI</span></Show>
        </div>
        <div class="sidebar-project-label">DỰ ÁN</div>
        <div class="sidebar-project-row">
          <span class="sidebar-project-avatar" aria-hidden="true">{currentProject()?.name.slice(0, 1).toLocaleUpperCase('vi') ?? '…'}</span>
          <Show when={!collapsed()}>
            <div class="sidebar-project-copy"><span>{currentProject()?.name ?? (loading() ? 'Đang mở dự án...' : 'Dự án')}</span><small>Không gian làm việc</small></div>
            <ChevronRight size={14} class="sidebar-project-chevron" />
          </Show>
        </div>
        <div class="sidebar-group-label">WORKSPACE</div>
        <nav class="workspace-nav" aria-label="Điều hướng dự án">
          <button class="workspace-nav-item" classList={{ active: section() === 'download' }} type="button" aria-label="Tải video" title={collapsed() ? 'Tải video' : undefined} aria-current={section() === 'download' ? 'page' : undefined} onClick={() => setSection('download')}>
            <Download size={18} strokeWidth={1.8} aria-hidden="true" /><Show when={!collapsed()}><span>Tải video</span></Show>
          </button>
          <button class="workspace-nav-item" classList={{ active: section() === 'merge' }} type="button" aria-label="Gộp Video" title={collapsed() ? 'Gộp Video' : undefined} aria-current={section() === 'merge' ? 'page' : undefined} onClick={() => setSection('merge')}>
            <Merge size={18} strokeWidth={1.8} aria-hidden="true" /><Show when={!collapsed()}><span>Gộp Video</span></Show>
          </button>
          <button class="workspace-nav-item" classList={{ active: section() === 'translate' }} type="button" title={collapsed() ? 'Dịch' : undefined} aria-current={section() === 'translate' ? 'page' : undefined} onClick={() => setSection('translate')}>
            <AudioLines size={18} strokeWidth={1.8} /><Show when={!collapsed()}><span>Dịch</span></Show>
          </button>
        </nav>
        <div class="sidebar-bottom">
          <button class="sidebar-utility" type="button" title={collapsed() ? 'Trợ giúp' : undefined} onClick={() => setNotice({ kind: 'info', message: 'Trợ giúp sẽ có trong phiên bản sau.' })}>
            <CircleHelp size={17} /><Show when={!collapsed()}><span>Trợ giúp</span></Show>
          </button>
          <button class="sidebar-utility sidebar-collapse" type="button" aria-label={collapsed() ? 'Mở rộng thanh bên' : 'Thu gọn thanh bên'} onClick={() => setCollapsed(!collapsed())}>
            {collapsed() ? <PanelLeftOpen size={17} /> : <PanelLeftClose size={17} />}<Show when={!collapsed()}><span>Thu gọn</span></Show>
          </button>
        </div>
      </aside>

      <div class="workspace-main">
        <header class="workspace-topbar">
          <div class="workspace-breadcrumbs">
            <A class="back-to-projects" href="/" onClick={event => { event.preventDefault(); void leave('/'); }}><ArrowLeft size={15} /><span>Dự án</span></A>
            <span class="breadcrumb-divider">/</span>
            <strong>{currentProject()?.name ?? (loading() ? 'Đang mở...' : 'Dự án')}</strong>
          </div>
          <div class="workspace-topbar-right">
            <button class="button button-quiet workspace-settings-button" type="button" onClick={() => setProjectSettingsOpen(true)}><SlidersHorizontal size={14} /> Cài đặt dự án</button>
            <button class="button button-quiet workspace-settings-button" type="button" onClick={() => void leave(`/settings?returnTo=${encodeURIComponent(location.pathname)}`)}><Settings2 size={14} /> Cài đặt ứng dụng</button>
            <Show when={currentProject()}><button class="button button-quiet workspace-batch-button" type="button" aria-label="Dịch hàng loạt video" onClick={() => setBatchOpen(true)} title="Thêm video và xử lý lần lượt bằng cấu hình mẫu"><Layers3 size={14} /> Dịch hàng loạt</button></Show>
            <Show when={currentProject()?.source.videoPath}>
              <button class="button button-primary workspace-export-button" type="button" onClick={() => setExportOpen(true)}><Download size={14} /> Export</button>
            </Show>
            <span class="save-indicator" classList={{ 'save-indicator-error': projectSaveState() === 'error', 'save-indicator-pending': projectSaveState() === 'saving' }}>
              <span class="status-pip" />{projectSaveState() === 'saved' ? 'Đã lưu' : projectSaveState() === 'saving' ? 'Đang lưu…' : 'Chưa lưu được'}
            </span>
            <span class="topbar-separator" />
            <span class="workspace-state-pill">BẢN NHÁP</span>
          </div>
        </header>

        <Show when={section() === 'translate'}>
          <main class="workspace-content translation-layout" style={{ '--preview-ratio': `${previewRatio()}fr`, '--panel-ratio': `${100 - previewRatio()}fr` }}>
            <TranslationView />
            <div
              class="column-resize-handle"
              role="separator"
              aria-label="Thay đổi kích thước khu vực xem trước và bảng điều khiển"
              aria-orientation="vertical"
              aria-valuemin="38"
              aria-valuemax="70"
              aria-valuenow={Math.round(previewRatio())}
              tabindex="0"
              onPointerDown={startResize}
              onKeyDown={adjustResize}
            ><span /></div>
            <div class="workspace-right-panel"><TranslationPanel /></div>
          </main>
        </Show>
        <Show when={section() === 'download' || section() === 'merge'}>
          <VideoToolsView workflow={section() === 'merge' ? 'merge' : 'tiktok'} />
        </Show>
      </div>
      <Show when={exportOpen()}><ExportDialog onClose={() => setExportOpen(false)} /></Show>
      <Show when={projectSettingsOpen() && currentProject()}><ProjectSettingsDialog project={currentProject()!} onClose={() => setProjectSettingsOpen(false)} /></Show>
      <Show when={batchOpen() && currentProject()}>{(project) => <BatchVideoDialog project={project()} onClose={() => setBatchOpen(false)} />}</Show>
    </div>
  );
}
