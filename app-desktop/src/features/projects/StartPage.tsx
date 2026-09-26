import { createMemo, createSignal, For, onMount, onCleanup, Show } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import {
  ArrowDownWideNarrow, ArrowUpRight, Clock3, FileVideo2, FolderOpen, FolderPlus,
  LoaderCircle, MoreHorizontal, Search, Sparkles, X, ArrowUpWideNarrow,
  Check, Layers3, ListChecks, Settings2,
} from 'lucide-solid';
import Brand from '../../shared/components/Brand';
import ProjectDialog, { type ProjectDialogMode } from './ProjectDialog';
import { projectService } from '../../app/services/project-service';
import {
  applyProjectList, currentProject, projects, projectsLoading, projectLoadWarnings,
  setAppStartupReady, setCurrentProject, setNotice, setProjects, setProjectsLoading,
} from '../../app/project-state';
import type { Project } from './types';
import BatchProcessDialog from './BatchProcessDialog';
import { runtimeService, type RuntimeHealth } from '../../app/services/runtime-service';

export default function StartPage() {
  const navigate = useNavigate();
  const [query, setQuery] = createSignal('');
  const [dialogMode, setDialogMode] = createSignal<ProjectDialogMode>();
  const [dialogProject, setDialogProject] = createSignal<Project>();
  const [openMenu, setOpenMenu] = createSignal<string>();
  const [openingId, setOpeningId] = createSignal<string>();
  const [newestFirst, setNewestFirst] = createSignal(true);
  const [selectionMode, setSelectionMode] = createSignal(false);
  const [selectedProjectIds, setSelectedProjectIds] = createSignal<string[]>([]);
  const [batchOpen, setBatchOpen] = createSignal(false);
  const [runtimeHealth, setRuntimeHealth] = createSignal<RuntimeHealth>();
  let searchInput!: HTMLInputElement;

  const refresh = async () => {
    setProjectsLoading(true);
    try {
      applyProjectList(await projectService.list());
    } catch (error) {
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể đọc danh sách dự án.' });
    } finally {
      setProjectsLoading(false);
      setAppStartupReady(true);
    }
  };

  onMount(() => {
    void refresh();
    void runtimeService.health().then(setRuntimeHealth).catch(() => undefined);
  });

  const filteredProjects = createMemo(() => {
    const needle = query().trim().toLocaleLowerCase('vi');
    return [...projects()]
      .filter((project) => project.name.toLocaleLowerCase('vi').includes(needle))
      .sort((a, b) => newestFirst() ? b.updatedAt.localeCompare(a.updatedAt) : a.updatedAt.localeCompare(b.updatedAt));
  });

  const openProject = async (project: Project) => {
    if (selectionMode()) {
      setSelectedProjectIds((items) => items.includes(project.id) ? items.filter((id) => id !== project.id) : [...items, project.id]);
      return;
    }
    if (openingId()) return;
    setOpeningId(project.id);
    setOpenMenu(undefined);
    try {
      const loaded = await projectService.get(project.id);
      setCurrentProject(loaded);
      navigate(`/project/${loaded.id}/translate`);
    } catch (error) {
      setNotice({ kind: 'error', message: error instanceof Error ? error.message : 'Không thể mở dự án.' });
      void refresh();
    } finally {
      setOpeningId(undefined);
    }
  };

  const created = (project: Project) => {
    setProjects((items) => [project, ...items]);
    setDialogMode(undefined);
    setCurrentProject(project);
    navigate(`/project/${project.id}/translate`);
  };

  const renamed = (project: Project) => {
    setProjects((items) => items.map((item) => item.id === project.id ? project : item));
    if (currentProject()?.id === project.id) setCurrentProject(project);
    setDialogMode(undefined);
    setNotice({ kind: 'success', message: 'Đã cập nhật tên dự án.' });
  };

  const deleted = (id: string) => {
    setProjects((items) => items.filter((item) => item.id !== id));
    setSelectedProjectIds((items) => items.filter((item) => item !== id));
    if (currentProject()?.id === id) {
      setCurrentProject(undefined);
      navigate('/');
    }
    setDialogMode(undefined);
    setDialogProject(undefined);
    setNotice({ kind: 'success', message: 'Đã xóa dự án và dữ liệu của dự án.' });
  };

  const showDialog = (mode: ProjectDialogMode, project?: Project) => {
    setOpenMenu(undefined);
    setDialogProject(project);
    setDialogMode(mode);
  };

  const toggleSelectAll = () => {
    const visible = filteredProjects().map((project) => project.id);
    const everyVisibleSelected = visible.length > 0 && visible.every((id) => selectedProjectIds().includes(id));
    setSelectedProjectIds((items) => everyVisibleSelected
      ? items.filter((id) => !visible.includes(id))
      : [...new Set([...items, ...visible])]);
  };

  const updatedLabel = (date: string) => {
    const time = new Date(date).getTime();
    if (!Number.isFinite(time)) return 'Chưa rõ thời gian';
    const elapsed = Math.max(0, Date.now() - time);
    const minutes = Math.floor(elapsed / 60_000);
    if (minutes < 1) return 'Vừa cập nhật';
    if (minutes < 60) return `${minutes} phút trước`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `${hours} giờ trước`;
    const days = Math.floor(hours / 24);
    if (days < 7) return `${days} ngày trước`;
    return new Intl.DateTimeFormat('vi-VN', { dateStyle: 'medium' }).format(new Date(date));
  };

  const createdLabel = (date: string) => {
    const parsed = new Date(date);
    return Number.isNaN(parsed.getTime()) ? 'Ngày tạo chưa rõ' : `Tạo ${new Intl.DateTimeFormat('vi-VN', { day: '2-digit', month: 'short', year: 'numeric' }).format(parsed)}`;
  };

  onMount(() => {
    const handler = (event: MouseEvent) => {
      if (!(event.target instanceof Element) || !event.target.closest('[data-project-menu]')) setOpenMenu(undefined);
    };
    document.addEventListener('pointerdown', handler);
    onCleanup(() => document.removeEventListener('pointerdown', handler));
  });

  onMount(() => {
    const handler = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {
        event.preventDefault();
        searchInput?.focus();
      }
    };
    document.addEventListener('keydown', handler);
    onCleanup(() => document.removeEventListener('keydown', handler));
  });

  return (
    <main class="start-screen">
      <div class="start-glow start-glow-one" />
      <div class="start-glow start-glow-two" />
      <header class="start-header">
        <Brand />
        <div class="start-header-actions">
          <button class="button button-quiet start-settings-button" type="button" onClick={() => navigate('/settings?returnTo=%2F')}><Settings2 size={15} /> Cài đặt</button>
          <div class="header-context"><span class="status-pip" /> KHÔNG GIAN LÀM VIỆC CỤC BỘ</div>
        </div>
      </header>

      <div class="start-content">
        <Show when={runtimeHealth() && (!runtimeHealth()!.projectStorageAvailable || !runtimeHealth()!.ffmpegAvailable || !runtimeHealth()!.ffprobeAvailable || !runtimeHealth()!.workerAvailable)}>
          <div class="runtime-warning" role="status"><strong>Một số thành phần ứng dụng chưa sẵn sàng</strong><span>{!runtimeHealth()!.projectStorageAvailable ? 'Không ghi được thư mục dữ liệu. ' : ''}{!runtimeHealth()!.ffmpegAvailable || !runtimeHealth()!.ffprobeAvailable ? 'Thiếu FFmpeg/FFprobe. ' : ''}{!runtimeHealth()!.workerAvailable ? 'Thiếu AI Worker.' : ''} Mở Cài đặt → Nâng cao để xem trạng thái.</span></div>
        </Show>
        <section class="welcome-block">
          <div class="eyebrow"><Sparkles size={14} /> VIDEO, MỌI NGÔN NGỮ</div>
          <div class="welcome-heading-row">
            <div>
              <h1>Project Manager<span class="heading-period">.</span></h1>
              <p class="welcome-subtitle">Chọn nơi bạn muốn tiếp tục — hoặc bắt đầu một dự án mới.</p>
            </div>
            <button class="button button-primary create-project-button" type="button" onClick={() => showDialog('create')}>
              <FolderPlus size={17} /> Tạo dự án mới
            </button>
          </div>
        </section>

        <section class="projects-section" aria-labelledby="projects-heading">
          <div class="projects-toolbar">
            <div class="section-heading-group">
              <div class="section-label">DỰ ÁN GẦN ĐÂY</div>
              <div class="section-heading-line">
                <h2 id="projects-heading">Dự án của bạn</h2>
                <Show when={!projectsLoading()}><span class="count-pill">{projects().length}</span></Show>
              </div>
            </div>
            <div class="project-controls">
              <Show when={projects().length > 0}>
                <Show when={selectionMode()} fallback={<button class="sort-button project-select-mode" type="button" onClick={() => { setSelectionMode(true); setSelectedProjectIds([]); }}><ListChecks size={15} /><span>Chọn project</span></button>}>
                  <button class="sort-button" type="button" onClick={toggleSelectAll}><Check size={15} /><span>Chọn tất cả hiện</span></button>
                  <span class="selected-project-count">Đã chọn {selectedProjectIds().length}</span>
                  <button class="button button-primary batch-process-button" type="button" disabled={!selectedProjectIds().length} onClick={() => setBatchOpen(true)}><Layers3 size={15} /> Xử lý nhiều project</button>
                  <button class="sort-button" type="button" onClick={() => { setSelectionMode(false); setSelectedProjectIds([]); }}>Xong</button>
                </Show>
              </Show>
              <label class="search-box">
                <Search size={16} aria-hidden="true" />
                <input ref={searchInput} aria-label="Tìm kiếm dự án" placeholder="Tìm dự án..." value={query()} onInput={(event) => setQuery(event.currentTarget.value)} />
                <Show when={query()}><button type="button" aria-label="Xóa tìm kiếm" class="search-clear" onClick={() => setQuery('')}><X size={13} /></button></Show>
                <kbd>Ctrl K</kbd>
              </label>
              <button class="sort-button" type="button" aria-label="Đổi thứ tự sắp xếp theo thời gian cập nhật" title={newestFirst() ? 'Mới cập nhật nhất · nhấn để đổi' : 'Cũ nhất · nhấn để đổi'} onClick={() => setNewestFirst(!newestFirst())}>
                {newestFirst() ? <ArrowDownWideNarrow size={16} /> : <ArrowUpWideNarrow size={16} />}<span>{newestFirst() ? 'Mới cập nhật' : 'Cũ nhất'}</span>
              </button>
            </div>
          </div>

          <Show when={projectLoadWarnings().length > 0}>
            <aside class="project-warning" role="status">
              <span>{projectLoadWarnings().length} dự án cần kiểm tra. Dữ liệu bị lỗi được giữ nguyên, không tự xóa.</span>
              <button type="button" onClick={() => void refresh()}>Tải lại</button>
            </aside>
          </Show>

          <Show when={projectsLoading()} fallback={
            <Show when={filteredProjects().length > 0} fallback={
              <Show when={query().trim()} fallback={
                <div class="empty-state">
                  <div class="empty-art" aria-hidden="true">
                    <div class="empty-art-back"><FileVideo2 size={35} strokeWidth={1.25} /></div>
                    <div class="empty-art-front"><Sparkles size={20} /></div>
                  </div>
                  <h3>Đây là nơi mọi thứ bắt đầu.</h3>
                  <p>Tạo dự án để thêm video, chuẩn bị bản dịch và xem lại kết quả — tất cả trong một không gian riêng.</p>
                  <button class="button button-primary" type="button" onClick={() => showDialog('create')}><FolderPlus size={17} /> Tạo dự án đầu tiên</button>
                </div>
              }>
                <div class="no-search-results">
                  <Search size={20} />
                  <h3>Không tìm thấy dự án phù hợp</h3>
                  <p>Thử một tên khác hoặc xóa nội dung tìm kiếm.</p>
                  <button class="button button-quiet" onClick={() => setQuery('')}>Xóa tìm kiếm</button>
                </div>
              </Show>
            }>
              <div class="project-grid">
                <For each={filteredProjects()}>{(project, index) => (
                  <article class={`project-card project-card-${index() % 4}`}>
                    <button class="project-open" type="button" onClick={() => void openProject(project)} aria-label={selectionMode() ? `Chọn dự án ${project.name}` : `Mở dự án ${project.name}`}>
                      <div class="project-art">
                        <div class="art-grid" />
                        <div class="art-shape art-shape-large" />
                        <div class="art-shape art-shape-small" />
                        <div class="art-mono">{project.name.slice(0, 1).toLocaleUpperCase('vi')}</div>
                        <div class="art-open"><ArrowUpRight size={16} /></div>
                      </div>
                      <div class="project-card-body">
                        <div class="project-card-title-row">
                          <div class="project-folder-icon"><FolderOpen size={16} /></div>
                          <h3 title={project.name}>{project.name}</h3>
                          <span class="project-state"><span /> Đã tạo</span>
                        </div>
                        <div class="project-meta-row">
                          <span>{createdLabel(project.createdAt)}</span>
                          <span class="meta-divider" />
                          <span class="updated-meta"><Clock3 size={13} /> {updatedLabel(project.updatedAt)}</span>
                        </div>
                      </div>
                    </button>
                    <Show when={selectionMode()}><button class="project-select-check" type="button" aria-label={`${selectedProjectIds().includes(project.id) ? 'Bỏ chọn' : 'Chọn'} ${project.name}`} aria-pressed={selectedProjectIds().includes(project.id)} onClick={() => void openProject(project)}>{selectedProjectIds().includes(project.id) ? <Check size={14} /> : null}</button></Show>
                    <div class="project-menu-anchor" data-project-menu>
                      <button
                        class="icon-button project-menu-trigger"
                        type="button"
                        aria-label={`Tùy chọn dự án ${project.name}`}
                        aria-haspopup="menu"
                        aria-expanded={openMenu() === project.id}
                        onClick={() => setOpenMenu(openMenu() === project.id ? undefined : project.id)}
                      ><MoreHorizontal size={18} /></button>
                      <Show when={openMenu() === project.id}>
                        <div class="project-menu" role="menu">
                          <button role="menuitem" onClick={() => showDialog('rename', project)}>Đổi tên</button>
                          <div class="menu-rule" />
                          <button role="menuitem" class="menu-danger" onClick={() => showDialog('delete', project)}>Xóa dự án</button>
                        </div>
                      </Show>
                    </div>
                    <Show when={openingId() === project.id}>
                      <div class="project-opening"><LoaderCircle class="spin" size={16} /> Đang mở...</div>
                    </Show>
                  </article>
                )}</For>
                <button class="new-project-tile" type="button" onClick={() => showDialog('create')}>
                  <span class="new-tile-icon"><FolderPlus size={19} /></span>
                  <span><strong>Dự án mới</strong><small>Bắt đầu với không gian trống</small></span>
                  <span class="new-tile-arrow">+</span>
                </button>
              </div>
            </Show>
          }>
            <div class="loading-grid" aria-label="Đang tải dự án">
              <For each={[1, 2, 3]}>{() => <div class="loading-project"><div class="loading-art" /><div class="loading-lines"><span /><span /></div></div>}</For>
            </div>
          </Show>
        </section>

        <footer class="start-footer">
          <span><span class="footer-shield">✳</span> Dữ liệu dự án được lưu trên thiết bị của bạn</span>
          <span class="footer-version">JC AI <i /> V{runtimeHealth()?.appVersion ?? '…'}</span>
        </footer>
      </div>

      <Show when={dialogMode()}>
        <ProjectDialog
          mode={dialogMode()!}
          project={dialogProject()}
          onClose={() => { setDialogMode(undefined); setDialogProject(undefined); }}
          onCreated={created}
          onRenamed={renamed}
          onDeleted={deleted}
        />
      </Show>
      <Show when={batchOpen()}><BatchProcessDialog projectIds={selectedProjectIds()} onClose={() => setBatchOpen(false)} /></Show>
    </main>
  );
}
