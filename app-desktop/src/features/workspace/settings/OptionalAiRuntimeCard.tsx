import { createSignal, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { AudioLines, Cpu, Sparkles, Zap } from 'lucide-solid';
import { listen } from '@tauri-apps/api/event';
import { optionalRuntimeService, type OptionalFeature, type OptionalRuntimeStatus, type RuntimeProfile } from '../../../app/services/optional-runtime-service';
import Modal from '../../../shared/components/Modal';

const FEATURE_TEXT: Record<OptionalFeature, { title: string; description: string }> = {
  whisper_openvino: { title: 'Whisper · Intel GPU', description: 'Nhận dạng lời thoại trên GPU Intel.' },
  ocr_openvino: {
    title: 'Tăng tốc Intel · đọc chữ',
    description: 'Dùng model OCR hiện có trên GPU Intel.',
  },
  whisperx: {
    title: 'WhisperX · căn thời gian',
    description: 'Cài bộ xử lý và các model cần cho ngôn ngữ đang chọn trong một lần. Model đã có được dùng lại.',
  },
  bandit: {
    title: 'Tách thoại gốc · BandIt',
    description: 'Cài bộ xử lý và model tách tiếng nói khỏi nhạc, hiệu ứng trong một lần.',
  },
  acceleration: {
    title: 'Tăng tốc NVIDIA',
    description: 'Tăng tốc nhận dạng, đọc chữ và giọng đọc đã có trong ứng dụng. Chỉ cần khi muốn chạy các tính năng đó bằng GPU.',
  },
  sttn: {
    title: 'Xóa chữ bằng AI · STTN',
    description: 'Dùng thông tin các khung hình lân cận để phục hồi nền phía sau chữ; cần cài bộ xử lý và model STTN một lần.',
  },
};

function estimatedExtra(feature: OptionalFeature, profile: RuntimeProfile, sharedProfile: RuntimeProfile | null): string {
  if (sharedProfile === profile) {
    if (feature === 'bandit') return 'khoảng 0,3 GB';
    if (feature === 'sttn') return 'khoảng 0,2 GB';
    return 'khoảng 1 GB';
  }
  if (feature === 'sttn') return profile === 'cuda' ? 'khoảng 4–6 GB' : 'khoảng 2,5 GB';
  if (profile === 'cuda') return 'khoảng 6–7 GB';
  if (feature === 'bandit') return 'khoảng 2,5 GB';
  return 'khoảng 3,3 GB';
}

interface OptionalAiRuntimeCardProps {
  feature: OptionalFeature;
  embedded?: boolean;
  preferredProfile?: RuntimeProfile;
  available?: boolean;
  modelReady?: boolean;
  modelDescription?: string;
  configuration?: JSX.Element;
  installDisabledReason?: string;
  installModels?: (progress: (message: string) => void) => Promise<void>;
  onInstalled: () => void;
}

export default function OptionalAiRuntimeCard(props: OptionalAiRuntimeCardProps) {
  const [status, setStatus] = createSignal<OptionalRuntimeStatus>();
  const [loading, setLoading] = createSignal(true);
  const [installing, setInstalling] = createSignal(false);
  const [progress, setProgress] = createSignal('');
  const [error, setError] = createSignal('');
  const [confirmProfile, setConfirmProfile] = createSignal<RuntimeProfile>();
  const [selectedProfile, setSelectedProfile] = createSignal<RuntimeProfile>();
  let removeProgressListener: (() => void) | undefined;
  let removeModelProgressListener: (() => void) | undefined;
  let disposed = false;
  const refreshStatus = async () => {
    setLoading(true);
    try { setStatus(await optionalRuntimeService.status()); setError(''); }
    catch { setError('Không thể kiểm tra thành phần xử lý bổ sung.'); }
    finally { setLoading(false); }
  };
  onMount(() => {
    void refreshStatus();
    void optionalRuntimeService.onProgress((message) => {
      setProgress(message);
      if (message === 'Đã cài tính năng được chọn') void refreshStatus();
    }).then((unlisten) => {
      if (disposed) unlisten();
      else removeProgressListener = unlisten;
    });
    if (props.feature === 'sttn') {
      void listen<{ downloadedBytes: number; totalBytes: number; message: string }>('sttn-model-progress', ({ payload }) => {
        const downloaded = (payload.downloadedBytes / 1024 / 1024).toFixed(0);
        const total = (payload.totalBytes / 1024 / 1024).toFixed(0);
        setProgress(`${payload.message} · ${downloaded} / ${total} MiB`);
      }).then((unlisten) => {
        if (disposed) unlisten();
        else removeModelProgressListener = unlisten;
      });
    }
  });
  onCleanup(() => { disposed = true; removeProgressListener?.(); removeModelProgressListener?.(); });
  const install = async (profile: RuntimeProfile) => {
    setConfirmProfile(undefined);
    setSelectedProfile(profile);
    setInstalling(true);
    setError('');
    setProgress('Đang chuẩn bị tải xuống…');
    try {
      setStatus(await optionalRuntimeService.install(props.feature, profile));
      if (props.installModels) {
        setProgress('Đang chuẩn bị model đã chọn…');
        await props.installModels(setProgress);
      }
    } catch (cause) {
      setError(typeof cause === 'string' ? cause : cause instanceof Error ? cause.message : 'Không thể cài thành phần bổ sung.');
    } finally {
      props.onInstalled();
      setInstalling(false);
      setProgress('');
    }
  };
  const completeModels = async () => {
    if (!props.installModels || props.installDisabledReason) return;
    setInstalling(true);
    setError('');
    setProgress('Đang chuẩn bị model đã chọn…');
    try {
      await props.installModels(setProgress);
    } catch (cause) {
      setError(typeof cause === 'string' ? cause : cause instanceof Error ? cause.message : 'Không thể cài model.');
    } finally {
      props.onInstalled();
      setInstalling(false);
      setProgress('');
    }
  };
  const installed = () => props.available || (status()?.installedFeatures.includes(props.feature) ?? false);
  const modelReady = () => props.modelReady ?? true;
  const activeProfile = (): RuntimeProfile => selectedProfile() ?? props.preferredProfile ?? (status()?.profile === 'openvino' ? 'cpu' : status()?.profile) ?? 'cpu';
  const icon = () => props.feature === 'whisperx' ? <AudioLines size={18} /> : props.feature === 'bandit' || props.feature === 'sttn' ? <Cpu size={18} /> : <Zap size={18} />;
  return <div class="settings-card optional-runtime-card" classList={{ 'optional-runtime-embedded': !!props.embedded }}>
    <Show when={!props.embedded}><div class="tts-model-status-row">
      <div class="provider-card-icon">{icon()}</div>
      <div class="provider-card-main"><span class="provider-eyebrow">TÙY CHỌN · CHỈ CÀI KHI CẦN</span><h3>{FEATURE_TEXT[props.feature].title}</h3>
        <p>{FEATURE_TEXT[props.feature].description}</p>
      </div>
    </div></Show>
    <Show when={props.embedded}><p class="provider-help">{FEATURE_TEXT[props.feature].description}</p></Show>
    {props.configuration}
    <Show when={props.installDisabledReason}><p class="asr-config-warning" role="status">{props.installDisabledReason}</p></Show>
    <Show when={props.embedded && props.feature === 'sttn' && !installed()}>
      <label class="provider-field"><span>Thiết bị xử lý STTN</span><select class="settings-select" value={activeProfile()} onChange={(event) => setSelectedProfile(event.currentTarget.value as RuntimeProfile)}><option value="cpu">CPU</option><option value="cuda" disabled={!status()?.nvidiaDetected}>NVIDIA · CUDA</option></select></label>
    </Show>
    <Show when={props.embedded} fallback={<Show when={installed()} fallback={<div class="optional-runtime-options">
      <Show when={props.feature !== 'acceleration' && !status()?.installedFeatures.includes('acceleration')}><div><Cpu size={17} /><span><strong>Dùng CPU</strong><small>Phần cài thêm: {estimatedExtra(props.feature, 'cpu', status()?.profile ?? null)}</small></span>
        <button class="button button-quiet" type="button" disabled={loading() || installing() || !!props.installDisabledReason} onClick={() => setConfirmProfile('cpu')}>Cài cho CPU</button></div></Show>
      <div><Sparkles size={17} /><span><strong>Dùng NVIDIA</strong><small>{status()?.nvidiaDetected ? `Phần cài thêm: ${estimatedExtra(props.feature, 'cuda', status()?.profile ?? null)}` : 'Chưa nhận diện GPU NVIDIA'}</small></span>
        <button class="button button-quiet" type="button" disabled={loading() || installing() || !status()?.nvidiaDetected || !!props.installDisabledReason} onClick={() => setConfirmProfile('cuda')}>Cài cho NVIDIA</button></div>
    </div>}>
      <p class="provider-help">{props.available && !status()?.installedFeatures.includes(props.feature) ? 'Bộ xử lý đã có trên máy.' : `Bộ xử lý đã cài cho ${status()?.profile === 'cuda' ? 'NVIDIA' : 'CPU'}.`} {props.installModels ? modelReady() ? 'Model cần thiết đã sẵn sàng.' : props.modelDescription ?? 'Model đang chọn chưa được cài.' : 'Các model đang dùng được giữ nguyên.'}</p>
      <Show when={!modelReady() && props.installModels}>
        <button class="button button-primary" type="button" disabled={loading() || installing() || !!props.installDisabledReason} onClick={() => void completeModels()}>{installing() ? 'Đang cài…' : 'Hoàn tất cài đặt model'}</button>
      </Show>
      <Show when={props.feature !== 'acceleration' && status()?.installedFeatures.includes(props.feature) && status()?.nvidiaDetected && status()?.profile === 'cpu'}>
        <button class="button button-quiet" type="button" disabled={loading() || installing()} onClick={() => setConfirmProfile('cuda')}>Chuyển sang NVIDIA</button>
      </Show>
    </Show>}>
      <Show when={props.feature === 'bandit' || props.feature === 'sttn'}><label class="provider-field"><span>{props.feature === 'sttn' ? 'Thiết bị xử lý STTN' : 'Thiết bị tách thoại'}</span><select class="settings-select" value={activeProfile()} onChange={(event) => { const profile = event.currentTarget.value as RuntimeProfile; event.currentTarget.value = activeProfile(); setConfirmProfile(profile); }}><option value="cpu" disabled={status()?.profile === 'cuda'}>CPU</option><option value="cuda" disabled={!status()?.nvidiaDetected}>NVIDIA · CUDA</option></select></label></Show>
      <Show when={installed() && modelReady()}><p class="provider-help">{props.feature === 'sttn' && !props.installModels ? 'Bộ xử lý STTN đã sẵn sàng' : 'Bộ xử lý và model đã sẵn sàng'}{status()?.profile === 'cuda' ? ' trên NVIDIA' : ' trên CPU'}.</p></Show>
      <Show when={!installed() || !modelReady()}><button class="button button-primary" type="button" disabled={loading() || installing() || !!props.installDisabledReason} onClick={() => setConfirmProfile(activeProfile())}>{installing() ? 'Đang cài…' : props.feature === 'bandit' ? 'Cài tách thoại gốc' : props.feature === 'sttn' && !props.installModels ? 'Cài bộ xử lý STTN' : props.feature === 'sttn' ? 'Cài STTN và model' : 'Cài WhisperX và model'}</button></Show>
    </Show>
    <Show when={status()?.profile && !installed()}><p class="provider-help">Các gói còn trong cache cài đặt sẽ được dùng lại để tiết kiệm mạng. Ứng dụng chuẩn bị bản cài mới trước khi thay bộ xử lý đang dùng.</p></Show>
    <Show when={status() && !status()!.nvidiaDetected}><p class="provider-help">Nếu máy có NVIDIA, hãy cập nhật driver rồi <button class="optional-runtime-refresh" type="button" disabled={loading() || installing()} onClick={() => void refreshStatus()}>kiểm tra lại</button>.</p></Show>
    <Show when={installing()}><div class="provider-help" role="status"><span class="spin status-spinner" /> {progress()}</div></Show>
    <Show when={error()}><div class="provider-feedback provider-feedback-error" role="alert">{error()}</div></Show>
    <Show when={confirmProfile()}>{(profile) => <Modal title={`Cài ${FEATURE_TEXT[props.feature].title}?`} description={`Ứng dụng sẽ cài bộ xử lý (${estimatedExtra(props.feature, profile(), status()?.profile ?? null)} dung lượng lưu trữ dự kiến)${props.modelDescription ? ` và ${props.modelDescription.toLowerCase()}` : ''} trong cùng lượt. Những phần đã có được dùng lại. Cần Internet và đủ dung lượng trống.`} onClose={() => setConfirmProfile(undefined)}>
      <footer class="modal-actions"><button class="button button-quiet" type="button" onClick={() => setConfirmProfile(undefined)}>Hủy</button><button class="button button-primary" type="button" onClick={() => void install(profile())}>Tải và cài</button></footer>
    </Modal>}</Show>
  </div>;
}
