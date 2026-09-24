import { createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { useNavigate, useSearchParams } from '@solidjs/router';
import { convertFileSrc } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import {
  AudioLines, Check, ChevronRight, CircleHelp, Cpu, FolderOpen, Globe2, Languages, LockKeyhole,
  ArrowLeft, Info, Monitor, SlidersHorizontal, Sparkles, Volume2,
} from 'lucide-solid';
import { translationService, type AppSettings } from '../../../app/services/translation-service';
import { projectService } from '../../../app/services/project-service';
import { ttsService } from '../../../app/services/tts-service';
import { formatBytes, storageService, type StorageUsage } from '../../../app/services/storage-service';
import { runtimeService, type RuntimeHealth } from '../../../app/services/runtime-service';
import type { TranslationProviderId } from '../../transcript/types';
import type { AsrEnvironment, TtsEnvironment } from '../../transcript/types';
import { estimatedModelSize } from '../../transcript/transcript-utils';
import Modal from '../../../shared/components/Modal';
import { setAppStartupReady } from '../../../app/project-state';

type SettingsSection = 'General' | 'Translation' | 'Speech Recognition' | 'Voice / TTS' | 'Advanced' | 'About';

const settingsNavigation: Array<{ name: SettingsSection; caption: string; icon: typeof Monitor }> = [
  { name: 'General', caption: 'Ứng dụng', icon: Monitor },
  { name: 'Translation', caption: 'Nhà cung cấp', icon: Languages },
  { name: 'Speech Recognition', caption: 'Nhận dạng', icon: AudioLines },
  { name: 'Voice / TTS', caption: 'Giọng nói', icon: Volume2 },
  { name: 'Advanced', caption: 'Nâng cao', icon: SlidersHorizontal },
  { name: 'About', caption: 'Thông tin', icon: Info },
];

export default function SettingsView() {
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const [section, setSection] = createSignal<SettingsSection>('General');
  const [settings, setSettings] = createSignal<AppSettings>();
  const [geminiConfigured, setGeminiConfigured] = createSignal(false);
  const [cloudConfigured, setCloudConfigured] = createSignal(false);
  const [secret, setSecret] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  const [settingsMessage, setSettingsMessage] = createSignal('');
  const [settingsError, setSettingsError] = createSignal(false);
  const [testBusy, setTestBusy] = createSignal(false);
  const [testMessage, setTestMessage] = createSignal('');
  const [testError, setTestError] = createSignal(false);
  const [asrEnvironment, setAsrEnvironment] = createSignal<AsrEnvironment>();
  const [asrEnvironmentError, setAsrEnvironmentError] = createSignal('');
  const [asrEnvironmentLoading, setAsrEnvironmentLoading] = createSignal(false);
  const [installingAsrModel, setInstallingAsrModel] = createSignal(false);
  const [removingAsrModel, setRemovingAsrModel] = createSignal(false);
  const [showAsrModelConfirm, setShowAsrModelConfirm] = createSignal(false);
  const [showRemoveAsrModelConfirm, setShowRemoveAsrModelConfirm] = createSignal(false);
  const [installingWhisperXAlignment, setInstallingWhisperXAlignment] = createSignal(false);
  const [ttsEnvironment, setTtsEnvironment] = createSignal<TtsEnvironment>();
  const [ttsEnvironmentError, setTtsEnvironmentError] = createSignal('');
  const [ttsEnvironmentLoading, setTtsEnvironmentLoading] = createSignal(false);
  const [showTtsModelConfirm, setShowTtsModelConfirm] = createSignal(false);
  const [installingTtsModel, setInstallingTtsModel] = createSignal(false);
  const [removingTtsModel, setRemovingTtsModel] = createSignal(false);
  const [showRemoveTtsModelConfirm, setShowRemoveTtsModelConfirm] = createSignal(false);
  const [ttsModelProgress, setTtsModelProgress] = createSignal(0);
  const [ttsModelMessage, setTtsModelMessage] = createSignal('');
  const [previewBusy, setPreviewBusy] = createSignal(false);
  const [voicePreviewUrl, setVoicePreviewUrl] = createSignal('');
  const [showVoiceProfileDialog, setShowVoiceProfileDialog] = createSignal(false);
  const [voiceProfileName, setVoiceProfileName] = createSignal('');
  const [voiceProfileFile, setVoiceProfileFile] = createSignal('');
  const [voiceProfileConsent, setVoiceProfileConsent] = createSignal(false);
  const [voiceProfileBusy, setVoiceProfileBusy] = createSignal(false);
  const [storageUsage, setStorageUsage] = createSignal<StorageUsage>();
  const [storageBusy, setStorageBusy] = createSignal(false);
  const [storageError, setStorageError] = createSignal('');
  const [runtimeHealth, setRuntimeHealth] = createSignal<RuntimeHealth>();
  const [runtimeHealthError, setRuntimeHealthError] = createSignal('');
  const [diagnosticsBusy, setDiagnosticsBusy] = createSignal(false);
  let unlistenTtsModel: (() => void) | undefined;
  let asrEnvironmentRevision = 0;
  const returnPath = () => {
    const value = searchParams.returnTo;
    return typeof value === 'string' && value.startsWith('/') && !value.startsWith('//') ? value : '/';
  };

  const showMessage = (message: unknown, failed: boolean) => {
    setSettingsError(failed);
    setSettingsMessage(message instanceof Error ? message.message : String(message));
  };
  const refreshStorage = async () => {
    setStorageBusy(true); setStorageError('');
    try { setStorageUsage(await storageService.usage()); }
    catch (error) { setStorageError(error instanceof Error ? error.message : String(error)); }
    finally { setStorageBusy(false); }
  };
  const openStorageFolder = async (openFolder: () => Promise<string>) => {
    setStorageError('');
    try { await openFolder(); }
    catch (error) { setStorageError(error instanceof Error ? error.message : String(error)); }
  };
  const refreshRuntimeHealth = async () => {
    setRuntimeHealthError('');
    try { setRuntimeHealth(await runtimeService.health()); }
    catch (error) { setRuntimeHealthError(error instanceof Error ? error.message : String(error)); }
  };
  const exportDiagnostics = async () => {
    setDiagnosticsBusy(true);
    try {
      const selected = await open({ directory: true, multiple: false, title: 'Chọn thư mục lưu diagnostics' });
      if (typeof selected !== 'string') return;
      const path = await runtimeService.exportDiagnostics(selected);
      showMessage(`Đã lưu gói chẩn đoán: ${path}`, false);
    } catch (error) { showMessage(error, true); }
    finally { setDiagnosticsBusy(false); }
  };
  onMount(() => {
    setAppStartupReady(true);
    void translationService.getSettings().then((result) => {
      setSettings(result.settings);
      setGeminiConfigured(result.geminiCredentialConfigured);
      setCloudConfigured(result.googleCloudCredentialConfigured);
      void refreshAsrEnvironment(result.settings.asr.model, result.settings.asr.language);
    }).catch((error: unknown) => showMessage(error, true));
    void refreshTtsEnvironment();
    void refreshStorage();
    void refreshRuntimeHealth();
    void listen<{ progress?: number; message?: string }>('tts-model-progress', (event) => {
      setTtsModelProgress(event.payload.progress ?? 0);
      setTtsModelMessage(event.payload.message ?? 'Đang tải model…');
    }).then((unlisten) => { unlistenTtsModel = unlisten; }).catch(() => undefined);
  });
  onCleanup(() => unlistenTtsModel?.());
  const refreshTtsEnvironment = async () => {
    setTtsEnvironmentLoading(true);
    setTtsEnvironmentError('');
    try { setTtsEnvironment(await ttsService.getEnvironment()); }
    catch (error) { setTtsEnvironmentError(error instanceof Error ? error.message : 'Không thể kiểm tra VieNeu-TTS.'); }
    finally { setTtsEnvironmentLoading(false); }
  };
  const refreshAsrEnvironment = async (model = settings()?.asr.model ?? 'small', language = settings()?.asr.language ?? 'auto') => {
    const revision = ++asrEnvironmentRevision;
    setAsrEnvironmentLoading(true);
    setAsrEnvironment(undefined);
    setAsrEnvironmentError('');
    try {
      const environment = await projectService.getAsrEnvironment(model, language);
      if (revision === asrEnvironmentRevision) setAsrEnvironment(environment);
    } catch (error) {
      if (revision === asrEnvironmentRevision) setAsrEnvironmentError(error instanceof Error ? error.message : 'Không thể kiểm tra Speech Recognition.');
    } finally {
      if (revision === asrEnvironmentRevision) setAsrEnvironmentLoading(false);
    }
  };
  const updateTranslation = (update: Partial<AppSettings['translation']>) => {
    setSettings((value) => value ? { ...value, translation: { ...value.translation, ...update } } : value);
  };
  const updateTts = (update: Partial<AppSettings['tts']>) => {
    setSettings((value) => value ? { ...value, tts: { ...value.tts, ...update } } : value);
  };
  const updateAsr = (update: Partial<AppSettings['asr']>) => {
    setSettings((value) => value ? { ...value, asr: { ...value.asr, ...update } } : value);
  };
  const saveSettings = async () => {
    const value = settings();
    if (!value || busy()) return;
    setBusy(true);
    try {
      const result = await translationService.saveSettings(value);
      setSettings(result.settings);
      setGeminiConfigured(result.geminiCredentialConfigured);
      setCloudConfigured(result.googleCloudCredentialConfigured);
      showMessage('Đã lưu cài đặt translation trên thiết bị.', false);
    } catch (error) { showMessage(error, true); }
    finally { setBusy(false); }
  };
  const saveCredential = async (): Promise<boolean> => {
    const provider = settings()?.translation.provider;
    if (!provider || provider === 'codexCli' || !secret().trim() || busy()) return false;
    setBusy(true);
    try {
      await translationService.setCredential(provider, secret());
      setSecret('');
      const refreshed = await translationService.getSettings();
      setGeminiConfigured(refreshed.geminiCredentialConfigured);
      setCloudConfigured(refreshed.googleCloudCredentialConfigured);
      showMessage('Đã lưu credential trong kho bảo mật của hệ điều hành.', false);
      return true;
    } catch (error) { showMessage(error, true); return false; }
    finally { setBusy(false); }
  };
  const removeCredential = async () => {
    const provider = settings()?.translation.provider;
    if (!provider || provider === 'codexCli' || busy()) return;
    setBusy(true);
    try {
      await translationService.deleteCredential(provider);
      if (provider === 'gemini') setGeminiConfigured(false);
      else setCloudConfigured(false);
      showMessage('Đã xóa credential khỏi kho bảo mật.', false);
    } catch (error) { showMessage(error, true); }
    finally { setBusy(false); }
  };
  const testConnection = async () => {
    const value = settings();
    if (!value || testBusy()) return;
    setTestBusy(true);
    setTestMessage('');
    setTestError(false);
    try {
      if (secret().trim() && value.translation.provider !== 'codexCli' && !await saveCredential()) {
        throw new Error(settingsMessage() || 'Không thể lưu credential an toàn.');
      }
      await translationService.saveSettings(value);
      const result = await translationService.testProvider(value.translation.provider);
      setTestMessage(result.message);
    } catch (error) {
      setTestError(true);
      setTestMessage(error instanceof Error ? error.message : 'Không thể kiểm tra provider.');
    } finally { setTestBusy(false); }
  };
  const installTtsModel = async () => {
    setShowTtsModelConfirm(false);
    setInstallingTtsModel(true);
    setTtsModelProgress(0);
    setTtsModelMessage('Chuẩn bị tải model VieNeu-TTS…');
    try {
      await ttsService.installModel();
      await refreshTtsEnvironment();
      setTtsModelProgress(1);
      setTtsModelMessage('Model VieNeu-TTS đã sẵn sàng.');
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Không thể tải model VieNeu-TTS.';
      setTtsEnvironmentError(message);
      setTtsModelMessage(message);
    } finally { setInstallingTtsModel(false); }
  };
  const removeTtsModel = async () => {
    setShowRemoveTtsModelConfirm(false);
    setRemovingTtsModel(true);
    try {
      await ttsService.removeModel();
      await refreshTtsEnvironment();
      showMessage('Đã gỡ model VieNeu-TTS khỏi thư mục model của ứng dụng.', false);
    } catch (error) { showMessage(error, true); }
    finally { setRemovingTtsModel(false); }
  };
  const installAsrModel = async () => {
    const currentSettings = settings();
    const model = currentSettings?.asr.model;
    if (!currentSettings || !model || installingAsrModel() || busy()) return;
    setShowAsrModelConfirm(false);
    setInstallingAsrModel(true);
    setAsrEnvironmentError('');
    try {
      const saved = await translationService.saveSettings(currentSettings);
      setSettings(saved.settings);
      await projectService.installAsrModel(model);
      await refreshAsrEnvironment(model, currentSettings.asr.language);
    } catch (error) {
      setAsrEnvironmentError(error instanceof Error ? error.message : 'Không thể cài model faster-whisper.');
    } finally { setInstallingAsrModel(false); }
  };
  const removeAsrModel = async () => {
    const model = settings()?.asr.model;
    if (!model || removingAsrModel()) return;
    setShowRemoveAsrModelConfirm(false);
    setRemovingAsrModel(true);
    try {
      await projectService.removeAsrModel(model);
      await refreshAsrEnvironment(model, settings()?.asr.language ?? 'auto');
      showMessage(`Đã gỡ model faster-whisper ${model}.`, false);
    } catch (error) { showMessage(error, true); }
    finally { setRemovingAsrModel(false); }
  };
  const installWhisperXAlignment = async () => {
    const value = settings();
    const language = value?.asr.language;
    if (!value || !language || language === 'auto' || installingWhisperXAlignment()) return;
    setInstallingWhisperXAlignment(true);
    setAsrEnvironmentError('');
    try {
      const saved = await translationService.saveSettings(value);
      setSettings(saved.settings);
      await projectService.installWhisperXAlignment(language);
      await refreshAsrEnvironment(value.asr.model, language);
      showMessage(`Đã cài WhisperX aligner cho ${language.toUpperCase()}.`, false);
    } catch (error) {
      setAsrEnvironmentError(error instanceof Error ? error.message : 'Không thể cài WhisperX aligner.');
    } finally { setInstallingWhisperXAlignment(false); }
  };
  const previewTtsVoice = async () => {
    const voice = settings()?.tts.defaultVoice;
    if (!voice || previewBusy()) return;
    setPreviewBusy(true);
    setVoicePreviewUrl('');
    try { setVoicePreviewUrl(convertFileSrc(await ttsService.previewVoice(voice))); }
    catch (error) { showMessage(error, true); }
    finally { setPreviewBusy(false); }
  };
  const browseVoiceReference = async () => {
    try {
      const selected = await open({ multiple: false, directory: false, filters: [{ name: 'Audio reference', extensions: ['wav', 'mp3', 'm4a', 'aac', 'flac', 'ogg', 'opus'] }] });
      if (typeof selected === 'string') setVoiceProfileFile(selected);
    } catch (error) { showMessage(error, true); }
  };
  const addVoiceProfile = async () => {
    if (!voiceProfileName().trim() || !voiceProfileFile() || !voiceProfileConsent() || voiceProfileBusy()) return;
    setVoiceProfileBusy(true);
    try {
      await ttsService.addVoiceProfile(voiceProfileName(), voiceProfileFile(), true);
      const refreshed = await translationService.getSettings();
      setSettings(refreshed.settings);
      setVoiceProfileName(''); setVoiceProfileFile(''); setVoiceProfileConsent(false); setShowVoiceProfileDialog(false);
      showMessage('Đã thêm voice profile. Audio tham chiếu được chuẩn hóa và lưu cục bộ.', false);
    } catch (error) { showMessage(error, true); }
    finally { setVoiceProfileBusy(false); }
  };
  const removeVoiceProfile = async (profileId: string) => {
    if (voiceProfileBusy()) return;
    setVoiceProfileBusy(true);
    try {
      await ttsService.deleteVoiceProfile(profileId);
      const refreshed = await translationService.getSettings();
      setSettings(refreshed.settings);
      showMessage('Đã xóa voice profile khỏi thiết bị.', false);
    } catch (error) { showMessage(error, true); }
    finally { setVoiceProfileBusy(false); }
  };

  return (
    <main class="settings-page">
      <header class="settings-page-header">
        <div class="settings-page-title"><button class="settings-back-link" type="button" onClick={() => navigate(returnPath())}><ArrowLeft size={14} /> Quay lại</button><div class="section-label">PREFERENCES</div><h1>Cài đặt</h1><p>Thiết lập môi trường dịch và lồng tiếng cho các dự án trên thiết bị này.</p></div>
        <span class="settings-scope-pill"><LockKeyhole size={13} /> CÀI ĐẶT TOÀN CỤC</span>
      </header>
      <div class="settings-layout">
        <nav class="settings-nav" aria-label="Nhóm cài đặt">
          <span class="settings-nav-label">THIẾT LẬP</span>
          <For each={settingsNavigation}>{(item) => {
            const Icon = item.icon;
            return <button type="button" class="settings-nav-item" classList={{ selected: section() === item.name }} aria-current={section() === item.name ? 'page' : undefined} onClick={() => { setSection(item.name); if (item.name === 'Advanced') void refreshStorage(); }}>
              <span class="settings-nav-icon"><Icon size={17} /></span><span class="settings-nav-copy"><strong>{item.name}</strong><small>{item.caption}</small></span><Show when={section() === item.name}><ChevronRight size={15} class="settings-nav-chevron" /></Show>
            </button>;
          }}</For>
          <div class="settings-nav-note"><CircleHelp size={15} /><span>Provider là cài đặt toàn cục; ngôn ngữ và phụ đề được lưu theo từng dự án.</span></div>
        </nav>

        <section class="settings-detail" aria-live="polite">
          <Show when={section() === 'General'}>
            <SettingsSectionHeader title="General" subtitle="Tùy chỉnh ứng dụng và cách hiển thị." />
            <div class="settings-card">
              <SettingRow icon={<Monitor size={17} />} title="Giao diện" description="Chủ đề màu của ứng dụng."><span class="setting-value"><span class="theme-swatch" /> Tối <Check size={14} /></span></SettingRow>
              <div class="settings-row-divider" />
              <SettingRow icon={<Globe2 size={17} />} title="Ngôn ngữ ứng dụng" description="Ngôn ngữ hiển thị trong giao diện."><span class="setting-value">Tiếng Việt <ChevronRight size={14} /></span></SettingRow>
              <div class="settings-row-divider" />
              <SettingRow icon={<Sparkles size={17} />} title="Phiên bản" description="JC AI desktop."><span class="version-chip">{runtimeHealth()?.appVersion ?? '…'} · Windows desktop</span></SettingRow>
            </div>
            <div class="settings-info-note"><LockKeyhole size={15} /><span>Dự án được lưu cục bộ trong thư mục dữ liệu ứng dụng của hệ điều hành.</span></div>
          </Show>

          <Show when={section() === 'Translation'}>
            <SettingsSectionHeader title="Translation" subtitle="Chọn provider cho các project trên thiết bị này. API credentials được giữ riêng trong kho bảo mật hệ điều hành." />
            <Show when={settings()} fallback={<div class="settings-card provider-settings-card">Đang đọc cài đặt provider…</div>}>
              <div class="settings-card provider-settings-card">
                <label class="provider-field"><span>Provider đang dùng</span><select class="settings-select" value={settings()!.translation.provider} onChange={(event) => { setSecret(''); setTestMessage(''); setSettingsMessage(''); updateTranslation({ provider: event.currentTarget.value as TranslationProviderId }); }}>
                  <option value="gemini">Gemini</option><option value="googleCloud">Google Cloud Translation</option><option value="codexCli">Codex CLI</option>
                </select></label>
                <div class="provider-settings-divider" />
                <Show when={settings()!.translation.provider === 'gemini'}>
                  <div class="provider-config-grid">
                    <label class="provider-field provider-field-wide"><span>Gemini API key</span><input type="password" autocomplete="new-password" value={secret()} placeholder={geminiConfigured() ? 'Đã lưu an toàn · nhập để thay thế' : 'Nhập API key'} onInput={(event) => setSecret(event.currentTarget.value)} /></label>
                    <label class="provider-field"><span>Model ID</span><input value={settings()!.translation.gemini.modelId} onInput={(event) => updateTranslation({ gemini: { ...settings()!.translation.gemini, modelId: event.currentTarget.value } })} /></label>
                    <label class="provider-field"><span>Temperature</span><input type="number" min="0" max="2" step="0.1" value={settings()!.translation.gemini.temperature} onInput={(event) => updateTranslation({ gemini: { ...settings()!.translation.gemini, temperature: Number(event.currentTarget.value) } })} /></label>
                  </div>
                  <p class="provider-help">Dịch theo batch với JSON output có cấu trúc. Key không được ghi vào project hoặc file cài đặt.</p>
                </Show>
                <Show when={settings()!.translation.provider === 'googleCloud'}>
                  <div class="provider-config-grid">
                    <label class="provider-field provider-field-wide"><span>Google Cloud API key</span><input type="password" autocomplete="new-password" value={secret()} placeholder={cloudConfigured() ? 'Đã lưu an toàn · nhập để thay thế' : 'Nhập API key'} onInput={(event) => setSecret(event.currentTarget.value)} /></label>
                    <label class="provider-field"><span>Translation model</span><select class="settings-select" value={settings()!.translation.googleCloud.model} onChange={(event) => updateTranslation({ googleCloud: { model: event.currentTarget.value } })}><option value="nmt">NMT</option></select></label>
                  </div>
                  <p class="provider-help">Dùng Cloud Translation Basic API v2 qua HTTPS. Batch hỗ trợ nhiều đoạn trong một request; provider này không dùng ngữ cảnh.</p>
                </Show>
                <Show when={settings()!.translation.provider === 'codexCli'}>
                  <div class="provider-config-grid">
                    <label class="provider-field provider-field-wide"><span>Codex executable</span><input value={settings()!.translation.codexCli.executable} onInput={(event) => updateTranslation({ codexCli: { ...settings()!.translation.codexCli, executable: event.currentTarget.value } })} /></label>
                    <label class="provider-field"><span>Model ID (tùy chọn)</span><input value={settings()!.translation.codexCli.modelId ?? ''} placeholder="Dùng model mặc định" onInput={(event) => updateTranslation({ codexCli: { ...settings()!.translation.codexCli, modelId: event.currentTarget.value || null } })} /></label>
                    <label class="provider-field"><span>Timeout (giây)</span><input type="number" min="5" max="600" value={settings()!.translation.codexCli.timeoutSeconds} onInput={(event) => updateTranslation({ codexCli: { ...settings()!.translation.codexCli, timeoutSeconds: Number(event.currentTarget.value) } })} /></label>
                  </div>
                  <p class="provider-help">Codex chạy ở chế độ non-interactive, sandbox read-only, không có quyền ghi project. Cấu hình cần CLI đã cài và đăng nhập.</p>
                </Show>
                <div class="provider-settings-divider" />
                <div class="provider-config-grid provider-common-config">
                  <label class="provider-field"><span>Segments / batch</span><input type="number" min="1" max="100" value={settings()!.translation.batchSize} onInput={(event) => updateTranslation({ batchSize: Number(event.currentTarget.value) })} /></label>
                  <label class="provider-field"><span>Ngữ cảnh mỗi phía</span><input type="number" min="0" max="8" value={settings()!.translation.contextSegments} onInput={(event) => updateTranslation({ contextSegments: Number(event.currentTarget.value) })} /></label>
                  <label class="provider-field"><span>Timeout HTTP (giây)</span><input type="number" min="5" max="600" value={settings()!.translation.timeoutSeconds} onInput={(event) => updateTranslation({ timeoutSeconds: Number(event.currentTarget.value) })} /></label>
                </div>
                <div class="provider-settings-divider" />
                <p class="provider-help">Prompt bổ sung phong cách và giọng dịch được lưu riêng trong Cài đặt dự án. Yêu cầu dịch mặc định luôn được áp dụng.</p>
                <div class="provider-action-row">
                  <Show when={settings()!.translation.provider !== 'codexCli'}>
                    <button class="button button-quiet provider-action-secondary" type="button" disabled={busy() || testBusy() || !secret().trim()} onClick={() => void saveCredential()}>{busy() ? 'Đang lưu…' : 'Lưu credential'}</button>
                    <button class="button button-quiet provider-action-secondary" type="button" disabled={busy() || testBusy() || !(settings()!.translation.provider === 'gemini' ? geminiConfigured() : cloudConfigured())} onClick={() => void removeCredential()}>Xóa credential</button>
                  </Show>
                  <button class="button button-quiet provider-action-secondary" type="button" disabled={busy() || testBusy()} onClick={() => void saveSettings()}>{busy() ? 'Đang lưu…' : 'Lưu cấu hình'}</button>
                  <button class="button button-primary" type="button" disabled={busy() || testBusy()} onClick={() => void testConnection()}>{testBusy() ? <><span class="spin status-spinner" /> Đang kiểm tra…</> : 'Test Connection'}</button>
                </div>
                <Show when={settingsMessage()}><div class="provider-feedback" classList={{ 'provider-feedback-error': settingsError() }} role={settingsError() ? 'alert' : 'status'}>{settingsMessage()}</div></Show>
                <Show when={testMessage()}><div class="provider-feedback" classList={{ 'provider-feedback-error': testError() }} role={testError() ? 'alert' : 'status'}>{testMessage()}</div></Show>
              </div>
            </Show>
          </Show>

          <Show when={section() === 'Speech Recognition'}>
            <SettingsSectionHeader title="Speech Recognition" subtitle="Cấu hình nhận dạng lời thoại để tạo transcript." />
            <Show when={settings()} fallback={<div class="settings-card provider-settings-card">Đang đọc cấu hình Speech Recognition…</div>}>
              <div class="settings-card asr-settings-card">
                <div class="tts-model-status-row">
                  <div class="provider-card-icon"><AudioLines size={17} /></div>
                  <div class="provider-card-main"><span class="provider-eyebrow">LOCAL · SPEECH RECOGNITION</span><h3>{settings()!.asr.engine === 'whisperX' ? 'WhisperX' : 'faster-whisper'}</h3>
                    <p>{asrEnvironmentLoading() ? 'Đang kiểm tra AI Worker và model…' : !asrEnvironment()?.workerInstalled ? 'AI Worker chưa sẵn sàng.' : !asrEnvironment()?.modelCached ? `Model faster-whisper ${settings()!.asr.model} chưa được cài.` : settings()!.asr.engine === 'whisperX' ? !asrEnvironment()?.whisperXInstalled ? 'Thiếu runtime WhisperX; model faster-whisper hiện tại vẫn được giữ nguyên.' : settings()!.asr.language === 'auto' ? 'Chọn ngôn ngữ cụ thể để cài aligner và bật căn chỉnh timestamp.' : asrEnvironment()?.alignmentModelCached ? `Model ASR ${settings()!.asr.model} và aligner ${settings()!.asr.language} đã sẵn sàng.` : `Model ASR ${settings()!.asr.model} đã có; chưa cài aligner ${settings()!.asr.language}.` : `Model ${settings()!.asr.model} đã được cài trên thiết bị.`}</p>
                  </div>
                  <span class="provider-status-dot" classList={{ 'provider-status-dot-ready': !!asrEnvironment()?.modelCached && (settings()!.asr.engine !== 'whisperX' || (!!asrEnvironment()?.whisperXInstalled && !!asrEnvironment()?.alignmentModelCached)) }} />
                </div>
                <Show when={!asrEnvironmentLoading() && !asrEnvironment()?.workerInstalled}>
                  <div class="tts-install-instructions"><strong>AI Worker chưa sẵn sàng</strong><span>Hãy sửa chữa hoặc cài lại ứng dụng để khôi phục runtime. Model chỉ tải khi bạn xác nhận ở đây.</span></div>
                </Show>
                <div class="provider-config-grid">
                  <label class="provider-field"><span>Engine</span><select class="settings-select" value={settings()!.asr.engine} onChange={(event) => updateAsr({ engine: event.currentTarget.value as AppSettings['asr']['engine'] })}><option value="fasterWhisper">faster-whisper · nhận dạng nhanh</option><option value="whisperX">WhisperX · căn chỉnh timestamp</option></select></label>
                  <label class="provider-field"><span>Model ASR</span><select class="settings-select" value={settings()!.asr.model} onChange={(event) => { const model = event.currentTarget.value as AppSettings['asr']['model']; updateAsr({ model }); void refreshAsrEnvironment(model, settings()?.asr.language ?? 'auto'); }}><option value="tiny">tiny</option><option value="base">base</option><option value="small">small · khuyến nghị</option><option value="medium">medium</option><option value="large-v3">large-v3</option></select></label>
                  <label class="provider-field"><span>Thiết bị</span><select class="settings-select" value={settings()!.asr.device} onChange={(event) => updateAsr({ device: event.currentTarget.value as AppSettings['asr']['device'] })}><option value="auto">Auto</option><option value="cpu">CPU</option><option value="cuda" disabled={!asrEnvironment()?.cudaAvailable}>CUDA</option></select></label>
                  <label class="provider-field"><span>Ngôn ngữ</span><select class="settings-select" value={settings()!.asr.language} onChange={(event) => { const language = event.currentTarget.value as AppSettings['asr']['language']; updateAsr({ language }); void refreshAsrEnvironment(settings()?.asr.model ?? 'small', language); }}><option value="auto">Tự nhận diện</option><option value="en">English</option><option value="vi">Tiếng Việt</option><option value="ja">日本語</option><option value="ko">한국어</option><option value="zh">中文</option><option value="fr">Français</option><option value="de">Deutsch</option><option value="es">Español</option></select></label>
                </div>
                <Show when={settings()!.asr.engine === 'whisperX'}>
                  <div class="settings-info-note"><AudioLines size={15} /><span>WhisperX dùng chung model faster-whisper đã cài, sau đó dùng aligner theo ngôn ngữ để căn chỉnh timestamp. Cần chọn ngôn ngữ cụ thể; Auto không thể chọn aligner chính xác.</span></div>
                  <Show when={!asrEnvironment()?.whisperXInstalled && !asrEnvironmentLoading()}>
                    <div class="tts-install-instructions"><strong>Python Worker chưa có WhisperX</strong><span>WhisperX và PyTorch là tùy chọn dung lượng lớn, không tải ngầm. Development có thể cài bằng <code>scripts/setup-whisperx-dev.ps1</code>; sau đó bấm Kiểm tra lại. Worker release cần được build kèm WhisperX.</span></div>
                  </Show>
                  <Show when={asrEnvironment()?.whisperXInstalled && settings()!.asr.language === 'auto'}><p class="asr-config-warning" role="status">Chọn ngôn ngữ nguồn cụ thể để tải aligner WhisperX tương ứng.</p></Show>
                  <Show when={asrEnvironment()?.whisperXInstalled && settings()!.asr.language !== 'auto' && !asrEnvironment()?.alignmentModelCached}>
                    <p class="provider-help">Mô hình căn chỉnh được tải riêng một lần vào thư mục model dùng chung. Khi tạo transcript, ứng dụng chỉ nạp trọng số đã cài.</p>
                    <button class="button button-primary" type="button" disabled={busy() || installingWhisperXAlignment() || asrEnvironmentLoading()} onClick={() => void installWhisperXAlignment()}>{installingWhisperXAlignment() ? 'Đang cài aligner…' : `Cài WhisperX aligner · ${settings()!.asr.language}`}</button>
                  </Show>
                  <Show when={asrEnvironment()?.whisperXInstalled && asrEnvironment()?.alignmentModelCached}><span class="version-chip">WhisperX aligner · {settings()!.asr.language} · Đã cài</span></Show>
                </Show>
                <p class="provider-help">Video stereo có hai kênh khác nhau sẽ được nhận dạng riêng rồi gộp để giữ lời thoại chồng lấn. Aligner WhisperX bổ sung timestamp dựa trên lời đã nhận dạng; không khôi phục được tiếng bị che khi audio đã trộn mono.</p>
                <Show when={settings()!.asr.device === 'cuda' && asrEnvironment()?.workerInstalled && !asrEnvironment()?.cudaAvailable}><p class="asr-config-warning" role="alert">CUDA chưa khả dụng. Chọn Auto hoặc CPU để tiếp tục.</p></Show>
                <Show when={asrEnvironmentError()}><div class="provider-feedback provider-feedback-error" role="alert">{asrEnvironmentError()}</div></Show>
                <Show when={installingAsrModel() || installingWhisperXAlignment()}><div class="tts-install-progress" role="status"><div><span>{installingWhisperXAlignment() ? `Đang cài WhisperX aligner ${settings()!.asr.language}…` : `Đang cài model ${settings()!.asr.model}…`} Giữ ứng dụng mở cho đến khi hoàn tất.</span><span><span class="spin status-spinner" /></span></div><div class="job-progress-track"><span class="job-progress-indeterminate" /></div></div></Show>
                <div class="provider-action-row">
                  <button class="button button-quiet provider-action-secondary" type="button" disabled={busy() || installingAsrModel() || installingWhisperXAlignment()} onClick={() => void saveSettings()}>{busy() ? 'Đang lưu…' : 'Lưu cấu hình'}</button>
                  <button class="button button-quiet provider-action-secondary" type="button" disabled={asrEnvironmentLoading() || installingAsrModel() || installingWhisperXAlignment()} onClick={() => void refreshAsrEnvironment()}>{asrEnvironmentLoading() ? 'Đang kiểm tra…' : 'Kiểm tra lại'}</button>
                  <Show when={asrEnvironment()?.workerInstalled && !asrEnvironment()?.modelCached}>
                    <button class="button button-primary" type="button" disabled={busy() || installingAsrModel() || installingWhisperXAlignment() || asrEnvironmentLoading()} onClick={() => setShowAsrModelConfirm(true)}>{installingAsrModel() ? 'Đang cài model…' : `Cài model ${settings()!.asr.model}`}</button>
                  </Show>
                  <Show when={asrEnvironment()?.modelCached}><span class="version-chip">Đã cài · {settings()!.asr.model}</span><button class="button button-quiet" type="button" disabled={removingAsrModel() || installingAsrModel() || installingWhisperXAlignment()} onClick={() => setShowRemoveAsrModelConfirm(true)}>{removingAsrModel() ? 'Đang gỡ…' : 'Gỡ model'}</button></Show>
                </div>
              </div>
            </Show>
            <div class="settings-info-note"><Cpu size={15} /><span>Model faster-whisper và aligner WhisperX được lưu một lần trong thư mục model dùng chung. Nếu chỉ dùng faster-whisper, không cần cài PyTorch hoặc aligner.</span></div>
          </Show>

          <Show when={section() === 'Voice / TTS'}>
            <SettingsSectionHeader title="Voice / TTS" subtitle="VieNeu-TTS chạy cục bộ. Model chỉ được tải sau khi bạn xác nhận." />
            <Show when={settings()} fallback={<div class="settings-card provider-settings-card">Đang đọc cài đặt giọng đọc…</div>}>
              <div class="settings-card tts-settings-card">
                <div class="tts-model-status-row">
                  <div class="provider-card-icon"><Volume2 size={17} /></div>
                  <div class="provider-card-main"><span class="provider-eyebrow">LOCAL · VIETNAMESE</span><h3>VieNeu-TTS v3 Turbo</h3>
                    <p>{ttsEnvironmentLoading() ? 'Đang kiểm tra VieNeu SDK và model…' : !ttsEnvironment()?.workerInstalled ? 'VieNeu SDK chưa được cài trong Python runtime.' : ttsEnvironment()?.modelCached ? 'Model đã có trên thiết bị.' : 'Model chưa được tải về.'}</p>
                  </div>
                  <span class="provider-status-dot" classList={{ 'provider-status-dot-ready': !!ttsEnvironment()?.modelCached }} />
                </div>
                <Show when={!ttsEnvironmentLoading() && !ttsEnvironment()?.workerInstalled}>
                  <div class="tts-install-instructions"><strong>Thiếu VieNeu-TTS SDK</strong><span>{import.meta.env.DEV ? 'Bản development: chạy scripts/setup-vieneu-dev.ps1 trong thư mục dự án. Trọng số model được quản lý riêng và không cần tải lại nếu đã có.' : 'Runtime VieNeu không có trong bộ cài. Hãy dùng Repair hoặc cài lại ứng dụng; model được quản lý riêng.'}</span></div>
                </Show>
                <div class="provider-config-grid">
                  <label class="provider-field"><span>Provider</span><select class="settings-select" value={settings()!.tts.provider} disabled><option value="vieneu">VieNeu-TTS · chạy cục bộ</option></select></label>
                  <label class="provider-field"><span>Model</span><select class="settings-select" value={settings()!.tts.modelId} disabled><option value="v3-turbo">VieNeu-TTS v3 Turbo · 48 kHz</option></select></label>
                  <label class="provider-field"><span>Thiết bị</span><select class="settings-select" value={settings()!.tts.device} onChange={(event) => updateTts({ device: event.currentTarget.value as AppSettings['tts']['device'] })}><option value="auto">Auto</option><option value="cpu">CPU</option><option value="cuda" disabled={!ttsEnvironment()?.cudaAvailable}>CUDA</option></select></label>
                  <label class="provider-field"><span>Giọng mặc định</span><select class="settings-select" value={settings()!.tts.defaultVoice} onChange={(event) => updateTts({ defaultVoice: event.currentTarget.value })}><Show when={!ttsEnvironment()?.voices.includes(settings()!.tts.defaultVoice)}><option value={settings()!.tts.defaultVoice}>{settings()!.tts.defaultVoice}</option></Show><For each={ttsEnvironment()?.voices ?? []}>{(voice) => <option value={voice}>{voice}</option>}</For><For each={settings()!.tts.voiceProfiles}>{(profile) => <option value={profile.name}>{profile.name} · giọng tham chiếu</option>}</For></select></label>
                  <label class="provider-field"><span>Tốc độ <small>{settings()!.tts.speed.toFixed(2)}×</small></span><input type="range" min="0.85" max="2" step="0.01" value={settings()!.tts.speed} onInput={(event) => updateTts({ speed: Number(event.currentTarget.value) })} /></label>
                </div>
                <label class="property-control-row tts-fit-row"><span>Fit theo thời lượng segment <small>Giới hạn tăng tốc tối đa 2.00×</small></span><input type="checkbox" checked={settings()!.tts.fitToDuration} onChange={(event) => updateTts({ fitToDuration: event.currentTarget.checked })} /></label>
                <div class="tts-profiles-section"><div class="tts-profiles-heading"><strong>Voice profiles</strong><button class="button button-quiet" type="button" onClick={() => setShowVoiceProfileDialog(true)}>Thêm giọng tham chiếu</button></div>
                  <For each={settings()!.tts.voiceProfiles}>{(profile) => <div class="tts-profile-row"><span><strong>{profile.name}</strong><small>Audio tham chiếu đã lưu cục bộ</small></span><button class="mini-tool-button" type="button" disabled={voiceProfileBusy()} onClick={() => void removeVoiceProfile(profile.id)}>Xóa</button></div>}</For>
                  <Show when={!settings()!.tts.voiceProfiles.length}><small class="tts-profile-empty">Có thể dùng preset VieNeu hoặc thêm audio tham chiếu 3–8 giây. Cần quyền sử dụng và sự đồng ý phù hợp.</small></Show>
                </div>
                <Show when={ttsEnvironment()?.workerInstalled && !ttsEnvironment()?.modelCached}>
                  <button class="button button-primary tts-install-button" type="button" disabled={installingTtsModel()} onClick={() => setShowTtsModelConfirm(true)}>{installingTtsModel() ? 'Đang tải model…' : 'Tải model VieNeu-TTS'}</button>
                </Show>
                <Show when={ttsEnvironment()?.modelCached}><button class="button button-quiet tts-install-button" type="button" disabled={installingTtsModel() || removingTtsModel()} onClick={() => setShowRemoveTtsModelConfirm(true)}>{removingTtsModel() ? 'Đang gỡ model…' : 'Gỡ model VieNeu-TTS'}</button></Show>
                <Show when={installingTtsModel()}><div class="tts-install-progress" role="status"><div><span>{ttsModelMessage()}</span><span>{Math.round(ttsModelProgress() * 100)}%</span></div><div class="job-progress-track"><span style={{ width: `${ttsModelProgress() * 100}%` }} /></div></div></Show>
                <Show when={ttsEnvironmentError()}><div class="provider-feedback provider-feedback-error" role="alert">{ttsEnvironmentError()}</div></Show>
                <div class="provider-action-row">
                  <button class="button button-quiet provider-action-secondary" type="button" disabled={ttsEnvironmentLoading()} onClick={() => void refreshTtsEnvironment()}>{ttsEnvironmentLoading() ? 'Đang kiểm tra…' : 'Kiểm tra lại SDK'}</button>
                  <button class="button button-quiet provider-action-secondary" type="button" disabled={previewBusy() || !ttsEnvironment()?.modelCached} onClick={() => void previewTtsVoice()}>{previewBusy() ? 'Đang tạo giọng thử…' : 'Nghe thử giọng'}</button>
                  <button class="button button-primary" type="button" disabled={busy()} onClick={() => void saveSettings()}>{busy() ? 'Đang lưu…' : 'Lưu cấu hình TTS'}</button>
                </div>
                <Show when={voicePreviewUrl()}><audio class="tts-preview-audio" controls src={voicePreviewUrl()} aria-label="Nghe thử giọng VieNeu" /></Show>
                <Show when={settingsMessage()}><div class="provider-feedback" classList={{ 'provider-feedback-error': settingsError() }} role={settingsError() ? 'alert' : 'status'}>{settingsMessage()}</div></Show>
                <Show when={ttsEnvironment()?.workerInstalled && ttsEnvironment()?.modelCached && settings()!.tts.device === 'cuda' && !ttsEnvironment()?.cudaAvailable}><p class="asr-config-warning" role="alert">CUDA chưa khả dụng. Auto sẽ dùng CPU; cài PyTorch tương thích CUDA theo hướng dẫn VieNeu nếu cần GPU.</p></Show>
              </div>
            </Show>
          </Show>

          <Show when={section() === 'Advanced'}>
            <SettingsSectionHeader title="Advanced" subtitle="Tùy chọn chẩn đoán và cấu hình nâng cao." />
            <div class="runtime-health-card"><div class="runtime-health-header"><div><span class="provider-eyebrow">RUNTIME HEALTH</span><h3>Thành phần ứng dụng</h3></div><button class="button button-quiet" type="button" onClick={() => void refreshRuntimeHealth()}>Kiểm tra lại</button></div>
              <div class="runtime-health-grid"><RuntimeStatus name="Project storage" ready={!!runtimeHealth()?.projectStorageAvailable} loading={!runtimeHealth()} /><RuntimeStatus name="FFmpeg" ready={!!runtimeHealth()?.ffmpegAvailable} detail={runtimeHealth()?.ffmpegVersion ?? undefined} loading={!runtimeHealth()} /><RuntimeStatus name="FFprobe" ready={!!runtimeHealth()?.ffprobeAvailable} loading={!runtimeHealth()} /><RuntimeStatus name="AI Worker" ready={!!runtimeHealth()?.workerAvailable} detail={runtimeHealth()?.workerMode} loading={!runtimeHealth()} /></div>
              <Show when={runtimeHealthError()}><div class="provider-feedback provider-feedback-error" role="alert">{runtimeHealthError()}</div></Show>
              <div class="runtime-diagnostics-row"><p>Chẩn đoán chỉ gồm thông tin runtime đã lọc và tối đa 5 log gần nhất. Không gồm video, transcript, API key hoặc credential.</p><button class="button button-primary" type="button" disabled={diagnosticsBusy()} onClick={() => void exportDiagnostics()}>{diagnosticsBusy() ? 'Đang tạo…' : 'Xuất diagnostics'}</button></div>
            </div>
            <div class="settings-card">
              <SettingRow icon={<Cpu size={17} />} title="Giới hạn tài nguyên" description="Mặc định an toàn: một tác vụ model GPU, một tác vụ CPU nặng và một FFmpeg export cùng lúc."><span class="resource-default-badge">GPU 1 · CPU 1 · FFmpeg 1</span></SettingRow>
              <div class="settings-row-divider" />
              <SettingRow icon={<SlidersHorizontal size={17} />} title="Cache và file tạm" description={storageUsage()?.cacheDirectory ?? 'Đường dẫn cache cục bộ theo hệ điều hành.'}>
                <button class="button button-quiet storage-open-button" type="button" onClick={() => void openStorageFolder(() => storageService.openCacheFolder())}>Mở thư mục</button>
              </SettingRow>
            </div>
            <div class="storage-overview-card">
              <div class="storage-overview-header"><div><span class="provider-eyebrow">LOCAL STORAGE</span><h3>Dung lượng sử dụng</h3></div><button class="button button-quiet" type="button" disabled={storageBusy()} onClick={() => void refreshStorage()}>{storageBusy() ? 'Đang đọc…' : 'Làm mới'}</button></div>
              <div class="storage-stat-grid">
                <div class="storage-model-stat">
                  <small>Models · tổng</small><strong>{storageUsage() ? formatBytes(storageUsage()!.modelsBytes) : '—'}</strong>
                  <div class="storage-model-breakdown">
                    <div class="storage-model-location"><span><b>faster-whisper</b><small title={storageUsage()?.fasterWhisperDirectory}>{storageUsage()?.fasterWhisperDirectory ?? 'Đường dẫn model Speech Recognition'}</small></span><strong>{storageUsage() ? formatBytes(storageUsage()!.fasterWhisperBytes) : '—'}</strong><button class="storage-model-open" type="button" aria-label="Mở thư mục faster-whisper" title="Mở thư mục faster-whisper" onClick={() => void openStorageFolder(() => storageService.openModelFolder('fasterWhisper'))}><FolderOpen size={13} /></button></div>
                    <div class="storage-model-location"><span><b>VieNeu-TTS</b><small title={storageUsage()?.vieNeuTtsDirectory}>{storageUsage()?.vieNeuTtsDirectory ?? 'Đường dẫn model Voice / TTS'}</small></span><strong>{storageUsage() ? formatBytes(storageUsage()!.vieNeuTtsBytes) : '—'}</strong><button class="storage-model-open" type="button" aria-label="Mở thư mục VieNeu-TTS" title="Mở thư mục VieNeu-TTS" onClick={() => void openStorageFolder(() => storageService.openModelFolder('vieNeuTts'))}><FolderOpen size={13} /></button></div>
                    <Show when={(storageUsage()?.otherModelDataBytes ?? 0) > 0}><span class="storage-model-other">Khác · profile giọng hoặc model cũ: {formatBytes(storageUsage()!.otherModelDataBytes)}</span></Show>
                  </div>
                </div>
                <div><small>Projects</small><strong>{storageUsage() ? formatBytes(storageUsage()!.projectsBytes) : '—'}</strong><span>Video, transcript, audio, export</span></div>
                <div><small>Project cache</small><strong>{storageUsage() ? formatBytes(storageUsage()!.projectCacheBytes) : '—'}</strong><span>Render temp được tính riêng</span></div>
                <div><small>Temporary</small><strong>{storageUsage() ? formatBytes(storageUsage()!.temporaryBytes) : '—'}</strong><span>Render và file tải dở</span></div>
                <div><small>Logs</small><strong>{storageUsage() ? formatBytes(storageUsage()!.logsBytes) : '—'}</strong><span>Luân phiên tối đa 10 × 10 MB</span></div>
              </div>
              <Show when={(storageUsage()?.warnings.length ?? 0) > 0}><div class="provider-feedback" role="status"><For each={storageUsage()?.warnings}>{(warning) => <div>{warning}</div>}</For></div></Show>
              <div class="storage-cleanup-row"><p>Chỉ xóa render tạm và file tạm đã tải. Video nguồn, transcript, bản dịch, audio TTS, export và model được giữ nguyên.</p><button class="button button-quiet" type="button" disabled={storageBusy() || !storageUsage()?.temporaryBytes} onClick={() => { setStorageBusy(true); void storageService.clearTemporary().then((bytes) => { showMessage(`Đã dọn ${formatBytes(bytes)} file tạm.`, false); return refreshStorage(); }).catch((error) => setStorageError(error instanceof Error ? error.message : String(error))).finally(() => setStorageBusy(false)); }}>Dọn file tạm</button></div>
              <Show when={storageError()}><div class="provider-feedback provider-feedback-error" role="alert">{storageError()}</div></Show>
            </div>
            <div class="settings-info-note"><CircleHelp size={15} /><span>GPU jobs và FFmpeg export được giới hạn ở một tiến trình mỗi loại để tránh tranh chấp VRAM và ổ đĩa. Job history giữ tối đa 200 tác vụ đã dừng.</span></div>
          </Show>

          <Show when={section() === 'About'}>
            <SettingsSectionHeader title="About" subtitle="Thông tin phiên bản, runtime và giấy phép thành phần." />
            <div class="settings-card about-card">
              <SettingRow icon={<Sparkles size={17} />} title="JC AI" description="Windows desktop application."><span class="version-chip">{runtimeHealth()?.appVersion ?? '…'}</span></SettingRow>
              <div class="settings-row-divider" />
              <SettingRow icon={<SlidersHorizontal size={17} />} title="Build" description="Phiên bản build."><span class="version-chip">{runtimeHealth()?.buildCommit ?? 'unknown'} · {runtimeHealth()?.os ?? '—'} {runtimeHealth()?.architecture ?? ''}</span></SettingRow>
              <div class="settings-row-divider" />
              <SettingRow icon={<Cpu size={17} />} title="FFmpeg" description="Dùng để phân tích media, preview proxy và export."><span class="version-chip">{runtimeHealth()?.ffmpegVersion ?? 'Không khả dụng'}</span></SettingRow>
              <div class="settings-row-divider" />
              <SettingRow icon={<AudioLines size={17} />} title="AI Worker" description="Runtime đóng gói độc lập; không cần cài Python."><span class="version-chip">{runtimeHealth()?.workerVersion ?? runtimeHealth()?.workerMode ?? '—'}</span></SettingRow>
            </div>
            <div class="settings-info-note"><Info size={15} /><span>Giấy phép third-party được cài cùng ứng dụng trong thư mục resources/THIRD_PARTY_LICENSES.txt. Model weights tải theo lựa chọn có giấy phép riêng tại nguồn cung cấp.</span></div>
          </Show>
        </section>
      </div>
      <Show when={showAsrModelConfirm()}>
        <Modal title={`Cài model faster-whisper ${settings()?.asr.model ?? ''}?`} description="Model được cài bền vững trong thư mục faster-whisper dùng chung cho các project. Cài đặt bắt đầu sau khi bạn xác nhận." onClose={() => setShowAsrModelConfirm(false)}>
          <div class="model-confirm-card"><div class="job-banner-icon"><Cpu size={17} /></div><div><strong>{settings()?.asr.model} · khoảng {estimatedModelSize(settings()?.asr.model ?? 'small')}</strong><p>Model được tải một lần từ Hugging Face và giữ cục bộ. Cần kết nối Internet và đủ dung lượng trống. Tác vụ Generate Transcript không tải model.</p></div></div>
          <footer class="modal-actions"><button class="button button-quiet" type="button" onClick={() => setShowAsrModelConfirm(false)}>Hủy</button><button class="button button-primary" type="button" onClick={() => void installAsrModel()}>Tải model</button></footer>
        </Modal>
      </Show>
      <Show when={showRemoveAsrModelConfirm()}>
        <Modal title={`Gỡ model faster-whisper ${settings()?.asr.model ?? ''}?`} description="Chỉ xóa model faster-whisper đã cài. Transcript và dữ liệu dự án không bị xóa." onClose={() => setShowRemoveAsrModelConfirm(false)}>
          <footer class="modal-actions"><button class="button button-quiet" type="button" onClick={() => setShowRemoveAsrModelConfirm(false)}>Hủy</button><button class="button button-primary" type="button" onClick={() => void removeAsrModel()}>Gỡ model</button></footer>
        </Modal>
      </Show>
      <Show when={showTtsModelConfirm()}>
        <Modal title="Tải model VieNeu-TTS?" description="Model và bộ giải mã được lưu bền vững trong thư mục VieNeu-TTS dùng chung cho các project. Tải chỉ bắt đầu sau khi bạn xác nhận." onClose={() => setShowTtsModelConfirm(false)}>
          <div class="model-confirm-card"><div class="job-banner-icon"><Cpu size={17} /></div><div><strong>VieNeu-TTS v3 Turbo · khoảng 1.8 GB</strong><p>Model và bộ giải mã tải từ Hugging Face. Dung lượng thay đổi theo phiên bản và file đi kèm. Cần kết nối Internet và đủ dung lượng trống.</p></div></div>
          <footer class="modal-actions"><button class="button button-quiet" type="button" onClick={() => setShowTtsModelConfirm(false)}>Hủy</button><button class="button button-primary" type="button" onClick={() => void installTtsModel()}>Tải model</button></footer>
        </Modal>
      </Show>
      <Show when={showRemoveTtsModelConfirm()}>
        <Modal title="Gỡ model VieNeu-TTS?" description="Chỉ xóa trọng số VieNeu và ONNX codec đã cài. Project, transcript và các file audio TTS hiện có được giữ nguyên." onClose={() => setShowRemoveTtsModelConfirm(false)}>
          <footer class="modal-actions"><button class="button button-quiet" type="button" onClick={() => setShowRemoveTtsModelConfirm(false)}>Hủy</button><button class="button button-primary" type="button" onClick={() => void removeTtsModel()}>Gỡ model</button></footer>
        </Modal>
      </Show>
      <Show when={showVoiceProfileDialog()}>
        <Modal title="Thêm voice profile" description="Chọn audio tham chiếu giọng dài 3–8 giây. Ứng dụng chuẩn hóa và lưu bản sao cục bộ; file gốc không bị thay đổi." onClose={() => setShowVoiceProfileDialog(false)}>
          <div class="voice-profile-form">
            <label class="provider-field"><span>Tên profile</span><input value={voiceProfileName()} maxLength={48} onInput={(event) => setVoiceProfileName(event.currentTarget.value)} placeholder="Ví dụ: Giọng dẫn chuyện" /></label>
            <div class="voice-profile-file-row"><span>{voiceProfileFile() ? voiceProfileFile().split(/[\\/]/).pop() : 'Chưa chọn audio tham chiếu'}</span><button class="button button-quiet" type="button" onClick={() => void browseVoiceReference()}>Chọn audio</button></div>
            <label class="voice-profile-consent"><input type="checkbox" checked={voiceProfileConsent()} onChange={(event) => setVoiceProfileConsent(event.currentTarget.checked)} /><span>Tôi có quyền sử dụng audio này và đã có sự đồng ý cần thiết từ người có giọng nói.</span></label>
            <p class="provider-help">FFprobe xác nhận stream và thời lượng; FFmpeg lưu bản WAV mono trong thư mục dữ liệu ứng dụng. Profile không được ghi vào project.</p>
          </div>
          <footer class="modal-actions"><button class="button button-quiet" type="button" onClick={() => setShowVoiceProfileDialog(false)}>Hủy</button><button class="button button-primary" type="button" disabled={!voiceProfileName().trim() || !voiceProfileFile() || !voiceProfileConsent() || voiceProfileBusy()} onClick={() => void addVoiceProfile()}>{voiceProfileBusy() ? 'Đang kiểm tra audio…' : 'Thêm voice profile'}</button></footer>
        </Modal>
      </Show>
    </main>
  );
}

function SettingsSectionHeader(props: { title: string; subtitle: string }) {
  return <header class="settings-detail-header"><div class="settings-detail-kicker">APPLICATION SETTINGS <span>/</span> {props.title.toLocaleUpperCase()}</div><h2>{props.title}</h2><p>{props.subtitle}</p></header>;
}

function SettingRow(props: { icon: import('solid-js').JSX.Element; title: string; description: string; children: import('solid-js').JSX.Element }) {
  return <div class="settings-row"><span class="settings-row-icon">{props.icon}</span><div class="settings-row-copy"><strong>{props.title}</strong><span>{props.description}</span></div><div class="settings-row-value">{props.children}</div></div>;
}

function RuntimeStatus(props: { name: string; ready: boolean; detail?: string; loading?: boolean }) {
  return <div class="runtime-status"><span class="runtime-status-dot" classList={{ ready: props.ready, loading: props.loading }} /><div><strong>{props.name}</strong><small>{props.loading ? 'Đang kiểm tra…' : props.ready ? props.detail || 'Sẵn sàng' : 'Không khả dụng'}</small></div></div>;
}
